//! Symbols' history digests (`file_format::recency::Recency`): how recently,
//! and how much, each symbol's code changed, which crossref computes as it
//! reads each file's analysis, for `/query/`'s recency facets.
//!
//! The history's files-delta journals record each revision's (or, for older
//! weeks, each week's) changes to a file by lexical context (the syntax token
//! files' contexts, ex: `mozilla::dom::Foo::Bar`), which aren't crossref's
//! semantic symbols, so crossref links them by location: a definition's symbol
//! gets the changes of the context its definition is in (a container's own
//! tokens, like its name, are in it; see `cst_tokenizer`), including those of
//! the contexts nested in it, if that's the symbol's own context (by name; see
//! `is_own_context`).  Otherwise the context's changes aren't the symbol's (ex:
//! a method only declared in its class, or a definition the tokenizer didn't
//! make a context of, so it's in its namespace, or misparsed into another
//! context), and no data is better than wrong data, so it gets none (see
//! `Linkage`).  The syntax token files don't say where their
//! tokens are, so we find them in the source, in order (they're its text
//! without the whitespace), which also means that the contexts are the
//! history's own, whichever version of the tokenizer made them.

use std::collections::{BTreeMap, HashSet};
use std::ops::Bound;
use std::path::Path;

use chrono::{DateTime, NaiveDate, Weekday};
use git2::Oid;

use crate::file_format::config::{GitData, ThreadLocalRepository, timeline_commit_to_meta};
use crate::file_format::history::syntax_files::{split_token_line, token_file_lines};
use crate::file_format::history::syntax_files_struct::{FileStructureHeader, FileStructureRow};
use crate::file_format::history::timeline_common::JournalVersionRef;
use crate::file_format::history::timeline_files_delta::FileDeltaRecord;
use crate::file_format::recency::{FileRecency, Recency};
use crate::hyperblame::journals::{JournalKind, JournalReader};

/// The history contexts of a file's source, as the byte offsets where runs of
/// tokens with the same context start.
pub struct FileContexts {
    runs: Vec<(u32, String)>,
}

impl FileContexts {
    /// Line up a file's syntax token file (`files/PATH` in the syntax repo)
    /// with its source, or None if they don't match (ex: the history is of
    /// another revision of the file).  Text can be between tokens, since not
    /// all of it is tokens (ex: the words of tree-sitter-rust's `//` comments,
    /// whose nodes only have a child for the `//`).
    pub fn align(source: &str, token_file: &str) -> Option<FileContexts> {
        let mut runs: Vec<(u32, String)> = vec![];
        let mut pos = 0;
        for line in token_file_lines(token_file) {
            let token = split_token_line(line);
            let start = pos + source[pos..].find(token.token)?;
            pos = start + token.token.len();
            if runs.last().map(|(_, context)| context.as_str()) != Some(token.context) {
                runs.push((u32::try_from(start).ok()?, token.context.to_string()));
            }
        }
        Some(FileContexts { runs })
    }

    /// The context of the token at (or before) byte `offset`.
    pub fn context_at(&self, offset: u32) -> Option<&str> {
        let run = self.runs.partition_point(|(start, _)| *start <= offset);
        Some(self.runs.get(run.checked_sub(1)?)?.1.as_str())
    }
}

