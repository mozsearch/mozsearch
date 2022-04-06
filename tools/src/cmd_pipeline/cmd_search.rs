use async_trait::async_trait;
use json_structural_diff::JsonDiff;
use serde_json::{json, Map, Value};
use structopt::StructOpt;

use super::interface::{JsonValue, PipelineCommand, PipelineValues};
use crate::{
    abstract_server::{AbstractServer, Result},
};

/// Run a traditional searchfox search against the web server.  This will turn
/// into a no-op when run against a local index at this time, but in the future
/// may be able to spin up the necessary pieces.
#[derive(Debug, StructOpt)]
pub struct Search {
  /// Query string
  query: String,

  /// Diff the results of this query against the previous command's output,
  /// producing diff output.
  #[structopt(short, long)]
  diff: bool,

  /// Normalize "bounds" out of existence because they can differ when using
  /// different query strings.
  #[structopt(short, long)]
  normalize: bool,

  /// Convert arrays of "path"-keyed objects to a dict whose keys are the "path"
  /// values and the value is still the same value, including "path".  This is
  /// meant to make the diff option more friendly in cases where ordering is not
  /// a concern.
  #[structopt(short, long)]
  dictify: bool,
}


pub struct SearchCommand {
  pub args: Search,
}

/// Recursively transforms JSON values, removing any "bounds" value it finds
/// while recursing into any object or array values it finds.
fn normalize_bounds(val: &mut Value) {
  match val {
    Value::Object(o) => {
      o.remove(&"bounds".to_string());

      for v in o.values_mut() {
        normalize_bounds(v);
      }
    }
    Value::Array(a) => {
      for entry in a {
        normalize_bounds(entry);
      }
    }
    _ => {}
  };
}

fn dictify(val: &mut Value) -> Option<Value>{
  match val {
    Value::Object(o) => {
      for v in o.values_mut() {
        if let Some(replacement) = dictify(v) {
          *v = replacement;
        }
      }
    }
    Value::Array(a) => {
      // Check if the first entry exists and is an object and has a "path".
      if a.iter().any(|entry| entry.get("path").is_some()) {
        // We're going to create a new Value and return that, as this is a
        // transform where we are changing our container; all other transforms
        // are happening within the same existing outer container.
        let mut obj = Map::new();
        for entry in a {
          if let Some(Value::String(path)) = entry.get("path") {
            obj.insert(path.clone(), entry.take());
          }
        }
        return Some(Value::Object(obj));
      }

      for entry in a {
        if let Some(replacement) = dictify(entry) {
          *entry = replacement;
        }
      }
    }
    _ => {}
  };

  return None;
}

fn dictify_root(mut val: Value) -> Value {
  if let Some(replacement) = dictify(&mut val) {
    return replacement;
  }
  return val;
}

#[async_trait]
impl PipelineCommand for SearchCommand {
    async fn execute(
        &self,
        server: &Box<dyn AbstractServer + Send + Sync>,
        input: PipelineValues,
    ) -> Result<PipelineValues> {
        let mut value = server.perform_query(&self.args.query).await?;

        if self.args.diff {
          let mut input_json = match input {
            PipelineValues::JsonValue(j) => j.value,
            _ => json!({}),
          };

          if self.args.normalize {
            normalize_bounds(&mut input_json);
            normalize_bounds(&mut value);
          }

          if self.args.dictify {
            input_json = dictify_root(input_json);
            value = dictify_root(value);
          }

          let json_diff = JsonDiff::diff(&input_json, &value, false);
          value = json_diff.diff.unwrap_or_else(|| json!({}));
        }

        Ok(PipelineValues::JsonValue(JsonValue {
          value
        }))
    }
}
