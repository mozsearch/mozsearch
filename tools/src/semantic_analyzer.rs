//! Configurable tree-sitter-based analysis of data formats (see
//! data-semantics.md in the notes): mappings (TOML, in
//! `languages/semantic_mappings/`) say which files a grammar parses and which
//! of their nodes are definitions and uses of which symbols, with tree-sitter
//! queries or data paths, and Liquid templates, and `semantic-analyze` turns
//! their matches into analysis records.
//!
//! A mapping has:
//! - `grammar`: the grammar which parses the files (ex: "fluent", "yaml",
//!   "mozcpp", "rust", or "typescript", which also parses JS), and `paths`,
//!   globs of the files it applies to (ex: "**/*.ftl"), except for
//!   `exclude_paths`, and if `contains` has any strings, only files which
//!   contain one of them (so that, ex, a mapping for C++'s uses of prefs only
//!   parses the files mentioning `Preferences::`);
//! - `when`: conditions on the document's data which must all hold for the
//!   mapping to apply to a file (ex: Glean metrics files' `$schema`);
//! - `scopes`: queries capturing an enclosing node as `@scope` and its name as
//!   `@name` (ex: a Fluent message), with templates for its symbol and pretty
//!   name, which are the context of the records inside it (and which the
//!   templates of the scopes and rules inside it can use);
//! - `rules`: queries capturing the node of a definition or use as `@id`
//!   (other captures are template variables too), with templates for its
//!   symbol and pretty name, and its syntax kind (the source records' pretty
//!   prefix, ex: "message"), and optionally its target records' `kind`, if
//!   not the role's (ex: "doc-def").  A definition can have a `nesting`
//!   capture, whose node's lines are the definition's nesting range (for
//!   sticky headers).  With a `regex`, a rule's records are at its matches in
//!   the `@id` node's text (ex: a comment's), at their `id` groups (or the
//!   whole matches, if it has none), whose named groups are template
//!   variables too; matches without the `id` group have no records.  A use
//!   can `resolve` to a definition in any file (see `Resolve`), for which its
//!   `sym` is the definitions' symbols' prefix.
//! - `data`: rules for data paths (for grammars with a data view, which is
//!   YAML's maps, sequences, and scalars), whose records are at the keys or the
//!   scalar values of the entries the paths match.  A path's segments are a
//!   key, `*` (any key of a map), `[]` (any item of a sequence), or `**` (any
//!   number of levels), separated by `/` (ex: `[]/name` is each top-level
//!   item's `name`), and a key or `*` can require its value to be a map with
//!   a key and scalar value (ex: `run[using=mozharness]/script`).  A rule can
//!   leave out entries by their keys or values (`except_keys`,
//!   `except_values`), and with `each` (a
//!   template whose output's words are `item`s), have records for each of
//!   several symbols.  Besides definitions and uses, a data rule can be for
//!   keys (source records only, with the `key` syntax, and with a nesting
//!   range for a value spanning lines), and its definitions can have
//!   structured records with binding slots (by symbol, or by the pretty name
//!   of a symbol, for crossref to resolve, ex: a C++ getter generated from a
//!   pref).
//!
//! Query rules' templates see each capture's text by name, `scope.<name>` for
//! the innermost enclosing scope of each name (with its `name`, `sym`, and
//! `pretty`), `context` for the innermost enclosing scope, and `outer` for the
//! outermost one (ex: the message whose attribute's pattern a variable is in).
//! The scope a record names (ex: a message's own identifier) doesn't enclose
//! it.  Data rules' templates see `value` (the key's or scalar's value),
//! `text` (its source text, ex: with quotes), `key` (the entry's key), `keys`
//! (the keys from the document's root to the entry), and the file's `path`,
//! `dir_path` (its directory's path), and `dir` (its directory's name).
//! Besides Liquid's filters, templates have `at_escape`, `camel_case`,
//! `pascal_case`, `snake_case`, and `regex_expansions` (see
//! `templating::liquid_exts`).

use std::ops::Range;

use globset::{Glob, GlobSet, GlobSetBuilder};
use include_dir::{Dir, include_dir};
use serde::Deserialize;
use serde_json::json;
use tree_sitter::StreamingIterator;

use crate::file_format::analysis::parse_location;
use crate::file_format::analysis::{
    AnalysisKind, AnalysisSource, AnalysisTarget, BindingImplKind, BindingOwnerLang,
    BindingSlotKind, BindingSlotLang, LineRange, Location, SourceRange, SourceTag, TargetTag,
    WithLocation,
};
use crate::file_format::analysis_manglings::mangle_file;
use crate::templating::liquid_exts::{
    AtEscapeFilterParser, CamelCaseFilterParser, PascalCaseFilterParser,
    RegexExpansionsFilterParser, SnakeCaseFilterParser,
};

static MAPPINGS_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/languages/semantic_mappings");

#[derive(Debug, Deserialize)]
pub struct Mapping {
    pub grammar: String,
    pub paths: Vec<String>,
    #[serde(default)]
    pub exclude_paths: Vec<String>,
    #[serde(default)]
    pub contains: Vec<String>,
    #[serde(default)]
    pub when: Vec<Condition>,
    #[serde(default)]
    pub scopes: Vec<ScopeRule>,
    #[serde(default)]
    pub rules: Vec<QueryRule>,
    #[serde(default)]
    pub data: Vec<DataRule>,
}

/// A data path to a scalar whose value starts with `prefix`.
#[derive(Debug, Deserialize)]
pub struct Condition {
    pub path: String,
    pub prefix: String,
}

#[derive(Debug, Deserialize)]
pub struct ScopeRule {
    pub name: String,
    pub query: String,
    pub sym: String,
    pub pretty: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Def,
    Use,
    /// A key of data, with source records only.
    Key,
}

#[derive(Debug, Deserialize)]
pub struct QueryRule {
    pub role: Role,
    pub query: String,
    pub sym: String,
    pub pretty: String,
    pub syntax_kind: String,
    #[serde(default)]
    pub nesting: Option<String>,
    /// The target records' kind, if not the role's (ex: "doc-def").
    #[serde(default)]
    pub kind: Option<AnalysisKind>,
    #[serde(default)]
    pub regex: Option<String>,
    #[serde(default)]
    pub resolve: Option<Resolve>,
}

/// How a use names a definition in any file (with the use rule's `sym` as
/// the prefix of the definitions' symbols), which `resolve_uses` finds once
/// all of the files are analyzed.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Resolve {
    /// The definition whose pretty name shares the most leading words with
    /// the use's text (ignoring case, and punctuation around words), if just
    /// one does (ex: "Atomics.wait for more details." names "Atomics.wait,
    /// Atomics.waitAsync, and Atomics.notify"), at those words.
    DefPrefix,
}

/// Which scalar of a matched entry a data rule's records are at.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DataSubject {
    #[default]
    Key,
    Value,
}