/// Each context's changes (not counting moves) by age, from a file's
/// files-delta records (newest first), as of `indexed` (the indexed revision's
/// date), without backouts or the revisions they back out (which detail
/// records say; weekly summaries can't).
pub fn context_changes(
    records: &[FileDeltaRecord],
    indexed: NaiveDate,
) -> BTreeMap<String, Recency> {
    let mut changes: BTreeMap<String, Recency> = BTreeMap::new();
    let mut backed_out: HashSet<&str> = HashSet::new();
    for record in records {
        let (date, group) = match record {
            FileDeltaRecord::Detail(detail) => {
                if !detail.desc.backs_out.is_empty() {
                    backed_out.extend(detail.desc.backs_out.iter().map(String::as_str));
                    continue;
                }
                if backed_out.contains(detail.desc.source_rev.as_str()) {
                    continue;
                }
                let date = detail
                    .desc
                    .iso_date
                    .get(..10)
                    .and_then(|day| NaiveDate::parse_from_str(day, "%Y-%m-%d").ok());
                (date, &detail.delta.symbol_group)
            }
            FileDeltaRecord::Summary(summary) => {
                let (year, newest_week, _) = summary.desc.iso_week_range;
                let date =
                    NaiveDate::from_isoywd_opt(year as i32, newest_week as u32, Weekday::Thu);
                (date, &summary.symbol_group)
            }
        };
        let Some(date) = date else {
            continue;
        };
        let bin = Recency::bin_for_age_days((indexed - date).num_days());
        for (context, delta) in &group.symbol_deltas {
            let totals = &delta.token_totals;
            let tokens = totals.added + totals.removed + totals.evolved_from + totals.evolved_into;
            if tokens > 0 {
                changes.entry(context.clone()).or_default().add(bin, tokens);
            }
        }
    }
    changes
}

/// The last segment of a pretty name or context (ex: "Bar" for "Foo::Bar", or
/// JS's "Foo.bar"), without whitespace (contexts encode it as "%20").
fn last_segment(name: &str) -> String {
    raw_last_segment(name)
        .replace("%20", "")
        .split_whitespace()
        .collect()
}

/// The last segment of a pretty name as its source text, without template
/// arguments (ex: "~Wrapper" for "ns::~Wrapper<Func>", but "operator<" for
/// "Foo::operator<").
fn raw_last_segment(name: &str) -> &str {
    let last = name
        .rsplit("::")
        .next()
        .unwrap_or(name)
        .rsplit('.')
        .next()
        .unwrap_or(name);
    match last.find('<') {
        Some(start) if start > 0 && last.ends_with('>') && !last.starts_with("operator") => {
            &last[..start]
        }
        _ => last,
    }
}

/// Whether the source at a definition's location is the symbol `pretty`'s
/// name, which it isn't for definitions from macros (whose locations are the
/// macros' invocations, ex: js/src/jit/LIR-shared.h's classes).
pub fn names_symbol(source: &str, offset: u32, pretty: &str) -> bool {
    source
        .get(offset as usize..)
        .is_some_and(|text| text.starts_with(raw_last_segment(pretty)))
}

/// Whether a definition at byte `offset` of `source` is from a macro whose
/// text names it: in a `#define` (ex: js/src/jit/MIR.h's `ALLOW_CLONE`),
/// an argument of an ALL-CAPS macro call (ex: an X-macro list's
/// `MACRO_(abort, "abort")`), or in a Rust macro invocation (ex:
/// windows-sys's `link!(... fn CompareStringA(...))`).  Tokenizers don't make
/// contexts of those.
pub fn is_from_macro(source: &str, offset: u32) -> bool {
    let offset = (offset as usize).min(source.len());
    let line_start = source[..offset].rfind('\n').map_or(0, |i| i + 1);
    let prefix = &source[line_start..offset];
    // (A Rust macro invocation's opening, ex: `link!(`.)
    if prefix.contains("!(") || prefix.contains("!{") || prefix.contains("![") {
        return true;
    }
    // (An ALL-CAPS macro's first argument.)
    if let Some(call) = prefix.trim_end().strip_suffix('(') {
        let name = call
            .trim_end()
            .rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .next()
            .unwrap_or("");
        if name.len() > 1
            && name.chars().any(|c| c.is_ascii_uppercase())
            && !name.chars().any(|c| c.is_ascii_lowercase())
        {
            return true;
        }
    }
    // (A `#define`'s line, or one continued from it.)
    let mut start = line_start;
    loop {
        if source[start..].trim_start().starts_with('#')
            && source[start..]
                .trim_start()
                .trim_start_matches('#')
                .trim_start()
                .starts_with("define")
        {
            return true;
        }
        if start == 0 {
            return false;
        }
        let previous = source[..start - 1].rfind('\n').map_or(0, |i| i + 1);
        if !source[previous..start - 1].trim_end().ends_with('\\') {
            return false;
        }
        start = previous;
    }
}

