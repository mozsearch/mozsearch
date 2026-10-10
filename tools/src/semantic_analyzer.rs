//! Configurable tree-sitter-based analysis of data formats (see
//! data-semantics.md in the notes): mappings (TOML, in
//! `languages/semantic_mappings/`) say which files a grammar parses and which
//! of their nodes are definitions and uses of which symbols, with tree-sitter
//! queries and Liquid templates, and `semantic-analyze` turns their matches
//! into analysis records.
//!
//! A mapping has:
//! - `grammar`: the grammar which parses the files (ex: "fluent"), and
//!   `paths`, globs of the files it applies to (ex: "**/*.ftl");
//! - `scopes`: queries capturing an enclosing node as `@scope` and its name as
//!   `@name` (ex: a Fluent message), with templates for its symbol and pretty
//!   name, which are the context of the records inside it (and which the
//!   templates of the scopes and rules inside it can use);
//! - `rules`: queries capturing the node of a definition or use as `@id`
//!   (other captures are template variables too), with templates for its
//!   symbol and pretty name, and its syntax kind (the source records' pretty
//!   prefix, ex: "message").  A definition can have a `nesting` capture, whose
//!   node's lines are the definition's nesting range (for sticky headers).
//!
//! Templates see each capture's text by name, `scope.<name>` for the
//! innermost enclosing scope of each name (with its `name`, `sym`, and
//! `pretty`), `context` for the innermost enclosing scope, and `outer` for the
//! outermost one (ex: the message whose attribute's pattern a variable is in).
//! The scope a record names (ex: a message's own identifier) doesn't enclose
//! it.

use globset::{Glob, GlobSet, GlobSetBuilder};
use include_dir::{Dir, include_dir};
use serde::Deserialize;
use tree_sitter::StreamingIterator;

use crate::file_format::analysis::{
    AnalysisKind, AnalysisSource, AnalysisTarget, LineRange, Location, SourceRange, SourceTag,
    TargetTag, WithLocation,
};
use crate::file_format::analysis_manglings::mangle_file;

static MAPPINGS_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/languages/semantic_mappings");

#[derive(Debug, Deserialize)]
pub struct Mapping {
    pub grammar: String,
    pub paths: Vec<String>,
    #[serde(default)]
    pub scopes: Vec<ScopeRule>,
    #[serde(default)]
    pub rules: Vec<QueryRule>,
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
}

/// The language of a mapping's grammar.
fn grammar_language(grammar: &str) -> Result<tree_sitter::Language, String> {
    match grammar {
        "fluent" => Ok(tree_sitter_fluent::LANGUAGE.into()),
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
}

/// A mapping ready to analyze files with.
pub struct CompiledMapping {
    pub name: String,
    paths: GlobSet,
    language: tree_sitter::Language,
    scopes: Vec<CompiledScope>,
    rules: Vec<CompiledRule>,
}

impl CompiledMapping {
    pub fn new(name: &str, mapping: &Mapping) -> Result<CompiledMapping, String> {
        let language = grammar_language(&mapping.grammar)?;
        let liquid = liquid::ParserBuilder::with_stdlib()
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
        let mut paths = GlobSetBuilder::new();
        for path in &mapping.paths {
            paths.add(Glob::new(path).map_err(|e| e.to_string())?);
        }
        Ok(CompiledMapping {
            name: name.to_string(),
            paths: paths.build().map_err(|e| e.to_string())?,
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
                    Ok(CompiledRule {
                        role: rule.role,
                        query: query(&rule.query)?,
                        sym: template(&rule.sym)?,
                        pretty: template(&rule.pretty)?,
                        syntax_kind: rule.syntax_kind.clone(),
                        nesting: rule.nesting.clone(),
                    })
                })
                .collect::<Result<_, String>>()?,
            language,
        })
    }

    /// Whether the mapping applies to the file at `path` (relative to the
    /// tree's root).
    pub fn applies_to(&self, path: &str) -> bool {
        self.paths.is_match(path)
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

/// An analysis record for a file, in the order they're written.
pub enum Record {
    Source(WithLocation<AnalysisSource<String>>),
    Target(WithLocation<AnalysisTarget<String>>),
}

impl Record {
    fn loc(&self) -> &Location {
        match self {
            Record::Source(source) => &source.loc,
            Record::Target(target) => &target.loc,
        }
    }

    pub fn to_json(&self) -> String {
        match self {
            Record::Source(source) => serde_json::to_string(source).unwrap(),
            Record::Target(target) => serde_json::to_string(target).unwrap(),
        }
    }
}

/// Analyze the file at `path` (with `source` contents) with `mapping`: its file
/// definition, and the definitions and uses its rules find.
pub fn analyze(mapping: &CompiledMapping, path: &str, source: &str) -> Result<Vec<Record>, String> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&mapping.language)
        .map_err(|e| e.to_string())?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| format!("Couldn't parse {}", path))?;
    let root = tree.root_node();

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
    let mut records = vec![];
    let file_loc = Location {
        lineno: 1,
        col_start: 0,
        col_end: 0,
    };
    records.push(Record::Target(WithLocation {
        loc: file_loc,
        data: AnalysisTarget {
            target: TargetTag::Target,
            kind: AnalysisKind::Def,
            pretty: path.to_string(),
            sym: format!("FILE_{}", mangle_file(path)),
            context: String::new(),
            contextsym: String::new(),
            peek_range: LineRange::default(),
            arg_ranges: vec![],
        },
    }));
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
            let globals = template_globals(&enclosing, &captures);
            let sym = rule.sym.render(&globals).map_err(|e| e.to_string())?;
            let pretty = rule.pretty.render(&globals).map_err(|e| e.to_string())?;
            let (context, contextsym) = enclosing
                .last()
                .map(|s| (s.pretty.clone(), s.sym.clone()))
                .unwrap_or_default();
            let loc = location(&id);
            // (From the definition's line, since the nesting node can start
            // with a line break, ex: a Fluent attribute's.)
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
            let (syntax, kind) = match rule.role {
                Role::Def => ("def", AnalysisKind::Def),
                Role::Use => ("use", AnalysisKind::Use),
            };
            records.push(Record::Source(WithLocation {
                loc,
                data: AnalysisSource {
                    source: SourceTag::Source,
                    syntax: vec![syntax.to_string()],
                    pretty: format!("{} {}", rule.syntax_kind, pretty),
                    sym: vec![sym.clone()],
                    no_crossref: false,
                    nesting_range,
                    type_pretty: None,
                    type_sym: None,
                    arg_ranges: vec![],
                    expansion_info: None,
                    confidence: None,
                },
            }));
            records.push(Record::Target(WithLocation {
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
            }));
        }
    }
    // (Stable, so a location's source comes before its target.)
    records.sort_by_key(|record| {
        let loc = record.loc();
        (loc.lineno, loc.col_start)
    });
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fluent_mapping() {
        let mappings = builtin_mappings().unwrap();
        let fluent = mappings.iter().find(|m| m.applies_to("a/b.ftl")).unwrap();
        let source =
            "-brand = Firefox\nmsg = { -brand } { $n }\n    .title = { other.label } { $n }\n";
        let records = analyze(fluent, "a/b.ftl", source).unwrap();
        let targets: Vec<(String, String, String)> = records
            .iter()
            .filter_map(|r| match r {
                Record::Target(t) => Some((
                    format!("{:?}", t.data.kind),
                    t.data.sym.clone(),
                    t.data.contextsym.clone(),
                )),
                Record::Source(_) => None,
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
}