#[derive(Debug, Deserialize)]
pub struct DataRule {
    pub path: String,
    #[serde(default)]
    pub at: DataSubject,
    pub role: Role,
    pub sym: String,
    pub pretty: String,
    /// The source records' pretty prefix, if any.
    #[serde(default)]
    pub syntax_kind: String,
    /// The target records' kind, if not the role's (ex: "glean").
    #[serde(default)]
    pub kind: Option<AnalysisKind>,
    /// Whether keys whose values span lines have nesting ranges.
    #[serde(default)]
    pub nesting: bool,
    #[serde(default)]
    pub structured: Option<StructuredRule>,
    /// Keys of entries the rule leaves out (ex: "task-defaults"), and values
    /// of its subjects (ex: keyed-by "default"s).
    #[serde(default)]
    pub except_keys: Vec<String>,
    #[serde(default)]
    pub except_values: Vec<String>,
    /// A template whose output's (whitespace-separated) words each get the
    /// rule's records, with the word as `item`.
    #[serde(default)]
    pub each: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct StructuredRule {
    pub kind: String,
    pub impl_kind: String,
    #[serde(default)]
    pub bindings: Vec<BindingRule>,
}

/// A binding slot, to `sym` or to the symbol whose pretty name is `pretty`
/// (templates; a slot whose template renders empty is left out).
#[derive(Debug, Deserialize)]
pub struct BindingRule {
    pub slot_kind: BindingSlotKind,
    pub slot_lang: BindingSlotLang,
    pub owner_lang: BindingOwnerLang,
    #[serde(default)]
    pub impl_kind: Option<BindingImplKind>,
    #[serde(default)]
    pub sym: Option<String>,
    #[serde(default)]
    pub pretty: Option<String>,
}

/// The language of a mapping's grammar.
fn grammar_language(grammar: &str) -> Result<tree_sitter::Language, String> {
    match grammar {
        "fluent" => Ok(tree_sitter_fluent::LANGUAGE.into()),
        "yaml" => Ok(tree_sitter_yaml::LANGUAGE.into()),
        "mozcpp" => Ok(tree_sitter_mozcpp::LANGUAGE.into()),
        "rust" => Ok(tree_sitter_rust::LANGUAGE.into()),
        "typescript" => Ok(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        _ => Err(format!("Unknown grammar {}", grammar)),
    }
}

struct CompiledScope {
    name: String,
    query: tree_sitter::Query,
    sym: liquid::Template,
    pretty: liquid::Template,
}

struct CompiledRule {
    role: Role,
    query: tree_sitter::Query,
    sym: liquid::Template,
    pretty: liquid::Template,
    syntax_kind: String,
    nesting: Option<String>,
    kind: Option<AnalysisKind>,
    regex: Option<regex::Regex>,
    /// How its uses resolve, and the prefix of the symbols they resolve to.
    resolve: Option<(Resolve, String)>,
}

#[derive(Debug, PartialEq, Eq)]
enum SegmentKind {
    Key(String),
    AnyKey,
    Item,
    AnyDepth,
}

/// A data path's segment, with the key and scalar value which the value of an
/// entry it matches must have, if any (`key[name=value]`).
#[derive(Debug, PartialEq, Eq)]
struct Segment {
    kind: SegmentKind,
    predicate: Option<(String, String)>,
}

fn parse_data_path(path: &str) -> Vec<Segment> {
    path.split('/')
        .map(|segment| {
            let (name, predicate) = match segment.strip_suffix(']').and_then(|s| s.split_once('['))
            {
                Some((name, predicate)) if !predicate.is_empty() => (
                    name,
                    predicate
                        .split_once('=')
                        .map(|(key, value)| (key.to_string(), value.to_string())),
                ),
                _ => (segment, None),
            };
            let kind = match name {
                "*" => SegmentKind::AnyKey,
                "[]" => SegmentKind::Item,
                "**" => SegmentKind::AnyDepth,
                key => SegmentKind::Key(key.to_string()),
            };
            Segment { kind, predicate }
        })
        .collect()
}

struct CompiledBinding {
    slot_kind: BindingSlotKind,
    slot_lang: BindingSlotLang,
    owner_lang: BindingOwnerLang,
    impl_kind: Option<BindingImplKind>,
    sym: Option<liquid::Template>,
    pretty: Option<liquid::Template>,
}

struct CompiledStructured {
    kind: String,
    impl_kind: String,
    bindings: Vec<CompiledBinding>,
}

struct CompiledDataRule {
    path: Vec<Segment>,
    at: DataSubject,
    role: Role,
    sym: liquid::Template,
    pretty: liquid::Template,
    syntax_kind: String,
    kind: Option<AnalysisKind>,
    nesting: bool,
    structured: Option<CompiledStructured>,
    except_keys: Vec<String>,
    except_values: Vec<String>,
    each: Option<liquid::Template>,
}

/// A mapping ready to analyze files with.
pub struct CompiledMapping {
    pub name: String,
    grammar: String,
    paths: GlobSet,
    exclude_paths: GlobSet,
    contains: Vec<String>,
    language: tree_sitter::Language,
    when: Vec<(Vec<Segment>, String)>,
    scopes: Vec<CompiledScope>,
    rules: Vec<CompiledRule>,
    data: Vec<CompiledDataRule>,
}

impl CompiledMapping {
    pub fn new(name: &str, mapping: &Mapping) -> Result<CompiledMapping, String> {
        let language = grammar_language(&mapping.grammar)?;
        let liquid = liquid::ParserBuilder::with_stdlib()
            .filter(AtEscapeFilterParser)
            .filter(CamelCaseFilterParser)
            .filter(PascalCaseFilterParser)
            .filter(SnakeCaseFilterParser)
            .filter(RegexExpansionsFilterParser)
            .build()
            .map_err(|e| e.to_string())?;
        let template = |source: &str| {
            liquid
                .parse(source)
                .map_err(|e| format!("{}: template {:?}: {}", name, source, e))
        };
        let query = |source: &str| {
            tree_sitter::Query::new(&language, source)
                .map_err(|e| format!("{}: query {:?}: {}", name, source, e.message))
        };
        let globs = |globs: &[String]| {
            let mut set = GlobSetBuilder::new();
            for glob in globs {
                set.add(Glob::new(glob).map_err(|e| e.to_string())?);
            }
            set.build().map_err(|e| e.to_string())
        };
        if !mapping.data.is_empty() || !mapping.when.is_empty() {
            data_view_kind(&mapping.grammar)
                .ok_or_else(|| format!("{}: {} has no data view", name, mapping.grammar))?;
        }
        Ok(CompiledMapping {
            name: name.to_string(),
            grammar: mapping.grammar.clone(),
            paths: globs(&mapping.paths)?,
            exclude_paths: globs(&mapping.exclude_paths)?,
            contains: mapping.contains.clone(),
            when: mapping
                .when
                .iter()
                .map(|condition| (parse_data_path(&condition.path), condition.prefix.clone()))
                .collect(),
            scopes: mapping
                .scopes
                .iter()
                .map(|scope| {
                    Ok(CompiledScope {
                        name: scope.name.clone(),
                        query: query(&scope.query)?,
                        sym: template(&scope.sym)?,
                        pretty: template(&scope.pretty)?,
                    })
                })
                .collect::<Result<_, String>>()?,
            rules: mapping
                .rules
                .iter()
                .map(|rule| {
                    let sym = template(&rule.sym)?;
                    let resolve = match rule.resolve {
                        Some(resolve) if rule.role == Role::Use => {
                            let prefix = sym
                                .render(&liquid::Object::new())
                                .map_err(|e| e.to_string())?;
                            Some((resolve, prefix))
                        }
                        Some(_) => return Err(format!("{}: only uses resolve", name)),
                        None => None,
                    };
                    Ok(CompiledRule {
                        role: rule.role,
                        query: query(&rule.query)?,
                        sym,
                        pretty: template(&rule.pretty)?,
                        syntax_kind: rule.syntax_kind.clone(),
                        nesting: rule.nesting.clone(),
                        kind: rule.kind,
                        regex: rule
                            .regex
                            .as_deref()
                            .map(regex::Regex::new)
                            .transpose()
                            .map_err(|e| format!("{}: regex: {}", name, e))?,
                        resolve,
                    })
                })
                .collect::<Result<_, String>>()?,
            data: mapping
                .data
                .iter()
                .map(|rule| {
                    let structured = match &rule.structured {
                        Some(structured) => Some(CompiledStructured {
                            kind: structured.kind.clone(),
                            impl_kind: structured.impl_kind.clone(),
                            bindings: structured
                                .bindings
                                .iter()
                                .map(|binding| {
                                    Ok(CompiledBinding {
                                        slot_kind: binding.slot_kind,
                                        slot_lang: binding.slot_lang,
                                        owner_lang: binding.owner_lang,
                                        impl_kind: binding.impl_kind,
                                        sym: binding.sym.as_deref().map(template).transpose()?,
                                        pretty: binding
                                            .pretty
                                            .as_deref()
                                            .map(template)
                                            .transpose()?,
                                    })
                                })
                                .collect::<Result<_, String>>()?,
                        }),
                        None => None,
                    };
                    Ok(CompiledDataRule {
                        path: parse_data_path(&rule.path),
                        at: rule.at,
                        role: rule.role,
                        sym: template(&rule.sym)?,
                        pretty: template(&rule.pretty)?,
                        syntax_kind: rule.syntax_kind.clone(),
                        kind: rule.kind,
                        nesting: rule.nesting,
                        structured,
                        except_keys: rule.except_keys.clone(),
                        except_values: rule.except_values.clone(),
                        each: rule.each.as_deref().map(template).transpose()?,
                    })
                })
                .collect::<Result<_, String>>()?,
            language,
        })
    }

    /// Whether the mapping applies to the file at `path` (relative to the
    /// tree's root), if its `when` conditions hold for the file.
    pub fn applies_to(&self, path: &str) -> bool {
        self.paths.is_match(path) && !self.exclude_paths.is_match(path)
    }

    /// Whether the mapping applies to a file with `source` (see `contains`).
    pub fn applies_to_source(&self, source: &str) -> bool {
        self.contains.is_empty() || self.contains.iter().any(|s| source.contains(s.as_str()))
    }

    /// The prefixes of the symbols of the definitions which its uses resolve
    /// to (see `Resolve`).
    pub fn resolving_prefixes(&self) -> Vec<String> {
        self.rules
            .iter()
            .filter_map(|rule| Some(rule.resolve.as_ref()?.1.clone()))
            .collect()
    }
}

/// The mappings in `languages/semantic_mappings/`, by file name.
pub fn builtin_mappings() -> Result<Vec<CompiledMapping>, String> {
    let mut mappings = vec![];
    let mut files: Vec<_> = MAPPINGS_DIR.files().collect();
    files.sort_by_key(|file| file.path());
    for file in files {
        let name = file.path().to_string_lossy().into_owned();
        let text = file
            .contents_utf8()
            .ok_or_else(|| format!("{} isn't UTF-8", name))?;
        let mapping: Mapping = toml::from_str(text).map_err(|e| format!("{}: {}", name, e))?;
        mappings.push(CompiledMapping::new(&name, &mapping)?);
    }
    Ok(mappings)
}

// ## Data views

#[derive(Clone, Copy)]
enum DataViewKind {
    Yaml,
}

fn data_view_kind(grammar: &str) -> Option<DataViewKind> {
    match grammar {
        "yaml" => Some(DataViewKind::Yaml),
        _ => None,
    }
}

/// A scalar of a document's data, with its node and its value (ex: without
/// its quotes).
struct Scalar<'t> {
    node: tree_sitter::Node<'t>,
    value: String,
}

/// A document's data, from its syntax tree.
enum Data<'t> {
    Map(Vec<Entry<'t>>),
    Seq(Vec<Entry<'t>>),
    Scalar(Scalar<'t>),
    /// Null, aliases, and keys which aren't scalars.
    Other,
}

/// An entry of a map (with a key) or a sequence (without).
struct Entry<'t> {
    key: Option<Scalar<'t>>,
    value: Data<'t>,
    value_node: Option<tree_sitter::Node<'t>>,
}

impl<'t> Data<'t> {
    fn entries(&self) -> &[Entry<'t>] {
        match self {
            Data::Map(entries) | Data::Seq(entries) => entries,
            _ => &[],
        }
    }
}

/// YAML which Mozilla's build preprocesses: a value that's a substitution
/// (`@NAME@`, ex: in StaticPrefList.yaml) isn't valid YAML, so it becomes a
/// quoted string of the same length, `"NAME"`.  (A plain scalar can't start
/// with `@`, so this only changes YAML that isn't valid otherwise.)
pub(crate) fn preprocess_yaml(source: &str) -> Option<String> {
    lazy_static::lazy_static! {
        static ref SUBSTITUTION: regex::Regex =
            regex::Regex::new(r"(?m)((?:^|:|-)[ ]+)@([A-Za-z_][A-Za-z0-9_]*)@([ ]*(?:#.*)?)$")
                .unwrap();
    }
    if !SUBSTITUTION.is_match(source) {
        return None;
    }
    Some(SUBSTITUTION.replace_all(source, "$1\"$2\"$3").into_owned())
}

/// A YAML scalar's value: plain scalars' lines folded, quoted scalars' quotes
/// and escapes removed.
fn yaml_scalar_value(node: &tree_sitter::Node, text: &str) -> String {
    match node.kind() {
        "single_quote_scalar" => text
            .strip_prefix('\'')
            .and_then(|t| t.strip_suffix('\''))
            .unwrap_or(text)
            .replace("''", "'"),
        "double_quote_scalar" => {
            let inner = text
                .strip_prefix('"')
                .and_then(|t| t.strip_suffix('"'))
                .unwrap_or(text);
            let mut value = String::new();
            let mut chars = inner.chars();
            while let Some(c) = chars.next() {
                if c != '\\' {
                    value.push(c);
                    continue;
                }
                match chars.next() {
                    Some('n') => value.push('\n'),
                    Some('t') => value.push('\t'),
                    Some('0') => value.push('\0'),
                    Some(other) => value.push(other),
                    None => {}
                }
            }
            value
        }
        "plain_scalar" => text.lines().map(str::trim).collect::<Vec<_>>().join(" "),
        _ => text.to_string(),
    }
}

/// The data of a tree-sitter-yaml node (for a stream, its first document's).
fn yaml_data<'t>(node: tree_sitter::Node<'t>, source: &str) -> Data<'t> {
    let content = |node: tree_sitter::Node<'t>| {
        let mut cursor = node.walk();
        node.named_children(&mut cursor)
            .find(|child| !matches!(child.kind(), "anchor" | "tag" | "comment"))
    };
    let entry = |key: Option<tree_sitter::Node<'t>>, value: Option<tree_sitter::Node<'t>>| {
        let key = key.and_then(|key| match yaml_data(key, source) {
            Data::Scalar(scalar) => Some(scalar),
            _ => None,
        });
        Entry {
            key,
            value: value
                .map(|value| yaml_data(value, source))
                .unwrap_or(Data::Other),
            value_node: value,
        }
    };
    match node.kind() {
        "stream" => {
            let mut cursor = node.walk();
            let document = node
                .named_children(&mut cursor)
                .find(|child| child.kind() == "document" || child.kind() == "ERROR");
            document.map_or(Data::Other, |document| yaml_data(document, source))
        }
        "document" | "block_node" | "flow_node" => {
            content(node).map_or(Data::Other, |content| yaml_data(content, source))
        }
        "block_mapping" | "flow_mapping" => {
            let mut cursor = node.walk();
            Data::Map(
                node.named_children(&mut cursor)
                    .filter_map(|child| match child.kind() {
                        "block_mapping_pair" | "flow_pair" => Some(entry(
                            child.child_by_field_name("key"),
                            child.child_by_field_name("value"),
                        )),
                        // (A flow mapping's key without a value.)
                        "flow_node" => Some(entry(Some(child), None)),
                        _ => None,
                    })
                    .collect(),
            )
        }
        "block_sequence" | "flow_sequence" => {
            let mut cursor = node.walk();
            Data::Seq(
                node.named_children(&mut cursor)
                    .filter_map(|child| match child.kind() {
                        "block_sequence_item" => Some(entry(None, content(child))),
                        "flow_node" => Some(entry(None, Some(child))),
                        "flow_pair" => Some(Entry {
                            key: None,
                            value: Data::Map(vec![entry(
                                child.child_by_field_name("key"),
                                child.child_by_field_name("value"),
                            )]),
                            value_node: Some(child),
                        }),
                        _ => None,
                    })
                    .collect(),
            )
        }
        "plain_scalar" | "single_quote_scalar" | "double_quote_scalar" | "block_scalar" => {
            Data::Scalar(Scalar {
                node,
                value: yaml_scalar_value(&node, &source[node.start_byte()..node.end_byte()]),
            })
        }
        // (Error recovery: the pairs which parsed, and keys whose values
        // didn't.)
        "ERROR" => {
            let mut cursor = node.walk();
            let children: Vec<_> = node.children(&mut cursor).collect();
            let mut entries = vec![];
            for (i, child) in children.iter().enumerate() {
                match child.kind() {
                    "block_mapping_pair" | "flow_pair" => entries.push(entry(
                        child.child_by_field_name("key"),
                        child.child_by_field_name("value"),
                    )),
                    "flow_node" if children.get(i + 1).is_some_and(|next| next.kind() == ":") => {
                        entries.push(entry(Some(*child), None))
                    }
                    "block_node" | "block_mapping" | "ERROR" => {
                        if let Data::Map(more) = yaml_data(*child, source) {
                            entries.extend(more);
                        }
                    }
                    _ => {}
                }
            }
            Data::Map(entries)
        }
        _ => Data::Other,
    }
}

