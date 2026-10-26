//! The tree's specs' exported definitions, which ReSpec specs' references to
//! terms they don't define are to (ex: `[=tree=]`, DOM's), found as ReSpec's
//! xref service (respec.org/xref, from webref's crawls) finds them, but among
//! the tree's specs, offline: by the term (and for concepts, its plurals and
//! other variations, case-insensitively), the specs its context cites (in
//! order), its types (concepts', IDL's, or the reference's), and what it's
//! for (or that it's for nothing).  Only a unique definition is found.

use std::collections::HashMap;

use super::SpecEntry;

/// IDL definitions' types (the xref service's).
pub const IDL_TYPES: &[&str] = &[
    "attribute",
    "callback",
    "const",
    "dict-member",
    "dictionary",
    "enum-value",
    "enum",
    "exception",
    "extended-attribute",
    "interface",
    "method",
    "typedef",
];

/// Concepts' types (the xref service's).
pub const CONCEPT_TYPES: &[&str] = &["dfn", "event", "element", "permission"];

/// An exported definition.
#[derive(Clone, Debug, PartialEq)]
pub struct Term {
    /// Its spec's name (ex: "css-grid-3").
    pub spec: String,
    /// Its spec's symbols' shortname (see `SpecEntry::shortname`).
    pub shortname: String,
    /// Its spec's series (its shortname, if it isn't in one).
    pub series: String,
    pub dfn_type: String,
    pub fors: Vec<String>,
    pub anchor: String,
}

/// A definition's export, as an analysis gives it: its linking texts, type,
/// what it's for, and its anchor.  (Saved with the index, for analyzing
/// specs' pull requests without the other specs; see spec-analyze's
/// `--terms-out` and `--terms`.)
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Export {
    pub texts: Vec<String>,
    pub dfn_type: String,
    pub fors: Vec<String>,
    pub anchor: String,
}

/// A reference to look up.
pub struct Query<'a> {
    /// Its term (as written: IDL terms are case-sensitive).
    pub term: &'a str,
    /// Its types, ex: `["_CONCEPT_"]`, `["_IDL_"]`, `["method"]`.
    pub types: &'a [&'a str],
    /// The specs its context cites, in order (the closest first), as lists
    /// of names, lowercased.
    pub specs: &'a [Vec<String>],
    /// What it's for, if it says.
    pub dfn_for: Option<&'a str>,
}

#[derive(Default)]
pub struct Terms {
    terms: Vec<Term>,
    /// Linking texts -> terms.
    by_text: HashMap<String, Vec<usize>>,
    /// Lowercased linking texts -> the linking texts.
    by_lower: HashMap<String, Vec<String>>,
}

