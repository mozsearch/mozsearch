//! MDN's browser-compat-data (BCD, https://github.com/mdn/browser-compat-data),
//! whose features (ex: `api.Node.appendChild`, `css.properties.gap`) have, as
//! curated by MDN, their specs' URLs (`spec_url`) and their MDN pages'
//! (`mdn_url`).  spec-analyze gives, in supplement records (see
//! `AnalysisStructured::supplement`):
//! - The tree's WebIDL symbols (webidl-analyze's `WEBIDL_<interface>` and
//!   `WEBIDL_<interface>_<member>`, by their interfaces' and members' names,
//!   with a mixin's members by the interfaces including it, which BCD lists
//!   them under) the definitions in the specs which they reflect, as `spec`
//!   binding slots (and doc-uses of them, in the WebIDL symbols), and their
//!   MDN pages, as `docs`.
//! - The tree's specs' definitions (a spec tree's) their MDN pages.
//!
//! Only as BCD says, never by matching names with the specs' definitions, and
//! only for spec URLs whose anchors the specs define.

use std::collections::HashMap;
use std::path::Path;

use lazy_static::lazy_static;
use regex::Regex;
use serde_json::{Value, json};

use super::SpecsConfig;
use super::spec_symbol;
use crate::file_format::analysis::{AnalysisKind, Location, read_analysis, read_structured};
use crate::semantic_analyzer::{Record, target_record};

lazy_static! {
    /// A WebIDL `includes` statement: the interface, and the mixin.
    static ref INCLUDES: Regex = Regex::new(r"(\w+)\s+includes\s+(\w+)\s*;").unwrap();
    /// A WebIDL comment.
    static ref COMMENT: Regex = Regex::new(r"(?s)//[^\n]*|/\*.*?\*/").unwrap();
}

/// A BCD feature's spec URLs and MDN page.
#[derive(Debug, Default, PartialEq)]
pub struct Feature {
    pub spec_urls: Vec<String>,
    pub mdn_url: Option<String>,
}

/// BCD's features, by their paths (ex: "api.Node.appendChild").
#[derive(Debug, Default)]
pub struct Bcd {
    pub features: HashMap<String, Feature>,
}

impl Bcd {
    /// BCD's `data.json` (its npm package's), at `path`.
    pub fn load(path: &Path) -> Result<Bcd, String> {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("{}: {}", path.display(), e))?;
        Bcd::parse(&text).map_err(|e| format!("{}: {}", path.display(), e))
    }

    pub fn parse(text: &str) -> Result<Bcd, String> {
        let data: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let mut bcd = Bcd::default();
        let Value::Object(top) = data else {
            return Err("not an object".to_string());
        };
        for (key, value) in &top {
            // (Not the browsers' data, or the release's.)
            if key == "browsers" || key.starts_with("__") {
                continue;
            }
            bcd.add(key, value);
        }
        Ok(bcd)
    }

    fn add(&mut self, path: &str, value: &Value) {
        let Value::Object(object) = value else {
            return;
        };
        if let Some(compat) = object.get("__compat") {
            let spec_urls = match compat.get("spec_url") {
                Some(Value::String(url)) => vec![url.clone()],
                Some(Value::Array(urls)) => urls
                    .iter()
                    .filter_map(|url| url.as_str().map(str::to_string))
                    .collect(),
                _ => vec![],
            };
            let mdn_url = compat
                .get("mdn_url")
                .and_then(|url| url.as_str())
                .map(str::to_string);
            self.features
                .insert(path.to_string(), Feature { spec_urls, mdn_url });
        }
        for (key, child) in object {
            if !key.starts_with("__") {
                self.add(&format!("{}.{}", path, key), child);
            }
        }
    }
}

