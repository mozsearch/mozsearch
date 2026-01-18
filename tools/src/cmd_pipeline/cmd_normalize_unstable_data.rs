use async_trait::async_trait;
use clap::Args;
use itertools::Itertools;
use lazy_static::lazy_static;
use lol_html::{element, rewrite_str, RewriteStrSettings};
use regex::Regex;
use serde_json::Value;

use super::interface::{
    JsonRecords, JsonRecordsByFile, JsonValue, PipelineCommand, PipelineValues,
};
use crate::{
    abstract_server::{AbstractServer, Result},
    cmd_pipeline::interface::{HtmlExcerpts, HtmlExcerptsByFile, TextFile},
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
/// - Replacing .blame-strip data with BLAME, removing c1/c2 class and aria-label.
/// - Replacing line numbers with N
/// - Replacing data-i values with "NORM".
/// - Replacing the permalink revision with REV.
fn norm_html_value(s: String) -> String {
    let element_content_handlers = vec![
        element!(r#"div.blame-strip"#, |el| {
            if el.has_attribute("data-blame") {
                el.set_attribute("data-blame", "BLAME").unwrap();
            }
            el.remove_attribute("aria-label");
            {
                let classes = el.get_attribute("class");
                let classes = classes.as_deref().unwrap_or("");
                let filtered_classes = classes
                    .split_ascii_whitespace()
                    .into_iter()
                    .filter(|&class| class != "c1" && class != "c2")
                    .join(" ");
                if filtered_classes.is_empty() {
                    el.remove_attribute("class");
                } else {
                    el.set_attribute("class", &filtered_classes).unwrap();
                }
            }

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
        element!(r#"#rev-id a"#, |el| {
            if let Some(url) = el.get_attribute("href") {
                lazy_static! {
                    static ref PATTERN: Regex = Regex::new("/commit/[0-9a-f]+").unwrap();
                }
                let url = PATTERN.replace_all(&url, "/commit/REV");
                el.set_attribute("href", &url).unwrap();
                el.set_inner_content("REV", lol_html::html_content::ContentType::Text);
            }
            Ok(())
        }),
        element!(r"a#panel-permalink", |el| {
            for attribute in ["href", "data-link"] {
                if let Some(url) = el.get_attribute(attribute) {
                    lazy_static! {
                        static ref PATTERN: Regex = Regex::new("/rev/[^/]+/").unwrap();
                    }
                    let url = PATTERN.replace_all(&url, "/rev/REV/");
                    el.set_attribute(attribute, &url).unwrap();
                }
            }

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
            PipelineValues::TextFile(tf) if tf.mime_type == "text/html" => {
                PipelineValues::TextFile(TextFile {
                    contents: norm_html_value(tf.contents),
                    ..tf
                })
            }
            PipelineValues::FlattenedResultsBundle(mut frb) if frb.content_type == "text/html" => {
                for path_kind_result in &mut frb.path_kind_results {
                    for kind_group in &mut path_kind_result.kind_groups {
                        for file in &mut kind_group.by_file {
                            for line_span in &mut file.line_spans {
                                let contents = core::mem::take(&mut line_span.contents);
                                line_span.contents = norm_html_value(contents);
                            }
                        }
                    }
                }

                PipelineValues::FlattenedResultsBundle(frb)
            }
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