/// Whether a history context is the symbol `pretty`'s own, by their last
/// segments (contexts can lack the namespaces' segments, ex: Rust modules', or
/// differ, ex: JS objects'), so that its changes are the symbol's.
pub fn is_own_context(pretty: &str, context: &str) -> bool {
    context != "%" && !pretty.is_empty() && last_segment(pretty) == last_segment(context)
}

/// The pretty name of the symbol `pretty`'s parent (ex: "Foo" for "Foo::Bar"
/// or "Foo.bar"), if it has one.
fn parent_of(pretty: &str) -> Option<&str> {
    let cut = [pretty.rfind("::"), pretty.rfind('.')]
        .into_iter()
        .flatten()
        .max()?;
    Some(&pretty[..cut])
}

/// How a definition (or declaration) links to the history (see
/// `FileDigests::linkage`).
#[derive(Debug, PartialEq)]
pub enum Linkage<'a> {
    /// The history context at the definition is the symbol's own: its changes,
    /// with those of the contexts nested in it (which might be none).
    Own(Recency, &'a str),
    /// A namespace's own declaration, which doesn't get a digest (namespaces
    /// span files, and their contexts' changes would be too broad anyway).
    Namespace,
    /// The history context at the definition isn't the symbol's own: the top
    /// level ("%"), a namespace, or another context (ex: its class).
    Other(&'a str),
    /// The history has no token there.
    Missing,
}

/// A context's changes, including those of the contexts nested in it.
pub fn nested_changes(changes: &BTreeMap<String, Recency>, context: &str) -> Recency {
    let mut recency = Recency::default();
    for (key, key_changes) in changes.range::<str, _>((Bound::Included(context), Bound::Unbounded))
    {
        let Some(rest) = key.strip_prefix(context) else {
            break;
        };
        if rest.is_empty() || rest.starts_with("::") {
            recency.accumulate(key_changes);
        }
    }
    recency
}

/// A tree's history as of its indexed (checked out) revision.
pub struct HistoryDigests<'a> {
    syntax: &'a ThreadLocalRepository,
    timeline: &'a ThreadLocalRepository,
    syntax_tree: Oid,
    timeline_rev: Oid,
    indexed: NaiveDate,
}