/// The definition in the specs which a spec URL is of (as citations resolve
/// them, see `citations::citation_records`), if `defined` (by its symbol,
/// ex: with a structured record, see `Defined::has_meta`).  A URL of a
/// Bikeshed link's id (ex: BCD's of `Element.append`, "#ref-for-dom-parentnode-
/// append①", in its IDL block), is of the definition it links to.
pub fn url_definition(
    url: &str,
    config: &SpecsConfig,
    defined: &dyn Fn(&str) -> bool,
) -> Option<String> {
    let (spec, anchor, _) = config.url_citation(url)?;
    let anchor = anchor.split(":~:").next().unwrap_or_default();
    if anchor.is_empty() {
        return None;
    }
    let linked = anchor
        .strip_prefix("ref-for-")
        .map(|id| id.trim_end_matches(|c: char| !c.is_ascii()));
    let shortnames: Vec<&str> = std::iter::once(spec.shortname())
        .chain(config.fallbacks(spec))
        .collect();
    linked
        .into_iter()
        .chain(std::iter::once(anchor))
        .flat_map(|anchor| {
            shortnames
                .iter()
                .map(move |shortname| spec_symbol(shortname, anchor))
        })
        .find(|sym| defined(sym))
}

/// The MDN pages' docs entries for `urls`.
fn mdn_docs(urls: &[&String]) -> Vec<Value> {
    let mut docs: Vec<Value> = vec![];
    for url in urls {
        let doc = json!({ "site": "mdn", "url": url });
        if !docs.contains(&doc) {
            docs.push(doc);
        }
    }
    docs
}

/// What `webidl_supplements` found, for reporting.
#[derive(Debug, Default)]
pub struct WebIdlStats {
    /// The WebIDL symbols.
    pub symbols: usize,
    /// Of those, those BCD has.
    pub in_bcd: usize,
    /// Of those, those with a spec definition, and with MDN pages.
    pub spec: usize,
    pub mdn: usize,
}

/// The supplement records of the tree's WebIDL files' symbols (`webidl`, by
/// their paths and sources, in the analysis webidl-analyze wrote in
/// `analysis_root`), by their files: their spec definitions (as `defined`,
/// see `url_definition`) and MDN pages, as BCD has them.
pub fn webidl_supplements(
    bcd: &Bcd,
    webidl: &[(String, String)],
    analysis_root: &Path,
    config: &SpecsConfig,
    defined: &dyn Fn(&str) -> bool,
) -> (Vec<(String, Vec<Record>)>, WebIdlStats) {
    let mut stats = WebIdlStats::default();
    // The interfaces including each mixin.
    let mut including: HashMap<String, Vec<String>> = HashMap::new();
    for (_, source) in webidl {
        let source = COMMENT.replace_all(source, "");
        for m in INCLUDES.captures_iter(&source) {
            including
                .entry(m[2].to_string())
                .or_default()
                .push(m[1].to_string());
        }
    }
    let mut files = vec![];
    for (path, _) in webidl {
        let analysis = analysis_root.join(path);
        if !analysis.exists() {
            continue;
        }
        let mut records = vec![];
        for structured in read_analysis(&analysis.to_string_lossy(), &mut read_structured) {
            for data in &structured.data {
                if data.supplement || &*data.impl_kind != "idl" || !data.sym.starts_with("WEBIDL_")
                {
                    continue;
                }
                let Some((slot_kind, paths)) = bcd_paths(&data.kind, &data.pretty, &including)
                else {
                    continue;
                };
                stats.symbols += 1;
                let features: Vec<&Feature> =
                    paths.iter().filter_map(|p| bcd.features.get(p)).collect();
                if features.is_empty() {
                    continue;
                }
                stats.in_bcd += 1;
                let spec = features
                    .iter()
                    .flat_map(|f| &f.spec_urls)
                    .find_map(|url| url_definition(url, config, defined));
                let mdn: Vec<&String> =
                    features.iter().filter_map(|f| f.mdn_url.as_ref()).collect();
                stats.spec += usize::from(spec.is_some());
                stats.mdn += usize::from(!mdn.is_empty());
                if spec.is_none() && mdn.is_empty() {
                    continue;
                }
                // (And it's a doc-use of the spec's definition, in the WebIDL
                // symbol, ex: for the definition's steps' view's code citing
                // it, and calls diagrams, where it implements the definition.)
                if let Some(spec_sym) = &spec {
                    let pretty = config.symbol_spec(spec_sym).map_or_else(
                        || spec_sym.clone(),
                        |(spec, anchor)| format!("{} #{}", spec.shortname(), anchor),
                    );
                    records.push(target_record(
                        structured.loc,
                        AnalysisKind::DocUse,
                        pretty,
                        spec_sym.clone(),
                        data.pretty.to_string(),
                        data.sym.to_string(),
                    ));
                }
                records.push(supplement(
                    structured.loc,
                    &data.sym,
                    spec.map(|sym| (slot_kind, sym)),
                    mdn_docs(&mdn),
                ));
            }
        }
        if !records.is_empty() {
            files.push((path.clone(), records));
        }
    }
    (files, stats)
}

