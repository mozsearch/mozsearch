use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use clap::Args;
use fantoccini::{Client, ClientBuilder};
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

use super::interface::{PipelineCommand, PipelineValues};

use crate::abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError};

/// Runs the webtests (`tests/webtest/test_*.js`) whose paths contain the
/// filter, in the browsers of the WebDriver servers `--webdrivers` lists (see
/// `scripts/webtest.sh`), at once: sorted by name, and dealt to them in turn,
/// so each browser's tests are the same each run.  A test's leading comment
/// lines can say `// webtest-group: <name>` (and why), to run with the group's other
/// tests, in order, in one browser (ex: tests sharing a try push's import), or
/// `// webtest-exclusive`, to run once the others are done, alone (ex: tests
/// whose timing others' load would change).
#[derive(Debug, Args)]
pub struct Webtest {
    filter: Option<String>,

    /// The WebDriver servers to run tests in, comma-separated.
    #[clap(long, default_value = "http://localhost:4444")]
    webdrivers: String,
}

/// A test's outcome: whether it failed, and its log.
struct TestOutcome {
    path: String,
    failed: bool,
    subtests: usize,
    log: Vec<(String, String)>,
}

/// How a test runs among the others (see `Webtest`).
fn test_placement(path: &str) -> (Option<String>, bool) {
    let Ok(text) = fs::read_to_string(path) else {
        return (None, false);
    };
    let mut group = None;
    let mut exclusive = false;
    for line in text.lines().take_while(|line| {
        let line = line.trim();
        line.is_empty() || line.starts_with("//") || line.starts_with("\"use strict\"")
    }) {
        let line = line.trim().trim_start_matches("//").trim();
        // (Either can be followed by why.)
        if let Some(rest) = line.strip_prefix("webtest-group:") {
            group = rest.split_whitespace().next().map(str::to_string);
        } else if line.starts_with("webtest-exclusive") {
            exclusive = true;
        }
    }
    (group, exclusive)
}

#[derive(Debug)]
pub struct WebtestCommand {
    pub args: Webtest,
}

fn print_log(ty: &str, msg: String) {
    let mut stderr = StandardStream::stderr(ColorChoice::Always);

    let mut spec = ColorSpec::new();

    let color = match ty {
        "INFO" => Color::Blue,
        "DEBUG" => Color::Black,
        "PASS" => Color::Green,
        "FAIL" => Color::Red,
        "STACK" => Color::Red,
        "TEST_START" => Color::Yellow,
        "TEST_END" => Color::Yellow,
        _ => Color::Cyan,
    };
    spec.set_fg(Some(color));

    if ty == "DEBUG" {
        spec.set_dimmed(true);
    }

    stderr.set_color(&spec).unwrap();
    write!(&mut stderr, "{}", ty).unwrap();

    stderr.reset().unwrap();
    writeln!(&mut stderr, " - {}", msg).unwrap();
}

fn println_color(color: Color, msg: &str) {
    let mut stderr = StandardStream::stderr(ColorChoice::Always);
    stderr
        .set_color(ColorSpec::new().set_fg(Some(color)))
        .unwrap();
    writeln!(&mut stderr, "{}", msg).unwrap();
    stderr.reset().unwrap();
}

fn println_bold(msg: &str) {
    let mut stderr = StandardStream::stderr(ColorChoice::Always);
    stderr.set_color(ColorSpec::new().set_bold(true)).unwrap();
    writeln!(&mut stderr, "{}", msg).unwrap();
    stderr.reset().unwrap();
}

type TestResult<T> = std::result::Result<T, String>;

impl WebtestCommand {
    async fn setup_webdriver_and_run_tests(&self) -> TestResult<bool> {
        let mut clients = vec![];
        for url in self
            .args
            .webdrivers
            .split(',')
            .filter(|url| !url.is_empty())
        {
            let mut caps = serde_json::map::Map::new();
            let opts = serde_json::json!({ "args": ["--headless"] });
            caps.insert("moz:firefoxOptions".to_string(), opts);
            let client = ClientBuilder::native()
                .capabilities(caps)
                .connect(url)
                .await
                .map_err(|e| format!("{:?}", e))?;
            clients.push(client);
        }
        if clients.is_empty() {
            return Err("No WebDriver servers".to_string());
        }

        let result = self.run_tests(&clients).await;

        for client in clients {
            client.close().await.map_err(|e| format!("{:?}", e))?;
        }

        let passed = result.map_err(|e| format!("{:?}", e))?;

        Ok(passed)
    }

    /// The tests to run: units of tests to run in order in one browser (a
    /// test, or a group's tests), and the exclusive tests.
    fn tests(&self) -> (Vec<Vec<String>>, Vec<String>) {
        let mut paths: Vec<String> = fs::read_dir("tests/webtest/")
            .unwrap()
            .flatten()
            .filter_map(|file| {
                let name = file.file_name().into_string().ok()?;
                (name.starts_with("test_") && name.ends_with(".js"))
                    .then(|| file.path().to_str().unwrap().to_string())
            })
            .filter(|path| match &self.args.filter {
                Some(filter) => path.contains(filter.as_str()),
                None => true,
            })
            .collect();
        paths.sort();
        let mut units: Vec<Vec<String>> = vec![];
        let mut groups: HashMap<String, usize> = HashMap::new();
        let mut exclusive = vec![];
        for path in paths {
            match test_placement(&path) {
                (_, true) => exclusive.push(path),
                (Some(group), false) => match groups.get(&group) {
                    Some(&unit) => units[unit].push(path),
                    None => {
                        groups.insert(group, units.len());
                        units.push(vec![path]);
                    }
                },
                (None, false) => units.push(vec![path]),
            }
        }
        (units, exclusive)
    }