impl<'a> HistoryDigests<'a> {
    /// The history of the tree's indexed revision, if it has one.
    pub fn open(git: &'a GitData) -> Option<HistoryDigests<'a>> {
        let history = git.history.as_ref()?;
        let head = git.repo.head().ok()?.peel_to_commit().ok()?;
        let timeline_commit = history.timeline_commit(head.id())?;
        let meta = timeline_commit_to_meta(&timeline_commit);
        let syntax_tree = history.syntax.find_commit(meta.syntax_rev).ok()?.tree_id();
        Some(HistoryDigests {
            syntax: &history.syntax,
            timeline: &history.timeline,
            syntax_tree,
            timeline_rev: timeline_commit.id(),
            indexed: DateTime::from_timestamp(head.time().seconds(), 0)?.date_naive(),
        })
    }

    /// Whether the history has (the tokens of) a file.
    pub fn has_file(&self, path: &str) -> bool {
        self.syntax
            .find_tree(self.syntax_tree)
            .is_ok_and(|tree| tree.get_path(Path::new(&format!("files/{}", path))).is_ok())
    }

    /// The digests of a file's contexts, if the history has the file and it
    /// lines up with `source`.
    pub fn file(&self, path: &str, source: &str) -> Option<FileDigests> {
        let syntax = &**self.syntax;
        let tree = syntax.find_tree(self.syntax_tree).ok()?;
        let read = |path: &str| -> Option<String> {
            let entry = tree.get_path(Path::new(path)).ok()?;
            let blob = syntax.find_blob(entry.id()).ok()?;
            String::from_utf8(blob.content().to_vec()).ok()
        };
        let contexts = FileContexts::align(source, &read(&format!("files/{}", path))?)?;
        let structure = read(&format!("files-struct/{}", path)).unwrap_or_default();
        let mut structure = structure.lines();
        // (Files in languages without grammars, ex: Kotlin, Java, and
        // Objective-C++, are plain text, without contexts.)
        let header: FileStructureHeader = structure
            .next()
            .and_then(|line| serde_json::from_str(line).ok())
            .unwrap_or_default();
        let namespaces = structure
            .filter_map(|line| serde_json::from_str::<FileStructureRow>(line).ok())
            .filter(|row| row.kind == "namespace")
            .map(|row| row.pretty)
            .collect();
        Some(FileDigests {
            contexts,
            namespaces,
            has_grammar: header.lang.is_some_and(|lang| lang != "none"),
            changes: context_changes(&self.records(path)?, self.indexed),
        })
    }

    /// The whole-file digest of a file without analysis (ex: docs), whose
    /// lines can't have digests of their own (see `FileRecency`), if the
    /// history has changes to it.
    pub fn unanalyzed_file(&self, path: &str) -> Option<FileRecency> {
        let mut file = Recency::default();
        for changes in context_changes(&self.records(path)?, self.indexed).values() {
            file.accumulate(changes);
        }
        (!file.is_empty()).then(|| FileRecency {
            file,
            scopes: BTreeMap::new(),
            analyzed: false,
        })
    }

    /// The records of a file's files-delta journal (none if it doesn't have
    /// one).
    fn records(&self, path: &str) -> Option<Vec<FileDeltaRecord>> {
        JournalReader::new(self.timeline)
            .records::<FileDeltaRecord>(&JournalVersionRef {
                timeline_rev: self.timeline_rev.to_string(),
                path: JournalKind::FilesDelta.journal_path(path),
            })
            .ok()
    }
}

/// A file's history contexts, with their changes.
pub struct FileDigests {
    contexts: FileContexts,
    namespaces: HashSet<String>,
    /// Whether the history tokenized the file with a grammar, so that it has
    /// contexts.
    pub has_grammar: bool,
    changes: BTreeMap<String, Recency>,
}

impl FileDigests {
    /// The file's digests for what symbols' don't cover (see `FileRecency`).
    /// Files without a grammar are as if they had no analysis, since their
    /// lines have no contexts.
    pub fn file_recency(&self) -> FileRecency {
        let mut recency = FileRecency {
            analyzed: self.has_grammar,
            ..Default::default()
        };
        for (context, changes) in &self.changes {
            recency.file.accumulate(changes);
            if self.has_grammar && (context == "%" || self.namespaces.contains(context)) {
                recency.scopes.insert(context.clone(), *changes);
            }
        }
        recency
    }

    /// Whether a context is one of the file's namespaces.
    pub fn is_namespace(&self, context: &str) -> bool {
        self.namespaces.contains(context)
    }

    /// Whether the definition of the symbol `pretty` in `context` (not its
    /// own) means the history's contexts are wrong there (ex: a misparse put
    /// it in another class, or didn't make a context of it): if its parent is
    /// one of the file's contexts but it isn't in it (where members, ex: enum
    /// variants and fields, are), or if it's a function or method (which
    /// should have its own context) at the top level or in a namespace.
    /// (Otherwise it could be an item nested in a function, ex: a Rust
    /// `lazy_static!` static, which scip names by its module.)
    pub fn is_misplaced(&self, pretty: &str, context: &str, is_function: bool) -> bool {
        let at_scope = context == "%" || self.namespaces.contains(context);
        let Some(parent) = parent_of(pretty) else {
            return is_function && at_scope;
        };
        let parent = last_segment(parent);
        if last_segment(context) == parent {
            return false;
        }
        let parent_is_context = self
            .contexts
            .runs
            .iter()
            .any(|(_, context)| last_segment(context) == parent);
        parent_is_context || (is_function && at_scope)
    }

