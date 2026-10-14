use liquid_core::Expression;
use liquid_core::FilterParameters;
use liquid_core::FromFilterParameters;
use liquid_core::Result;
use liquid_core::Runtime;
use liquid_core::{Display_filter, Filter, FilterReflection, ParseFilter};
use liquid_core::{Value, ValueView};
use regex::Regex;
use serde_json::to_string_pretty;

use crate::cmd_pipeline::recency_html::blot;
use crate::file_format::analysis_manglings::mangle_file;
use crate::file_format::recency::{BINS, Recency};

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

#[derive(Debug, FilterParameters)]
struct RecencyBlotArgs {
    #[parameter(
        description = "Whose history the digest is (ex: a path).",
        arg_type = "str"
    )]
    of: Expression,
}

#[derive(Clone, ParseFilter, FilterReflection)]
#[filter(
    name = "recency_blot",
    description = "The blot (HTML) of a history digest (see `cmd_pipeline::recency_html`).",
    parameters(RecencyBlotArgs),
    parsed(RecencyBlotFilter)
)]
pub struct RecencyBlotFilterParser;

#[derive(Debug, FromFilterParameters, Display_filter)]
#[name = "recency_blot"]
struct RecencyBlotFilter {
    #[parameters]
    args: RecencyBlotArgs,
}

impl Filter for RecencyBlotFilter {
    fn evaluate(&self, input: &dyn ValueView, runtime: &dyn Runtime) -> Result<Value> {
        let args = self.args.evaluate(runtime)?;
        let label = format!("History of {}", args.of);
        Ok(Value::scalar(blot(&recency_of(input), Some(&label), "")))
    }
}

/// A digest from its liquid value (its bins' array).
fn recency_of(input: &dyn ValueView) -> Recency {
    let mut recency = Recency::default();
    if let Some(bins) = input.as_array() {
        for (bin, tokens) in bins.values().take(BINS).enumerate() {
            let tokens = tokens.as_scalar().and_then(|s| s.to_integer()).unwrap_or(0);
            recency.add(bin, u32::try_from(tokens).unwrap_or(0));
        }
    }
    recency
}

#[derive(Clone, ParseFilter, FilterReflection)]
#[filter(
    name = "thousands",
    description = "Format an integer with commas separating groups of thousands.",
    parsed(ThousandsFilter)
)]
pub struct ThousandsFilterParser;

#[derive(Debug, Default, Display_filter)]
#[name = "thousands"]
struct ThousandsFilter;

impl Filter for ThousandsFilter {
    fn evaluate(&self, input: &dyn ValueView, _runtime: &dyn Runtime) -> Result<Value> {
        let s = input.to_kstr();
        let (sign, digits) = match s.strip_prefix('-') {
            Some(rest) => ("-", rest),
            None => ("", s.as_str()),
        };
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Ok(Value::scalar(s.to_string()));
        }
        let mut result = String::from(sign);
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i) % 3 == 0 {
                result.push(',');
            }
            result.push(c);
        }
        Ok(Value::scalar(result))
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
        if s.starts_with("http") {
            Ok(Value::scalar(s.to_string()))
        } else {
            Ok(Value::scalar(format!(
                "https://bugzilla.mozilla.org/show_bug.cgi?id={}",
                s
            )))
        }
    }
}

#[derive(Clone, ParseFilter, FilterReflection)]
#[filter(
    name = "linkify",
    description = "Escape text for HTML, with its URLs and bug numbers as links (like comments' in source listings)",
    parsed(LinkifyFilter)
)]
pub struct LinkifyFilterParser;

#[derive(Debug, Default, Display_filter)]
#[name = "linkify"]
struct LinkifyFilter;

impl Filter for LinkifyFilter {
    fn evaluate(&self, input: &dyn ValueView, _runtime: &dyn Runtime) -> Result<Value> {
        let escaped = input
            .to_kstr()
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        Ok(Value::scalar(crate::links::linkify_comment(None, escaped)))
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

// Filters for the symbols of the semantic analyzer's mappings: escaping like
// `FILE_` symbols', and the casing of glean_parser's generated names
// (glean_parser/util.py's `camelize`, `Camelize`, and `snake_case`).

#[derive(Clone, ParseFilter, FilterReflection)]
#[filter(
    name = "at_escape",
    description = "Escape everything but letters, digits, '_' and '/' as @XX, like FILE_ symbols.",
    parsed(AtEscapeFilter)
)]
pub struct AtEscapeFilterParser;

#[derive(Debug, Default, Display_filter)]
#[name = "at_escape"]
struct AtEscapeFilter;

impl Filter for AtEscapeFilter {
    fn evaluate(&self, input: &dyn ValueView, _runtime: &dyn Runtime) -> Result<Value> {
        Ok(Value::scalar(mangle_file(&input.to_kstr())))
    }
}

/// glean_parser's `to_camel_case`: words split at '.', '-', and '_',
/// capitalized (so lower-cased after their first letters), and joined.
fn glean_camel_case(s: &str, capitalize_first: bool) -> String {
    let mut out = String::new();
    for (i, word) in s
        .split(['.', '-', '_'])
        .filter(|w| !w.is_empty())
        .enumerate()
    {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            if i == 0 && !capitalize_first {
                out.extend(first.to_lowercase());
            } else {
                out.extend(first.to_uppercase());
            }
            out.extend(chars.flat_map(|c| c.to_lowercase()));
        }
    }
    out
}

#[derive(Clone, ParseFilter, FilterReflection)]
#[filter(
    name = "camel_case",
    description = "camelCase, like glean_parser's `camelize`.",
    parsed(CamelCaseFilter)
)]
pub struct CamelCaseFilterParser;

