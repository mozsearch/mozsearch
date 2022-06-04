use liquid_core::Result;
use liquid_core::Runtime;
use liquid_core::{Display_filter, Filter, FilterReflection, ParseFilter};
use liquid_core::{Value, ValueView};
use serde_json::to_string_pretty;

#[derive(Clone, ParseFilter, FilterReflection)]
#[filter(
    name = "json",
    description = "Render the provided object into pretty-printed JSON.",
    parsed(JsonFilter)
)]
pub struct JsonFilterParser;

#[derive(Debug, Default, Display_filter)]
#[name = "downcase"]
struct JsonFilter;

impl Filter for JsonFilter {
    fn evaluate(&self, input: &dyn ValueView, _runtime: &dyn Runtime) -> Result<Value> {
        let s = to_string_pretty(&input.to_value()).unwrap_or_else(|_e| "".to_string());
        Ok(Value::scalar(s))
    }
}


#[derive(Clone, ParseFilter, FilterReflection)]
#[filter(
    name = "fileext",
    description = "Extract the file extension from a path string, defaulting to the empty string.",
    parsed(FileExtFilter)
)]

pub struct FileExtFilterParser;

#[derive(Debug, Default, Display_filter)]
#[name = "fileext"]
struct FileExtFilter;

impl Filter for FileExtFilter {
    fn evaluate(&self, input: &dyn ValueView, _runtime: &dyn Runtime) -> Result<Value> {
        let s = input.to_kstr();
        let ext = match s.rfind('.') {
            Some(offset) => s[offset+1..].to_string(),
            None => "".to_string()
        };
        Ok(Value::scalar(ext))
    }
}
