use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    iter::FromIterator,
};

use query_parser::{parse, TermValue};
use serde::{Deserialize, Serialize};
use toml::value::Table;

use crate::abstract_server::{ErrorDetails, ErrorLayer, Result, ServerError};

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
pub struct QueryPipelineGroupBuilder {
    pub groups: BTreeMap<String, PipelineGroup>,
    pub junctions: BTreeMap<String, JunctionNode>,
}

fn apply_transforms(user_val: String, transforms: &Vec<String>) -> String {
    let mut val = user_val;
    for transform in transforms.iter() {
        val = match transform.as_str() {
            "regexp_escape" => regex::escape(&val),
            _ => val,
        }
    }
    val
}

fn flatten_args(user_val: &str, args: &Table) -> Vec<String> {
    let mut flattened = vec![];
    for (key, arg_val) in args.iter() {
        if key.as_str() == "positional" {
            if let Some(arg_str) = arg_val.as_str() {
                flattened.push(arg_str.replace("$0", user_val));
            }
        } else if let Some(arg_bool) = arg_val.as_bool() {
            // boolean command-line args should be omitted if false
            if arg_bool {
                flattened.push(format!("--{}", key));
            }
        } else if let Some(arg_str) = arg_val.as_str() {
            let replaced_arg = arg_str.replace("$0", user_val);
            flattened.push(format!("--{}={}", key, shell_words::quote(&replaced_arg)))
        }
    }
    flattened
}

impl QueryPipelineGroupBuilder {
    fn ensure_pipeline_step(&mut self, group_name: String, command: String, mut args: Vec<String>) {
        let group = self
            .groups
            .entry(group_name)
            .or_insert_with(|| PipelineGroup::default());

        match group
            .segments
            .iter_mut()
            .rfind(|seg| seg.command == command)
        {
            Some(seg) => {
                seg.args.append(&mut args);
            }
            None => {
                group.segments.push(PipelineSegment { command, args });
            }
        }
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
                        let flattened_args = flatten_args(&term_value, &pipe_use.args);
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

#[derive(Default, Serialize)]
pub struct JunctionNode {
    pub inputs: Vec<String>,
    pub command: PipelineSegment,
    pub output: Option<String>,
    pub depth: u32,
}

#[derive(Default, Serialize)]
pub struct PipelineSegment {
    pub command: String,
    pub args: Vec<String>,
}

pub fn chew_query(full_arg_str: &str) -> Result<QueryPipelineGroupBuilder> {
    let mut builder = QueryPipelineGroupBuilder::default();
    let q = parse(full_arg_str);
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

    let mut unprocessed_groups: VecDeque<String> = builder.groups.keys().cloned().collect();
    let mut unprocessed_junctions: VecDeque<String> = VecDeque::new();
    // The set of groups without an input which means they should go in the
    // first `ParallelPipelines` instance.  We remove groups from this set as we
    // determine that they are actually depdendent on some earlier pipeline.
    let mut root_groups = BTreeSet::from_iter(unprocessed_groups.iter().cloned());

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
                if group.input.is_some() {
                    root_groups.remove(&group_name);
                }
                // group.output will be set and next/junction will be processed
                // in the 1st phase of the loop above; we're just ensuring the
                // group exists, establishing the "input" link, and adding any
                // commands/args listed.
                unprocessed_groups.push_back(group_name);

                for cmd in &group_config.commands {
                    let flattened_args = flatten_args("", &cmd.args);
                    group.segments.push(PipelineSegment {
                        command: cmd.command.clone(),
                        args: flattened_args,
                    });
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
                junction.inputs.push(input);
                // junction.output will be set and next will be processed in the
                // 1st phase of the loop above; we're just ensuring the junction
                // exists and adding the "input" to the list.
                //
                // Logic to run only the first time we're processing the
                // junction:
                if junction.command.command.is_empty() {
                    junction.command.command = junction_config.command.clone();
                    junction.command.args = flatten_args("", &junction_config.args);
                    unprocessed_junctions.push_back(junction_name);
                }
            }
        }
    }

    Ok(builder)
}