#[derive(Debug, Default, Display_filter)]
#[name = "camel_case"]
struct CamelCaseFilter;

impl Filter for CamelCaseFilter {
    fn evaluate(&self, input: &dyn ValueView, _runtime: &dyn Runtime) -> Result<Value> {
        Ok(Value::scalar(glean_camel_case(&input.to_kstr(), false)))
    }
}

#[derive(Clone, ParseFilter, FilterReflection)]
#[filter(
    name = "pascal_case",
    description = "PascalCase, like glean_parser's `Camelize`.",
    parsed(PascalCaseFilter)
)]
pub struct PascalCaseFilterParser;

#[derive(Debug, Default, Display_filter)]
#[name = "pascal_case"]
struct PascalCaseFilter;

impl Filter for PascalCaseFilter {
    fn evaluate(&self, input: &dyn ValueView, _runtime: &dyn Runtime) -> Result<Value> {
        Ok(Value::scalar(glean_camel_case(&input.to_kstr(), true)))
    }
}

#[derive(Clone, ParseFilter, FilterReflection)]
#[filter(
    name = "snake_case",
    description = "snake_case, like glean_parser's `snake_case`: lower case, with '.' and '-' as '_'.",
    parsed(SnakeCaseFilter)
)]
pub struct SnakeCaseFilterParser;

#[derive(Debug, Default, Display_filter)]
#[name = "snake_case"]
struct SnakeCaseFilter;

impl Filter for SnakeCaseFilter {
    fn evaluate(&self, input: &dyn ValueView, _runtime: &dyn Runtime) -> Result<Value> {
        Ok(Value::scalar(
            input.to_kstr().to_lowercase().replace(['.', '-'], "_"),
        ))
    }
}

/// The strings which a regular expression of only literals, groups,
/// alternations, and `?`s matches (ex: `a(|-b|-c)?` is `a`, `a-b`, and
/// `a-c`), or None if it has other constructs or more than 1000 strings.
pub fn regex_expansions(pattern: &str) -> Option<Vec<String>> {
    const LIMIT: usize = 1000;
    fn alternation(chars: &mut std::iter::Peekable<std::str::Chars>) -> Option<Vec<String>> {
        let mut all = sequence(chars)?;
        while chars.peek() == Some(&'|') {
            chars.next();
            all.extend(sequence(chars)?);
            if all.len() > LIMIT {
                return None;
            }
        }
        Some(all)
    }
    fn sequence(chars: &mut std::iter::Peekable<std::str::Chars>) -> Option<Vec<String>> {
        let mut all = vec![String::new()];
        while let Some(&c) = chars.peek() {
            let atom = match c {
                '|' | ')' => break,
                '(' => {
                    chars.next();
                    let group = alternation(chars)?;
                    if chars.next() != Some(')') {
                        return None;
                    }
                    group
                }
                '\\' => {
                    chars.next();
                    vec![chars.next()?.to_string()]
                }
                '.' | '*' | '+' | '[' | ']' | '{' | '}' | '^' | '$' | '?' => return None,
                c => {
                    chars.next();
                    vec![c.to_string()]
                }
            };
            let atom = if chars.peek() == Some(&'?') {
                chars.next();
                let mut optional = vec![String::new()];
                optional.extend(atom);
                optional
            } else {
                atom
            };
            all = all
                .iter()
                .flat_map(|prefix| {
                    atom.iter()
                        .map(move |suffix| format!("{}{}", prefix, suffix))
                })
                .collect();
            if all.len() > LIMIT {
                return None;
            }
        }
        Some(all)
    }
    let mut chars = pattern.chars().peekable();
    let all = alternation(&mut chars)?;
    if chars.next().is_some() {
        return None;
    }
    let mut unique = vec![];
    for s in all {
        if !unique.contains(&s) {
            unique.push(s);
        }
    }
    Some(unique)
}

#[derive(Clone, ParseFilter, FilterReflection)]
#[filter(
    name = "regex_expansions",
    description = "The strings a regular expression of literals, groups, alternations, and '?'s matches, space-separated (or the input, if it has other constructs).",
    parsed(RegexExpansionsFilter)
)]
pub struct RegexExpansionsFilterParser;

#[derive(Debug, Default, Display_filter)]
#[name = "regex_expansions"]
struct RegexExpansionsFilter;

impl Filter for RegexExpansionsFilter {
    fn evaluate(&self, input: &dyn ValueView, _runtime: &dyn Runtime) -> Result<Value> {
        let pattern = input.to_kstr();
        Ok(Value::scalar(match regex_expansions(&pattern) {
            Some(all) => all.join(" "),
            None => pattern.to_string(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::regex_expansions;

    #[test]
    fn test_regex_expansions() {
        assert_eq!(
            regex_expansions("b-linux(|-docker(|-x?large)-amd|-kvm)").unwrap(),
            [
                "b-linux",
                "b-linux-docker-amd",
                "b-linux-docker-large-amd",
                "b-linux-docker-xlarge-amd",
                "b-linux-kvm"
            ]
        );
        assert_eq!(
            regex_expansions("t-linux-(talos|netperf)(-1804)?").unwrap(),
            [
                "t-linux-talos",
                "t-linux-talos-1804",
                "t-linux-netperf",
                "t-linux-netperf-1804"
            ]
        );
        assert_eq!(regex_expansions("plain").unwrap(), ["plain"]);
        assert_eq!(regex_expansions("images.*"), None);
        assert_eq!(regex_expansions("a(b"), None);
    }
}
