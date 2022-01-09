use serde_json::{json, Value};
use tokio::fs::{read_dir, read_to_string};
use tools::{
    abstract_server::ServerError,
    cmd_pipeline::{build_pipeline, PipelineValues},
};

/// Glob-style insta test where we process all of the searchfox-tool command
/// lines in TREE/checks/inputs and output the results of those pipelines to
/// TREE/checks/snapshots using `insta` which provides diff functionality.
///
/// This very dubiously currently relies on having an environment variable
/// CHECK_ROOT defined to tell us where the TREE is.  One might wonder whether
/// this should actually be a test at all or whether it should be its own binary
/// or maybe searchfox-tool should know how to do this or what.
///
/// The reality is that we expect this to be invoked indirectly via
/// `check-index.sh` and never directly triggered via `cargo test`, so... yeah,
/// maybe we could do this in a more clean fashion.  Better opinions accepted!
///
/// `insta` does provide support for binding settings using an `async` function,
/// but its "glob" mechanism does not support `async` so we attempt to reproduce
/// the subset we need from tokio::fs.  (We don't need to use tokio for this,
/// but since we've already started down that road, we stay on the road.  The
/// use of tokio for this is separate from the async limitations on insta's glob
/// which necessitate us doing our own file-finding.)
#[tokio::test(flavor = "multi_thread")]
async fn test_check_glob() -> Result<(), std::io::Error> {
    if let Ok(check_root) = std::env::var("CHECK_ROOT") {
        let input_path = format!("{}/inputs", check_root);
        let snapshot_path = format!("{}/snapshots", check_root);

        let mut settings = insta::Settings::clone_current();
        settings.set_snapshot_path(snapshot_path);
        settings.set_prepend_module_to_snapshot(false);

        // ## Figure out the list of input files and sort them.
        let mut dir = read_dir(input_path).await?;
        let mut input_names = Vec::new();

        while let Some(child) = dir.next_entry().await? {
            if child.metadata().await?.is_file() {
                input_names.push(
                    child
                        .path()
                        .file_name()
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_string(),
                );
            }
        }

        for input_name in input_names {
            let input_path = format!("{}/inputs/{}", check_root, input_name);
            settings.set_input_file(&input_path);
            settings.set_snapshot_suffix(input_name);

            settings
                .bind_async(async {
                    let command = read_to_string(input_path).await.unwrap();

                    let pipeline = match build_pipeline(&"searchfox-tool", &command) {
                        Ok((pipeline, _)) => pipeline,
                        Err(err) => {
                            insta::assert_snapshot!(format!("Pipeline Build Error: {:?}", err));
                            return;
                        }
                    };
                    let results = pipeline.run().await;

                    match results {
                        Ok(PipelineValues::Void) => {
                            insta::assert_snapshot!("void");
                        }
                        Ok(PipelineValues::IdentifierList(il)) => {
                            insta::assert_json_snapshot!(json!(il.identifiers));
                        }
                        Ok(PipelineValues::SymbolList(sl)) => match sl.from_identifiers {
                            Some(identifiers) => {
                                let mut pairs = vec![];
                                for (sym, ident) in sl.symbols.iter().zip(identifiers.iter()) {
                                    pairs.push(json!({
                                        "sym": sym,
                                        "id": ident,
                                    }));
                                }
                                insta::assert_json_snapshot!(json!(pairs));
                            }
                            None => {
                                insta::assert_json_snapshot!(json!(sl.symbols));
                            }
                        },
                        Ok(PipelineValues::SymbolCrossrefInfoList(sl)) => {
                            let crossref_json = json!(sl
                                .symbol_crossref_infos
                                .into_iter()
                                .map(|sci| sci.crossref_info)
                                .collect::<Value>());
                            insta::assert_json_snapshot!(crossref_json);
                        }
                        Ok(PipelineValues::HtmlExcerpts(he)) => {
                            let mut aggr_str = String::new();
                            for file_excerpts in he.by_file {
                                for str in file_excerpts.excerpts {
                                    aggr_str += &str;
                                }
                            }
                            insta::assert_snapshot!(&aggr_str);
                        }
                        Ok(PipelineValues::JsonRecords(jr)) => {
                            let mut json_results = vec![];
                            for file_records in jr.by_file {
                                json_results.extend(file_records.records);
                            }

                            insta::assert_json_snapshot!(&json_results);
                        }
                        Ok(PipelineValues::JsonValue(jv)) => {
                            insta::assert_json_snapshot!(&jv.value);
                        }
                        Err(ServerError::Unsupported) => {
                            // We're intentionally skipping doing anything here.
                            // Our assumption is that this error will only be
                            // returned in cases like the local index server
                            // being unable to handle "query" commands.
                        }
                        Err(err) => {
                            insta::assert_snapshot!(format!("Pipeline Error: {:?}", err));
                        }
                    }
                })
                .await;
        }
    }

    Ok(())
}
