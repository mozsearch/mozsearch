use async_trait::async_trait;
use clap::Args;
use serde_json::{from_value, json, to_value, Value};

use jaq_core;
use jaq_json;
use jaq_std;
use jaq_std::ValT;

use super::interface::{
    JsonRecords, JsonRecordsByFile, JsonValue, PipelineCommand, PipelineValues,
};
use crate::abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError};

fn to_serde_json(v: &jaq_json::Val) -> Value {
    let from_utf8 = |s| String::from_utf8_lossy(s).into_owned();
    match v {
        jaq_json::Val::Null => serde_json::Value::Null,
        jaq_json::Val::Bool(b) => serde_json::Value::Bool(*b),
        jaq_json::Val::Num(jaq_json::Num::Int(i)) => serde_json::Value::Number((*i).into()),
        jaq_json::Val::Num(jaq_json::Num::Float(f)) => serde_json::Number::from_f64(*f)
            .map_or(serde_json::Value::Null, serde_json::Value::Number),
        jaq_json::Val::Num(n) => {
            serde_json::Value::Number(serde_json::from_str(&n.to_string()).unwrap())
        }
        jaq_json::Val::Str(s, jaq_json::Tag::Utf8) => serde_json::Value::String(from_utf8(&*s)),
        jaq_json::Val::Str(s, jaq_json::Tag::Bytes) => {
            serde_json::Value::String(s.iter().copied().map(char::from).collect())
        }
        jaq_json::Val::Arr(a) => serde_json::Value::Array(a.iter().map(to_serde_json).collect()),
        jaq_json::Val::Obj(o) => serde_json::Value::Object(
            o.iter()
                .map(|(k, v)| (from_utf8(k.as_utf8_bytes().unwrap()), to_serde_json(v)))
                .collect(),
        ),
    }
}

/// Apply jq on JSON.
#[derive(Debug, Args)]
pub struct JQ {
    #[clap(value_parser)]
    filter: String,
}

#[derive(Debug)]
pub struct JQCommand {
    pub args: JQ,
}

impl JQCommand {
    fn filter(&self, val: Value) -> Result<Value> {
        let program = jaq_core::load::File {
            code: self.args.filter.as_str(),
            path: (),
        };

        let loader = jaq_core::load::Loader::new(jaq_std::defs());
        let arena = jaq_core::load::Arena::default();

        let modules = match loader.load(&arena, program) {
            Ok(m) => m,
            Err(errors) => {
                return Err(ServerError::StickyProblem(ErrorDetails {
                    layer: ErrorLayer::DataLayer,
                    message: format!("jq load error: {:?}", errors),
                }));
            }
        };

        let filter_result = jaq_core::Compiler::default()
            .with_funs(jaq_std::funs())
            .compile(modules);
        let filter = match filter_result {
            Ok(f) => f,
            Err(errors) => {
                return Err(ServerError::StickyProblem(ErrorDetails {
                    layer: ErrorLayer::DataLayer,
                    message: format!("jq compile error: {:?}", errors),
                }));
            }
        };

        let ctx = jaq_core::Ctx::<jaq_core::data::JustLut<jaq_json::Val>>::new(
            &filter.lut,
            jaq_core::Vars::new([]),
        );

        let mut out = filter.id.run((ctx, from_value(val).unwrap()));

        match out.next() {
            Some(Ok(v)) => Ok(to_serde_json(&v)),
            Some(Err(e)) => Err(ServerError::StickyProblem(ErrorDetails {
                layer: ErrorLayer::DataLayer,
                message: format!("jq filter error: {:?}", e),
            })),
            None => Err(ServerError::StickyProblem(ErrorDetails {
                layer: ErrorLayer::DataLayer,
                message: "no output from jq".to_string(),
            })),
        }
    }
}

#[async_trait]
impl PipelineCommand for JQCommand {
    async fn execute(
        &self,
        _server: &(dyn AbstractServer + Send + Sync),
        input: PipelineValues,
    ) -> Result<PipelineValues> {
        Ok(match input {
            PipelineValues::IdentifierList(il) => PipelineValues::JsonValue(JsonValue {
                value: self.filter(json!(il.identifiers))?,
            }),
            PipelineValues::SymbolList(sl) => PipelineValues::JsonValue(JsonValue {
                value: self.filter(to_value(sl).unwrap())?,
            }),
            PipelineValues::SymbolCrossrefInfoList(scil) => PipelineValues::JsonValue(JsonValue {
                value: self.filter(to_value(scil).unwrap())?,
            }),
            PipelineValues::SymbolGraphCollection(sgc) => PipelineValues::JsonValue(JsonValue {
                value: self.filter(sgc.to_json())?,
            }),
            PipelineValues::JsonValue(jv) => PipelineValues::JsonValue(JsonValue {
                value: self.filter(jv.value)?,
            }),
            PipelineValues::JsonValueList(jvl) => PipelineValues::JsonValue(JsonValue {
                value: self.filter(to_value(jvl).unwrap())?,
            }),
            PipelineValues::JsonRecords(jr) => {
                let mut by_file = vec![];

                for jrbf in jr.by_file {
                    let mut records = vec![];

                    for val in jrbf.records {
                        records.push(self.filter(val)?);
                    }

                    by_file.push(JsonRecordsByFile {
                        file: jrbf.file,
                        records: records,
                    });
                }

                PipelineValues::JsonRecords(JsonRecords { by_file: by_file })
            }
            PipelineValues::FileMatches(fm) => PipelineValues::JsonValue(JsonValue {
                value: self.filter(to_value(fm).unwrap())?,
            }),
            PipelineValues::TextMatches(tm) => PipelineValues::JsonValue(JsonValue {
                value: self.filter(to_value(tm).unwrap())?,
            }),
            PipelineValues::HtmlExcerpts(he) => PipelineValues::JsonValue(JsonValue {
                value: self.filter(to_value(he).unwrap())?,
            }),
            PipelineValues::FlattenedResultsBundle(frb) => PipelineValues::JsonValue(JsonValue {
                value: self.filter(to_value(frb).unwrap())?,
            }),
            PipelineValues::GraphResultsBundle(grb) => PipelineValues::JsonValue(JsonValue {
                value: self.filter(to_value(grb).unwrap())?,
            }),
            PipelineValues::TextFile(fb) => PipelineValues::JsonValue(JsonValue {
                value: self.filter(to_value(fb).unwrap())?,
            }),
            PipelineValues::BatchGroups(bg) => PipelineValues::JsonValue(JsonValue {
                value: self.filter(to_value(bg).unwrap())?,
            }),
            PipelineValues::SymbolTreeTableList(sttl) => PipelineValues::JsonValue(JsonValue {
                value: self.filter(to_value(sttl).unwrap())?,
            }),
            PipelineValues::Void => PipelineValues::Void,
        })
    }
}
