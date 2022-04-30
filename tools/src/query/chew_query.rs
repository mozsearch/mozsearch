use std::collections::{BTreeMap, VecDeque};

use query_parser::{parse, TermValue};
use serde::{Deserialize, Serialize};
use toml::value::Table;

use crate::abstract_server::{ErrorDetails, ErrorLayer, Result, ServerError};

/*
  Queries are translated into pipelines by building individiual pipelines on
  a per-group basis.
*/

#[derive(Deserialize)]
pub struct QueryConfig {
    pub term: BTreeMap<String, TermConfig>,
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
    pub args: Table,
}

lazy_static! {
    static ref QUERY_CORE: QueryConfig = toml::from_str(include_str!("query_core.toml")).unwrap();
}

#[derive(Default, Serialize)]
pub struct QueryPipelineGroupBuilder {
    pub groups: BTreeMap<String, PipelineGroup>,
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
    pub segments: Vec<PipelineSegment>,
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

    Ok(builder)
}
