//! A file's analysis records, indexed by where they start, for joining them to
//! tree-sitter nodes (whose rows are 0-based and columns are bytes; records'
//! lines are 1-based and columns are bytes, for clang's and scip's).

use std::collections::HashMap;

use serde_json::Value;
use ustr::{Ustr, ustr};

use crate::file_format::analysis::parse_location;

/// A source record (or the parts of one we use).
#[derive(Clone, Debug, Default)]
pub struct SourceRec {
    pub line: u32,
    pub col: u32,
    pub col_end: u32,
    pub syms: Vec<Ustr>,
    /// The pretty name without the syntax kind prefix (ex: "aRetVal" for
    /// "variable aRetVal").
    pub pretty: String,
    pub syntax: Vec<String>,
    pub type_pretty: Option<String>,
    pub type_sym: Option<Ustr>,
    /// Whether it's in a macro expansion at its location (its symbol isn't
    /// spelled there).
    pub in_expansion: bool,
    /// For a macro expansion's records: the expansions' keys (the macros'
    /// symbols) and platforms, and the byte offsets in that expansion's text
    /// where its symbol is spelled.
    pub expansion_offsets: Vec<(String, String, Vec<usize>)>,
    /// For a macro's record: its expansions' keys and platforms, and texts.
    pub expands_to: Vec<(String, String, String)>,
}

impl SourceRec {
    pub fn has_syntax(&self, kind: &str) -> bool {
        self.syntax.iter().any(|s| s == kind)
    }

    pub fn sym(&self) -> Ustr {
        self.syms.first().copied().unwrap_or_default()
    }

    /// The last segment of its pretty name (ex: "GetBoolPref" for
    /// "nsPrefBranch::GetBoolPref").
    pub fn short_name(&self) -> &str {
        short_name(&self.pretty)
    }

    /// Whether it's the definition of a field of a lambda's class, its
    /// capture's (ex: "PrepareConnInfoForRetry::(lambda class1)::(anonymous)",
    /// `this`'s).
    pub fn is_lambda_capture(&self) -> bool {
        self.has_syntax("def") && self.has_syntax("field") && self.pretty.contains("(lambda class")
    }
}

/// The last segment of a pretty name.
pub fn short_name(pretty: &str) -> &str {
    let pretty = pretty.rsplit("::").next().unwrap_or(pretty);
    pretty.rsplit('#').next().unwrap_or(pretty)
}

/// The type of a JVM value (an argument, field or local) named `name`, from
/// its record's type, which scip-kotlinc and scip-javac give as its
/// declaration (ex: "input: String", "private final val settings:
/// DataflowSettings", "public static final int javaConst"), and SemanticDB's
/// scip-java as its type (ex: "String", "int"), as it is.
fn jvm_value_type(declaration: &str, name: &str) -> String {
    let declaration = declaration.trim();
    // (Kotlin's: `[modifiers] [val|var] name: Type`.)
    if let Some((head, ty)) = declaration.split_once(": ")
        && head.split_whitespace().last() == Some(name)
    {
        return ty.to_string();
    }
    // (Java's: `[modifiers] Type name`.)
    let words: Vec<&str> = declaration.split_whitespace().collect();
    if let [ty @ .., last] = &words[..]
        && *last == name
    {
        let ty: Vec<&str> = ty
            .iter()
            .copied()
            .skip_while(|word| {
                matches!(
                    *word,
                    "public"
                        | "protected"
                        | "private"
                        | "static"
                        | "final"
                        | "transient"
                        | "volatile"
                )
            })
            .collect();
        if !ty.is_empty() {
            return ty.join(" ");
        }
    }
    declaration.to_string()
}

/// A target record (or the parts of one we use).
#[derive(Clone, Debug, Default)]
pub struct TargetRec {
    pub line: u32,
    pub col: u32,
    pub kind: String,
    pub sym: Ustr,
    pub pretty: String,
    pub contextsym: Ustr,
    /// The arguments' ranges, for calls: (line, column) of their first tokens
    /// and of their last tokens' starts.
    pub arg_ranges: Vec<((u32, u32), (u32, u32))>,
}

#[derive(Default)]
pub struct FileRecords {
    sources: HashMap<(u32, u32), Vec<SourceRec>>,
    targets: HashMap<(u32, u32), Vec<TargetRec>>,
    structured: HashMap<Ustr, Value>,
    /// Function definitions' source records, in order.
    function_defs: Vec<(u32, u32)>,
}

