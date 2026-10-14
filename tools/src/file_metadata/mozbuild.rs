//! moz.build files' `Files` rules (ex: `with Files("**/*.cpp"):
//! BUG_COMPONENT = ("Core", "DOM: Streams")`), read statically with
//! tree-sitter-python, and evaluated for a file the way mozbuild's
//! `BuildReader.files_info` does, for the rule giving a file its Bugzilla
//! component; and the test manifests they declare (`*_MANIFESTS`).
//!
//! The static reading handles literals, module-level constants (ex:
//! `component_engine = ("Core", "JavaScript Engine")`), string
//! concatenation, and `for` loops over literal lists, which is all that
//! Firefox's moz.build files use around `Files` (its derivation matched
//! `mach file-info bugzilla-automation`'s for every file).

use std::collections::{HashMap, HashSet};

use regex::Regex;
use serde::Serialize;
use tree_sitter::{Node, Parser};

use super::{LineIndex, mozpath_regex};

/// The directories whose moz.build files mozbuild's `BuildReader` ignores
/// (its fake moz.build files for testing it, and objdirs; and hidden
/// directories), as `mozpack.path.match` patterns.
const IGNORED_DIRS: &[&str] = &[
    "python/mozbuild/mozbuild/test",
    "testing/mozbase/moztest/tests/data",
    "obj*",
];

/// Whether mozbuild reads the moz.build file at the path.
pub fn is_read_mozbuild(path: &str) -> bool {
    lazy_static! {
        static ref IGNORED: Vec<Regex> = IGNORED_DIRS.iter().map(|p| mozpath_regex(p)).collect();
    }
    path.ends_with("moz.build")
        && (path == "moz.build" || path.ends_with("/moz.build"))
        && !path.split('/').any(|part| part.starts_with('.'))
        && !IGNORED.iter().any(|re| re.is_match(path))
}

#[derive(Clone, Debug, PartialEq)]
enum PyValue {
    Str(String),
    Seq(Vec<PyValue>),
    Bool(bool),
}

impl PyValue {
    fn as_str(&self) -> Option<&str> {
        match self {
            PyValue::Str(s) => Some(s),
            _ => None,
        }
    }
}

/// A `Files` context's assignment.
#[derive(Clone, Debug)]
pub struct Assignment {
    pub key: String,
    /// For `BUG_COMPONENT`, its (product, component).
    pub component: Option<(String, String)>,
    /// For `FINAL`, whether it's `True`.
    pub is_final: bool,
    pub line: usize,
}

/// A `with Files(...)` context.
#[derive(Clone, Debug)]
pub struct FilesRule {
    /// Its patterns, relative to its moz.build's directory.
    pub patterns: Vec<String>,
    regexes: Vec<Option<Regex>>,
    pub assignments: Vec<Assignment>,
    pub line: usize,
}

impl FilesRule {
    /// Whether it applies to a path relative to its moz.build's directory:
    /// a pattern equal to it, or with a `*` and matching it.
    fn matches(&self, relpath: &str) -> bool {
        self.patterns
            .iter()
            .zip(&self.regexes)
            .any(|(pattern, re)| pattern == relpath || re.as_ref().is_some_and(|re| re.is_match(relpath)))
    }
}

/// What a moz.build file declares.
#[derive(Debug, Default)]
pub struct Mozbuild {
    pub rules: Vec<FilesRule>,
    /// The test manifests (`.toml` values of `*_MANIFESTS` variables, in any
    /// branch), relative to its directory, with their variables.
    pub manifests: Vec<(String, String)>,
    /// Constructs it couldn't evaluate around `Files` rules.
    pub unresolved: Vec<String>,
}

struct Reader<'a> {
    source: &'a str,
    lines: LineIndex,
    out: Mozbuild,
}

/// Read a moz.build file's `Files` rules and declared test manifests.
pub fn read_mozbuild(source: &str) -> Mozbuild {
    let mut parser = Parser::new();
    let language: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
    if parser.set_language(&language).is_err() {
        return Mozbuild::default();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Mozbuild::default();
    };
    let mut reader = Reader {
        source,
        lines: LineIndex::new(source),
        out: Mozbuild::default(),
    };
    let mut env = HashMap::new();
    reader.statements(tree.root_node(), &mut env, false);
    reader.out
}

