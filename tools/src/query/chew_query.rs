use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    iter::FromIterator,
};

use query_parser::{parse, TermValue};
use serde::{Deserialize, Serialize};
use toml::value::Table;

use crate::{
    abstract_server::{ErrorDetails, ErrorLayer, Result, ServerError},
    cmd_pipeline::transforms::path_glob_transform,
};

/*
  Queries are translated into pipelines in the following steps:

  1. We parse the query string with the `query-parser` crate which provides us
     with a list of terms and values from `term:value` with a special case for
     bare values where there is no term.
  2. We look up each term (including "default" for bare values) which contain
     some combination of:
     - Term aliases: We just re-process the term as if the aliased term had been
       used.  This is intended for short-hands like "C" for "context" where we
       want our UI to act like "context" had been used when "C" is observed so
       that we can explain to the user what is going on without being cryptic.
     - Term expansions: We re-process the term as one or more other terms,
       potentially transforming the value associated with the term.  Allowing
       expansion to multiple terms lets us have a single query run against
       multiple data sources.  For example, our default term expands to
       "file" for filename/path search, "idprefix" for identifier lookup by
       prefix, and "text" for full-text search.  These will result in parallel
       execution pipelines which are stitched back together later via the
       "group" and "junction" config dictionaries.
     - Pipeline command invocations placed in a specific group.  Command
       invocations will create a command with the given name if it does not
       already exist, or reuse an existing command if one already exists
       (searching from the most recently added command).  Arguments are then
       contributed to the command.  This allows terms to add additional
       constraints or settings to a single pipeline command.
     - Maybe in the future: The ability to set some kind of global variable so
       that a single term can influence multiple pipeline commands that may or
       may not exist (and without bringing them into existence)?
  3. After the terms have produced the starting groups, we consult the "group"
     and "junction" nodes that have not yet been processed.  For each group /
     junction, we look up its config settings and:
     - For each group, we set its "output" and if there is a "next" group or a
       "junction" that should process its output, we create the group /
       junction if it does not exist and add/set the group's "output" as the/a
       input to the group/junction.  We populate the new group with a "command"
       and any "args" if they were provided.  We add the group/junction to the
       to-do list.
    - For each junction we similarly look at its "output" and any "next" group.
*/

#[derive(Deserialize)]
pub struct QueryConfig {
    pub term: BTreeMap<String, TermConfig>,
    pub group: BTreeMap<String, GroupConfig>,
    pub junction: BTreeMap<String, JunctionConfig>,
}

#[derive(Deserialize)]
pub struct TermConfig {
    pub alias: Option<String>,
    #[serde(default)]
    pub conflicts: Vec<String>,
    #[serde(default)]
    pub expand: Vec<TermExpansion>,
    #[serde(default)]
    pub group: BTreeMap<String, Vec<PipelineUse>>,
}

#[derive(Deserialize)]
pub struct TermExpansion {
    pub term: String,
    #[serde(default)]
    pub transforms: Vec<String>,
}

#[derive(Deserialize)]
pub struct PipelineUse {
    pub command: String,
    #[serde(default)]
    pub priority: u32,
    #[serde(default)]
    pub args: Table,
}

#[derive(Deserialize)]
pub struct GroupConfig {
    pub output: String,
    #[serde(default)]
    pub commands: Vec<PipelineUse>,
    pub junction: Option<String>,
    pub next: Option<String>,
}

#[derive(Deserialize)]
pub struct JunctionConfig {
    pub command: String,
    #[serde(default)]
    pub args: Table,
    pub output: String,
    pub next: Option<String>,
}

lazy_static! {
    static ref QUERY_CORE: QueryConfig = toml::from_str(include_str!("query_core.toml")).unwrap();
}

#[derive(Default, Serialize)]
pub struct PipelinePhase {
    pub groups: Vec<Vec<String>>,
    pub junctions: Vec<String>,
}

#[derive(Default, Serialize)]
pub struct QueryPipelineGroupBuilder {
    pub groups: BTreeMap<String, PipelineGroup>,
    pub junctions: BTreeMap<String, JunctionNode>,
    pub phases: Vec<PipelinePhase>,
}