/// An entry which a data path matched, with the keys from the root to it.
struct DataMatch<'d, 't> {
    keys: Vec<String>,
    entry: &'d Entry<'t>,
}

fn match_data_path<'d, 't>(
    data: &'d Data<'t>,
    path: &[Segment],
    keys: &mut Vec<String>,
    out: &mut Vec<DataMatch<'d, 't>>,
) {
    let Some((first, rest)) = path.split_first() else {
        return;
    };
    let is_map = matches!(data, Data::Map(_));
    let any_depth = first.kind == SegmentKind::AnyDepth;
    for entry in data.entries() {
        let accepted = match &first.kind {
            SegmentKind::Key(key) => is_map && entry.key.as_ref().is_some_and(|k| k.value == *key),
            SegmentKind::AnyKey => is_map && entry.key.is_some(),
            SegmentKind::Item => !is_map,
            SegmentKind::AnyDepth => true,
        };
        let satisfies = |(key, value): &(String, String)| {
            entry.value.entries().iter().any(|child| {
                child.key.as_ref().is_some_and(|k| k.value == *key)
                    && matches!(&child.value, Data::Scalar(scalar) if scalar.value == *value)
            })
        };
        if !accepted || first.predicate.as_ref().is_some_and(|p| !satisfies(p)) {
            continue;
        }
        let pushed = entry.key.as_ref().map(|key| keys.push(key.value.clone()));
        if any_depth {
            // (One more level; zero levels are below.)
            match_data_path(&entry.value, path, keys, out);
        } else if rest.is_empty() {
            out.push(DataMatch {
                keys: keys.clone(),
                entry,
            });
        } else {
            match_data_path(&entry.value, rest, keys, out);
        }
        if pushed.is_some() {
            keys.pop();
        }
    }
    if any_depth {
        match_data_path(data, rest, keys, out);
    }
}

