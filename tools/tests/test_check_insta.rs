use regex::Regex;
use tokio::fs::{read_dir, read_to_string};
use tools::cmd_pipeline::{build_pipeline, PipelineValues};

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

        // Regexp to help with stripping coverage and blame lines which are not
        // expected to be stable in production for HTML lines.
        //
        // TODO: Make this something that doesn't impact the mozsearch test
        // repository, since its coverage data is expected to always be canned.
        let strip_stripper =
            Regex::new(r#"(?m)^  <div role="cell"><div class="(?:cov|blame)-strip.+</div></div>\n"#)
                .unwrap();

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
                        Ok(PipelineValues::HtmlExcerpts(he)) => {
                            let mut aggr_str = String::new();
                            for file_excerpts in he.by_file {
                                //println!("HTML excerpts from: {}", file_excerpts.file);
                                for str in file_excerpts.excerpts {
                                    // Normalize out the blame/coverage lines.
                                    // See the regexp def for more info.
                                    aggr_str += &strip_stripper.replace_all(&str, "");
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
