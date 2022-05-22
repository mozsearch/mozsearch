use std::{
    io, str,
    sync::{Arc, Mutex, MutexGuard, TryLockError},
};

use serde_json::{from_str, json, Value, to_value};
use tokio::fs::{create_dir_all, read_to_string, write};
use tools::{
    abstract_server::ServerError,
    cmd_pipeline::{build_pipeline, PipelineValues},
    glob_helper::block_in_place_glob_tree,
    templating::builder::build_and_parse_pipeline_explainer,
};
use tracing::dispatcher::Dispatch;
use tracing_subscriber::fmt::MakeWriter;

/// MockWriter and MockMakeWriter is currently taken verbatim from the MIT licensed
/// tracing project that is an existing dependency.  It exists only as a cfg(test)
/// module in
/// https://github.com/tokio-rs/tracing/blob/master/tracing-subscriber/src/fmt/mod.rs
/// hence the need to replicate the code here.
pub struct MockWriter {
    buf: Arc<Mutex<Vec<u8>>>,
}

impl MockWriter {
    pub(crate) fn new(buf: Arc<Mutex<Vec<u8>>>) -> Self {
        Self { buf }
    }

    pub(crate) fn map_error<Guard>(err: TryLockError<Guard>) -> io::Error {
        match err {
            TryLockError::WouldBlock => io::Error::from(io::ErrorKind::WouldBlock),
            TryLockError::Poisoned(_) => io::Error::from(io::ErrorKind::Other),
        }
    }

    pub(crate) fn buf(&self) -> io::Result<MutexGuard<'_, Vec<u8>>> {
        self.buf.try_lock().map_err(Self::map_error)
    }
}

impl io::Write for MockWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf()?.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.buf()?.flush()
    }
}

#[derive(Clone, Default)]
pub(crate) struct MockMakeWriter {
    buf: Arc<Mutex<Vec<u8>>>,
}

impl MockMakeWriter {
    // currently unused
    /*
    pub(crate) fn new(buf: Arc<Mutex<Vec<u8>>>) -> Self {
        Self { buf }
    }

    pub(crate) fn buf(&self) -> MutexGuard<'_, Vec<u8>> {
        self.buf.lock().unwrap()
    }
    */

    pub(crate) fn get_string(&self) -> String {
        let mut buf = self.buf.lock().expect("lock shouldn't be poisoned");
        let string = std::str::from_utf8(&buf[..])
            .expect("formatter should not have produced invalid utf-8")
            .to_owned();
        buf.clear();
        string
    }
}

impl<'a> MakeWriter<'a> for MockMakeWriter {
    type Writer = MockWriter;

    fn make_writer(&'a self) -> Self::Writer {
        MockWriter::new(self.buf.clone())
    }
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
#[tokio::test(flavor = "multi_thread")]
async fn test_check_glob() -> Result<(), std::io::Error> {
    if let Ok(check_root) = std::env::var("CHECK_ROOT") {
        let explain_template = build_and_parse_pipeline_explainer();

        let input_path = format!("{}/inputs", check_root);
        let snapshot_root_path = format!("{}/snapshots", check_root);

        let mut settings = insta::Settings::clone_current();
        settings.set_prepend_module_to_snapshot(false);

        // ## Figure out the list of input files
        let input_names = block_in_place_glob_tree(&input_path, "**/*");

        for (rel_path, filename) in input_names {
            let input_path = format!("{}/inputs/{}{}", check_root, rel_path, filename);
            settings.set_input_file(&input_path);
            let snapshot_path = format!("{}/{}", snapshot_root_path, rel_path);
            create_dir_all(snapshot_path.clone()).await?;
            settings.set_snapshot_path(snapshot_path);
            settings.set_snapshot_suffix(filename.clone());

            let make_writer = MockMakeWriter::default();
            let subscriber = tracing_subscriber::fmt()
                .json()
                .with_env_filter("tools=trace")
                .with_current_span(false)
                .with_writer(make_writer.clone())
                .finish();

            let dispatcher = Dispatch::new(subscriber);
            let _trace_guard = tracing::dispatcher::set_default(&dispatcher);
            let mut server_kind = "unknown".to_string();

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
                        Ok(PipelineValues::HtmlExcerpts(he)) => {
                            let mut aggr_str = String::new();
                            for file_excerpts in he.by_file {
                                for str in file_excerpts.excerpts {
                                    aggr_str += &str;
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
                        Ok(PipelineValues::FileMatches(fm)) => {
                            insta::assert_json_snapshot!(&to_value(fm).unwrap());
                        }
                        Ok(PipelineValues::TextMatches(tm)) => {
                            insta::assert_json_snapshot!(&to_value(tm).unwrap());
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

            let log_values: Vec<Value> = make_writer
                .get_string()
                .lines()
                .map(|s| from_str(s).unwrap())
                .collect();

            let explain_dir = format!("{}/explanations/{}", check_root, rel_path);
            create_dir_all(explain_dir.clone()).await?;
            let explain_path = format!("{}{}-{}.md", explain_dir, filename, server_kind);

            let globals = liquid::object!({
                "logs": log_values,
            });

            let output = explain_template.render(&globals).unwrap();
            write(explain_path, output).await?;
        }
    }

    Ok(())
}
