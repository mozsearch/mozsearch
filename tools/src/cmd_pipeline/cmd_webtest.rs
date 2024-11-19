use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::thread;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use clap::Args;
use fantoccini::{Client, ClientBuilder};
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

use super::interface::{PipelineCommand, PipelineValues};

use crate::abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError};

/// Runs the specified
#[derive(Debug, Args)]
pub struct Webtest {
    filter: Option<String>,
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
        let mut caps = serde_json::map::Map::new();
        let opts = serde_json::json!({ "args": ["--headless"] });
        caps.insert("moz:firefoxOptions".to_string(), opts);
        let client = ClientBuilder::native()
            .capabilities(caps)
            .connect("http://localhost:4444")
            .await
            .map_err(|e| format!("{:?}", e))?;

        let result = self.run_tests(&client).await;

        client.close().await.map_err(|e| format!("{:?}", e))?;

        let passed = result.map_err(|e| format!("{:?}", e))?;

        Ok(passed)
    }

    async fn run_tests(
        &self,
        client: &Client,
    ) -> std::result::Result<bool, fantoccini::error::CmdError> {
        let entire_start = Instant::now();
        let mut test_count = 0;
        let mut subtest_count = 0;
        let mut failed_tests = vec![];
        let mut failed_log = HashMap::new();

        let files = fs::read_dir("tests/webtest/").unwrap();
        for file in files {
            if file.is_err() {
                continue;
            }

            let file = file.unwrap();

            let name = file.file_name().clone().into_string().unwrap();
            if !name.starts_with("test_") {
                continue;
            }
            if !name.ends_with(".js") {
                continue;
            }

            let path = file.path().to_str().unwrap().to_string();

            if let Some(filter) = &self.args.filter {
                eprintln!("Filter: {} on {}", filter, path);
                if !path.contains(filter) {
                    continue;
                }
            }

            let url = "http://localhost/tests/webtest/webtest.html";
            print_log("INFO", format!("Navigate to {}", url));
            client.goto(url).await?;

            print_log("INFO", format!("Loading {}", path));
            client
                .execute(
                    "window.TestHarness.loadTest(...arguments);",
                    vec![serde_json::json!(path)],
                )
                .await?;

            let start = Instant::now();
            let mut failed = false;

            // TODO: Add special log command to increase the timeout.
            let timeout = 30 * 1000;

            'test_loop: loop {
                let log_value = client
                    .execute("return window.TestHarness.getNewLogs();", vec![])
                    .await?;
                let log: Vec<(String, String)> = serde_json::value::from_value(log_value)?;
                for (ty, msg) in log {
                    if ty == "SUBTEST" {
                        subtest_count += 1;
                        continue;
                    }

                    print_log(ty.as_str(), msg.clone());

                    if ty == "FAIL" {
                        failed = true;
                    }
                    if ty == "FAIL" || ty == "STACK" {
                        if !failed_log.contains_key(&path) {
                            failed_log.insert(path.clone(), vec![]);
                        }
                        failed_log
                            .get_mut(&path)
                            .unwrap()
                            .push((ty.clone(), msg.clone()));
                    }
                    if ty == "TEST_END" {
                        break 'test_loop;
                    }
                }
                let elapsed_time = start.elapsed();
                if elapsed_time > Duration::from_millis(timeout) {
                    failed = true;
                    print_log("FAIL", format!("{} | Test timed out", path));
                    break 'test_loop;
                }

                thread::sleep(Duration::from_millis(100));
            }

            if failed {
                failed_tests.push(path.clone());

                let filename = format!("/tmp/screen-{}.png", name);
                print_log("INFO", format!("Saving screenshot to {}", filename));
                let data = client.screenshot().await?;
                fs::write(filename, data)?;
            }

            test_count += 1;
        }

        let elapsed_time = entire_start.elapsed();
        eprintln!();
        println_color(Color::Yellow, "Overall Summary");
        println_color(Color::Yellow, "===============");
        eprintln!(
            "Ran {} tests and {} subtests in {:.3}s.",
            test_count,
            subtest_count,
            elapsed_time.as_millis() as f64 / 1000.0
        );
        eprintln!("Passed: {} tests", test_count - failed_tests.len());
        eprintln!("Failed: {} tests", failed_tests.len());
        eprintln!();

        let passed = failed_tests.is_empty();

        if !failed_tests.is_empty() {
            println_color(Color::Yellow, "Unexpected Results");
            println_color(Color::Yellow, "------------------");
            for path in failed_tests {
                println_bold(path.as_str());
                if let Some(log) = failed_log.get(&path) {
                    for (ty, msg) in log {
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
        _server: &Box<dyn AbstractServer + Send + Sync>,
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