fn conditions_hold(conditions: &[(Vec<Segment>, String)], data: &Data) -> bool {
    conditions.iter().all(|(path, prefix)| {
        let mut matches = vec![];
        match_data_path(data, path, &mut vec![], &mut matches);
        matches.iter().any(|m| match &m.entry.value {
            Data::Scalar(scalar) => scalar.value.starts_with(prefix.as_str()),
            _ => false,
        })
    })
}

// ## Records

/// An enclosing scope in a file.
struct ScopeInstance {
    start_byte: usize,
    end_byte: usize,
    /// The id of the node of its name, so that the record which names it isn't
    /// in it.
    name_node: usize,
    kind: String,
    name: String,
    sym: String,
    pretty: String,
}

fn capture_index(query: &tree_sitter::Query, name: &str) -> Option<u32> {
    query
        .capture_names()
        .iter()
        .position(|capture| *capture == name)
        .map(|i| i as u32)
}

fn node_text<'s>(node: &tree_sitter::Node, source: &'s str) -> &'s str {
    &source[node.start_byte()..node.end_byte()]
}

/// The template variables for the scopes in `enclosing` (outermost first),
/// plus `captures`.
fn template_globals(enclosing: &[&ScopeInstance], captures: &[(String, String)]) -> liquid::Object {
    let mut scope = liquid::Object::new();
    let mut context = liquid::Object::new();
    let mut outer = None;
    for instance in enclosing {
        let object = liquid::object!({
            "name": instance.name,
            "sym": instance.sym,
            "pretty": instance.pretty,
        });
        scope.insert(
            instance.kind.clone().into(),
            liquid::model::Value::Object(object.clone()),
        );
        outer.get_or_insert_with(|| object.clone());
        context = object;
    }
    let mut globals = liquid::object!({
        "scope": scope,
        "context": context,
        "outer": outer.unwrap_or_default(),
    });
    for (name, text) in captures {
        globals.insert(
            name.clone().into(),
            liquid::model::Value::scalar(text.clone()),
        );
    }
    globals
}

/// The offsets of the starts of `source`'s lines.
fn line_starts_of(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(source.match_indices('\n').map(|(i, _)| i + 1));
    starts
}

/// The location of the bytes `range` of `source` (whose lines start at
/// `line_starts`), which are on one line (or else the first line's).
fn byte_location(source: &str, line_starts: &[usize], range: Range<usize>) -> Location {
    let line = line_starts.partition_point(|&start| start <= range.start) - 1;
    let end = source[range.clone()]
        .find('\n')
        .map_or(range.end, |i| range.start + i);
    Location {
        lineno: line as u32 + 1,
        col_start: (range.start - line_starts[line]) as u32,
        col_end: (end - line_starts[line]) as u32,
    }
}

fn location(node: &tree_sitter::Node) -> Location {
    let start = node.start_position();
    let end = node.end_position();
    Location {
        lineno: start.row as u32 + 1,
        col_start: start.column as u32,
        // (A record's node is on one line; if it isn't, its first line's.)
        col_end: if end.row == start.row {
            end.column as u32
        } else {
            start.column as u32
        },
    }
}

/// The (1-based) last line with any of `node`'s text, other than comments
/// (which a YAML block can end with, even when they're outdented).
fn last_line(node: &tree_sitter::Node) -> u32 {
    let mut node = *node;
    // (Scalars' children, if any, aren't all of their text, ex: a block
    // scalar's only child is its `|`.)
    while !node.kind().ends_with("_scalar") {
        let mut cursor = node.walk();
        let last = node
            .named_children(&mut cursor)
            .filter(|child| child.kind() != "comment")
            .last();
        match last {
            Some(child) => node = child,
            None => break,
        }
    }
    let start = node.start_position();
    let end = node.end_position();
    if end.column == 0 && end.row > start.row {
        end.row as u32
    } else {
        end.row as u32 + 1
    }
}

/// An analysis record for a file, in the order they're written.
pub enum Record {
    Source(WithLocation<AnalysisSource<String>>),
    Target(WithLocation<AnalysisTarget<String>>),
    Structured(Location, serde_json::Value),
}

impl Record {
    fn loc(&self) -> &Location {
        match self {
            Record::Source(source) => &source.loc,
            Record::Target(target) => &target.loc,
            Record::Structured(loc, _) => loc,
        }
    }

    pub fn to_json(&self) -> String {
        match self {
            Record::Source(source) => serde_json::to_string(source).unwrap(),
            Record::Target(target) => serde_json::to_string(target).unwrap(),
            Record::Structured(_, value) => serde_json::to_string(value).unwrap(),
        }
    }
}

pub(crate) fn source_record(
    loc: Location,
    syntax: &str,
    pretty: String,
    sym: String,
    nesting_range: SourceRange,
) -> Record {
    Record::Source(WithLocation {
        loc,
        data: AnalysisSource {
            source: SourceTag::Source,
            syntax: vec![syntax.to_string()],
            pretty,
            sym: vec![sym],
            no_crossref: false,
            nesting_range,
            type_pretty: None,
            type_sym: None,
            arg_ranges: vec![],
            expansion_info: None,
            confidence: None,
        },
    })
}

pub(crate) fn target_record(
    loc: Location,
    kind: AnalysisKind,
    pretty: String,
    sym: String,
    context: String,
    contextsym: String,
) -> Record {
    Record::Target(WithLocation {
        loc,
        data: AnalysisTarget {
            target: TargetTag::Target,
            kind,
            pretty,
            sym,
            context,
            contextsym,
            peek_range: LineRange::default(),
            arg_ranges: vec![],
        },
    })
}

/// A rule's source (and, unless it's for keys, target) records.
#[allow(clippy::too_many_arguments)]
fn rule_records(
    records: &mut Vec<Record>,
    role: Role,
    loc: Location,
    syntax_kind: &str,
    kind: Option<AnalysisKind>,
    pretty: String,
    sym: String,
    nesting_range: SourceRange,
    context: (String, String),
) {
    let syntax = match role {
        Role::Def => "def",
        Role::Use => "use",
        Role::Key => "key",
    };
    let source_pretty = if syntax_kind.is_empty() {
        pretty.clone()
    } else {
        format!("{} {}", syntax_kind, pretty)
    };
    records.push(source_record(
        loc,
        syntax,
        source_pretty,
        sym.clone(),
        nesting_range,
    ));
    let kind = match (role, kind) {
        (Role::Key, _) => return,
        (_, Some(kind)) => kind,
        (Role::Def, None) => AnalysisKind::Def,
        (Role::Use, None) => AnalysisKind::Use,
    };
    records.push(target_record(loc, kind, pretty, sym, context.0, context.1));
}

/// A use which names a definition in any file (see `Resolve`).
#[derive(Debug)]
pub struct UnresolvedUse {
    resolve: Resolve,
    /// The prefix of the symbols of the definitions it can name.
    prefix: String,
    /// What names the definition, at `loc` (on one line).
    text: String,
    loc: Location,
    syntax_kind: String,
    kind: Option<AnalysisKind>,
    context: (String, String),
}

/// A file's analysis, with the uses which name definitions in any file.
pub struct Analysis {
    pub records: Vec<Record>,
    pub unresolved: Vec<UnresolvedUse>,
}

/// The words of `text`, with their bytes, without the punctuation around
/// them, lowercased, for comparing names.
fn name_words(text: &str) -> Vec<(Range<usize>, String)> {
    let punctuation = |c: char| ".,;:!?\"'`()[]{}<>".contains(c);
    let mut words = vec![];
    let mut pos = 0;
    for word in text.split_whitespace() {
        let start = pos + text[pos..].find(word).unwrap_or(0);
        pos = start + word.len();
        let trimmed = word.trim_matches(punctuation);
        if trimmed.is_empty() {
            continue;
        }
        let start = start + word.find(trimmed).unwrap_or(0);
        words.push((start..start + trimmed.len(), trimmed.to_lowercase()));
    }
    words
}

/// The records of `unresolved` uses which name `defs` (pretty names and
/// symbols of definitions, in any file).
pub fn resolve_uses(defs: &[(String, String)], unresolved: &[UnresolvedUse]) -> Vec<Record> {
    let def_words: Vec<Vec<String>> = defs
        .iter()
        .map(|(pretty, _)| name_words(pretty).into_iter().map(|(_, w)| w).collect())
        .collect();
    let mut records = vec![];
    for using in unresolved {
        match using.resolve {
            Resolve::DefPrefix => {
                let words = name_words(&using.text);
                // (The definitions sharing the most leading words.)
                let mut best: Vec<usize> = vec![];
                let mut best_count = 0;
                for (i, (_, sym)) in defs.iter().enumerate() {
                    if !sym.starts_with(&using.prefix) {
                        continue;
                    }
                    let count = words
                        .iter()
                        .zip(&def_words[i])
                        .take_while(|((_, a), b)| a == *b)
                        .count();
                    if count > best_count {
                        best = vec![i];
                        best_count = count;
                    } else if count == best_count && count > 0 && defs[best[0]].1 != *sym {
                        best.push(i);
                    }
                }
                let [def] = best[..] else {
                    continue;
                };
                let (start, end) = (words[0].0.start, words[best_count - 1].0.end);
                let loc = Location {
                    lineno: using.loc.lineno,
                    col_start: using.loc.col_start + start as u32,
                    col_end: using.loc.col_start + end as u32,
                };
                rule_records(
                    &mut records,
                    Role::Use,
                    loc,
                    &using.syntax_kind,
                    using.kind,
                    defs[def].0.clone(),
                    defs[def].1.clone(),
                    SourceRange::default(),
                    using.context.clone(),
                );
            }
        }
    }
    records
}

