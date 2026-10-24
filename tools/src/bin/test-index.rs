use std::{
    collections::{BTreeSet, HashMap},
    env, str,
};

use serde_json::{json, to_value};
use tokio::fs::{create_dir_all, read_to_string, write};
use tools::{
    abstract_server::ServerError,
    cmd_pipeline::{PipelineValues, build_pipeline},
    glob_helper::block_in_place_glob_tree,
    logging::{LoggedSpan, init_logging},
    templating::builder::build_and_parse_pipeline_explainer,
};
use tracing::Instrument;
use url::Url;

/// The servers an input's checks run against, by its `# servers: ...`
/// comment line (a shell comment, which the pipeline's parsing ignores):
/// `local` (the default), `remote` (a web server, ex: `RemoteServer` over
/// the data API), or both.  So the remote pass runs only the inputs which opt
/// in (ex: a transport suite checking that the remote server's results are
/// the local index's), rather than every pipeline's logic again.
fn input_servers(command: &str) -> Result<BTreeSet<String>, String> {
    let mut servers = None;
    for line in command.lines() {
        let Some(list) = line
            .trim()
            .strip_prefix('#')
            .and_then(|comment| comment.trim().strip_prefix("servers:"))
        else {
            continue;
        };
        let kinds: BTreeSet<String> = list.split_whitespace().map(str::to_string).collect();
        if kinds.is_empty() || kinds.iter().any(|kind| kind != "local" && kind != "remote") {
            return Err(format!("bad `# servers:` line: {:?}", line));
        }
        if servers.replace(kinds).is_some() {
            return Err("more than one `# servers:` line".to_string());
        }
    }
    Ok(servers.unwrap_or_else(|| BTreeSet::from(["local".to_string()])))
}