impl Reader<'_> {
    fn text(&self, node: Node) -> &str {
        &self.source[node.byte_range()]
    }

    fn line(&self, node: Node) -> usize {
        self.lines.line(node.start_byte())
    }

    /// A block's statements, in order.  In a conditional branch, only the
    /// test manifests count (mozbuild's metadata reading doesn't take
    /// branches by the build's configuration; Firefox has no `Files` rules in
    /// them).
    fn statements(&mut self, block: Node, env: &mut HashMap<String, PyValue>, conditional: bool) {
        let mut cursor = block.walk();
        let children: Vec<Node> = block.named_children(&mut cursor).collect();
        for stmt in children {
            match stmt.kind() {
                "expression_statement" => {
                    let Some(expr) = stmt.named_child(0) else {
                        continue;
                    };
                    self.assignment(expr, env, conditional);
                }
                "with_statement" => self.with_statement(stmt, env, conditional),
                "for_statement" => {
                    let (Some(left), Some(right), Some(body)) = (
                        stmt.child_by_field_name("left"),
                        stmt.child_by_field_name("right"),
                        stmt.child_by_field_name("body"),
                    ) else {
                        continue;
                    };
                    match (left.kind(), self.evaluate(right, env)) {
                        ("identifier", Some(PyValue::Seq(values))) => {
                            let name = self.text(left).to_string();
                            let saved = env.get(&name).cloned();
                            for value in values {
                                env.insert(name.clone(), value);
                                self.statements(body, env, conditional);
                            }
                            match saved {
                                Some(value) => env.insert(name, value),
                                None => env.remove(&name),
                            };
                        }
                        _ => {
                            if self.has_files(body) {
                                self.unresolved(stmt, "for loop around Files");
                            }
                            self.statements(body, &mut env.clone(), true);
                        }
                    }
                }
                "if_statement" => {
                    if self.has_files(stmt) {
                        self.unresolved(stmt, "Files under if");
                    }
                    let mut cursor = stmt.walk();
                    let branches: Vec<Node> = stmt.named_children(&mut cursor).collect();
                    for branch in branches {
                        let block = match branch.kind() {
                            "block" => Some(branch),
                            "elif_clause" => branch.child_by_field_name("consequence"),
                            "else_clause" => branch.child_by_field_name("body"),
                            _ => None,
                        };
                        if let Some(block) = block {
                            self.statements(block, &mut env.clone(), true);
                        }
                    }
                }
                "function_definition" | "decorated_definition" => {
                    if self.has_files(stmt) {
                        self.unresolved(stmt, "Files in a function");
                    }
                }
                _ => {}
            }
        }
    }

    fn assignment(&mut self, expr: Node, env: &mut HashMap<String, PyValue>, conditional: bool) {
        let (Some(left), Some(right)) = (
            expr.child_by_field_name("left"),
            expr.child_by_field_name("right"),
        ) else {
            return;
        };
        if left.kind() != "identifier" {
            return;
        }
        let name = self.text(left).to_string();
        let value = self.evaluate(right, env);
        if name.ends_with("_MANIFESTS") {
            for manifest in self.strings(right, env) {
                if manifest.ends_with(".toml") {
                    self.out.manifests.push((manifest, name.clone()));
                }
            }
        }
        if expr.kind() == "assignment" && !conditional {
            match value {
                Some(value) => env.insert(name, value),
                None => env.remove(&name),
            };
        }
    }

    /// The strings of a list (or a string), where they evaluate.
    fn strings(&self, node: Node, env: &HashMap<String, PyValue>) -> Vec<String> {
        match self.evaluate(node, env) {
            Some(PyValue::Str(s)) => vec![s],
            Some(PyValue::Seq(values)) => values
                .into_iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
            _ => {
                // (Some elements may evaluate even if others don't.)
                let mut cursor = node.walk();
                node.named_children(&mut cursor)
                    .filter_map(|e| match self.evaluate(e, env) {
                        Some(PyValue::Str(s)) => Some(s),
                        _ => None,
                    })
                    .collect()
            }
        }
    }

    fn with_statement(&mut self, stmt: Node, env: &mut HashMap<String, PyValue>, conditional: bool) {
        let Some(body) = stmt.child_by_field_name("body") else {
            return;
        };
        let call = stmt
            .named_child(0)
            .and_then(|clause| clause.named_child(0))
            .and_then(|item| item.child_by_field_name("value"))
            .filter(|value| value.kind() == "call");
        let is_files = call
            .and_then(|call| call.child_by_field_name("function"))
            .is_some_and(|f| self.text(f) == "Files");
        if !is_files {
            self.statements(body, env, conditional);
            return;
        }
        if conditional {
            return;
        }
        let call = call.unwrap();
        let mut patterns = vec![];
        if let Some(args) = call.child_by_field_name("arguments") {
            let mut cursor = args.walk();
            let args: Vec<Node> = args.named_children(&mut cursor).collect();
            for arg in args {
                match self.evaluate(arg, env) {
                    Some(PyValue::Str(s)) => patterns.push(s),
                    _ => {
                        self.unresolved(arg, "Files pattern");
                        return;
                    }
                }
            }
        }
        let mut assignments = vec![];
        let mut cursor = body.walk();
        let stmts: Vec<Node> = body.named_children(&mut cursor).collect();
        for s in stmts {
            let Some(expr) = s.named_child(0).filter(|e| e.kind() == "assignment") else {
                continue;
            };
            let (Some(left), Some(right)) = (
                expr.child_by_field_name("left"),
                expr.child_by_field_name("right"),
            ) else {
                continue;
            };
            let key = self.text(left).to_string();
            let line = self.line(expr);
            let value = self.evaluate(right, env);
            let mut assignment = Assignment {
                key: key.clone(),
                component: None,
                is_final: false,
                line,
            };
            match key.as_str() {
                "BUG_COMPONENT" => match value {
                    Some(PyValue::Seq(values)) if values.len() == 2 => {
                        match (&values[0], &values[1]) {
                            (PyValue::Str(product), PyValue::Str(component)) => {
                                assignment.component = Some((product.clone(), component.clone()));
                            }
                            _ => {
                                self.unresolved(right, "BUG_COMPONENT");
                                continue;
                            }
                        }
                    }
                    _ => {
                        self.unresolved(right, "BUG_COMPONENT");
                        continue;
                    }
                },
                "FINAL" => {
                    if value != Some(PyValue::Bool(true)) {
                        continue;
                    }
                    assignment.is_final = true;
                }
                _ => {}
            }
            assignments.push(assignment);
        }
        let regexes = patterns
            .iter()
            .map(|p| p.contains('*').then(|| mozpath_regex(p)))
            .collect();
        self.out.rules.push(FilesRule {
            patterns,
            regexes,
            assignments,
            line: self.line(stmt),
        });
    }

    fn has_files(&self, node: Node) -> bool {
        let mut cursor = node.walk();
        let mut stack = vec![node];
        while let Some(n) = stack.pop() {
            if n.kind() == "call"
                && n.child_by_field_name("function").is_some_and(|f| self.text(f) == "Files")
            {
                return true;
            }
            stack.extend(n.named_children(&mut cursor));
        }
        false
    }

    fn unresolved(&mut self, node: Node, what: &str) {
        self.out
            .unresolved
            .push(format!("{} on line {}", what, self.line(node)));
    }

    fn evaluate(&self, node: Node, env: &HashMap<String, PyValue>) -> Option<PyValue> {
        match node.kind() {
            "string" => self.string(node).map(PyValue::Str),
            "concatenated_string" => {
                let mut cursor = node.walk();
                let mut out = String::new();
                for s in node.named_children(&mut cursor) {
                    out.push_str(&self.string(s)?);
                }
                Some(PyValue::Str(out))
            }
            "tuple" | "list" => {
                let mut cursor = node.walk();
                let elements: Vec<Node> = node.named_children(&mut cursor).collect();
                elements
                    .into_iter()
                    .filter(|e| e.kind() != "comment")
                    .map(|e| self.evaluate(e, env))
                    .collect::<Option<Vec<_>>>()
                    .map(PyValue::Seq)
            }
            "parenthesized_expression" => node.named_child(0).and_then(|e| self.evaluate(e, env)),
            "identifier" => env.get(self.text(node)).cloned(),
            "true" => Some(PyValue::Bool(true)),
            "false" => Some(PyValue::Bool(false)),
            "binary_operator" => {
                let op = node.child_by_field_name("operator")?;
                if self.text(op) != "+" {
                    return None;
                }
                let left = self.evaluate(node.child_by_field_name("left")?, env)?;
                let right = self.evaluate(node.child_by_field_name("right")?, env)?;
                match (left, right) {
                    (PyValue::Str(a), PyValue::Str(b)) => Some(PyValue::Str(a + &b)),
                    (PyValue::Seq(mut a), PyValue::Seq(b)) => {
                        a.extend(b);
                        Some(PyValue::Seq(a))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// A string literal's value (not f-strings or bytes).
    fn string(&self, node: Node) -> Option<String> {
        let mut cursor = node.walk();
        let mut out = String::new();
        let mut raw = false;
        for child in node.children(&mut cursor) {
            match child.kind() {
                "string_start" => {
                    let prefix = self.text(child).trim_end_matches(['"', '\'']).to_lowercase();
                    if prefix.contains('f') || prefix.contains('b') {
                        return None;
                    }
                    raw = prefix.contains('r');
                }
                "string_content" => {
                    let text = self.text(child);
                    out.push_str(&if raw { text.to_string() } else { unescape(text) });
                }
                "interpolation" => return None,
                _ => {}
            }
        }
        Some(out)
    }
}

/// Python's simple string escapes.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some('\'') => out.push('\''),
            Some('"') => out.push('"'),
            Some('\n') => {}
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Where a file's metadata value comes from: a moz.build file's line.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct Origin {
    pub path: String,
    pub line: usize,
}

/// A tree's moz.build files' `Files` rules, by their directories.
#[derive(Default)]
pub struct FilesRules {
    by_dir: HashMap<String, Vec<FilesRule>>,
}

impl FilesRules {
    /// Add a moz.build file's rules (at its path).
    pub fn add(&mut self, mozbuild_path: &str, rules: Vec<FilesRule>) {
        let dir = mozbuild_path
            .rsplit_once('/')
            .map_or("", |(dir, _)| dir)
            .to_string();
        if !rules.is_empty() {
            self.by_dir.insert(dir, rules);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.by_dir.is_empty()
    }

    /// A file's Bugzilla component and the assignment giving it, like
    /// mozbuild's `files_info`: its ancestors' moz.build files from the
    /// root down, their matching `Files` contexts in order, the last
    /// assignment winning, but for those a context's `FINAL = True` froze
    /// (see `Files.__iadd__`).
    pub fn component(&self, path: &str) -> Option<((String, String), Origin)> {
        let mut component: Option<(&(String, String), Origin)> = None;
        let mut finalized: HashSet<&str> = HashSet::new();
        let mut dir_end = Some(0);
        while let Some(end) = dir_end {
            let dir = &path[..end];
            if let Some(rules) = self.by_dir.get(dir) {
                let relpath = if dir.is_empty() { path } else { &path[end + 1..] };
                for rule in rules {
                    if !rule.matches(relpath) {
                        continue;
                    }
                    for assignment in &rule.assignments {
                        if finalized.contains(assignment.key.as_str()) {
                            continue;
                        }
                        if assignment.is_final {
                            // (Its context's other keys, from now on.)
                            finalized.extend(
                                rule.assignments
                                    .iter()
                                    .map(|a| a.key.as_str())
                                    .filter(|k| *k != "FINAL"),
                            );
                            continue;
                        }
                        if let Some(value) = &assignment.component {
                            let mozbuild = if dir.is_empty() {
                                "moz.build".to_string()
                            } else {
                                format!("{}/moz.build", dir)
                            };
                            component = Some((
                                value,
                                Origin {
                                    path: mozbuild,
                                    line: assignment.line,
                                },
                            ));
                        }
                    }
                }
            }
            dir_end = next_dir_end(path, end);
        }
        component.map(|(value, origin)| (value.clone(), origin))
    }
}

/// After the ancestor directory `path[..end]` of a path (`end` 0 for the
/// root), the end of the next one down, if the path has one.
fn next_dir_end(path: &str, end: usize) -> Option<usize> {
    let from = if end == 0 { 0 } else { end + 1 };
    path[from..].find('/').map(|i| from + i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_mozbuild() {
        let source = r#"
component_engine = ("Core", "JavaScript Engine")
component_gc = ("Core", "JavaScript: GC")

with Files("**"):
    BUG_COMPONENT = component_engine

with Files("gc/**", "*.gc"):
    BUG_COMPONENT = component_gc
    SCHEDULES.exclusive += ["x"]

for header in ("a.h", "b.h"):
    with Files("public/" + header):
        BUG_COMPONENT = ("Core", "Headers")

with Files("final/**"):
    BUG_COMPONENT = ("Core", "Final")
    FINAL = True

MOCHITEST_MANIFESTS += ["test/mochitest.toml"]
if CONFIG["OS_TARGET"] == "WINNT":
    XPCSHELL_TESTS_MANIFESTS += ["test/unit/xpcshell.toml", "test/unit/other.ini"]
"#;
        let m = read_mozbuild(source);
        assert!(m.unresolved.is_empty(), "{:?}", m.unresolved);
        let rules: Vec<(Vec<&str>, Vec<(&str, Option<&str>)>, usize)> = m
            .rules
            .iter()
            .map(|r| {
                (
                    r.patterns.iter().map(String::as_str).collect(),
                    r.assignments
                        .iter()
                        .map(|a| (a.key.as_str(), a.component.as_ref().map(|c| c.1.as_str())))
                        .collect(),
                    r.line,
                )
            })
            .collect();
        assert_eq!(
            rules,
            vec![
                (vec!["**"], vec![("BUG_COMPONENT", Some("JavaScript Engine"))], 5),
                (
                    vec!["gc/**", "*.gc"],
                    vec![("BUG_COMPONENT", Some("JavaScript: GC"))],
                    8
                ),
                (vec!["public/a.h"], vec![("BUG_COMPONENT", Some("Headers"))], 13),
                (vec!["public/b.h"], vec![("BUG_COMPONENT", Some("Headers"))], 13),
                (
                    vec!["final/**"],
                    vec![("BUG_COMPONENT", Some("Final")), ("FINAL", None)],
                    16
                ),
            ]
        );
        assert_eq!(
            m.manifests,
            vec![
                ("test/mochitest.toml".to_string(), "MOCHITEST_MANIFESTS".to_string()),
                (
                    "test/unit/xpcshell.toml".to_string(),
                    "XPCSHELL_TESTS_MANIFESTS".to_string()
                ),
            ]
        );
    }

    #[test]
    fn test_component() {
        let mut rules = FilesRules::default();
        rules.add(
            "moz.build",
            read_mozbuild("with Files('**'):\n    BUG_COMPONENT = ('Core', 'General')\n").rules,
        );
        rules.add(
            "js/moz.build",
            read_mozbuild(
                "with Files('**'):\n    BUG_COMPONENT = ('Core', 'JS')\nwith Files('final/**'):\n    BUG_COMPONENT = ('Core', 'Final')\n    FINAL = True\nwith Files('README'):\n    BUG_COMPONENT = ('Core', 'Docs')\n",
            )
            .rules,
        );
        rules.add(
            "js/final/sub/moz.build",
            read_mozbuild("with Files('**'):\n    BUG_COMPONENT = ('Core', 'Ignored')\n").rules,
        );
        let c = |path: &str| {
            rules
                .component(path)
                .map(|(c, o)| (c.1, o.path, o.line))
        };
        assert_eq!(c("README"), Some(("General".to_string(), "moz.build".to_string(), 2)));
        assert_eq!(c("js/src/x.cpp"), Some(("JS".to_string(), "js/moz.build".to_string(), 2)));
        assert_eq!(c("js/README"), Some(("Docs".to_string(), "js/moz.build".to_string(), 7)));
        // (A literal pattern doesn't match a directory's files.)
        assert_eq!(c("js/README/x").map(|c| c.0), Some("JS".to_string()));
        // (FINAL freezes it, also for deeper moz.build files.)
        assert_eq!(
            c("js/final/sub/x.cpp"),
            Some(("Final".to_string(), "js/moz.build".to_string(), 4))
        );
    }

    #[test]
    fn test_is_read_mozbuild() {
        assert!(is_read_mozbuild("moz.build"));
        assert!(is_read_mozbuild("dom/moz.build"));
        assert!(!is_read_mozbuild("python/mozbuild/mozbuild/test/frontend/data/x/moz.build"));
        assert!(!is_read_mozbuild("objdir/moz.build"));
        assert!(!is_read_mozbuild(".hidden/moz.build"));
        assert!(!is_read_mozbuild("dom/shared.mozbuild"));
    }
}
