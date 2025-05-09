use std::env::args_os;

use serde_json::{to_string_pretty, to_value, Value};
use tools::{
    abstract_server::{ErrorDetails, ErrorLayer, ServerError},
    cmd_pipeline::{builder::build_pipeline, parser::OutputFormat, PipelineValues},
};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    let mut os_args: Vec<String> = args_os()
        .map(|os| os.into_string().unwrap_or("".to_string()))
        .collect();

    // We're expecting a single argument
    if os_args.len() == 1 {
        println!("!!! NOTE !!!");
        println!(
            "This command expects a single argument that it can parse up; quote in your shell."
        );
        println!("Example: `searchfox-tool 'cmd1 --arg | cmd2 --arg | cmd3'");
        println!();
        println!(
            "The built-in help will work, but the arg parser gets invoked once for each pipe."
        );
        println!("---");

        os_args.push("--help".to_string())
    } else if os_args.len() > 2 {
        println!("!!! TOO MANY ARGS !!!");
        println!(
            "This command expects a single argument that it can parse up; quote in your shell."
        );
        println!("Example: `searchfox-tool 'cmd1 --arg | cmd2 --arg | cmd3'");
        println!("^^^");
        std::process::exit(2);
    }

    let (pipeline, output_format) = match build_pipeline(&os_args[0], &os_args[1]) {
        Ok(pipeline) => pipeline,
        Err(ServerError::StickyProblem(ErrorDetails {
            layer: ErrorLayer::BadInput,
            message,
        })) => {
            println!("{}", message);
            std::process::exit(1);
        }
        Err(err) => {
            panic!("You did not specify a good pipeline!\n {:?}", err);
        }
    };

    let results = pipeline.run(false).await;

    let emit_json = |val: &Value| {
        if output_format == OutputFormat::Concise {
            println!("{}", val);
        } else if output_format == OutputFormat::Pretty {
            if let Ok(pretty) = to_string_pretty(val) {
                println!("{}", pretty);
            }
        }
    };

    std::process::exit(match results {
        Ok(PipelineValues::Void) => {
            println!("Void result.");
            0
        }
        Ok(PipelineValues::IdentifierList(il)) => {
            for identifier in il.identifiers {
                println!("{}", identifier);
            }
            0
        }
        Ok(PipelineValues::SymbolList(sl)) => {
            emit_json(&to_value(sl).unwrap());
            0
        }
        Ok(PipelineValues::SymbolCrossrefInfoList(sl)) => {
            for symbol_info in sl.symbol_crossref_infos {
                emit_json(&symbol_info.crossref_info);
            }
            0
        }
        Ok(PipelineValues::SymbolGraphCollection(sgc)) => {
            emit_json(&sgc.to_json());
            0
        }
        Ok(PipelineValues::FlattenedResultsBundle(frb)) => {
            emit_json(&to_value(frb).unwrap());
            0
        }
        Ok(PipelineValues::GraphResultsBundle(grb)) => {
            emit_json(&to_value(grb).unwrap());
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
        Ok(PipelineValues::TextFile(fb)) => {
            println!("{}", fb.contents);
            0
        }
        Ok(PipelineValues::JsonRecords(jr)) => {
            for file_records in jr.by_file {
                for value in file_records.records {
                    emit_json(&value);
                }
            }
            0
        }
        Ok(PipelineValues::JsonValue(jv)) => {
            emit_json(&jv.value);
            0
        }
        Ok(PipelineValues::JsonValueList(jvl)) => {
            emit_json(&to_value(jvl).unwrap());
            0
        }
        Ok(PipelineValues::FileMatches(fm)) => {
            emit_json(&to_value(fm).unwrap());
            0
        }
        Ok(PipelineValues::TextMatches(tm)) => {
            emit_json(&to_value(tm).unwrap());
            0
        }
        Ok(PipelineValues::BatchGroups(bg)) => {
            emit_json(&to_value(bg).unwrap());
            0
        }
        Ok(PipelineValues::SymbolTreeTableList(sttl)) => {
            emit_json(&to_value(sttl).unwrap());
            0
        }
        Err(err) => {
            println!("Pipeline Error!");
            println!("{:?}", err);
            1
        }
    });
}