    /// Run a test in `client`'s browser.
    async fn run_test(
        &self,
        client: &Client,
        path: &str,
    ) -> std::result::Result<TestOutcome, fantoccini::error::CmdError> {
        let mut outcome = TestOutcome {
            path: path.to_string(),
            failed: false,
            subtests: 0,
            log: vec![],
        };
        let url = "http://localhost:16995/tests/webtest/webtest.html";
        outcome
            .log
            .push(("INFO".to_string(), format!("Navigate to {}", url)));
        client.goto(url).await?;

        outcome
            .log
            .push(("INFO".to_string(), format!("Loading {}", path)));
        client
            .execute(
                "window.TestHarness.loadTest(...arguments);",
                vec![serde_json::json!(path)],
            )
            .await?;

        let start = Instant::now();

        // TODO: Add special log command to increase the timeout.
        let timeout = 30 * 1000;

        'test_loop: loop {
            let log_value = client
                .execute("return window.TestHarness.getNewLogs();", vec![])
                .await?;
            let log: Vec<(String, String)> = serde_json::value::from_value(log_value)?;
            for (ty, msg) in log {
                if ty == "SUBTEST" {
                    outcome.subtests += 1;
                    continue;
                }
                if ty == "FAIL" {
                    outcome.failed = true;
                }
                let end = ty == "TEST_END";
                outcome.log.push((ty, msg));
                if end {
                    break 'test_loop;
                }
            }
            let elapsed_time = start.elapsed();
            if elapsed_time > Duration::from_millis(timeout) {
                outcome.failed = true;
                outcome
                    .log
                    .push(("FAIL".to_string(), format!("{} | Test timed out", path)));
                break 'test_loop;
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        if outcome.failed {
            let name = path.rsplit('/').next().unwrap_or(path);
            let filename = format!("/tmp/screen-{}.png", name);
            outcome.log.push((
                "INFO".to_string(),
                format!("Saving screenshot to {}", filename),
            ));
            let data = client.screenshot().await?;
            fs::write(filename, data)?;
        }

        Ok(outcome)
    }

    /// Run `units` in `client`'s browser, one after another, printing each
    /// test's log once it's done (so that the browsers' logs don't mix).
    async fn run_units(
        &self,
        client: &Client,
        units: Vec<Vec<String>>,
    ) -> std::result::Result<Vec<TestOutcome>, fantoccini::error::CmdError> {
        let mut outcomes = vec![];
        for path in units.into_iter().flatten() {
            let outcome = self.run_test(client, &path).await?;
            for (ty, msg) in &outcome.log {
                print_log(ty.as_str(), msg.clone());
            }
            outcomes.push(outcome);
        }
        Ok(outcomes)
    }

    async fn run_tests(
        &self,
        clients: &[Client],
    ) -> std::result::Result<bool, fantoccini::error::CmdError> {
        let entire_start = Instant::now();

        // (Dealt to the browsers in turn.)
        let (units, exclusive) = self.tests();
        let mut shards: Vec<Vec<Vec<String>>> = vec![vec![]; clients.len()];
        for (i, unit) in units.into_iter().enumerate() {
            shards[i % clients.len()].push(unit);
        }
        let runs = clients
            .iter()
            .zip(shards)
            .map(|(client, units)| self.run_units(client, units));
        let mut outcomes = vec![];
        for result in futures_util::future::join_all(runs).await {
            outcomes.extend(result?);
        }
        outcomes.extend(
            self.run_units(
                &clients[0],
                exclusive.into_iter().map(|path| vec![path]).collect(),
            )
            .await?,
        );

        let test_count = outcomes.len();
        let subtest_count: usize = outcomes.iter().map(|outcome| outcome.subtests).sum();
        let failed_tests: Vec<&TestOutcome> =
            outcomes.iter().filter(|outcome| outcome.failed).collect();

        let elapsed_time = entire_start.elapsed();
        eprintln!();
        println_color(Color::Yellow, "Overall Summary");
        println_color(Color::Yellow, "===============");
        eprintln!(
            "Ran {} tests and {} subtests in {:.3}s in {} browsers.",
            test_count,
            subtest_count,
            elapsed_time.as_millis() as f64 / 1000.0,
            clients.len()
        );
        eprintln!("Passed: {} tests", test_count - failed_tests.len());
        eprintln!("Failed: {} tests", failed_tests.len());
        eprintln!();

        let passed = failed_tests.is_empty();

        if !failed_tests.is_empty() {
            println_color(Color::Yellow, "Unexpected Results");
            println_color(Color::Yellow, "------------------");
            for outcome in failed_tests {
                println_bold(outcome.path.as_str());
                for (ty, msg) in &outcome.log {
                    if ty == "FAIL" || ty == "STACK" {
                        print_log(ty.as_str(), msg.clone());
                    }
                }
            }
        } else {
            eprintln!("OK");
        }

        Ok(passed)
    }
}

#[async_trait]
impl PipelineCommand for WebtestCommand {
    async fn execute(
        &self,
        _server: &(dyn AbstractServer + Send + Sync),
        _input: PipelineValues,
    ) -> Result<PipelineValues> {
        let passed = self.setup_webdriver_and_run_tests().await.map_err(|e| {
            ServerError::TransientProblem(ErrorDetails {
                layer: ErrorLayer::ConfigLayer,
                message: e,
            })
        })?;

        if !passed {
            return Err(ServerError::TransientProblem(ErrorDetails {
                layer: ErrorLayer::ConfigLayer,
                message: "Test failed".to_string(),
            }));
        }

        Ok(PipelineValues::Void)
    }
}
