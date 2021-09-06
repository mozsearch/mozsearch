use std::env::args_os;

use serde_json::to_string_pretty;
use tools::cmd_pipeline::{builder::build_pipeline, parser::OutputFormat, PipelineValues};

#[tokio::main]
async fn main() {
    let os_args: Vec<String> = args_os()
        .map(|os| os.into_string().unwrap_or("".to_string()))
        .collect();

    let (pipeline, output_format) = match build_pipeline(&os_args[0], &os_args[1]) {
        Ok(pipeline) => pipeline,
        Err(err) => {
            panic!("You did not specify a good pipeline!\n {:?}", err);
        }
    };

    let results = pipeline.run().await;

    std::process::exit(match results {
        Ok(PipelineValues::Void) => {
            println!("Void result.");
            0
        }
        Ok(PipelineValues::HtmlExcerpts(he)) => {
            for file_excerpts in he.by_file {
                //println!("HTML excerpts from: {}", file_excerpts.file);
                for str in file_excerpts.excerpts {
                    println!("{}", str);
                }
            }
            0
        }
        Ok(PipelineValues::JsonRecords(jr)) => {
            for file_records in jr.by_file {
                for value in file_records.records {
                    if output_format == OutputFormat::Concise {
                        println!("{}", value);
                    } else if output_format == OutputFormat::Pretty {
                        if let Ok(pretty) = to_string_pretty(&value) {
                            println!("{}", pretty);
                        }
                    }
                }
            }
            0
        }
        Ok(PipelineValues::JsonValue(jv)) => {
            if output_format == OutputFormat::Concise {
                println!("{}", jv.value);
            } else if output_format == OutputFormat::Pretty {
                if let Ok(pretty) = to_string_pretty(&jv.value) {
                    println!("{}", pretty);
                }
            }
            0
        }
        Err(err) => {
            println!("Pipeline Error!");
            println!("{:?}", err);
            1
        }
    })
}
