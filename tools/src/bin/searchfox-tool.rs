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

    let mut cur_values = PipelineValues::Void;

    for cmd in pipeline.commands {
        match cmd.execute(&pipeline.server, cur_values).await {
            Ok(next_values) => {
                cur_values = next_values;
            }
            Err(err) => {
                println!("Pipeline Error!");
                println!("{:?}", err);
                return;
            }
        }
    }

    match cur_values {
        PipelineValues::Void => {
            println!("Void result.");
        }
        PipelineValues::HtmlExcerpts(he) => {
            for file_excerpts in he.by_file {
                //println!("HTML excerpts from: {}", file_excerpts.file);
                for str in file_excerpts.excerpts {
                    println!("{}", str);
                }
            }
        }
        PipelineValues::JsonRecords(jr) => {
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
        }
    }
}