/// The records of a mapping's query rules, and its uses which name
/// definitions in any file.
fn query_records(
    mapping: &CompiledMapping,
    root: tree_sitter::Node,
    source: &str,
    records: &mut Vec<Record>,
    unresolved: &mut Vec<UnresolvedUse>,
) -> Result<(), String> {
    // ## Scopes, outermost first.
    struct RawScope<'m> {
        start_byte: usize,
        end_byte: usize,
        name_node: usize,
        scope: &'m CompiledScope,
        name: String,
        captures: Vec<(String, String)>,
    }
    let mut raw_scopes: Vec<RawScope> = vec![];
    for scope in &mapping.scopes {
        let (Some(scope_capture), Some(name_capture)) = (
            capture_index(&scope.query, "scope"),
            capture_index(&scope.query, "name"),
        ) else {
            return Err(format!(
                "{}: scope {} needs @scope and @name",
                mapping.name, scope.name
            ));
        };
        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&scope.query, root, source.as_bytes());
        while let Some(m) = matches.next() {
            let scope_node = m.captures().iter().find(|c| c.index == scope_capture);
            let name_node = m.captures().iter().find(|c| c.index == name_capture);
            if let (Some(scope_node), Some(name_node)) = (scope_node, name_node) {
                let captures = m
                    .captures()
                    .iter()
                    .map(|c| {
                        (
                            scope.query.capture_names()[c.index as usize].to_string(),
                            node_text(&c.node, source).to_string(),
                        )
                    })
                    .collect();
                raw_scopes.push(RawScope {
                    start_byte: scope_node.node.start_byte(),
                    end_byte: scope_node.node.end_byte(),
                    name_node: name_node.node.id(),
                    scope,
                    name: node_text(&name_node.node, source).to_string(),
                    captures,
                });
            }
        }
    }
    raw_scopes.sort_by_key(|raw| (raw.start_byte, std::cmp::Reverse(raw.end_byte)));
    let mut scopes: Vec<ScopeInstance> = vec![];
    for RawScope {
        start_byte,
        end_byte,
        name_node,
        scope,
        name,
        captures,
    } in raw_scopes
    {
        let enclosing: Vec<&ScopeInstance> = scopes
            .iter()
            .filter(|s| s.start_byte <= start_byte && end_byte <= s.end_byte)
            .collect();
        let globals = template_globals(&enclosing, &captures);
        let sym = scope.sym.render(&globals).map_err(|e| e.to_string())?;
        let pretty = scope.pretty.render(&globals).map_err(|e| e.to_string())?;
        scopes.push(ScopeInstance {
            start_byte,
            end_byte,
            name_node,
            kind: scope.name.clone(),
            name,
            sym,
            pretty,
        });
    }

    // ## Records.
    let mut line_starts = None;
    for rule in &mapping.rules {
        let Some(id_capture) = capture_index(&rule.query, "id") else {
            return Err(format!(
                "{}: a rule needs @id: {:?}",
                mapping.name, rule.syntax_kind
            ));
        };
        let nesting_capture = rule
            .nesting
            .as_deref()
            .and_then(|name| capture_index(&rule.query, name));
        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&rule.query, root, source.as_bytes());
        while let Some(m) = matches.next() {
            let Some(id) = m.captures().iter().find(|c| c.index == id_capture) else {
                continue;
            };
            let id = id.node;
            let enclosing: Vec<&ScopeInstance> = scopes
                .iter()
                .filter(|s| {
                    s.start_byte <= id.start_byte()
                        && id.end_byte() <= s.end_byte
                        && s.name_node != id.id()
                })
                .collect();
            let captures: Vec<(String, String)> = m
                .captures()
                .iter()
                .map(|c| {
                    (
                        rule.query.capture_names()[c.index as usize].to_string(),
                        node_text(&c.node, source).to_string(),
                    )
                })
                .collect();
            let context = enclosing
                .last()
                .map(|s| (s.pretty.clone(), s.sym.clone()))
                .unwrap_or_default();
            // (With a regex, the records are at its matches in the node.)
            let Some(regex) = &rule.regex else {
                let globals = template_globals(&enclosing, &captures);
                let sym = rule.sym.render(&globals).map_err(|e| e.to_string())?;
                let pretty = rule.pretty.render(&globals).map_err(|e| e.to_string())?;
                let loc = location(&id);
                // (From the definition's line, since the nesting node can
                // start with a line break, ex: a Fluent attribute's.)
                let nesting_range = nesting_capture
                    .and_then(|index| m.captures().iter().find(|c| c.index == index))
                    .map(|c| {
                        let end = c.node.end_position();
                        SourceRange {
                            start_lineno: loc.lineno,
                            start_col: loc.col_start,
                            end_lineno: end.row as u32 + 1,
                            end_col: end.column as u32,
                        }
                    })
                    // (Only for definitions spanning lines.)
                    .filter(|range| range.end_lineno > range.start_lineno)
                    .unwrap_or_default();
                rule_records(
                    records,
                    rule.role,
                    loc,
                    &rule.syntax_kind,
                    rule.kind,
                    pretty,
                    sym,
                    nesting_range,
                    context,
                );
                continue;
            };
            let node_start = id.start_byte();
            let line_starts = line_starts.get_or_insert_with(|| line_starts_of(source));
            let has_id_group = regex.capture_names().flatten().any(|name| name == "id");
            for regex_match in regex.captures_iter(node_text(&id, source)) {
                // (Matches without the `id` group, if the regex has one, are
                // what the rule leaves out, ex: definitions, for uses.)
                let at = match regex_match.name("id") {
                    Some(at) => at,
                    None if has_id_group => continue,
                    None => regex_match.get(0).unwrap(),
                };
                let mut captures = captures.clone();
                for name in regex.capture_names().flatten() {
                    if let Some(group) = regex_match.name(name) {
                        captures.push((name.to_string(), group.as_str().to_string()));
                    }
                }
                let globals = template_globals(&enclosing, &captures);
                let loc = byte_location(
                    source,
                    line_starts,
                    node_start + at.start()..node_start + at.end(),
                );
                if let Some((resolve, prefix)) = &rule.resolve {
                    unresolved.push(UnresolvedUse {
                        resolve: *resolve,
                        prefix: prefix.clone(),
                        text: at.as_str().to_string(),
                        loc,
                        syntax_kind: rule.syntax_kind.clone(),
                        kind: rule.kind,
                        context: context.clone(),
                    });
                    continue;
                }
                let sym = rule.sym.render(&globals).map_err(|e| e.to_string())?;
                let pretty = rule.pretty.render(&globals).map_err(|e| e.to_string())?;
                rule_records(
                    records,
                    rule.role,
                    loc,
                    &rule.syntax_kind,
                    rule.kind,
                    pretty,
                    sym,
                    SourceRange::default(),
                    context.clone(),
                );
            }
        }
    }
    Ok(())
}

