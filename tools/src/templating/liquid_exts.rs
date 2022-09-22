use liquid_core::Expression;
use liquid_core::FilterParameters;
use liquid_core::FromFilterParameters;
use liquid_core::Result;
use liquid_core::Runtime;
use liquid_core::{Display_filter, Filter, FilterReflection, ParseFilter};
use liquid_core::{Value, ValueView};
use regex::Regex;
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
            Some(offset) => s[offset + 1..].to_string(),
            None => "".to_string(),
        };
        Ok(Value::scalar(ext))
    }
}

#[derive(Clone, ParseFilter, FilterReflection)]
#[filter(
    name = "compact_pathlike",
    description = "Remove excess whitespace in a path-like string",
    parsed(CompactPathlikeFilter)
)]

pub struct CompactPathlikeFilterParser;

#[derive(Debug, Default, Display_filter)]
#[name = "compact_pathlike"]
struct CompactPathlikeFilter;

impl Filter for CompactPathlikeFilter {
    fn evaluate(&self, input: &dyn ValueView, _runtime: &dyn Runtime) -> Result<Value> {
        lazy_static! {
            // We want to each whitespace adjactent to slashes.
            static ref RE_SLASH_WHITESPACE: Regex = Regex::new(r" */ *").unwrap();
            // We want to compress consecutive whitespace.
            static ref RE_CONSECUTIVE_WHITESPACE: Regex = Regex::new(r" {2,}").unwrap();
        }

        let s = input.to_kstr();
        let slash_normed = RE_SLASH_WHITESPACE.replace_all(&s, "/");
        let consecutived = RE_CONSECUTIVE_WHITESPACE.replace_all(&slash_normed, " ");
        Ok(Value::scalar(consecutived.trim().to_string()))
    }
}

#[derive(Clone, ParseFilter, FilterReflection)]
#[filter(
    name = "ensure_bug_url",
    description = "Given something that may be a bug URL or a bug ID, provide a bug URL",
    parsed(EnsureBugUrlFilter)
)]

pub struct EnsureBugUrlFilterParser;

#[derive(Debug, Default, Display_filter)]
#[name = "ensure_bug_url"]
struct EnsureBugUrlFilter;

impl Filter for EnsureBugUrlFilter {
    fn evaluate(&self, input: &dyn ValueView, _runtime: &dyn Runtime) -> Result<Value> {

        let s = input.to_kstr();
        if s.starts_with(&"http") {
            Ok(Value::scalar(s.to_string()))
        } else {
            Ok(Value::scalar(format!("https://bugzilla.mozilla.org/show_bug.cgi?id={}", s)))
        }
    }
}

#[derive(Debug, FilterParameters)]
struct StripPrefixArgs {
    #[parameter(description = "The prefix to remove if it exists.", arg_type = "str")]
    prefix: Expression,
}

#[derive(Clone, ParseFilter, FilterReflection)]
#[filter(
    name = "strip_prefix_or_empty",
    description = "Strip the prefix of the input string if it matches otherwise return an empty string",
    parameters(StripPrefixArgs),
    parsed(StripPrefixOrEmptyFilter)
)]

pub struct StripPrefixOrEmptyFilterParser;

#[derive(Debug, FromFilterParameters, Display_filter)]
#[name = "strip_prefix_or_empty"]
struct StripPrefixOrEmptyFilter {
    #[parameters]
    args: StripPrefixArgs,
}

impl Filter for StripPrefixOrEmptyFilter {
    fn evaluate(&self, input: &dyn ValueView, runtime: &dyn Runtime) -> Result<Value> {
        let args = self.args.evaluate(runtime)?;

        let s = input.to_kstr();
        if let Some(stripped) = s.strip_prefix(args.prefix.as_str()) {
            Ok(Value::scalar(stripped.to_string()))
        } else {
            Ok(Value::scalar("".to_string()))
        }
    }
}