/// The BCD paths of a WebIDL symbol, by its structured record's kind and
/// pretty name ("Interface" or "Interface::member"), and the kind of its
/// spec's definition's slot: its interface's, or the interfaces including it
/// if it's a mixin, which BCD lists its members under.
fn bcd_paths(
    kind: &str,
    pretty: &str,
    including: &HashMap<String, Vec<String>>,
) -> Option<(&'static str, Vec<String>)> {
    let (interface, member) = match pretty.split_once("::") {
        Some((interface, member)) => (interface, Some(member)),
        None => (pretty, None),
    };
    let slot_kind = match (kind, member) {
        ("class", None) => "interface",
        ("method", Some(_)) => "method",
        ("field", Some(_)) => "attribute",
        ("const", Some(_)) => "const",
        _ => return None,
    };
    let interfaces: Vec<&str> = std::iter::once(interface)
        .chain(
            including
                .get(interface)
                .into_iter()
                .flatten()
                .map(String::as_str),
        )
        .collect();
    let paths = interfaces
        .iter()
        .map(|interface| match member {
            // (BCD's constructors are named after their interfaces.)
            Some("constructor") => format!("api.{}.{}", interface, interface),
            Some(member) => format!("api.{}.{}", interface, member),
            None => format!("api.{}", interface),
        })
        .collect();
    Some((slot_kind, paths))
}

/// A supplement record of `sym`, with the spec definition it reflects (and
/// its slot's kind), and its docs.
fn supplement(loc: Location, sym: &str, spec: Option<(&str, String)>, docs: Vec<Value>) -> Record {
    let mut value = json!({
        "loc": loc,
        "structured": 1,
        "supplement": true,
        "sym": sym,
    });
    if let Some((slot_kind, spec_sym)) = spec {
        value["bindingSlots"] = json!([{
            "slotKind": slot_kind,
            "slotLang": "spec",
            "ownerLang": "idl",
            "sym": spec_sym,
        }]);
    }
    if !docs.is_empty() {
        value["docs"] = Value::Array(docs);
    }
    Record::Structured(loc, value)
}