impl Terms {
    /// Add `spec`'s exported definitions.
    pub fn add(&mut self, spec: &SpecEntry, exports: &[Export]) {
        let series = spec
            .series
            .clone()
            .unwrap_or_else(|| spec.shortname().to_string());
        for export in exports {
            let index = self.terms.len();
            self.terms.push(Term {
                spec: spec.name.clone(),
                shortname: spec.shortname().to_string(),
                series: series.clone(),
                dfn_type: export.dfn_type.clone(),
                fors: export.fors.clone(),
                anchor: export.anchor.clone(),
            });
            let mut texts = export.texts.clone();
            texts.dedup();
            for text in texts {
                let lower = text.to_lowercase();
                let exact = self.by_lower.entry(lower).or_default();
                if !exact.contains(&text) {
                    exact.push(text.clone());
                }
                self.by_text.entry(text).or_default().push(index);
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// The definition a reference is to, but in the spec `exclude` (the
    /// referring spec), if there's exactly one.
    pub fn find(&self, query: &Query, exclude: &str) -> Option<&Term> {
        let is_idl = query
            .types
            .iter()
            .any(|t| *t == "_IDL_" || IDL_TYPES.contains(t));
        let is_concept = query
            .types
            .iter()
            .any(|t| *t == "_CONCEPT_" || CONCEPT_TYPES.contains(t));
        let variations: Vec<String> = if is_concept && !is_idl {
            let lower = query.term.to_lowercase();
            let mut v = vec![query.term.to_string()];
            if lower != query.term {
                v.push(lower.clone());
            }
            v.extend(text_variations(&lower));
            v
        } else {
            vec![query.term.to_string()]
        };
        for term in variations {
            // (The term's own definitions, and then, for concepts, those of
            // its other cases.)
            let mut buckets: Vec<Vec<usize>> = vec![];
            if let Some(direct) = self.by_text.get(&term) {
                buckets.push(direct.clone());
            }
            if !is_idl && let Some(variants) = self.by_lower.get(&term.to_lowercase()) {
                let fallback: Vec<usize> = variants
                    .iter()
                    .filter(|v| **v != term)
                    .flat_map(|v| self.by_text[v].iter().copied())
                    .collect();
                if !fallback.is_empty() {
                    buckets.push(fallback);
                }
            }
            for bucket in buckets {
                let candidates: Vec<&Term> = bucket
                    .iter()
                    .map(|&i| &self.terms[i])
                    .filter(|t| t.spec != exclude && t.shortname != exclude)
                    .collect();
                let found = self.filter(query, candidates);
                if !found.is_empty() {
                    return match found[..] {
                        [term] => Some(term),
                        _ => None,
                    };
                }
            }
        }
        None
    }

    /// The candidates in the first of the query's specs with any, of its
    /// types, and for what it's for, preferring specs' latest versions.
    fn filter<'t>(&self, query: &Query, candidates: Vec<&'t Term>) -> Vec<&'t Term> {
        let by_spec: Vec<&Term> = if query.specs.is_empty() {
            candidates
        } else {
            query
                .specs
                .iter()
                .map(|specs| {
                    candidates
                        .iter()
                        .copied()
                        .filter(|t| {
                            specs.contains(&t.spec.to_lowercase())
                                || specs.contains(&t.shortname.to_lowercase())
                                || specs.contains(&t.series.to_lowercase())
                        })
                        .collect::<Vec<_>>()
                })
                .find(|found| !found.is_empty())
                .unwrap_or_default()
        };
        let is_idl = query.types.contains(&"_IDL_");
        let is_concept = query.types.contains(&"_CONCEPT_");
        let by_type = by_spec.into_iter().filter(|t| {
            query.types.is_empty()
                || query.types.contains(&t.dfn_type.as_str())
                || (is_idl && IDL_TYPES.contains(&t.dfn_type.as_str()))
                || (is_concept && CONCEPT_TYPES.contains(&t.dfn_type.as_str()))
        });
        let by_for: Vec<&Term> = by_type
            .filter(|t| match query.dfn_for {
                None | Some("") => t.fors.is_empty(),
                Some(f) => {
                    t.fors.iter().any(|tf| tf == f)
                        || (CONCEPT_TYPES.contains(&t.dfn_type.as_str())
                            && t.fors.iter().any(|tf| *tf == f.to_lowercase()))
                }
            })
            .collect();
        // (Of a series' versions' same anchors, the latest's.)
        let mut latest: Vec<&Term> = vec![];
        for term in by_for {
            match latest
                .iter_mut()
                .find(|t| t.series == term.series && t.anchor == term.anchor)
            {
                Some(t) => {
                    if version(&term.spec) > version(&t.spec) {
                        *t = term;
                    }
                }
                None => latest.push(term),
            }
        }
        latest.dedup_by(|a, b| a == b);
        latest
    }
}

/// A spec's version, the number its name ends with (ex: 3 for css-grid-3).
fn version(spec: &str) -> u32 {
    let digits: String = spec
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    digits.parse().unwrap_or(0)
}

/// A concept's term's variations, as the xref service tries them (ex:
/// "carrot" for "carrots", "parse" for "parsing").
fn text_variations(term: &str) -> Vec<String> {
    let chars: Vec<char> = term.chars().collect();
    let len = chars.len();
    let ends = |suffix: &str| term.ends_with(suffix);
    let cut = |n: usize| chars[..len.saturating_sub(n)].iter().collect::<String>();
    let last1 = chars.last().copied();
    let mut v = vec![];
    if ends("s") {
        v.push(cut(1));
    } else {
        v.push(format!("{}s", term));
    }
    if ends("ed") && len >= 4 && chars[len - 3] == chars[len - 4] {
        v.push(cut(3));
    } else if let Some(c) = last1.filter(|c| "bdfgklmnprstvz".contains(*c)) {
        v.push(format!("{}{}ed", term, c));
    }
    if ends("ed") {
        v.push(cut(2));
    } else {
        v.push(format!("{}ed", term));
    }
    if ends("d") {
        v.push(cut(1));
    } else {
        v.push(format!("{}d", term));
    }
    if ends("ing") {
        v.push(cut(3));
        v.push(format!("{}e", cut(3)));
    } else if ends("e") {
        v.push(format!("{}ing", cut(1)));
    } else {
        v.push(format!("{}ing", term));
    }
    if ends("ing") && len >= 5 && chars[len - 4] == chars[len - 5] {
        v.push(cut(4));
    } else if let Some(c) = last1.filter(|c| "bdfgkmnprstvz".contains(*c)) {
        v.push(format!("{}{}ing", term, c));
    }
    if ends("es") {
        v.push(cut(2));
    } else {
        v.push(format!("{}es", term));
    }
    if ends("ies") {
        v.push(format!("{}y", cut(3)));
    }
    if ends("y") {
        v.push(format!("{}ies", cut(1)));
    }
    if ends("ied") {
        v.push(format!("{}y", cut(3)));
    }
    if ends("y") {
        v.push(format!("{}ied", cut(1)));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str, series: Option<&str>) -> SpecEntry {
        toml::from_str(&format!(
            "name = \"{}\"\n{}",
            name,
            series.map_or(String::new(), |s| format!("series = \"{}\"\n", s))
        ))
        .unwrap()
    }

    fn export(text: &str, dfn_type: &str, fors: &[&str], anchor: &str) -> Export {
        Export {
            texts: vec![text.to_string()],
            dfn_type: dfn_type.to_string(),
            fors: fors.iter().map(|f| f.to_string()).collect(),
            anchor: anchor.to_string(),
        }
    }

    #[test]
    fn test_find() {
        let mut terms = Terms::default();
        terms.add(
            &spec("dom", None),
            &[
                export("tree", "dfn", &[], "concept-tree"),
                export("Node", "interface", &[], "interface-node"),
                export("root", "dfn", &["tree"], "concept-tree-root"),
                export("fire an event", "dfn", &[], "concept-event-fire"),
            ],
        );
        terms.add(
            &spec("html", None),
            &[
                export("tree", "dfn", &["navigable"], "nav-tree"),
                export("in parallel", "dfn", &[], "in-parallel"),
                export("root", "dfn", &[], "html-root"),
            ],
        );
        fn find(
            terms: &Terms,
            term: &str,
            types: &[&str],
            specs: &[Vec<String>],
            dfn_for: Option<&str>,
        ) -> Option<String> {
            terms
                .find(
                    &Query {
                        term,
                        types,
                        specs,
                        dfn_for,
                    },
                    "example",
                )
                .map(|t| format!("{} {}", t.shortname, t.anchor))
        }
        let concept: &[&str] = &["_CONCEPT_"];
        // (Not for anything: DOM's, not HTML's, which is for navigables.)
        assert_eq!(
            find(&terms, "tree", concept, &[], None).as_deref(),
            Some("dom concept-tree")
        );
        assert_eq!(
            find(&terms, "trees", concept, &[], None).as_deref(),
            Some("dom concept-tree")
        );
        assert_eq!(
            find(&terms, "Tree", concept, &[], None).as_deref(),
            Some("dom concept-tree")
        );
        assert_eq!(
            find(&terms, "tree", concept, &[], Some("navigable")).as_deref(),
            Some("html nav-tree")
        );
        assert_eq!(find(&terms, "firing an event", concept, &[], None), None);
        assert_eq!(
            find(&terms, "fired an event", concept, &[], None),
            None,
            "(only the term's own variations)"
        );
        assert_eq!(
            find(&terms, "Node", &["_IDL_"], &[], None).as_deref(),
            Some("dom interface-node")
        );
        assert_eq!(find(&terms, "node", &["_IDL_"], &[], None), None);
        // (Ambiguous, unless the context cites one of them.)
        assert_eq!(
            find(&terms, "root", concept, &[], Some("tree")).as_deref(),
            Some("dom concept-tree-root")
        );
        assert_eq!(
            find(&terms, "root", concept, &[], None).as_deref(),
            Some("html html-root")
        );
        terms.add(
            &spec("other", None),
            &[export("in parallel", "dfn", &[], "parallel")],
        );
        assert_eq!(find(&terms, "in parallel", concept, &[], None), None);
        assert_eq!(
            find(
                &terms,
                "in parallel",
                concept,
                &[vec!["html".to_string()]],
                None
            )
            .as_deref(),
            Some("html in-parallel")
        );
        // (A series' latest version's.)
        terms.add(
            &spec("css-grid-2", Some("css-grid")),
            &[export("grid", "dfn", &[], "grid")],
        );
        terms.add(
            &spec("css-grid-3", Some("css-grid")),
            &[export("grid", "dfn", &[], "grid")],
        );
        assert_eq!(
            find(&terms, "grid", concept, &[], None).as_deref(),
            Some("css-grid-3 grid")
        );
    }
}
