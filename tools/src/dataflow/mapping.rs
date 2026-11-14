//! What dataflow traces treat as black boxes (see `trace`), from
//! dataflow-mapping.toml (config_defaults', unless the config repo has its
//! own; see the file for what it says).  It's read when a trace runs, rather
//! than when indexing, so that changing it doesn't need a reindex.
//!
//! A black box's functions (ex: mfbt's `Result` and its helpers, or XPCOM's
//! component factories) are machinery the code's logic goes through rather
//! than logic of its own: a trace doesn't step into a call of one, but takes
//! its result (and for a constructor, the object it makes) as coming from its
//! arguments and object, as it does for calls it can't see into; and its
//! diagrams show a group's calls as one collapsed node, whose context menu
//! can show their internals (the trace again, stepping into them; see
//! `TraceOpts::internals`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use super::summary::untemplated_pretty;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MappingToml {
    #[serde(default, rename = "black-box")]
    black_boxes: Vec<BlackBoxToml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BlackBoxToml {
    group: String,
    name: String,
    why: String,
    functions: Vec<String>,
}

/// A group of functions a trace doesn't step into (see the module's
/// documentation).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BlackBox {
    /// Its key, for `TraceOpts::internals` (ex: "result").
    pub group: String,
    /// Its name, for people (ex: "Result machinery").
    pub name: String,
    pub why: String,
}

pub struct DataflowMapping {
    pub black_boxes: Vec<BlackBox>,
    /// A hash of its text (see `fingerprint`).
    fingerprint: String,
    /// The black boxes' functions, by their pretty names without template
    /// arguments (see `untemplated_pretty`).
    functions: HashMap<String, usize>,
    /// The black boxes' prefixes (ending in "::", a class's or namespace's
    /// functions), longest first.
    prefixes: Vec<(String, usize)>,
}

impl DataflowMapping {
    pub fn parse(text: &str) -> Result<DataflowMapping, String> {
        let toml: MappingToml = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut mapping = DataflowMapping {
            black_boxes: vec![],
            fingerprint: {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                text.hash(&mut hasher);
                format!("{:016x}", hasher.finish())
            },
            functions: HashMap::new(),
            prefixes: vec![],
        };
        for (i, bb) in toml.black_boxes.into_iter().enumerate() {
            if mapping
                .black_boxes
                .iter()
                .any(|other| other.group == bb.group)
            {
                return Err(format!("black box group {:?} is defined twice", bb.group));
            }
            for function in bb.functions {
                let function = untemplated_pretty(function.trim());
                if function.ends_with("::") {
                    mapping.prefixes.push((function, i));
                } else {
                    mapping.functions.insert(function, i);
                }
            }
            mapping.black_boxes.push(BlackBox {
                group: bb.group,
                name: bb.name,
                why: bb.why,
            });
        }
        mapping
            .prefixes
            .sort_by_key(|(prefix, _)| std::cmp::Reverse(prefix.len()));
        Ok(mapping)
    }

    /// The mapping of dataflow-mapping.toml's `text`, parsed once for each
    /// text (so that a changed file's mapping is its new one), or None if it
    /// doesn't parse (which it logs).
    pub fn cached(text: &str) -> Option<Arc<DataflowMapping>> {
        type Cached = Option<(String, Option<Arc<DataflowMapping>>)>;
        static CACHED: Mutex<Cached> = Mutex::new(None);
        let mut cached = CACHED.lock().unwrap();
        if let Some((cached_text, mapping)) = cached.as_ref()
            && cached_text == text
        {
            return mapping.clone();
        }
        let mapping = match DataflowMapping::parse(text) {
            Ok(mapping) => Some(Arc::new(mapping)),
            Err(e) => {
                error!("dataflow-mapping.toml: {}", e);
                None
            }
        };
        *cached = Some((text.to_string(), mapping.clone()));
        mapping
    }

    /// A hash of its text, for caching what traces find (see `step_cache`).
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// The black box a function is in, by its pretty name (with or without
    /// template arguments), if any.
    pub fn black_box(&self, pretty: &str) -> Option<&BlackBox> {
        let pretty = untemplated_pretty(pretty);
        let i = self.functions.get(&pretty).copied().or_else(|| {
            self.prefixes
                .iter()
                .find(|(prefix, _)| pretty.starts_with(prefix.as_str()))
                .map(|(_, i)| *i)
        })?;
        self.black_boxes.get(i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOML: &str = r#"
[[black-box]]
group = "result"
name = "Result machinery"
why = "Wrapping."
functions = ["mozilla::Result::", "mozilla::ToResult", "mozilla::detail::ResultImplementation::"]
"#;

    #[test]
    fn test_black_box_matching() {
        let mapping = DataflowMapping::parse(TOML).unwrap();
        let group = |pretty: &str| mapping.black_box(pretty).map(|bb| bb.group.as_str());
        assert_eq!(group("mozilla::Result::unwrap"), Some("result"));
        assert_eq!(group("mozilla::Result<V, E>::isErr"), Some("result"));
        assert_eq!(group("mozilla::ToResult"), Some("result"));
        assert_eq!(
            group(
                "mozilla::detail::ResultImplementation<type-parameter-0-0, type-parameter-0-1, mozilla::detail::PackingStrategy::Variant>::isOk"
            ),
            Some("result")
        );
        // (Whole names, or prefixes ending in "::".)
        assert_eq!(group("mozilla::ToResultInvoke"), None);
        assert_eq!(group("mozilla::ResultExtensions"), None);
    }

    #[test]
    fn test_duplicate_groups() {
        let twice = format!("{}{}", TOML, TOML);
        assert!(DataflowMapping::parse(&twice).is_err());
    }

    /// config_defaults/dataflow-mapping.toml parses.
    #[test]
    fn test_defaults_parse() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../config_defaults/dataflow-mapping.toml"
        );
        let Ok(text) = std::fs::read_to_string(path) else {
            eprintln!("Skipping: no config_defaults/dataflow-mapping.toml here");
            return;
        };
        let mapping = DataflowMapping::parse(&text).unwrap();
        assert!(mapping.black_box("mozilla::Result::unwrap").is_some());
    }
}