/// The supplement records of the specs' definitions (`definitions`, by their
/// symbols: their files and locations, of those with metas), by their files:
/// their MDN pages, for the BCD features whose spec URLs are of them.  Also returns how many
/// definitions have pages.
pub fn spec_supplements(
    bcd: &Bcd,
    config: &SpecsConfig,
    definitions: &HashMap<String, (String, Location)>,
    has_meta: &dyn Fn(&str) -> bool,
) -> (Vec<(String, Vec<Record>)>, usize) {
    let defined = |sym: &str| definitions.contains_key(sym) && has_meta(sym);
    let mut pages: HashMap<String, Vec<&String>> = HashMap::new();
    for feature in bcd.features.values() {
        let Some(mdn_url) = &feature.mdn_url else {
            continue;
        };
        for url in &feature.spec_urls {
            if let Some(sym) = url_definition(url, config, &defined) {
                pages.entry(sym).or_default().push(mdn_url);
            }
        }
    }
    let mut by_file: HashMap<String, Vec<Record>> = HashMap::new();
    let count = pages.len();
    for (sym, mut urls) in pages {
        let (file, loc) = &definitions[&sym];
        urls.sort();
        by_file.entry(file.clone()).or_default().push(supplement(
            *loc,
            &sym,
            None,
            mdn_docs(&urls),
        ));
    }
    let mut files: Vec<_> = by_file.into_iter().collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    // (In a stable order, since `pages`' isn't.)
    for (_, records) in &mut files {
        records.sort_by_key(|record| match record {
            Record::Structured(loc, value) => {
                (*loc, value["sym"].as_str().unwrap_or("").to_string())
            }
            _ => (Location::default(), String::new()),
        });
    }
    (files, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse() {
        let bcd = Bcd::parse(
            r#"{
            "__meta": {"version": "1"},
            "browsers": {"firefox": {"__compat": {"mdn_url": "x"}}},
            "api": {
                "Node": {
                    "__compat": {"spec_url": "https://dom.spec.whatwg.org/#interface-node", "mdn_url": "https://developer.mozilla.org/docs/Web/API/Node"},
                    "appendChild": {"__compat": {"spec_url": ["https://dom.spec.whatwg.org/#dom-node-appendchild", "https://example.org/#x"]}}
                }
            }
        }"#,
        )
        .unwrap();
        assert_eq!(bcd.features.len(), 2);
        assert_eq!(
            bcd.features["api.Node.appendChild"].spec_urls,
            vec![
                "https://dom.spec.whatwg.org/#dom-node-appendchild",
                "https://example.org/#x"
            ]
        );
        assert_eq!(
            bcd.features["api.Node"].mdn_url.as_deref(),
            Some("https://developer.mozilla.org/docs/Web/API/Node")
        );
    }

    #[test]
    fn test_bcd_paths() {
        let including = HashMap::from([(
            "ParentNode".to_string(),
            vec!["Document".to_string(), "Element".to_string()],
        )]);
        assert_eq!(
            bcd_paths("method", "ParentNode::append", &including),
            Some((
                "method",
                vec![
                    "api.ParentNode.append".to_string(),
                    "api.Document.append".to_string(),
                    "api.Element.append".to_string()
                ]
            ))
        );
        assert_eq!(
            bcd_paths("method", "Node::constructor", &including),
            Some(("method", vec!["api.Node.Node".to_string()]))
        );
        assert_eq!(
            bcd_paths("class", "Node", &including),
            Some(("interface", vec!["api.Node".to_string()]))
        );
        assert_eq!(bcd_paths("field", "Node", &including), None);
    }

    #[test]
    fn test_url_definition() {
        let config: SpecsConfig = toml::from_str(
            "[[spec]]\nname = \"dom\"\nrendered = \"https://dom.spec.whatwg.org/\"\n",
        )
        .unwrap();
        let defined = |sym: &str| sym == "SPEC_dom_dom@2Dnode@2Dappendchild";
        assert_eq!(
            url_definition(
                "https://dom.spec.whatwg.org/#dom-node-appendchild",
                &config,
                &defined
            )
            .as_deref(),
            Some("SPEC_dom_dom@2Dnode@2Dappendchild")
        );
        // Not if the spec doesn't define it, or isn't one of the specs.
        assert_eq!(
            url_definition(
                "https://dom.spec.whatwg.org/#dom-node-gone",
                &config,
                &defined
            ),
            None
        );
        assert_eq!(
            url_definition("https://example.org/#x", &config, &defined),
            None
        );
        // A link's id is of what it links to.
        assert_eq!(
            url_definition(
                "https://dom.spec.whatwg.org/#ref-for-dom-node-appendchild\u{2460}",
                &config,
                &defined
            )
            .as_deref(),
            Some("SPEC_dom_dom@2Dnode@2Dappendchild")
        );
    }
}