    /// How the definition (or declaration) of the symbol `pretty` at byte
    /// `offset` of the source links to the history.
    pub fn linkage(&self, pretty: &str, offset: u32) -> Linkage<'_> {
        let Some(context) = self.contexts.context_at(offset) else {
            return Linkage::Missing;
        };
        let own = is_own_context(pretty, context);
        if self.namespaces.contains(context) {
            if own {
                return Linkage::Namespace;
            }
            return Linkage::Other(context);
        }
        if !own {
            return Linkage::Other(context);
        }
        Linkage::Own(nested_changes(&self.changes, context), context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_align() {
        let source = "namespace a {\nvoid f() {\n  g(\"x y\");\n}\n}  // a\n";
        let token_file = "a k namespace\na i a\na o {\na::f i void\na::f i f\na::f o (\na::f o )\na::f o {\na::f i g\na::f o (\na::f s \"x\na::f s y\"\na::f o )\na::f o ;\na::f o }\na o }\na c //\na c a\n";
        let contexts = FileContexts::align(source, token_file).unwrap();
        let at = |needle: &str| contexts.context_at(source.find(needle).unwrap() as u32);
        assert_eq!(at("namespace"), Some("a"));
        assert_eq!(at("void"), Some("a::f"));
        assert_eq!(at("y\""), Some("a::f"));
        assert_eq!(at("}\n}"), Some("a::f"));
        assert_eq!(at("// a"), Some("a"));
        // Another revision's tokens don't line up.
        assert!(FileContexts::align("int main() {}\n", token_file).is_none());
        // Untokenized text (ex: tree-sitter-rust's `//` comments' words) is
        // skipped.
        let contexts = FileContexts::align(
            "// one two\nfn f() {}\n",
            "% c //\nf k fn\nf i f\nf o (\nf o )\nf o {\nf o }\n",
        )
        .unwrap();
        assert_eq!(contexts.context_at(12), Some("f"));
    }

    #[test]
    fn test_changes() {
        let record = |line: &str| serde_json::from_str::<FileDeltaRecord>(line).unwrap();
        let detail = |rev: &str, date: &str, backs_out: &str, deltas: &str| {
            record(&format!(
                r#"{{"type":"Detail","source_rev":"{}","syntax_rev":"s{}","iso_date":"{}","backs_out":[{}],"change":"changed","symbol_deltas":{}}}"#,
                rev, rev, date, backs_out, deltas
            ))
        };
        let changed = |tokens: u32| {
            format!(
                r#"{{"change":"changed","token_totals":{{"added":{},"moved":7}},"token_changes":{{}}}}"#,
                tokens
            )
        };
        let records = vec![
            // A backout of r2, newest first.
            detail(
                "r3",
                "2026-10-01T00:00:00Z",
                r#""r2""#,
                &format!(r#"{{"a::f":{}}}"#, changed(100)),
            ),
            detail(
                "r2",
                "2026-09-30T00:00:00Z",
                "",
                &format!(r#"{{"a::f":{}}}"#, changed(100)),
            ),
            detail(
                "r1",
                "2026-09-20T00:00:00Z",
                "",
                &format!(
                    r#"{{"a::f":{},"a::f::g":{},"a::fg":{}}}"#,
                    changed(3),
                    changed(4),
                    changed(5)
                ),
            ),
            record(&format!(
                r#"{{"type":"Summary","source_revs":["r0"],"preds":[],"iso_week_range":[2024,10,10],"symbol_deltas":{{"a::f":{}}}}}"#,
                changed(2)
            )),
        ];
        let changes = context_changes(&records, NaiveDate::from_ymd_opt(2026, 10, 2).unwrap());
        // r1 is 12 days old (1-2 weeks), and the summary's week ~2.5 years.
        assert_eq!(changes["a::f"], Recency([0, 3, 0, 0, 0, 0, 0, 0, 2, 0]));
        // With the nested `a::f::g` (but not `a::fg`).
        assert_eq!(
            nested_changes(&changes, "a::f"),
            Recency([0, 7, 0, 0, 0, 0, 0, 0, 2, 0])
        );
        assert_eq!(nested_changes(&changes, "b"), Recency::default());

        // The file's digests: all of it, and its top level's and namespaces'.
        let digests = FileDigests {
            contexts: FileContexts { runs: vec![] },
            namespaces: ["a".to_string()].into_iter().collect(),
            has_grammar: true,
            changes: [
                ("%".to_string(), Recency([1, 0, 0, 0, 0, 0, 0, 0, 0, 0])),
                ("a".to_string(), Recency([0, 2, 0, 0, 0, 0, 0, 0, 0, 0])),
                ("a::f".to_string(), Recency([4, 0, 0, 0, 0, 0, 0, 0, 0, 0])),
            ]
            .into_iter()
            .collect(),
        };
        let file = digests.file_recency();
        assert_eq!(file.file, Recency([5, 2, 0, 0, 0, 0, 0, 0, 0, 0]));
        assert_eq!(file.scopes.keys().collect::<Vec<_>>(), vec!["%", "a"]);

        // Definitions get their own contexts' changes, with nested ones, and
        // nothing from contexts that aren't theirs (ex: `y`, which the
        // tokenizer didn't make a context of, so it's in its namespace).
        let source = "int x;\nnamespace a {\nint* y();\nvoid f() {}\n}\n";
        let digests = FileDigests {
            contexts: FileContexts::align(
                source,
                "% i int\n% i x\n% o ;\na k namespace\na i a\na o {\na i int\na o *\na i y\na o (\na o )\na o ;\na::f i void\na::f i f\na::f o (\na::f o )\na::f o {\na::f o }\na o }\n",
            )
            .unwrap(),
            ..digests
        };
        let at = |pretty: &str, needle: &str| {
            digests.linkage(pretty, source.find(needle).unwrap() as u32)
        };
        assert_eq!(at("x", "x;"), Linkage::Other("%"));
        assert_eq!(at("a::y", "y()"), Linkage::Other("a"));
        assert_eq!(
            at("a::f", "f()"),
            Linkage::Own(Recency([4, 0, 0, 0, 0, 0, 0, 0, 0, 0]), "a::f")
        );
        assert_eq!(at("a", "a {"), Linkage::Namespace);

        // `a::y`, whose namespace is a context in the file, is misplaced at
        // its top level or in another context, but not in its parent, nor at
        // the top level if its parent isn't a context (ex: a Rust module).
        assert!(digests.is_misplaced("a::y", "%", false));
        assert!(digests.is_misplaced("a::y", "a::f", false));
        assert!(!digests.is_misplaced("a::y", "a", true));
        assert!(!digests.is_misplaced("module::Y", "%", false));
        assert!(!digests.is_misplaced("module::Y", "a::f", false));
        assert!(!digests.is_misplaced("Enum::Variant", "Enum", false));
        assert!(!digests.is_misplaced("x", "%", false));
        // (Functions and methods should have their own contexts, ex: an
        // out-of-line method definition whose class isn't a context in the
        // file.)
        assert!(digests.is_misplaced("Class::method", "a", true));
        assert!(digests.is_misplaced("function", "%", true));
    }

    #[test]
    fn test_own_contexts() {
        assert!(is_own_context(
            "mozilla::dom::Foo::Bar",
            "mozilla::dom::Foo::Bar"
        ));
        assert!(is_own_context(
            "cmd_pipeline::chunked_gzip::extract_rows",
            "extract_rows"
        ));
        assert!(is_own_context(
            "SessionStoreInternal.getClosedTabCount",
            "_SessionStore::getClosedTabCount"
        ));
        assert!(is_own_context("Foo::operator new", "Foo::operator%20new"));
        assert!(is_own_context(
            "ns::Wrapper::~Wrapper<Func>",
            "ns::Wrapper::~Wrapper"
        ));
        assert!(!is_own_context("Foo::operator<", "Foo"));
        assert!(names_symbol("void Foo::Bar() {}", 10, "ns::Foo::Bar"));
        assert!(!names_symbol("LIR_HEADER(Abs)", 0, "js::jit::LAbs::LAbs"));
        let macros = "#define ALLOW_CLONE(T) \\\n  bool canClone() const { return true; }\nMACRO_(abort, \"abort\")\nlink!(\"k32\" fn Compare());\nvoid f() {}\n";
        let at = |needle: &str| is_from_macro(macros, macros.find(needle).unwrap() as u32);
        assert!(at("canClone"));
        assert!(at("abort,"));
        assert!(at("Compare"));
        assert!(!at("f()"));
        assert!(!is_own_context(
            "mozilla::dom::Foo::Bar",
            "mozilla::dom::Foo"
        ));
        assert!(!is_own_context("mozilla::dom::Foo::Bar", "mozilla::dom"));
        assert!(!is_own_context("Bar", "%"));
    }
}