fn apply_transforms(user_val: String, transforms: &Vec<String>) -> String {
    let mut val = user_val;
    for transform in transforms.iter() {
        val = match transform.as_str() {
            "regexp_escape" => regex::escape(&val),
            "path_glob" => path_glob_transform(&val),
            _ => val,
        }
    }
    val
}

fn flatten_args(user_val: &str, priority: u32, args: &Table) -> PipelineArgs {
    let mut flattened = PipelineArgs::default();
    for (key, arg_val) in args.iter() {
        if key.as_str() == "positional" {
            if let Some(arg_str) = arg_val.as_str() {
                flattened
                    .positional_args
                    .push(arg_str.replace("$0", user_val));
            }
        } else if let Some(arg_bool) = arg_val.as_bool() {
            // boolean command-line args should be omitted if false
            if arg_bool {
                flattened.bool_args.insert(key.clone());
            }
        } else if let Some(arg_str) = arg_val.as_str() {
            let replaced_arg = arg_str.replace("$0", user_val);
            flattened
                .named_args
                .insert(key.clone(), (replaced_arg, priority));
        }
    }
    flattened
}

impl QueryPipelineGroupBuilder {
    fn ensure_pipeline_step(&mut self, group_name: String, command: String, args: PipelineArgs) {
        let group = self
            .groups
            .entry(group_name)
            .or_insert_with(|| PipelineGroup::default());

        group.ensure_pipeline_step(command, args);
    }

    pub fn ingest_term(&mut self, root_term: &str, value: &str) -> Result<()> {
        let mut terms_to_process: VecDeque<(String, String)> = VecDeque::new();
        terms_to_process.push_back((root_term.to_string(), value.to_string()));

        let mut terms_processed = vec![];
        while let Some((term_str, term_value)) = terms_to_process.pop_front() {
            if let Some(term) = QUERY_CORE.term.get(&term_str) {
                if let Some(alias) = &term.alias {
                    terms_to_process.push_back((alias.clone(), term_value.clone()));
                }

                for conflict in term.conflicts.iter() {
                    if terms_processed.iter().any(|x| x == conflict) {
                        return Err(ServerError::StickyProblem(ErrorDetails {
                            layer: ErrorLayer::BadInput,
                            message: format!("{} conflicts with {}", term_str, conflict),
                        }));
                    }
                }

                for expand in term.expand.iter() {
                    terms_to_process.push_back((
                        expand.term.clone(),
                        apply_transforms(term_value.clone(), &expand.transforms),
                    ));
                }

                for (group_name, pipeline_uses) in term.group.iter() {
                    for pipe_use in pipeline_uses.iter() {
                        let flattened_args = flatten_args(&term_value, 0, &pipe_use.args);
                        self.ensure_pipeline_step(
                            group_name.clone(),
                            pipe_use.command.clone(),
                            flattened_args,
                        );
                    }
                }
            }

            terms_processed.push(term_str);
        }

        Ok(())
    }
}

#[derive(Default, Serialize)]
pub struct PipelineGroup {
    pub input: Option<String>,
    pub segments: Vec<PipelineSegment>,
    pub output: Option<String>,
    pub depth: u32,
}

impl PipelineGroup {
    fn ensure_pipeline_step(&mut self, command: String, args: PipelineArgs) {
        match self.segments.iter_mut().rfind(|seg| seg.command == command) {
            Some(seg) => {
                seg.args.merge(args);
            }
            None => {
                self.segments.push(PipelineSegment { command, args });
            }
        }
    }
}

#[derive(Default, Serialize)]
pub struct JunctionNode {
    pub inputs: Vec<String>,
    pub command: PipelineSegment,
    pub output: Option<String>,
    pub depth: u32,
}

#[derive(Default, Serialize)]
pub struct PipelineArgs {
    pub bool_args: BTreeSet<String>,
    // Only the named args need a priority for deciding when to clobber.
    pub named_args: BTreeMap<String, (String, u32)>,
    pub positional_args: Vec<String>,
}

impl PipelineArgs {
    pub fn merge(&mut self, mut other: Self) {
        self.bool_args.append(&mut other.bool_args);
        for (key, (oth_val, oth_pri)) in other.named_args {
            if let Some(ptr) = self.named_args.get_mut(&key) {
                // Only clobber our current value if the new value has a higher priority.
                if oth_pri > ptr.1 {
                    *ptr = (oth_val, oth_pri);
                }
        } else {
                self.named_args.insert(key, (oth_val, oth_pri));
            }
        }
        self.positional_args.append(&mut other.positional_args);
    }

