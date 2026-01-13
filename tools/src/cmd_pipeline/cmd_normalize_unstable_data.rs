use async_trait::async_trait;
use clap::Args;
use lazy_static::lazy_static;
use lol_html::{element, rewrite_str, RewriteStrSettings};
use regex::Regex;
use serde_json::Value;

use super::interface::{
    JsonRecords, JsonRecordsByFile, JsonValue, PipelineCommand, PipelineValues,
};
use crate::{
    abstract_server::{AbstractServer, Result},
    cmd_pipeline::interface::{HtmlExcerpts, HtmlExcerptsByFile},
};

/// Normalize HTML or JSON records for production environment checks so that
/// details like line numbers or `data-i` indexes that are subject to churn
/// due to changes elsewhere in the file are normalized to "NORM".
#[derive(Debug, Args)]
pub struct NormalizeUnstableData {}

#[allow(dead_code)]
#[derive(Debug)]
pub struct NormalizeUnstableDataCommand {
    pub args: NormalizeUnstableData,
}

/// Normalize JSON values by:
/// - Converting "loc" properties from "LINE:COL-COL" to "NORML:COL-COL".
fn norm_json_value(mut val: Value) -> Value {
    lazy_static! {
        // The column portion can potentially be singular I think so we just
        // treat the half as its own group.
        static ref RE: Regex = Regex::new(r"^(?P<line>\d+):(?P<cols>.+)$").unwrap();
    }

    if let Some(loc_ref) = val.pointer_mut("/loc") {
        let existing_loc = loc_ref.as_str().unwrap();
        *loc_ref = RE.replace_all(existing_loc, "NORM:$cols").into();
    }

    val
}

/// Normalize HTML values by:
/// - Stripping .cov-strip elements.
/// - Stripping .blame-strip elements.
/// - Replacing line numbers with N
/// - Replacing data-i values with "NORM".
fn norm_html_value(s: String) -> String {
    let element_content_handlers = vec![
        element!(r#"div.cov-strip, div.blame-strip"#, |el| {
            el.remove();
            Ok(())
        }),
        // As a transient thing, remove data-i entirely since this will allow us
        // to update the production checks before landing.  This rule can be
        // removed after we've transitioned as "data-i" should no longer exist.
        element!(r#"span[data-i]"#, |el| {
            el.remove_attribute("data-i");
            Ok(())
        }),
        element!(r#"div.source-line-with-number"#, |el| {
            el.set_attribute("id", "line-NORM").unwrap();
            Ok(())
        }),
        element!(r#"div.line-number"#, |el| {
            el.set_attribute("data-line-number", "NORM").unwrap();
            Ok(())
        }),
    ];

    rewrite_str(
        &s,
        RewriteStrSettings {
            element_content_handlers,
            ..RewriteStrSettings::default()
        },
    )
    .unwrap()
}

#[async_trait]
impl PipelineCommand for NormalizeUnstableDataCommand {
    async fn execute(
        &self,
        _server: &(dyn AbstractServer + Send + Sync),
        input: PipelineValues,
    ) -> Result<PipelineValues> {
        Ok(match input {
            PipelineValues::JsonRecords(jr) => PipelineValues::JsonRecords(JsonRecords {
                by_file: jr
                    .by_file
                    .into_iter()
                    .map(|jrbf| JsonRecordsByFile {
                        file: jrbf.file,
                        records: jrbf.records.into_iter().map(norm_json_value).collect(),
                    })
                    .collect(),
            }),
            PipelineValues::HtmlExcerpts(he) => PipelineValues::HtmlExcerpts(HtmlExcerpts {
                by_file: he
                    .by_file
                    .into_iter()
                    .map(|hebf| HtmlExcerptsByFile {
                        file: hebf.file,
                        excerpts: hebf.excerpts.into_iter().map(norm_html_value).collect(),
                    })
                    .collect(),
            }),
            // We don't currently handle a lone JsonValue but I guess it could
            // just be the JsonRecords case that gets wrapped and then
            // unwrapped?
            PipelineValues::JsonValue(jv) => PipelineValues::JsonValue(JsonValue {
                value: norm_json_value(jv.value),
            }),
            other => other,
        })
    }
}