/// The records of a mapping's data rules, for the file at `path`.
fn data_records(
    mapping: &CompiledMapping,
    data: &Data,
    path: &str,
    source: &str,
    records: &mut Vec<Record>,
) -> Result<(), String> {
    let render = |template: &liquid::Template, globals: &liquid::Object| {
        template.render(globals).map_err(|e| e.to_string())
    };
    let (dir_path, _) = path.rsplit_once('/').unwrap_or(("", path));
    let dir = dir_path.rsplit('/').next().unwrap_or_default();
    for rule in &mapping.data {
        let mut matches = vec![];
        match_data_path(data, &rule.path, &mut vec![], &mut matches);
        for DataMatch { keys, entry } in matches {
            let key = entry
                .key
                .as_ref()
                .map(|key| key.value.clone())
                .unwrap_or_default();
            if rule.except_keys.contains(&key) {
                continue;
            }
            let subject = match rule.at {
                DataSubject::Key => entry.key.as_ref(),
                DataSubject::Value => match &entry.value {
                    Data::Scalar(scalar) => Some(scalar),
                    _ => None,
                },
            };
            let Some(subject) = subject else {
                continue;
            };
            if rule.except_values.contains(&subject.value) {
                continue;
            }
            let mut globals = liquid::object!({
                "value": subject.value,
                "text": node_text(&subject.node, source),
                "key": key,
                "keys": keys,
                "path": path,
                "dir_path": dir_path,
                "dir": dir,
            });
            let items: Vec<Option<String>> = match &rule.each {
                Some(each) => render(each, &globals)?
                    .split_whitespace()
                    .map(|item| Some(item.to_string()))
                    .collect(),
                None => vec![None],
            };
            let loc = location(&subject.node);
            // (Through the last line of the value, so that the line after it
            // ends the sticky header.)
            let nesting_range = match entry.value_node {
                Some(value) if rule.nesting && last_line(&value) > loc.lineno => SourceRange {
                    start_lineno: loc.lineno,
                    start_col: loc.col_start,
                    end_lineno: last_line(&value) + 1,
                    end_col: 0,
                },
                _ => SourceRange::default(),
            };
            for item in items {
                if let Some(item) = item {
                    globals.insert("item".into(), liquid::model::Value::scalar(item));
                }
                let sym = render(&rule.sym, &globals)?;
                let pretty = render(&rule.pretty, &globals)?;
                if let Some(structured) = &rule.structured {
                    let mut slots = vec![];
                    for binding in &structured.bindings {
                        let mut slot = json!({
                            "slotKind": binding.slot_kind,
                            "slotLang": binding.slot_lang,
                            "ownerLang": binding.owner_lang,
                        });
                        let (field, template) = match (&binding.sym, &binding.pretty) {
                            (Some(sym), _) => ("sym", sym),
                            (None, Some(pretty)) => ("pretty", pretty),
                            (None, None) => continue,
                        };
                        let value = render(template, &globals)?;
                        if value.is_empty() {
                            continue;
                        }
                        slot[field] = json!(value);
                        if let Some(impl_kind) = binding.impl_kind {
                            slot["implKind"] = json!(impl_kind);
                        }
                        slots.push(slot);
                    }
                    records.push(Record::Structured(
                        loc,
                        json!({
                            "loc": loc,
                            "structured": 1,
                            "pretty": pretty,
                            "sym": sym,
                            "kind": structured.kind,
                            "implKind": structured.impl_kind,
                            "bindingSlots": slots,
                        }),
                    ));
                }
                rule_records(
                    records,
                    rule.role,
                    loc,
                    &rule.syntax_kind,
                    rule.kind,
                    pretty,
                    sym,
                    nesting_range.clone(),
                    (String::new(), String::new()),
                );
            }
        }
    }
    Ok(())
}

/// The analysis lines of a file which already has analysis (ex: a C++ file's
/// from the clang plugin), with `records` added: the existing lines stay as
/// they are, and the records which aren't already there are added (but for
/// the file's definition, if the analysis has one).  Added targets without
/// contexts get the context of the existing target on their line which
/// starts nearest before them, or else first after them (ex: a C++ function's
/// call of `Preferences::GetBool`, for the use of the pref it names).
pub fn merge_records(existing: &str, mut records: Vec<Record>) -> Vec<String> {
    let mut lines: Vec<String> = existing
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect();
    let mut seen: std::collections::HashSet<String> = lines.iter().cloned().collect();
    let mut has_file_def = false;
    let mut contexts: std::collections::HashMap<u32, Vec<(u32, String, String)>> =
        Default::default();
    for line in &lines {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let (Some(_), Some(loc)) = (value.get("target"), value["loc"].as_str()) else {
            continue;
        };
        let loc = parse_location(loc);
        if loc.is_file_target()
            && value["sym"]
                .as_str()
                .is_some_and(|s| s.starts_with("FILE_"))
        {
            has_file_def = true;
        }
        let context = value["context"].as_str().unwrap_or_default();
        if !context.is_empty() {
            contexts.entry(loc.lineno).or_default().push((
                loc.col_start,
                context.to_string(),
                value["contextsym"].as_str().unwrap_or_default().to_string(),
            ));
        }
    }
    for record in &mut records {
        if let Record::Target(target) = record {
            if target.loc.is_file_target() && has_file_def {
                continue;
            }
            if target.data.context.is_empty()
                && let Some(candidates) = contexts.get(&target.loc.lineno)
            {
                let col = target.loc.col_start;
                let nearest = candidates
                    .iter()
                    .filter(|c| c.0 <= col)
                    .max_by_key(|c| c.0)
                    .or_else(|| candidates.iter().min_by_key(|c| c.0));
                if let Some((_, context, contextsym)) = nearest {
                    target.data.context = context.clone();
                    target.data.contextsym = contextsym.clone();
                }
            }
        }
        let json = record.to_json();
        if seen.insert(json.clone()) {
            lines.push(json);
        }
    }
    lines
}

/// The trees of `source`: its tree, and, if `continue_after_errors` and error
/// recovery ended the tree before the end of the text (ex: tree-sitter-yaml's,
/// at some invalid quoted strings), the trees of the rest, each from the line
/// after the previous tree's end.
pub(crate) fn parse_all(
    parser: &mut tree_sitter::Parser,
    source: &str,
    continue_after_errors: bool,
) -> Option<Vec<tree_sitter::Tree>> {
    let point = |byte: usize| {
        let before = &source[..byte];
        let row = before.matches('\n').count();
        let column = byte - before.rfind('\n').map_or(0, |i| i + 1);
        tree_sitter::Point { row, column }
    };
    let mut trees = vec![parser.parse(source, None)?];
    if !continue_after_errors {
        return Some(trees);
    }
    loop {
        let end = trees.last()?.root_node().end_byte();
        let Some(start) = source[end..].find('\n').map(|i| end + i + 1) else {
            break;
        };
        if source[start..].trim().is_empty() {
            break;
        }
        let range = tree_sitter::Range {
            start_byte: start,
            end_byte: source.len(),
            start_point: point(start),
            end_point: point(source.len()),
        };
        parser.set_included_ranges(&[range]).ok()?;
        let tree = parser.parse(source, None);
        parser.set_included_ranges(&[]).ok()?;
        match tree {
            Some(tree) if tree.root_node().end_byte() > start => trees.push(tree),
            _ => break,
        }
    }
    Some(trees)
}

/// Analyze the file at `path` (with `source` contents) with the `mappings`
/// which apply to it: its file definition, and the definitions, uses, and
/// keys the mappings' rules find, but for uses which name definitions in any
/// file (see `analyze_resolving`).
pub fn analyze(
    mappings: &[&CompiledMapping],
    path: &str,
    source: &str,
) -> Result<Vec<Record>, String> {
    Ok(analyze_resolving(mappings, path, source)?.records)
}

/// `analyze`, and the uses which name definitions in any file, for
/// `resolve_uses` once all of the files are analyzed.
pub fn analyze_resolving(
    mappings: &[&CompiledMapping],
    path: &str,
    source: &str,
) -> Result<Analysis, String> {
    let mut records = vec![file_def_record(path)];
    let mut unresolved = vec![];
    let mut parser = tree_sitter::Parser::new();
    // (Mappings with the same grammar share its parse.)
    let mut grammars: Vec<&str> = vec![];
    for mapping in mappings {
        if !grammars.contains(&mapping.grammar.as_str()) {
            grammars.push(&mapping.grammar);
        }
    }
    for grammar in grammars {
        let grammar_mappings: Vec<&&CompiledMapping> =
            mappings.iter().filter(|m| m.grammar == grammar).collect();
        let view = data_view_kind(grammar);
        let preprocessed = match view {
            Some(DataViewKind::Yaml) => preprocess_yaml(source),
            None => None,
        };
        let source = preprocessed.as_deref().unwrap_or(source);
        parser
            .set_language(&grammar_mappings[0].language)
            .map_err(|e| e.to_string())?;
        let trees = parse_all(&mut parser, source, view.is_some())
            .ok_or_else(|| format!("Couldn't parse {}", path))?;
        let roots: Vec<tree_sitter::Node> = trees.iter().map(|tree| tree.root_node()).collect();
        let data = match view {
            Some(DataViewKind::Yaml) => {
                let mut datas = roots.iter().map(|root| yaml_data(*root, source));
                let first = datas.next().unwrap_or(Data::Other);
                // (The rest of a document after errors is more of its map.)
                match first {
                    Data::Map(mut entries) => {
                        for more in datas {
                            if let Data::Map(more) = more {
                                entries.extend(more);
                            }
                        }
                        Data::Map(entries)
                    }
                    first => first,
                }
            }
            None => Data::Other,
        };
        for mapping in grammar_mappings {
            if !conditions_hold(&mapping.when, &data) {
                continue;
            }
            for root in &roots {
                query_records(mapping, *root, source, &mut records, &mut unresolved)?;
            }
            data_records(mapping, &data, path, source, &mut records)?;
        }
    }
    sort_records(&mut records);
    Ok(Analysis {
        records,
        unresolved,
    })
}

/// The definition of the file at `path`.
pub fn file_def_record(path: &str) -> Record {
    target_record(
        Location {
            lineno: 1,
            col_start: 0,
            col_end: 0,
        },
        AnalysisKind::Def,
        path.to_string(),
        format!("FILE_{}", mangle_file(path)),
        String::new(),
        String::new(),
    )
}