fn str_field(obj: &Value, key: &str) -> String {
    obj.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn parse_point(s: &str) -> Option<(u32, u32)> {
    let (line, col) = s.split_once(':')?;
    Some((line.parse().ok()?, col.parse().ok()?))
}

impl FileRecords {
    pub fn from_values(values: impl IntoIterator<Item = Value>) -> Self {
        let mut recs = FileRecords::default();
        for obj in values {
            let Some(loc) = obj.get("loc").and_then(|v| v.as_str()) else {
                continue;
            };
            let loc = parse_location(loc);
            let key = (loc.lineno, loc.col_start);
            if obj.get("source").is_some() {
                let pretty = str_field(&obj, "pretty");
                // (Pretty names are "<syntax kind> <name>", ex: "variable
                // aRetVal", except for some producers'.)
                let pretty = match pretty.split_once(' ') {
                    Some((_, rest)) => rest.to_string(),
                    None => pretty,
                };
                let syms: Vec<Ustr> = str_field(&obj, "sym")
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(ustr)
                    .collect();
                let syntax: Vec<String> = str_field(&obj, "syntax")
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect();
                // (A JVM value's type, from its declaration; see
                // `jvm_value_type`.)
                let jvm_value = syms.first().is_some_and(|s| s.starts_with("S_jvm_"))
                    && syntax
                        .iter()
                        .any(|s| matches!(s.as_str(), "arg" | "field" | "variable" | "local"));
                let type_pretty = obj.get("type").and_then(|v| v.as_str()).map(|ty| {
                    if jvm_value {
                        jvm_value_type(ty, short_name(&pretty))
                    } else {
                        ty.to_string()
                    }
                });
                let mut expansion_offsets = vec![];
                if let Some(at) = obj.get("inExpansionAt").and_then(|v| v.as_object()) {
                    for (key, platforms) in at {
                        let Some(platforms) = platforms.as_object() else {
                            continue;
                        };
                        for (platform, offsets) in platforms {
                            let offsets = offsets
                                .as_array()
                                .map(|a| {
                                    a.iter()
                                        .filter_map(|o| o.as_u64().map(|o| o as usize))
                                        .collect()
                                })
                                .unwrap_or_default();
                            expansion_offsets.push((key.clone(), platform.clone(), offsets));
                        }
                    }
                }
                let mut expands_to = vec![];
                if let Some(to) = obj.get("expandsTo").and_then(|v| v.as_object()) {
                    for (key, platforms) in to {
                        let Some(platforms) = platforms.as_object() else {
                            continue;
                        };
                        for (platform, text) in platforms {
                            if let Some(text) = text.as_str() {
                                expands_to.push((key.clone(), platform.clone(), text.to_string()));
                            }
                        }
                    }
                }
                let rec = SourceRec {
                    line: loc.lineno,
                    col: loc.col_start,
                    col_end: loc.col_end,
                    syms,
                    pretty,
                    syntax,
                    type_pretty,
                    type_sym: obj.get("typesym").and_then(|v| v.as_str()).map(ustr),
                    in_expansion: !expansion_offsets.is_empty(),
                    expansion_offsets,
                    expands_to,
                };
                if rec.has_syntax("def")
                    && (rec.has_syntax("function") || rec.has_syntax("method"))
                    && !rec.in_expansion
                {
                    recs.function_defs.push(key);
                }
                recs.sources.entry(key).or_default().push(rec);
            } else if obj.get("target").is_some() {
                let arg_ranges = obj
                    .get("argRanges")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|r| {
                                let (start, end) = r.as_str()?.split_once('-')?;
                                Some((parse_point(start)?, parse_point(end)?))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                recs.targets.entry(key).or_default().push(TargetRec {
                    line: loc.lineno,
                    col: loc.col_start,
                    kind: str_field(&obj, "kind"),
                    sym: ustr(&str_field(&obj, "sym")),
                    pretty: str_field(&obj, "pretty"),
                    contextsym: ustr(&str_field(&obj, "contextsym")),
                    arg_ranges,
                });
            } else if obj.get("structured").is_some() {
                let sym = ustr(&str_field(&obj, "sym"));
                recs.structured.insert(sym, obj);
            }
        }
        // (A lambda's implicit capture's field is defined where the lambda
        // first uses what it captures, ex: `this`'s at `mLock` in `[&] {
        // MutexAutoLock lock(mLock); }`: the name is the other record's.)
        for sources in recs.sources.values_mut() {
            sources.sort_by_key(|rec| rec.is_lambda_capture());
        }
        recs.function_defs.sort();
        recs.function_defs.dedup();
        recs
    }

    /// The source records starting at a position (1-based line, byte column)
    /// whose symbols are spelled there.
    pub fn sources_at(&self, line: u32, col: u32) -> impl Iterator<Item = &SourceRec> {
        self.sources
            .get(&(line, col))
            .into_iter()
            .flatten()
            .filter(|r| !r.in_expansion)
    }

    /// All the source records starting at a position, including macro
    /// expansions'.
    pub fn all_sources_at(&self, line: u32, col: u32) -> &[SourceRec] {
        self.sources
            .get(&(line, col))
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn targets_at(&self, line: u32, col: u32) -> &[TargetRec] {
        self.targets
            .get(&(line, col))
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn structured(&self, sym: &Ustr) -> Option<&Value> {
        self.structured.get(sym)
    }

    /// The source records of functions defined in macro expansions (ex:
    /// `NS_IMPL_ADDREF(Foo)`'s `Foo::AddRef`), and of the macros (with
    /// their expansions) at the same places.
    pub fn macro_function_defs(&self) -> Vec<(&SourceRec, &SourceRec)> {
        let mut out = vec![];
        for recs in self.sources.values() {
            let Some(mac) = recs.iter().find(|r| !r.expands_to.is_empty()) else {
                continue;
            };
            for rec in recs {
                if rec.in_expansion
                    && rec.has_syntax("def")
                    && (rec.has_syntax("function") || rec.has_syntax("method"))
                {
                    out.push((rec, mac));
                }
            }
        }
        out.sort_by_key(|(rec, _)| (rec.line, rec.col, rec.sym()));
        out
    }

    /// The symbols defined or declared on a line (ex: a new field's, a new
    /// method's), not in macro expansions.
    pub fn symbols_defined_at(&self, line: u32) -> Vec<Ustr> {
        let mut out: Vec<Ustr> = self
            .sources
            .iter()
            .filter(|((l, _), _)| *l == line)
            .flat_map(|(_, recs)| recs.iter())
            .filter(|r| !r.in_expansion && (r.has_syntax("def") || r.has_syntax("decl")))
            .flat_map(|r| r.syms.iter().copied())
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// The symbols used (or defined) on a line, not in macro expansions.
    pub fn symbols_at(&self, line: u32) -> Vec<Ustr> {
        let mut out: Vec<Ustr> = self
            .sources
            .iter()
            .filter(|((l, _), _)| *l == line)
            .flat_map(|(_, recs)| recs.iter())
            .filter(|r| !r.in_expansion)
            .flat_map(|r| r.syms.iter().copied())
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// The source records of the functions defined in the file.
    pub fn function_defs(&self) -> impl Iterator<Item = &SourceRec> {
        self.function_defs.iter().filter_map(|key| {
            self.sources.get(key)?.iter().find(|r| {
                r.has_syntax("def")
                    && (r.has_syntax("function") || r.has_syntax("method"))
                    && !r.in_expansion
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jvm_value_type() {
        // (scip-kotlinc's and scip-javac's declarations.)
        assert_eq!(jvm_value_type("input: String", "input"), "String");
        assert_eq!(
            jvm_value_type("private final val settings: DataflowSettings", "settings"),
            "DataflowSettings"
        );
        assert_eq!(
            jvm_value_type("local val trimmed: String", "trimmed"),
            "String"
        );
        assert_eq!(
            jvm_value_type("callback: (String) -> Unit", "callback"),
            "(String) -> Unit"
        );
        assert_eq!(
            jvm_value_type("public static final int javaConst", "javaConst"),
            "int"
        );
        assert_eq!(
            jvm_value_type("private final Map<String, Integer> counts", "counts"),
            "Map<String, Integer>"
        );
        // (SemanticDB's types, as they are.)
        assert_eq!(jvm_value_type("UrlSink", "sink"), "UrlSink");
        assert_eq!(jvm_value_type("int", "javaField"), "int");
    }

    #[test]
    fn test_lambda_capture_last() {
        // (clang's records of `mLock` in `[&] { MutexAutoLock lock(mLock); }`,
        // the first use of `this` in the lambda.)
        let recs = FileRecords::from_values([
            serde_json::json!({
                "loc": "01293:25-30", "source": 1, "syntax": "def,field",
                "pretty": "field Foo::Run::(lambda class1)::(anonymous)",
                "sym": "F_<T_834392f02c56cd99>_", "type": "class Foo *",
            }),
            serde_json::json!({
                "loc": "01293:25-30", "source": 1, "syntax": "field,use",
                "pretty": "field Foo::mLock", "sym": "F_<T_Foo>_mLock",
                "type": "mozilla::Mutex",
            }),
        ]);
        let syms: Vec<Ustr> = recs.sources_at(1293, 25).map(|rec| rec.sym()).collect();
        assert_eq!(
            syms,
            vec![ustr("F_<T_Foo>_mLock"), ustr("F_<T_834392f02c56cd99>_")]
        );
    }
}