/// Glob-style insta test where we process all of the searchfox-tool command
/// lines under TREE/checks/inputs and output the results of those pipelines to
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
#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), std::io::Error> {
    let check_root = env::args()
        .nth(1)
        .expect("test-index requires one argument");

    init_logging();

    let explain_template = build_and_parse_pipeline_explainer();

    let input_path = format!("{}/inputs", check_root);
    let snapshot_root_path = format!("{}/snapshots", check_root);

    let mut settings = insta::Settings::clone_current();
    settings.set_prepend_module_to_snapshot(false);

    // (As `build_pipeline` decides.)
    let pass_kind = match Url::parse(&env::var("SEARCHFOX_SERVER").unwrap_or_default()) {
        Ok(_) => "remote",
        Err(_) => "local",
    };
    let mut checked = 0;
    let mut not_for_pass = 0;
    // Inputs whose `# servers:` line is bad, or which are for this pass's
    // servers but which they don't support (opting in is a commitment).
    let mut failures = vec![];
    // How many inputs so far have each file name: the snapshots of inputs
    // with the same file name (in other directories) are named `{name}`,
    // `{name}-2`, ... in their order, as insta names repeated snapshots in a
    // run, but whether or not the others run in this pass (see `# servers:`).
    let mut name_counts: HashMap<String, usize> = HashMap::new();

    // ## Figure out the list of input files
    let input_names = block_in_place_glob_tree(&input_path, "**/*");

    for (rel_path, filename) in input_names {
        if filename.ends_with("~") {
            continue;
        }
        let snapshot_suffix = {
            let count = name_counts.entry(filename.clone()).or_default();
            *count += 1;
            match *count {
                1 => filename.clone(),
                count => format!("{}-{}", filename, count),
            }
        };

        let input_path = format!("{}/inputs/{}{}", check_root, rel_path, filename);
        let command = read_to_string(&input_path).await?;
        match input_servers(&command) {
            Ok(servers) if !servers.contains(pass_kind) => {
                not_for_pass += 1;
                continue;
            }
            Ok(_) => checked += 1,
            Err(err) => {
                failures.push(format!("{}{}: {}", rel_path, filename, err));
                continue;
            }
        }
        settings.set_input_file(&input_path);
        let snapshot_path = format!("{}/{}", snapshot_root_path, rel_path);
        create_dir_all(snapshot_path.clone()).await?;
        settings.set_snapshot_path(snapshot_path);
        settings.set_snapshot_suffix(snapshot_suffix);

        let logged_span = LoggedSpan::new_logged_span(&input_path);

        let mut server_kind = "unknown".to_string();
        let mut unsupported = false;

        settings
            .bind_async(async {
                let pipeline = match build_pipeline("searchfox-tool", &command) {
                    Ok((pipeline, _)) => pipeline,
                    Err(err) => {
                        insta::assert_snapshot!(format!("Pipeline Build Error: {:?}", err));
                        return;
                    }
                };
                server_kind = pipeline.server_kind.clone();
                let results = pipeline.run(true).await;

                // TODO: In theory we should perhaps block_in_place here, but also it doesn't
                // matter.
                match results {
                    Ok(PipelineValues::Void) => {
                        insta::assert_snapshot!("void");
                    }
                    Ok(PipelineValues::IdentifierList(il)) => {
                        insta::assert_json_snapshot!(json!(il.identifiers));
                    }
                    Ok(PipelineValues::SymbolList(sl)) => {
                        insta::assert_json_snapshot!(&to_value(sl).unwrap());
                    }
                    Ok(PipelineValues::SymbolCrossrefInfoList(scil)) => {
                        // We used to previously just turn this into a list of
                        // just the crossref values, but we now have important
                        // metadata.
                        insta::assert_json_snapshot!(&to_value(scil).unwrap());
                    }
                    Ok(PipelineValues::SymbolGraphCollection(sgc)) => {
                        insta::assert_json_snapshot!(sgc.to_json());
                    }
                    Ok(PipelineValues::FlattenedResultsBundle(frb)) => {
                        insta::assert_json_snapshot!(&to_value(frb).unwrap());
                    }
                    Ok(PipelineValues::GraphInput(graphs)) => {
                        insta::assert_json_snapshot!(&to_value(graphs).unwrap());
                    }
                    Ok(PipelineValues::GraphResultsBundle(grb)) => {
                        insta::assert_json_snapshot!(&to_value(grb).unwrap());
                    }
                    Ok(PipelineValues::HtmlExcerpts(he)) => {
                        let mut aggr_str = String::new();
                        for file_excerpts in he.by_file {
                            for str in file_excerpts.excerpts {
                                aggr_str += str.as_str();
                            }
                        }
                        insta::assert_snapshot!(&aggr_str);
                    }
                    Ok(PipelineValues::TextFile(fb)) => {
                        insta::assert_snapshot!(&fb.contents);
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
                    Ok(PipelineValues::JsonValueList(jvl)) => {
                        insta::assert_json_snapshot!(&to_value(jvl).unwrap());
                    }
                    Ok(PipelineValues::FileMatches(fm)) => {
                        insta::assert_json_snapshot!(&to_value(fm).unwrap());
                    }
                    Ok(PipelineValues::TextMatches(tm)) => {
                        insta::assert_json_snapshot!(&to_value(tm).unwrap());
                    }
                    Ok(PipelineValues::BatchGroups(bg)) => {
                        insta::assert_json_snapshot!(&to_value(bg).unwrap());
                    }
                    Ok(PipelineValues::SymbolTreeTableList(sttl)) => {
                        insta::assert_json_snapshot!(&to_value(sttl).unwrap());
                    }
                    Ok(PipelineValues::TokenHistory(th)) => {
                        insta::assert_json_snapshot!(&to_value(th).unwrap());
                    }
                    Ok(PipelineValues::HistoryGrep(hg)) => {
                        insta::assert_json_snapshot!(&to_value(hg).unwrap());
                    }
                    Ok(PipelineValues::TokenNames(tn)) => {
                        insta::assert_json_snapshot!(&to_value(tn).unwrap());
                    }
                    Err(ServerError::Unsupported) => {
                        unsupported = true;
                    }
                    Err(err) => {
                        insta::assert_snapshot!(format!("Pipeline Error: {:?}", err));
                    }
                }
            })
            .instrument(logged_span.span.clone())
            .await;

        if unsupported {
            failures.push(format!(
                "{}{}: unsupported by the {} server",
                rel_path, filename, pass_kind
            ));
        }

        let log_values = logged_span.retrieve_serde_json().await;

        let explain_dir = format!("{}/explanations/{}", check_root, rel_path);
        create_dir_all(explain_dir.clone()).await?;
        let explain_path = format!("{}{}-{}.md", explain_dir, filename, server_kind);

        let globals = liquid::object!({
            "logs": vec![log_values],
        });

        let output = explain_template.render(&globals).unwrap();
        write(explain_path, output).await?;
    }

    println!(
        "test-index ({}): {} inputs checked, {} not for {} servers (see their `# servers:` lines)",
        pass_kind, checked, not_for_pass, pass_kind
    );
    if !failures.is_empty() {
        for failure in &failures {
            println!("FAIL - {}", failure);
        }
        std::process::exit(1);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_servers() {
        let local = BTreeSet::from(["local".to_string()]);
        let both = BTreeSet::from(["local".to_string(), "remote".to_string()]);
        assert_eq!(input_servers("crossref-lookup A").unwrap(), local);
        assert_eq!(
            input_servers("# A comment.\n# servers: local remote\ncrossref-lookup A").unwrap(),
            both
        );
        assert_eq!(
            input_servers("#servers: remote\nquery A").unwrap(),
            BTreeSet::from(["remote".to_string()])
        );
        assert!(input_servers("# servers:\nquery A").is_err());
        assert!(input_servers("# servers: web\nquery A").is_err());
        assert!(input_servers("# servers: local\n# servers: remote\nquery A").is_err());
    }
}