/// Sort a file's records by their locations.
pub fn sort_records(records: &mut [Record]) {
    // (Stable, so a location's records stay in the order they're made: the
    // structured record, the source, and then the target.)
    records.sort_by_key(|record| {
        let loc = record.loc();
        (loc.lineno, loc.col_start)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapping(name: &str, toml: &str) -> CompiledMapping {
        CompiledMapping::new(name, &toml::from_str(toml).unwrap()).unwrap()
    }

    #[test]
    fn test_fluent_mapping() {
        let mappings = builtin_mappings().unwrap();
        let fluent = mappings.iter().find(|m| m.applies_to("a/b.ftl")).unwrap();
        let source =
            "-brand = Firefox\nmsg = { -brand } { $n }\n    .title = { other.label } { $n }\n";
        let records = analyze(&[fluent], "a/b.ftl", source).unwrap();
        let targets: Vec<(String, String, String)> = records
            .iter()
            .filter_map(|r| match r {
                Record::Target(t) => Some((
                    format!("{:?}", t.data.kind),
                    t.data.sym.clone(),
                    t.data.contextsym.clone(),
                )),
                _ => None,
            })
            .collect();
        let target = |kind: &str, sym: &str, contextsym: &str| {
            (kind.to_string(), sym.to_string(), contextsym.to_string())
        };
        assert_eq!(
            targets,
            vec![
                target("Def", "FILE_a/b@2Eftl", ""),
                target("Def", "FTL_-brand", ""),
                target("Def", "FTL_msg", ""),
                target("Use", "FTL_-brand", "FTL_msg"),
                target("Use", "FTL_msg$n", "FTL_msg"),
                target("Def", "FTL_msg.title", "FTL_msg"),
                target("Use", "FTL_other", "FTL_msg.title"),
                target("Use", "FTL_other.label", "FTL_msg.title"),
                // (A variable in an attribute is its message's.)
                target("Use", "FTL_msg$n", "FTL_msg.title"),
            ]
        );
        // Multi-line definitions have nesting ranges for sticky headers, from
        // their names' lines.
        let nesting: Vec<String> = records
            .iter()
            .filter_map(|r| match r {
                Record::Source(s) if !s.data.nesting_range.is_empty() => {
                    Some(format!("{} {:?}", s.data.sym[0], s.data.nesting_range))
                }
                _ => None,
            })
            .collect();
        assert_eq!(nesting.len(), 1, "{:?}", nesting);
        assert!(nesting[0].starts_with("FTL_msg "), "{:?}", nesting);
    }

    /// The JSON of records, without the file's definition.
    fn json(records: &[Record]) -> Vec<String> {
        records.iter().skip(1).map(Record::to_json).collect()
    }

    #[test]
    fn test_data_paths() {
        let keys = mapping(
            "keys",
            r#"
grammar = "yaml"
paths = ["**/*.yaml"]
[[data]]
path = "**/*"
role = "key"
sym = "K_{{ text | at_escape }}"
pretty = "{{ text }}"
nesting = true
"#,
        );
        let source = "a:\n  b: 1\n  'c d':\n    - e: 2\n    - [f, {g: 3}]\nh: x\n";
        let records = analyze(&[&keys], "x.yaml", source).unwrap();
        assert_eq!(
            json(&records),
            [
                r#"{"loc":"00001:0-1","source":1,"syntax":"key","pretty":"a","sym":"K_a","nestingRange":"1:0-6:0"}"#,
                r#"{"loc":"00002:2-3","source":1,"syntax":"key","pretty":"b","sym":"K_b"}"#,
                r#"{"loc":"00003:2-7","source":1,"syntax":"key","pretty":"'c d'","sym":"K_@27c@20d@27","nestingRange":"3:2-6:0"}"#,
                r#"{"loc":"00004:6-7","source":1,"syntax":"key","pretty":"e","sym":"K_e"}"#,
                r#"{"loc":"00005:11-12","source":1,"syntax":"key","pretty":"g","sym":"K_g"}"#,
                r#"{"loc":"00006:0-1","source":1,"syntax":"key","pretty":"h","sym":"K_h"}"#,
            ]
        );

        // Values, structured records with binding slots (by pretty name, or by
        // symbol, unless their templates are empty), and Mozilla's
        // preprocessor substitutions (without which the first item's value
        // isn't valid).
        let prefs = mapping(
            "prefs",
            r#"
grammar = "yaml"
paths = ["**/*.yaml"]
[[data]]
path = "[]/name"
at = "value"
role = "def"
sym = "P_{{ value | at_escape }}"
pretty = "{{ value }}"
syntax_kind = "Pref"
[data.structured]
kind = "prefs"
impl_kind = "Test"
[[data.structured.bindings]]
slot_kind = "getter"
slot_lang = "cpp"
owner_lang = "prefs"
pretty = "ns::{{ value | snake_case }}"
[[data.structured.bindings]]
slot_kind = "getter"
slot_lang = "js"
owner_lang = "prefs"
impl_kind = "binding"
sym = "{% if value contains 'js' %}JS#{{ value | camel_case }}{% endif %}"
"#,
        );
        let source = "- name: a.b-c\n  value: @IS_NIGHTLY@\n- name: \"js.d_e\"\n";
        let records = analyze(&[&prefs], "x.yaml", source).unwrap();
        assert_eq!(
            json(&records),
            [
                r#"{"loc":"00001:8-13","structured":1,"pretty":"a.b-c","sym":"P_a@2Eb@2Dc","kind":"prefs","implKind":"Test","bindingSlots":[{"slotKind":"getter","slotLang":"cpp","ownerLang":"prefs","pretty":"ns::a_b_c"}]}"#,
                r#"{"loc":"00001:8-13","source":1,"syntax":"def","pretty":"Pref a.b-c","sym":"P_a@2Eb@2Dc"}"#,
                r#"{"loc":"00001:8-13","target":1,"kind":"def","pretty":"a.b-c","sym":"P_a@2Eb@2Dc"}"#,
                r#"{"loc":"00003:8-16","structured":1,"pretty":"js.d_e","sym":"P_js@2Ed_e","kind":"prefs","implKind":"Test","bindingSlots":[{"slotKind":"getter","slotLang":"cpp","ownerLang":"prefs","pretty":"ns::js_d_e"},{"slotKind":"getter","slotLang":"js","ownerLang":"prefs","sym":"JS#jsDE","implKind":"binding"}]}"#,
                r#"{"loc":"00003:8-16","source":1,"syntax":"def","pretty":"Pref js.d_e","sym":"P_js@2Ed_e"}"#,
                r#"{"loc":"00003:8-16","target":1,"kind":"def","pretty":"js.d_e","sym":"P_js@2Ed_e"}"#,
            ]
        );
        // (Several mappings' records together.)
        let records = analyze(&[&prefs, &keys], "x.yaml", source).unwrap();
        assert_eq!(records.len(), 1 + 6 + 3);
    }

    #[test]
    fn test_data_conditions() {
        let metrics = mapping(
            "metrics",
            r#"
grammar = "yaml"
paths = ["**/*.yaml"]
[[when]]
path = "$schema"
prefix = "moz://test/"
[[data]]
path = "*/*"
role = "def"
sym = "M_{{ keys[0] | snake_case }}::{{ key }}"
pretty = "{{ keys[0] }}.{{ key }}"
kind = "glean"
"#,
        );
        let source = "$schema: moz://test/1\n$tags: [a]\ncat.x:\n  one: {}\n  two:\n    type: y\n";
        let targets: Vec<String> = analyze(&[&metrics], "x.yaml", source)
            .unwrap()
            .iter()
            .filter_map(|r| match r {
                Record::Target(t) => Some(format!("{:?} {}", t.data.kind, t.data.sym)),
                _ => None,
            })
            .collect();
        assert_eq!(
            targets,
            [
                "Def FILE_x@2Eyaml",
                "Glean M_cat_x::one",
                "Glean M_cat_x::two"
            ]
        );
        let source = source.replace("moz://test/1", "other");
        assert_eq!(analyze(&[&metrics], "x.yaml", &source).unwrap().len(), 1);
    }

    #[test]
    fn test_data_rule_options() {
        // Predicates, `except_keys`, `each`, and the file's path.
        let tasks = mapping(
            "tasks",
            r#"
grammar = "yaml"
paths = ["**/*.yml"]
[[data]]
path = "*"
role = "def"
except_keys = ["task-defaults"]
sym = "T_{{ dir }}/{{ value }}"
pretty = "{{ dir }}/{{ value }}"
[[data]]
path = "**/run[using=script]/script"
at = "value"
role = "use"
sym = "FILE_{{ dir_path | at_escape }}/{{ value | at_escape }}"
pretty = "{{ dir_path }}/{{ value }}"
[[data]]
path = "**/workers/*"
role = "def"
each = "{{ value | regex_expansions }}"
sym = "W_{{ item }}"
pretty = "{{ item }}"
"#,
        );
        let source = "task-defaults:\n  x: 1\na:\n  run:\n    using: script\n    script: a.sh\nb:\n  run:\n    using: other\n    script: b.sh\n  workers:\n    w(|-big): {}\n";
        let targets: Vec<String> = analyze(&[&tasks], "k/build/t.yml", source)
            .unwrap()
            .iter()
            .filter_map(|r| match r {
                Record::Target(t) => Some(format!("{} {}", t.loc.lineno, t.data.sym)),
                _ => None,
            })
            .collect();
        assert_eq!(
            targets,
            [
                "1 FILE_k/build/t@2Eyml",
                "3 T_build/a",
                "6 FILE_k/build/a@2Esh",
                "7 T_build/b",
                "12 W_w",
                "12 W_w-big"
            ]
        );
    }

    /// The (kind, sym) of `source`'s targets with the builtin mappings for
    /// `path`, but for the file's definition.
    fn builtin_targets(path: &str, source: &str) -> Vec<(String, String)> {
        let mappings = builtin_mappings().unwrap();
        let applying: Vec<&CompiledMapping> = mappings
            .iter()
            .filter(|m| m.applies_to(path) && m.applies_to_source(source))
            .collect();
        analyze(&applying, path, source)
            .unwrap()
            .iter()
            .skip(1)
            .filter_map(|r| match r {
                Record::Target(t) => Some((format!("{:?}", t.data.kind), t.data.sym.clone())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn test_pref_mappings() {
        let target = |kind: &str, sym: &str| (kind.to_string(), sym.to_string());
        // Prefs files, despite their preprocessor directives and substitutions.
        assert_eq!(
            builtin_targets(
                "a/all.js",
                "#ifdef XP_WIN\npref(\"a.b\", true);\n#endif\nsticky_pref(\"c.d-e\", @X@);\nuser_pref(\"a.b\", false);\nfoo.pref(\"no\", 1);\n"
            ),
            [
                target("Def", "PREFS_a@2Eb"),
                target("Def", "PREFS_c@2Ed@2De"),
                target("Use", "PREFS_a@2Eb"),
            ]
        );
        // C++'s Preferences calls, with literals or `_ns` ones.
        assert_eq!(
            builtin_targets(
                "a/b.cpp",
                "void f() {\n  Preferences::GetBool(\"a.b\", false);\n  mozilla::Preferences::SetCString(\"c.d\"_ns, x);\n  Preferences::RegisterCallback(Cb, \"e.f\"_ns);\n  Other::GetBool(\"no\");\n}\n"
            ),
            [
                target("Use", "PREFS_a@2Eb"),
                target("Use", "PREFS_c@2Ed"),
                target("Use", "PREFS_e@2Ef"),
            ]
        );
        assert_eq!(
            builtin_targets(
                "a/b.rs",
                "fn f() -> bool { static_prefs::pref!(\"a.b\") }\n"
            ),
            [target("Use", "PREFS_a@2Eb")]
        );
        // (Files without the mappings' strings aren't theirs.)
        let mappings = builtin_mappings().unwrap();
        assert!(
            !mappings
                .iter()
                .any(|m| m.applies_to("a/b.cpp") && m.applies_to_source("int x;"))
        );
    }

    #[test]
    fn test_merge_records() {
        let existing = concat!(
            r#"{"loc":"00001:0","target":1,"kind":"def","pretty":"a.cpp","sym":"FILE_a@2Ecpp"}"#,
            "\n",
            r#"{"loc":"00002:2-13","target":1,"kind":"use","pretty":"mozilla::Preferences::GetBool","sym":"_ZGetBool","context":"f","contextsym":"_Zf"}"#,
            "\n",
        );
        let source = "void f() {\n  Preferences::GetBool(\"a.b\", false);\n}\n";
        let mappings = builtin_mappings().unwrap();
        let applying: Vec<&CompiledMapping> = mappings
            .iter()
            .filter(|m| m.applies_to("a.cpp") && m.applies_to_source(source))
            .collect();
        let records = analyze(&applying, "a.cpp", source).unwrap();
        let merged = merge_records(existing, records);
        // (The existing lines, and the pref's source and target, in the
        // context of the call's.)
        assert_eq!(merged.len(), 4, "{:?}", merged);
        assert!(merged[3].contains(r#""sym":"PREFS_a@2Eb","context":"f","contextsym":"_Zf""#));
        // Merging again changes nothing.
        let again = merge_records(
            &merged.join("\n"),
            analyze(&applying, "a.cpp", source).unwrap(),
        );
        assert_eq!(again, merged);
    }

    #[test]
    fn test_yaml_errors() {
        // Keys and pairs which parse around an error are still entries.
        let keys = mapping(
            "keys",
            r#"
grammar = "yaml"
paths = ["**/*.yaml"]
[[data]]
path = "**/*"
role = "key"
sym = "{{ value }}"
pretty = "{{ value }}"
"#,
        );
        let source = "key:\n    a: 1\n    b: 'x\\'y'\n    c: 2\n";
        let syms: Vec<String> = analyze(&[&keys], "x.yaml", source)
            .unwrap()
            .iter()
            .filter_map(|r| match r {
                Record::Source(s) => Some(s.data.sym[0].clone()),
                _ => None,
            })
            .collect();
        // (Including the ones after the tree that error recovery ended.)
        for key in ["key", "a", "c"] {
            assert!(syms.contains(&key.to_string()), "{:?}", syms);
        }
    }

    #[test]
    fn test_smdoc_mappings() {
        let mappings = builtin_mappings().unwrap();
        let smdoc = mappings
            .iter()
            .find(|m| m.name == "smdoc_cpp.toml")
            .unwrap();
        assert!(smdoc.applies_to("js/src/vm/A.cpp"));
        let source = concat!(
            "// [SMDOC] Atomics.wait, Atomics.waitAsync, and Atomics.notify\n",
            "//\n",
            "// Details.\n",
            "\n",
            "/*\n",
            " * [SMDOC] \"JIT Register-Set (Live)\":\n",
            " */\n",
            "\n",
            "// See [SMDOC] Atomics.wait for details, \"[SMDOC] JIT Register-Set\", and\n",
            "// the [SMDOC] for Debug.\n",
            "int x;  // As in [SMDOC] JIT Register-Set (Live).\n",
            "int y;  // Not [SMDOC] Atomics.\n",
        );
        let analysis = analyze_resolving(&[smdoc], "js/src/vm/A.cpp", source).unwrap();
        let targets = |records: &[Record]| -> Vec<String> {
            records
                .iter()
                .filter_map(|record| match record {
                    Record::Target(t) if !t.loc.is_file_target() => Some(format!(
                        "{}:{}-{} {} {}",
                        t.loc.lineno,
                        t.loc.col_start,
                        t.loc.col_end,
                        t.data.kind.to_ustr(),
                        t.data.sym
                    )),
                    _ => None,
                })
                .collect()
        };
        // Definitions are at their titles, without their quotes and trailing
        // punctuation.
        assert_eq!(
            targets(&analysis.records),
            [
                "1:11-62 doc-def SMDOC_Atomics@2Ewait@2C@20Atomics@2EwaitAsync@2C@20and@20Atomics@2Enotify",
                "6:12-35 doc-def SMDOC_JIT@20Register@2DSet@20@28Live@29",
            ]
        );
        // Citations name the definitions sharing the most leading (whole)
        // words, at those words, if just one does.
        assert_eq!(analysis.unresolved.len(), 5);
        let defs: Vec<(String, String)> = analysis
            .records
            .iter()
            .filter_map(|record| match record {
                Record::Target(t) if t.data.kind == AnalysisKind::DocDef => {
                    Some((t.data.pretty.clone(), t.data.sym.clone()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            targets(&resolve_uses(&defs, &analysis.unresolved)),
            [
                "9:15-27 doc-use SMDOC_Atomics@2Ewait@2C@20Atomics@2EwaitAsync@2C@20and@20Atomics@2Enotify",
                "9:50-66 doc-use SMDOC_JIT@20Register@2DSet@20@28Live@29",
                "11:25-47 doc-use SMDOC_JIT@20Register@2DSet@20@28Live@29",
            ]
        );
        // (But not when several share as many.)
        let more = [
            defs.clone(),
            vec![("Atomics.wait in workers".to_string(), "SMDOC_x".to_string())],
        ]
        .concat();
        assert_eq!(targets(&resolve_uses(&more, &analysis.unresolved)).len(), 2);
    }

    #[test]
    fn test_preprocess_yaml() {
        assert_eq!(
            preprocess_yaml("a: @X_Y@\n- @Z@ # z\nb: '@W@'\n").as_deref(),
            Some("a: \"X_Y\"\n- \"Z\" # z\nb: '@W@'\n")
        );
        assert_eq!(preprocess_yaml("a: x@Y@\n"), None);
    }
}