    // ## Escaping
    //
    // We don't need to deal with shell escaping, but we do need to deal with
    // our string-based interaction with clap meaning that clap can't magically
    // distinguish between us indicating an argument by passing a string
    // prefixed with double-dashed and a value that we want to start with
    // double-dashes.  However, we are able to deal with this by making sure
    // that:
    //
    // * Named args use the `--arg=value` syntax since `--arg=--value` is
    //   unambiguous.
    // * Positional args only come after the magic `--` delimiter.
    pub fn to_vec(&self) -> Vec<String> {
        let mut args = vec![];
        for arg in &self.bool_args {
            args.push(format!("--{}", arg));
        }
        for (key, (val, _pri)) in &self.named_args {
            args.push(format!("--{}={}", key, val));
        }
        if self.positional_args.len() > 0 {
            args.push("--".to_string());
            for arg in &self.positional_args {
                args.push(arg.clone());
            }
        }
        args
    }
}

#[derive(Default, Serialize)]
pub struct PipelineSegment {
    pub command: String,
    pub args: PipelineArgs,
}

pub fn chew_query(full_arg_str: &str) -> Result<QueryPipelineGroupBuilder> {
    let mut builder = QueryPipelineGroupBuilder::default();
    // ## 1: Parse the Query
    let q = parse(full_arg_str);

    // ## 2: Ingest / process the terms
    for term in q.terms {
        match term.value {
            TermValue::Simple(value) => {
                if let Some(key) = term.key {
                    builder.ingest_term(&key, &value)?;
                } else {
                    builder.ingest_term("default", &value)?;
                }
            }
        }
    }

    // ## 3: Process group rules to build the graph suggested by the terms above
    let mut unprocessed_groups: VecDeque<String> = builder.groups.keys().cloned().collect();
    let mut unprocessed_junctions: VecDeque<String> = VecDeque::new();
    // The set of groups without an input which means they should go in the
    // first `ParallelPipelines` instance.  We remove groups from this set as we
    // determine that they are actually depdendent on some earlier pipeline.
    let mut root_groups = BTreeSet::from_iter(unprocessed_groups.iter().cloned());
    // For phase 4 it's useful for us to be able to map an input to the set of
    // groups that consume it.  This is important because the group/junction
    // names effectively exist in a separate namespace from the inputs/outputs
    // where the name of a group will usually be the name of its output.  This
    // probably isn't strictly necessary if we did more in this pass, but this
    // is all fairly complicated and I do hope having the in-between
    // representations is helpful for the explanations we generate, etc.
    let mut inputs_to_names = HashMap::new();

    while !unprocessed_groups.is_empty() || !unprocessed_junctions.is_empty() {
        let mut next_group: Option<String> = None;
        let mut next_junction: Option<String> = None;
        let mut use_input: Option<String> = None;

        // We process groups first because junctions must have inputs and they
        // should probably already exist as groups, so our life is probably
        // easier if we process them first.
        if let Some(group_name) = unprocessed_groups.pop_front() {
            if let (Some(group_config), Some(group)) = (
                QUERY_CORE.group.get(&group_name),
                builder.groups.get_mut(&group_name),
            ) {
                group.output = Some(group_config.output.clone());
                use_input = group.output.clone();
                if let Some(next_group_name) = &group_config.next {
                    next_group = Some(next_group_name.clone());
                } else if let Some(next_junction_name) = &group_config.junction {
                    next_junction = Some(next_junction_name.clone());
                }
            }
        } else if let Some(junction_name) = unprocessed_junctions.pop_front() {
            if let (Some(junction_config), Some(junction)) = (
                QUERY_CORE.junction.get(&junction_name),
                builder.junctions.get_mut(&junction_name),
            ) {
                junction.output = Some(junction_config.output.clone());
                use_input = junction.output.clone();
                if let Some(next_group_name) = &junction_config.next {
                    next_group = Some(next_group_name.clone());
                }
            }
        }

        // Make the requested thing.
        if let Some(group_name) = next_group {
            if let (Some(group_config), group) = (
                QUERY_CORE.group.get(&group_name),
                builder
                    .groups
                    .entry(group_name.clone())
                    .or_insert_with(|| PipelineGroup::default()),
            ) {
                group.input = use_input;
                if let Some(input_name) = &group.input {
                    root_groups.remove(&group_name);
                    inputs_to_names
                        .entry(input_name.clone())
                        .or_insert_with(|| vec![])
                        .push(group_name.clone());
                }
                // group.output will be set and next/junction will be processed
                // in the 1st phase of the loop above; we're just ensuring the
                // group exists, establishing the "input" link, and adding any
                // commands/args listed.
                unprocessed_groups.push_back(group_name);

                for cmd in &group_config.commands {
                    let flattened_args = flatten_args("", cmd.priority, &cmd.args);
                    group.ensure_pipeline_step(cmd.command.clone(), flattened_args);
                }
            }
        } else if let Some(junction_name) = next_junction {
            if let (Some(junction_config), junction, Some(input)) = (
                QUERY_CORE.junction.get(&junction_name),
                builder
                    .junctions
                    .entry(junction_name.clone())
                    .or_insert_with(|| JunctionNode::default()),
                use_input,
            ) {
                inputs_to_names
                    .entry(input.clone())
                    .or_insert_with(|| vec![])
                    .push(junction_name.clone());
                junction.inputs.push(input);

                // junction.output will be set and next will be processed in the
                // 1st phase of the loop above; we're just ensuring the junction
                // exists and adding the "input" to the list.
                //
                // Logic to run only the first time we're processing the
                // junction:
                if junction.command.command.is_empty() {
                    junction.command.command = junction_config.command.clone();
                    junction.command.args = flatten_args("", 0, &junction_config.args);
                    unprocessed_junctions.push_back(junction_name);
                }
            }
        }
    }

    // ## 4: Walk the graph to build parallel pipelines
    //
    // We want to accumulate linear chains of groups until they hit a junction
    // or terminate with a "result" output.  Once we've hit all junctions, then
    // we want to flush those chains and the junctions they hit as a phase.
    // Then we want to restart the cycle with the outputs of the junctions until
    // we are left with a single "result" output.
    //
    // The primary interesting case for is a scenario like the following:
    //   g-a1 - g-a4 -\
    //   g-a2 - g-a5 --- j1 ----- j2
    //   g-a3 - g-a6 -/        /
    //                        /
    //   g-b1 - g-b2 - g-b3 -/
    //
    // The specific characteristic of note here is that we have a chain of
    // groups that reaches a junction that itself depends on the output of
    // another junction; whether there are groups in between or not doesn't
    // entirely matter but both cases should work.
    //
    // Our execution semantics dictate that all junctions in a phase run in
    // parallel, which means the data-dependency in j2 means that j2 must be in
    // a second phase and j1 in the first.  We can schedule the g-bN nodes in
    // either phase and have things be valid, but we do prefer to do all work as
    // early as possible.  (Note that this does force j1 to wait for g-bN,
    // currently, but this is also just a hypothetical scenario and our goal is
    // to just have a high probability of things working at this point.)
    //
    // Our algorithm is then to maintain 2 key state structures beyond our
    // `cur_phase` that we build incrementally for the current phase:
    // 1. `next_groups`: the set of groups we know we need to investigate next
    //    for this phase, initialized with the content of `root_groups`.  This
    //    is expressed as VecDeque of tuples of (group name, index in the
    //    PipelinePhase::groups array to place this group in).
    // 2. `pending_junctions`: a map from the junction names that groups have
    //    arrived at so far to the number of other groups we are waiting to
    //    arrive at this node.  The value is initialized to the length of the
    //    `JunctionInvocation::input_names` and decremented for each group that
    //    arrives at the junction.
    //
    // Starting from the initial `next_groups` population of `root_groups`, we
    // iteratively consume that deque, looking up the names to find out if they
    // are groups or junctions (which live in the same namespace).  If it's a
    // group, we add the current group to the appropriate
    // `PipelinePhase::groups` vec slot and push the output's name onto
    // `next_groups` including that vec slot.  If it was a junction, we
    // ensure there's an entry in `pending_junctions` for the junction and
    // decrement its waiting count for the current group.  If the the junction's
    // waiting count reaches 0, we push it onto the `PipelinePhase::junctions`
    // vec.  We continue this process until we run out of `next_groups`.
    //
    // Once we have no more `next_groups`, we traverse the list of junctions in
    // the current phase, use those to populate `next_groups`.  Note that
    // "result" is a magic group name for the terminal node (which will also
    // have impacted the logic above) and which will not go into `next_groups`.
    // We then flush the current phase.  If there are `next_groups`, we repeat
    // the loop with a new phase, otherwise we're done.

    let mut next_groups: VecDeque<(String, Option<usize>)> =
        root_groups.into_iter().map(|x| (x, None)).collect();
    let mut pending_junctions = BTreeMap::new();
    let mut seen = HashSet::new();

    // Control flow structure:
    //
    // Each pass through the outer loop creates a new PipelinePhase and pushes
    // it into the list of phases.  Each inner loop fully processes the set of
    // `next_groups` which accumulate into the current phase, and when the inner
    // loop completes it looks for any groups that the junctions in that phase
    // produces.
    while next_groups.len() > 0 {
        let mut cur_phase = PipelinePhase::default();
        while let Some((thing_name, group_slot)) = next_groups.pop_front() {
            if let Some(group) = builder.groups.get(&thing_name) {
                // Check if we've processed this group before.  We can't do this
                // suppression check on adding things to `next_groups` because
                // we could be adding a junction, which absolutely can be
                // arrived at multiple times.
                if !seen.insert(thing_name.clone()) {
                    return Err(ServerError::StickyProblem(ErrorDetails {
                        layer: ErrorLayer::ConfigLayer,
                        message: format!("pipeline loop: group {} used multiple times", thing_name),
                    }));
                }

                // This is a group, put it in the current phase.
                let next_group_slot = match group_slot {
                    Some(slot) => {
                        cur_phase.groups[slot].push(thing_name.clone());
                        Some(slot)
                    }
                    None => {
                        let slot = cur_phase.groups.len();
                        cur_phase.groups.push(vec![thing_name.clone()]);
                        Some(slot)
                    }
                };

                // Figure out what's next for this group
                if let Some(next_input) = &group.output {
                    if next_input.as_str() == "result" {
                        // result is a terminal output and so there's nothing to do.
                    } else {
                        for next_group in inputs_to_names.get(next_input).ok_or_else(|| {
                            ServerError::StickyProblem(ErrorDetails {
                                layer: ErrorLayer::ConfigLayer,
                                message: format!(
                                    "group {} output {} is never consumed",
                                    thing_name, next_input,
                                ),
                            })
                        })? {
                            next_groups.push_back((next_group.clone(), next_group_slot));
                        }
                    }
                }
            } else if let Some(junction) = builder.junctions.get(&thing_name) {
                let waiting_count = pending_junctions
                    .entry(thing_name.clone())
                    .or_insert_with(|| junction.inputs.len());
                *waiting_count -= 1;

                if *waiting_count == 0 {
                    cur_phase.junctions.push(thing_name.clone());
                    pending_junctions.remove(&thing_name);
                }
            }
        }

        for junction_name in cur_phase.junctions.iter() {
            // Both of these Some()s should probably use ok_or_else to error,
            // as these should absolutely exist.
            if let Some(junction) = builder.junctions.get(junction_name) {
                // Complain if this isn't the first time we've processed this
                // junction as it does indicate some kind of loop.
                if !seen.insert(junction_name.clone()) {
                    return Err(ServerError::StickyProblem(ErrorDetails {
                        layer: ErrorLayer::ConfigLayer,
                        message: format!(
                            "pipeline loop: junction {} used multiple times",
                            junction_name
                        ),
                    }));
                }

                if let Some(output) = &junction.output {
                    if output.as_str() == "result" {
                        // result is a terminal output and there's nothing to do
                    } else {
                        for next_group in inputs_to_names.get(output).ok_or_else(|| {
                            ServerError::StickyProblem(ErrorDetails {
                                layer: ErrorLayer::ConfigLayer,
                                message: format!(
                                    "junction {} output {} is never consumed",
                                    junction_name, output,
                                ),
                            })
                        })? {
                            next_groups.push_back((next_group.clone(), None));
                        }
                    }
                }
            }
        }
        builder.phases.push(cur_phase);
    }

    Ok(builder)
}
