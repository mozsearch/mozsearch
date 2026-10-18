//! Citations of web specs in a tree's other files, ex: Gecko's comments
//! citing the HTML Standard's algorithms, and the steps they implement:
//!
//! ```cpp
//! // https://html.spec.whatwg.org/#navigate
//! nsresult nsDocShell::InternalLoad(...) {
//!   // Step 12.6. If ...
//! ```
//!
//! - Each URL with an anchor to one of the specs of the tree's `specs.toml`
//!   (or of its linked spec tree's) is a `doc-use` of the symbol it names
//!   (see `SpecsConfig::url_symbol`), wherever it is in the file (comments,
//!   or strings, ex: error messages).
//! - Each step comment ("Step 12.6.", "Steps 1-3:") in the scope of a line
//!   citing a spec is a `doc-use` of the same symbol, whose context has the
//!   step (ex: "nsDocShell::InternalLoad step 12.6").  A citing line's scope
//!   is the definition it precedes (one with a nesting range, in the file's
//!   other analysis, named within a few lines of its next line of code, ex:
//!   after a return type), or else the rest of the innermost definition it's
//!   in, or else, without nesting ranges, the rest of its indentation's block
//!   (see `indent_scope_end`).  A
//!   step is in the scope of the last citing line before it whose scope it's
//!   in, so later citations take over, and nested ones end with their scopes.
//! - A citing line saying which of its algorithm's steps it's of ("Step 5 and
//!   6 of <URL>", or "Step 5 and 6 of" before a line with only the URL) cites
//!   them (its context has them), and if it's only that, the steps in its
//!   scope are theirs (ex: "Step 1." is "5.1", unless it's one of them, ex:
//!   "Step 6.2."), and each such line's before the same definition (ex: a
//!   function implementing steps of several algorithms, as Gecko's streams'
//!   `PromisifyAlgorithm` does).
//! - A comment citing a section of an ecmarkup spec by its number and title,
//!   as SpiderMonkey's do (ex: "22.1.3.13 Array.prototype.join ( separator
//!   )", after a line naming the edition, ex: "ES2025 draft rev ..."), is a
//!   citation of the clause with that name (its abstract operation's, or else
//!   its title, see `Titles`), whose steps are cited the same way.  (Not by
//!   the number, which changes between editions.)
//! - A URL to an anchor its spec doesn't define, which another spec of its
//!   series does, or else one of its successors (see
//!   `SpecsConfig::fallbacks`), is a citation of that spec's (ex:
//!   css-sizing-4's `#valdef-width-stretch`, which css-sizing defines, or
//!   sanitizer-api's, whose content is the HTML Standard's now), if the
//!   specs' definitions are known (see `Defined`).
//! - Records' contexts are the definitions their scopes are of.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::Path;

use lazy_static::lazy_static;
use regex::Regex;

use super::{Lines, SpecsConfig, spec_symbol};
use crate::file_format::analysis::{
    AnalysisKind, WithLocation, read_analysis, read_source, read_target,
};
use crate::file_format::bisectable_mmap::BisectableMmap;
use crate::file_format::crossref::CrossrefData;
use crate::semantic_analyzer::{Record, sort_records, source_record, target_record};

lazy_static! {
    /// A URL, with `'`s before letters, as anchors have (ex: WHATWG specs'
    /// "#get-an-element's-target"), but not a string's closing `'`.
    static ref URL: Regex = Regex::new(r#"https?://(?:[^\s"'<>`\\{}|^]|'[A-Za-z])+"#).unwrap();
    /// A comment's citation of a section by its number and title, ex:
    /// "22.1.3.13 Array.prototype.join ( separator )", "ES2017 7.1.15
    /// ToLength.", "21.4.1.1 Time Values and Time Range".
    static ref SECTION: Regex = Regex::new(
        r"^[ \t]*(?:/{2,}|/\*+|\*+|#+)[!<]?[ \t]*(?:ES[ \t]?\d+(?:[ \t]+Intl)?[ \t]+)?(?P<citation>\d+(?:\.\d+)+\.?[ \t]+(?P<title>[^\n]*[^\s*/]))"
    )
    .unwrap();
    /// A comment's step, at its start (after the comment's markers), ex:
    /// "Step 12.6.", "Steps 3.a-c", "Step 4 and 5:".
    static ref STEP: Regex = Regex::new(
        r"(?i)^[ \t]*(?:/{2,}|/\*+|\*+|#+|--|<!--)?[!<]?[ \t]*\(?(steps?[ \t]+(?P<path>\d+(?:\.(?:\d+|[a-z]{1,4}))*(?:[ \t]*(?:-|–|to|through|and|,)[ \t]*(?:\d+|[a-z]{1,4})(?:\.(?:\d+|[a-z]{1,4}))*)*))"
    )
    .unwrap();
    /// A line's comment naming the steps of the algorithm its URL (after it,
    /// or on the next line) is of, ex: "// Step 5 and 6 of " (before
    /// "https://streams.spec.whatwg.org/#transformstream-set-up").
    static ref STEPS_OF: Regex = Regex::new(
        r"(?i)^[ \t]*(?:/{2,}|/\*+|\*+|#+|--)?[ \t]*\(?steps?[ \t]+(?P<path>\d+(?:\.(?:\d+|[a-z]{1,4}))*(?:[ \t]*(?:-|–|to|through|and|,)[ \t]*(?:\d+|[a-z]{1,4})(?:\.(?:\d+|[a-z]{1,4}))*)*)[ \t]+of[ \t]*$"
    )
    .unwrap();
    /// A line's text before or after a URL that's only a comment's markers.
    static ref COMMENT_MARKERS: Regex = Regex::new(r"^[ \t]*(?:/{2,}|/\*+|\*+|#+|--)?[ \t]*(?:\*/)?[ \t]*$").unwrap();
    /// A comment line that's only a section's title, with its parameters (as
    /// the specs space them, unlike calls in code, ex: "Date.UTC(2021, 1)")
    /// or as an accessor, ex: "Temporal.Instant ( epochNanoseconds )", "get
    /// Temporal.ZonedDateTime.prototype.hoursInDay", as Temporal's are.
    static ref TITLE: Regex = Regex::new(
        r"^[ \t]*(?:/{2,}|/\*+|\*+|#+)[!<]?[ \t]*(?P<citation>(?P<title>(?:get |set )?[%A-Za-z_][\w.%\[\] @]*[ \t]+\((?:[ \t][^()\n]*)?[ \t]\)|(?:get|set) [%A-Za-z_][\w.%\[\] @]*[\w\]%]))[ \t]*\.?[ \t]*(?:\*/)?[ \t]*$"
    )
    .unwrap();
}

/// A definition in a file's other analysis with a nesting range: the line of
/// its name, its nesting range's lines (1-based, `end` exclusive), and its
/// pretty name and symbol, which records in its scope have as their context.
#[derive(Clone, Debug, PartialEq)]
pub struct Def {
    pub line: u32,
    pub lines: Range<u32>,
    pub pretty: String,
    pub sym: String,
}

/// The definitions with nesting ranges in an analysis file, if it exists.
pub fn analysis_defs(analysis_path: &Path) -> Vec<Def> {
    let Some(path) = analysis_path.to_str() else {
        return vec![];
    };
    if !analysis_path.exists() {
        return vec![];
    }
    // (The source and target records, in one read, as `read_source` and
    // `read_target` would each read them: a file's analysis took most of
    // this, for each of firefox's ~33,000 files with URLs.)
    let records = read_analysis(path, &mut |value, loc, i| {
        let (source, target) = (value.get("source").is_some(), value.get("target").is_some());
        match (source, target) {
            (true, true) => Some((
                read_source(value.clone(), loc, i),
                read_target(value, loc, i),
            )),
            (true, false) => Some((read_source(value, loc, i), None)),
            (false, true) => Some((None, read_target(value, loc, i))),
            (false, false) => None,
        }
    });
    let mut sources = vec![];
    let mut targets = vec![];
    for group in records {
        let (s, t): (Vec<_>, Vec<_>) = group.data.into_iter().unzip();
        let s: Vec<_> = s.into_iter().flatten().collect();
        let t: Vec<_> = t.into_iter().flatten().collect();
        if !s.is_empty() {
            sources.push(WithLocation {
                data: s,
                loc: group.loc,
            });
        }
        if !t.is_empty() {
            targets.push(WithLocation {
                data: t,
                loc: group.loc,
            });
        }
    }
    let mut defs = vec![];
    for source in sources {
        for data in &source.data {
            let range = &data.nesting_range;
            if range.is_empty() || !data.syntax.iter().any(|s| s.as_str() == "def") {
                continue;
            }
            // (The target record of the definition, for its pretty name: the
            // targets are grouped by their locations, in order.)
            let Some(target) = targets
                .binary_search_by(|t| t.loc.cmp(&source.loc))
                .ok()
                .into_iter()
                .flat_map(|i| &targets[i].data)
                .find(|t| t.kind == AnalysisKind::Def && data.sym.contains(&t.sym))
            else {
                continue;
            };
            defs.push(Def {
                line: source.loc.lineno,
                lines: range.start_lineno.min(source.loc.lineno)..range.end_lineno + 1,
                pretty: target.pretty.to_string(),
                sym: target.sym.to_string(),
            });
        }
    }
    defs
}

/// The clauses of the ecmarkup specs that code cites by their sections'
/// titles (see the module's docs), by their names, in the order of the specs
/// to prefer them from (ex: ECMA-262's, then ECMA-402's, then proposals').
#[derive(Default)]
pub struct Titles {
    /// name -> (spec's rank, its shortname, the clause's symbol and pretty name)
    by_name: HashMap<String, Vec<(usize, String, String, String)>>,
    specs: usize,
    /// The shortname of the spec that comments mentioning Intl are of.
    intl: Option<String>,
}

impl Titles {
    /// Add a spec's clauses' names (from its biblio, see
    /// `ecmarkup::Biblio::clause_names`), after the specs added before it.
    pub fn add_spec<'a>(
        &mut self,
        shortname: &str,
        names: impl Iterator<Item = (&'a str, &'a str, &'a str)>,
    ) {
        let rank = self.specs;
        self.specs += 1;
        for (name, id, pretty) in names {
            self.by_name.entry(name.to_string()).or_default().push((
                rank,
                shortname.to_string(),
                spec_symbol(shortname, id),
                pretty.to_string(),
            ));
        }
    }

    /// Prefer this spec's clauses for comments mentioning Intl (ECMA-402).
    pub fn set_intl(&mut self, shortname: &str) {
        self.intl = Some(shortname.to_string());
    }

    /// The symbol and pretty name of the clause `name` names, of the spec the
    /// citation is most likely of (`intl`, if it mentions Intl), if it's one
    /// clause of that spec.
    fn resolve(&self, name: &str, intl: bool) -> Option<(&str, &str)> {
        let candidates = self.by_name.get(name)?;
        let preferred = self
            .intl
            .as_deref()
            .filter(|_| intl)
            .filter(|spec| candidates.iter().any(|c| c.1 == *spec));
        let rank = match preferred {
            Some(spec) => candidates.iter().find(|c| c.1 == spec)?.0,
            None => candidates.iter().map(|c| c.0).min()?,
        };
        let mut in_spec = candidates.iter().filter(|c| c.0 == rank);
        let clause = in_spec.next()?;
        // (Ex: "Evaluation", which many clauses are.)
        if in_spec.next().is_some() {
            return None;
        }
        Some((clause.2.as_str(), clause.3.as_str()))
    }
}

/// The name a section's title cites: its abstract operation's or method's
/// (ex: "Array.prototype.join" of "Array.prototype.join ( separator )", or
/// "IsIntegralNumber" of "IsIntegralNumber, step 1"), or the title itself
/// (ex: "Time Values and Time Range"), with older editions' well-known
/// symbols as the current ones (ex: "RegExp.prototype [ %Symbol.replace% ]"
/// for "RegExp.prototype [ @@replace ]").
fn title_name(title: &str) -> Option<String> {
    lazy_static! {
        static ref WELL_KNOWN: Regex = Regex::new(r"@@([A-Za-z]+)").unwrap();
    }
    let title = title.trim();
    let title = title
        .strip_prefix("Static Semantics:")
        .or_else(|| title.strip_prefix("Runtime Semantics:"))
        .unwrap_or(title)
        .trim_start();
    let name = title.split('(').next()?;
    let name = name.split(',').next()?;
    let name = name.trim_end().trim_end_matches(['.', ':']);
    // (Not ex: "Step 4." after a section's number.)
    if name.is_empty() || name.to_lowercase().starts_with("step") {
        return None;
    }
    Some(WELL_KNOWN.replace_all(name, "%Symbol.$1%").into_owned())
}

/// The analysis's counts, for reporting.
#[derive(Debug, Default)]
pub struct Stats {
    /// URLs to the specs with anchors.
    pub citations: usize,
    /// Step comments in their scopes.
    pub steps: usize,
    /// Of the citations, those of sections by their titles.
    pub sections: usize,
    /// Of the citations, those of anchors their specs don't define, of
    /// another spec's (see `SpecsConfig::fallbacks`).
    pub fallbacks: usize,
    /// Of the citations, those by outdated URLs (see `SpecEntry::old_urls`).
    pub outdated: usize,
}

/// The symbols the specs define, for citations of anchors their specs don't
/// define: those of their analysis (in their own tree), or of their tree's
/// crossref (in a tree citing its linked tree's specs).  Also which have
/// structured records (metas), unlike ex: links' ids (see `has_meta`).
pub enum Defined {
    /// The defined symbols, and those with structured records.
    Symbols(HashSet<String>, HashSet<String>),
    /// The crossref, and whether the symbols looked up so far are defined and
    /// have metas.
    Crossref(
        BisectableMmap<CrossrefData>,
        std::sync::Mutex<HashMap<String, (bool, bool)>>,
    ),
}

impl Defined {
    /// The crossref of the tree with the index at `index_path`, if it has
    /// one.
    pub fn crossref(index_path: &Path) -> Option<Defined> {
        let (inline, extra) = (
            index_path.join("crossref"),
            index_path.join("crossref-extra"),
        );
        if !inline.exists() || !extra.exists() {
            return None;
        }
        let crossref = BisectableMmap::new(inline.to_str()?, extra.to_str()?)?;
        Some(Defined::Crossref(crossref, Default::default()))
    }

    /// Add a spec's analysis's definitions.
    pub fn add(&mut self, records: &[Record]) {
        let Defined::Symbols(symbols, structured) = self else {
            return;
        };
        for record in records {
            match record {
                Record::Target(target) if target.data.kind == AnalysisKind::Def => {
                    symbols.insert(target.data.sym.to_string());
                }
                Record::Structured(_, value) => {
                    if let Some(sym) = value["sym"].as_str() {
                        structured.insert(sym.to_string());
                    }
                }
                _ => {}
            }
        }
    }

    /// Whether the symbol is defined, and whether it has a meta.
    fn lookup(&self, sym: &str) -> (bool, bool) {
        match self {
            Defined::Symbols(symbols, structured) => {
                (symbols.contains(sym), structured.contains(sym))
            }
            Defined::Crossref(crossref, known) => {
                if let Some(found) = known.lock().unwrap().get(sym) {
                    return *found;
                }
                // (Looked up without the lock, as files' citations are
                // found on several threads.)
                let data = crossref.lookup(sym).ok().flatten();
                let has_meta = data.as_ref().is_some_and(|data| data.meta.is_some());
                let defined = data
                    .and_then(|data| data.definitions)
                    .is_some_and(|definitions| !definitions.is_empty());
                known
                    .lock()
                    .unwrap()
                    .insert(sym.to_string(), (defined, has_meta));
                (defined, has_meta)
            }
        }
    }

    pub fn contains(&self, sym: &str) -> bool {
        self.lookup(sym).0
    }

    /// Whether the symbol is defined with a structured record (a meta), as
    /// dfns, headings, and IDL definitions are, but not ex: Bikeshed's links'
    /// ids, which are anchors too.
    pub fn has_meta(&self, sym: &str) -> bool {
        let (defined, has_meta) = self.lookup(sym);
        defined && has_meta
    }
}

/// A line citing a spec: its first citation's symbol and pretty name, and its
/// scope's last line and definition, and the steps of the algorithm it says
/// it's of, if it does ("Step 5 and 6 of <URL>", or on the line before it, see
/// `STEPS_OF`): their path, the line saying so, and whether the line is only
/// that (then the steps in its scope are theirs, see `relative_step`).
struct Citing {
    line: u32,
    sym: String,
    pretty: String,
    scope_end: u32,
    def: Option<Def>,
    steps: Option<(String, u32, bool)>,
}

/// Whether a step's path is of one of the steps a line citing an algorithm
/// says it's of (see `Citing::steps`, ex: "6.2" of "5 and 6", "3.a" of "2-4").
fn step_of(steps: &str, path: &str) -> bool {
    let numbers: Vec<u32> = steps
        .split(|c: char| !c.is_ascii_digit() && c != '.')
        .filter_map(|part| part.split('.').next()?.parse().ok())
        .collect();
    let ranged =
        steps.contains(['-', '–']) || steps.contains(" to ") || steps.contains(" through ");
    let first = path
        .split(['.', '-', ' ', ','])
        .next()
        .and_then(|n| n.parse::<u32>().ok());
    first.is_some_and(|n| match (ranged, numbers.as_slice()) {
        (true, [from, to, ..]) => (*from..=*to).contains(&n),
        _ => numbers.contains(&n),
    })
}

/// A step's path in the scope of a line citing an algorithm's `steps`, as one
/// of the first of them's (ex: "5.1", for "5 and 6"'s "1").
fn relative_step(steps: &str, path: &str) -> String {
    let step = steps
        .split(|c: char| c.is_whitespace() || matches!(c, '-' | '–' | ','))
        .next()
        .unwrap_or(steps);
    format!("{}.{}", step, path)
}

/// Whether a line is (part of) a comment, by its start, in the usual
/// languages' syntaxes.
fn is_comment(line: &str) -> bool {
    let line = line.trim_start();
    ["//", "/*", "*", "#", "<!--", "--", "\"\"\"", "'''"]
        .iter()
        .any(|marker| line.starts_with(marker))
        || line.trim_end().ends_with("*/")
}

/// Statements' keywords, whose blocks a comment before them is in rather than
/// about (ex: an algorithm's steps in a function's body, around an `if`).
const CONTROL: &[&str] = &[
    "if", "else", "elif", "for", "foreach", "while", "do", "switch", "match", "loop", "try",
    "catch", "with", "case", "unless", "until",
];

/// A line's indentation, in columns (tabs to 8).
fn indent(line: &str) -> usize {
    line.chars()
        .take_while(|c| c.is_whitespace())
        .map(|c| if c == '\t' { 8 } else { 1 })
        .sum()
}

/// A URL's end, without trailing punctuation that's the text's (ex: a
/// sentence's period, a parenthetical's or a block comment's end, or a
/// possessive's `'s`, unlike an anchor's `'`s, which are within it).
fn trim_url(url: &str) -> &str {
    let mut url = url;
    loop {
        let trimmed = url
            .trim_end_matches(['.', ',', ';', ':', '!', '?', '*', '\'', '"'])
            .trim_end_matches("*/");
        let trimmed = match trimmed.rfind('\'') {
            Some(i) if trimmed[i + 1..].bytes().all(|b| b.is_ascii_alphabetic()) => &trimmed[..i],
            _ => trimmed,
        };
        let trimmed = match trimmed.strip_suffix([')', ']']) {
            // (Unless it's the URL's own, ex: an anchor's `(` ... `)`.)
            Some(inner)
                if inner.matches(['(', '[']).count() <= inner.matches([')', ']']).count() =>
            {
                inner
            }
            _ => trimmed,
        };
        if trimmed.len() == url.len() {
            return url;
        }
        url = trimmed;
    }
}

/// The last line (1-based) of the scope of the citing line at `line` (by
/// index) without nesting ranges: if the code it precedes opens a block within
/// a few lines at its indentation (ex: a function's body, after a return type
/// and a signature, but not a statement's, like an `if`'s), that block,
/// through its closing line; or else (ex: statements in a function's body) the
/// rest of the block it's in, until a line indented less (or at the top
/// level, a blank line).
fn indent_scope_end(lines: &[&str], line: usize) -> u32 {
    let level = indent(lines[line]);
    let blank = |l: &str| l.trim().is_empty();
    let Some(code) = (line + 1..lines.len()).find(|&l| !blank(lines[l]) && !is_comment(lines[l]))
    else {
        return lines.len() as u32;
    };
    let keyword: String = lines[code]
        .trim_start()
        .trim_start_matches('}')
        .trim_start()
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    let next_code = |from: usize| (from..lines.len()).find(|&l| !blank(lines[l]));
    // (The line opening the block, if the code does.)
    let mut opening = None;
    if indent(lines[code]) <= level && !CONTROL.contains(&keyword.as_str()) {
        let mut at = Some(code);
        for _ in 0..4 {
            let Some(l) = at.filter(|&l| indent(lines[l]) <= level) else {
                break;
            };
            let next = next_code(l + 1);
            if lines[l].trim_end().ends_with(['{', ':'])
                || next.is_some_and(|n| indent(lines[n]) > level)
            {
                opening = Some(l);
                break;
            }
            at = next;
        }
    }
    if let Some(opening) = opening {
        for l in opening + 1..lines.len() {
            if blank(lines[l]) || indent(lines[l]) > level {
                continue;
            }
            let closing = lines[l].trim_start().starts_with(['}', ')', ']']);
            return if closing { l as u32 + 1 } else { l as u32 };
        }
        return lines.len() as u32;
    }
    (code..lines.len())
        .find(|&l| {
            if level == 0 {
                blank(lines[l])
            } else {
                !blank(lines[l]) && indent(lines[l]) < level
            }
        })
        .map_or(lines.len() as u32, |l| l as u32)
}

/// The records of the citations of `config`'s specs in a file's source, whose
/// other analysis has `defs`, given the symbols the specs define, if they're
/// known.
pub fn citation_records(
    src: &str,
    defs: &[Def],
    config: &SpecsConfig,
    titles: Option<&Titles>,
    defined: Option<&Defined>,
) -> (Vec<Record>, Stats) {
    let mut stats = Stats::default();
    let mut records = vec![];
    let lines_index = Lines::new(src);
    let lines: Vec<&str> = src.split('\n').collect();

    // The scope of the line citing a spec at `line` (1-based): the definition
    // it precedes, or the rest of the one it's in, or its indentation's.
    let scope = |line: u32| -> (u32, Option<Def>) {
        let index = line as usize - 1;
        let code = (index + 1..lines.len())
            .find(|&l| !lines[l].trim().is_empty() && !is_comment(lines[l]))
            .map(|l| l as u32 + 1);
        let innermost = |at: u32| {
            defs.iter()
                .filter(|d| d.lines.contains(&at))
                .min_by_key(|d| d.lines.end - d.lines.start)
        };
        // (Not past the end of the definition it's in, ex: a `}`.)
        let inside = innermost(line);
        let preceded = code
            .filter(|&code| {
                !lines[code as usize - 1]
                    .trim_start()
                    .starts_with(['}', ')', ']'])
            })
            .and_then(|code| {
                defs.iter()
                    .filter(|d| code <= d.line && d.line < code + 4)
                    .filter(|d| {
                        inside.is_none_or(|i| i.lines.start <= d.line && d.lines.end <= i.lines.end)
                    })
                    .min_by_key(|d| (d.line, std::cmp::Reverse(d.lines.end - d.lines.start)))
            });
        match preceded.or(inside) {
            Some(def) => (def.lines.end - 1, Some(def.clone())),
            None => (indent_scope_end(&lines, index), None),
        }
    };

    // The citations (and whether they're by outdated URLs), and the lines
    // citing specs.
    let mut citations: Vec<(Range<usize>, String, String, bool)> = vec![];
    let mut citing: Vec<Citing> = vec![];
    for m in URL.find_iter(src) {
        let url = trim_url(m.as_str());
        if !url.contains('#') {
            continue;
        }
        let Some((spec, anchor, outdated)) = config.url_citation(url) else {
            continue;
        };
        // (Without a text fragment's directive, ex: `#x:~:text=y`.)
        let anchor = anchor.split(":~:").next().unwrap_or_default().to_string();
        if anchor.is_empty() {
            continue;
        }
        let shortname = match defined {
            Some(defined) if !defined.contains(&spec_symbol(spec.shortname(), &anchor)) => {
                let fallback = config
                    .fallbacks(spec)
                    .into_iter()
                    .find(|shortname| defined.contains(&spec_symbol(shortname, &anchor)));
                stats.fallbacks += usize::from(fallback.is_some());
                fallback.unwrap_or(spec.shortname())
            }
            _ => spec.shortname(),
        };
        let sym = spec_symbol(shortname, &anchor);
        let pretty = format!("{} #{}", shortname, anchor);
        let range = m.start()..m.start() + url.len();
        let line = lines_index.location(&range).lineno;
        citations.push((range, sym.clone(), pretty.clone(), outdated));
        if citing.last().is_some_and(|c| c.line == line) {
            continue;
        }
        let (scope_end, def) = scope(line);
        // (The steps it says it's of, before it on its line, or on the line
        // before it, if it's alone on its own.)
        let line_start = src[..m.start()].rfind('\n').map_or(0, |at| at + 1);
        let line_end = src[m.start()..]
            .find('\n')
            .map_or(src.len(), |at| m.start() + at);
        let url_end = m.start() + url.len();
        let (before, after) = (
            &src[line_start..m.start()],
            &src[url_end..line_end.max(url_end)],
        );
        let steps = match STEPS_OF.captures(before) {
            Some(steps) => Some((steps["path"].to_string(), line)),
            None if line > 1 && COMMENT_MARKERS.is_match(before) => STEPS_OF
                .captures(lines[line as usize - 2])
                .map(|steps| (steps["path"].to_string(), line - 1)),
            None => None,
        }
        .map(|(path, at)| (path, at, COMMENT_MARKERS.is_match(after)));
        citing.push(Citing {
            line,
            sym,
            pretty,
            scope_end,
            def,
            steps,
        });
    }
    // Citations of sections by their titles, on lines without URLs.
    if let Some(titles) = titles {
        let mut offset = 0;
        for (index, text) in lines.iter().enumerate() {
            let start = offset;
            offset += text.len() + 1;
            let line = index as u32 + 1;
            if citing.iter().any(|c| c.line == line) {
                continue;
            }
            // (A title without a number is only a citation if it's an
            // abstract operation's, method's, or property's name, ex:
            // "ToString", "Temporal.Instant", not ex: "type ( ... )".)
            let section = SECTION.captures(text).or_else(|| {
                TITLE.captures(text).filter(|title| {
                    let title = title.name("title").map_or("", |t| t.as_str());
                    let name = title.trim_start_matches("get ").trim_start_matches("set ");
                    name.contains(['.', '%']) || name.starts_with(|c: char| c.is_ascii_uppercase())
                })
            });
            let Some(section) = section else {
                continue;
            };
            let (Some(citation), Some(title)) = (section.name("citation"), section.name("title"))
            else {
                continue;
            };
            let Some(name) = title_name(title.as_str()) else {
                continue;
            };
            // (ECMA-402's, if the comment mentions Intl, ex: "ES2025 Intl
            // draft rev ...", or the title, ex: "Intl.Locale ( tag )".)
            let nearby = &lines[index.saturating_sub(3)..(index + 4).min(lines.len())];
            let intl = nearby
                .iter()
                .any(|l| l.contains("Intl") || l.contains("402"));
            let Some((sym, pretty)) = titles.resolve(&name, intl) else {
                continue;
            };
            stats.sections += 1;
            let range = start + citation.start()..start + citation.end();
            citations.push((range, sym.to_string(), pretty.to_string(), false));
            let (scope_end, def) = scope(line);
            citing.push(Citing {
                line,
                sym: sym.to_string(),
                pretty: pretty.to_string(),
                scope_end,
                def,
                steps: None,
            });
        }
    }
    let context_of = |line: u32| -> Option<&Citing> {
        citing
            .iter()
            .filter(|c| c.line <= line && line <= c.scope_end)
            .max_by_key(|c| c.line)
    };
    for (range, sym, pretty, outdated) in citations {
        stats.citations += 1;
        stats.outdated += usize::from(outdated);
        let loc = lines_index.location(&range);
        let def = context_of(loc.lineno).and_then(|c| c.def.as_ref());
        // (Of the steps its line says it's of, if it does.)
        let steps = citing
            .iter()
            .find(|c| c.line == loc.lineno && c.sym == sym)
            .and_then(|c| c.steps.as_ref())
            .map(|(path, _, _)| path.clone());
        // (A citation by an outdated URL says so, in its pretty names, and in
        // its source record's syntax, which its code's view marks.)
        let pretty = if outdated {
            format!("{} (by an outdated URL)", pretty)
        } else {
            pretty
        };
        let mut source = source_record(
            loc,
            "use",
            format!("spec {}", pretty),
            sym.clone(),
            Default::default(),
        );
        if outdated && let Record::Source(source) = &mut source {
            source.data.syntax.push("outdated".into());
        }
        records.push(source);
        let context = match (def, steps) {
            (Some(def), Some(steps)) => format!("{} step {}", def.pretty, steps),
            (None, Some(steps)) => format!("step {}", steps),
            (Some(def), None) => def.pretty.clone(),
            (None, None) => String::new(),
        };
        records.push(target_record(
            loc,
            AnalysisKind::DocUse,
            pretty,
            sym,
            context,
            def.map_or(String::new(), |d| d.sym.clone()),
        ));
    }

    // Steps, in their scopes.
    let mut offset = 0;
    for (index, line) in lines.iter().enumerate() {
        let start = offset;
        offset += line.len() + 1;
        let lineno = index as u32 + 1;
        let Some(c) = context_of(lineno).filter(|c| c.line < lineno) else {
            continue;
        };
        // (Not a line saying which steps a URL after it is of.)
        if citing.iter().any(|c| {
            c.steps
                .as_ref()
                .is_some_and(|(_, at, _)| *at == lineno && c.line > lineno)
        }) {
            continue;
        }
        let Some(step) = STEP.captures(line) else {
            continue;
        };
        let (Some(whole), Some(path)) = (step.get(1), step.name("path")) else {
            continue;
        };
        stats.steps += 1;
        let path = path.as_str().trim_end_matches(['.', ':']);
        let loc = lines_index.location(&(start + whole.start()..start + whole.end()));
        // A step in the scope of lines citing algorithms' steps (only that,
        // ex: "// Step 5 and 6 of <URL>", "// Step 2 and 3 of <URL>", before
        // a function implementing them all) is each's (ex: "5.1" and "2.1"),
        // and else the last citing line's.
        let all_of = |g: &&Citing| {
            g.scope_end == c.scope_end
                && g.def == c.def
                && g.steps.as_ref().is_some_and(|(_, _, only)| *only)
                && (g.line.min(c.line)..g.line.max(c.line))
                    .all(|l| is_comment(lines[l as usize - 1]))
        };
        let group: Vec<&Citing> = match c.steps.as_ref().is_some_and(|(_, _, only)| *only) {
            true => citing.iter().filter(all_of).collect(),
            false => vec![c],
        };
        // (A step of the steps some of them say they're of is theirs, as it
        // is, ex: "Step 6.2" of "Step 5 and 6 of <URL>"'s; else, of each's.)
        let of_steps = |g: &&Citing| {
            g.steps
                .as_ref()
                .is_some_and(|(steps, _, only)| *only && step_of(steps, path))
        };
        let owners: Vec<&Citing> = group.iter().copied().filter(of_steps).collect();
        let (group, relative) = match owners.is_empty() {
            true => (group, true),
            false => (owners, false),
        };
        for g in group {
            let path = match &g.steps {
                Some((steps, _, true)) if relative => relative_step(steps, path),
                _ => path.to_string(),
            };
            let context = match &g.def {
                Some(def) => format!("{} step {}", def.pretty, path),
                None => format!("step {}", path),
            };
            records.push(source_record(
                loc.clone(),
                "use",
                format!("spec {}", g.pretty),
                g.sym.clone(),
                Default::default(),
            ));
            records.push(target_record(
                loc.clone(),
                AnalysisKind::DocUse,
                g.pretty.clone(),
                g.sym.clone(),
                context,
                g.def.as_ref().map_or(String::new(), |d| d.sym.clone()),
            ));
        }
    }
    sort_records(&mut records);
    (records, stats)
}

/// Mark a file's citations (and their steps) of stale anchors, ones their
/// specs no longer define but once did (by their symbols, see `stale`, in the
/// specs' `tree`): their source records' syntax has "stale", which their code's
/// view marks, their pretty names say so, and each such symbol gets a
/// structured record with its `stale` info (the tree, the revision which
/// removed it, and the definitions it may be now), which crossref makes its
/// meta, for the context menu.  Returns how many citations and steps it
/// marked.
pub fn mark_stale(
    records: &mut Vec<Record>,
    stale: &HashMap<String, super::stale::Stale>,
    tree: &str,
) -> usize {
    const NO_LONGER: &str = " (no longer in the spec)";
    let mut marked = 0;
    // (Each symbol's first citation's location, and pretty name.)
    let mut cited: Vec<(String, crate::file_format::analysis::Location, String)> = vec![];
    for record in records.iter_mut() {
        match record {
            Record::Source(source) => {
                let Some(sym) = source
                    .data
                    .sym
                    .first()
                    .filter(|sym| stale.contains_key(*sym))
                else {
                    continue;
                };
                if !cited.iter().any(|(s, _, _)| s == sym) {
                    let pretty = source.data.pretty.trim_start_matches("spec ");
                    let pretty = pretty.trim_end_matches(" (by an outdated URL)").to_string();
                    cited.push((sym.clone(), source.loc, pretty));
                }
                source.data.syntax.push("stale".into());
                source.data.pretty.push_str(NO_LONGER);
                marked += 1;
            }
            Record::Target(target) if stale.contains_key(&target.data.sym) => {
                target.data.pretty.push_str(NO_LONGER);
            }
            _ => {}
        }
    }
    for (sym, loc, pretty) in cited {
        let info = &stale[&sym];
        let value = serde_json::json!({
            "loc": loc,
            "structured": 1,
            "pretty": pretty,
            "sym": sym,
            "implKind": "spec",
            "stale": {
                "tree": tree,
                "removed": info.removed,
                "suggestions": info.suggestions,
            },
        });
        records.push(Record::Structured(loc, value));
    }
    sort_records(records);
    marked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(records: &[Record]) -> Vec<String> {
        records
            .iter()
            .filter_map(|record| match record {
                Record::Target(t) => Some(format!(
                    "{}:{}-{} {} {} | {} ({})",
                    t.loc.lineno,
                    t.loc.col_start,
                    t.loc.col_end,
                    t.data.kind.to_ustr(),
                    t.data.sym,
                    t.data.context,
                    t.data.contextsym
                )),
                _ => None,
            })
            .collect()
    }

    fn config() -> SpecsConfig {
        toml::from_str(
            "[[spec]]\nname = \"html\"\nrendered = \"https://html.spec.whatwg.org/\"\n\
             [[spec]]\nname = \"dom\"\nrendered = \"https://dom.spec.whatwg.org/\"\n",
        )
        .unwrap()
    }

    #[test]
    fn test_trim_url() {
        assert_eq!(trim_url("https://a.org/#b."), "https://a.org/#b");
        assert_eq!(trim_url("https://a.org/#b)."), "https://a.org/#b");
        assert_eq!(trim_url("https://a.org/#f(x)"), "https://a.org/#f(x)");
        assert_eq!(trim_url("https://a.org/#b*/"), "https://a.org/#b");
        assert_eq!(trim_url("https://a.org/#b's"), "https://a.org/#b");
        assert_eq!(trim_url("https://a.org/#b)'s"), "https://a.org/#b");
        assert_eq!(trim_url("https://a.org/#b's-c"), "https://a.org/#b's-c");
        let src = "// https://dom.spec.whatwg.org/#concept-node:~:text=node%20tree\n";
        let (records, _) = citation_records(src, &[], &config(), None, None);
        assert_eq!(
            summary(&records),
            vec!["1:3-63 doc-use SPEC_dom_concept@2Dnode |  ()"]
        );
        // (An anchor's `'`s, but not a string's.)
        let src = "// <https://dom.spec.whatwg.org/#concept-node's-x>\nx('https://dom.spec.whatwg.org/#y');\n";
        let (records, _) = citation_records(src, &[], &config(), None, None);
        assert_eq!(
            summary(&records),
            vec![
                "1:4-49 doc-use SPEC_dom_concept@2Dnode@27s@2Dx |  ()",
                "2:3-33 doc-use SPEC_dom_y |  ()",
            ]
        );
    }

    #[test]
    fn test_outdated_urls() {
        let config: SpecsConfig = toml::from_str(
            r#"
            [[spec]]
            name = "html"
            rendered = "https://html.spec.whatwg.org/"
            old_urls = ["https://dev.w3.org/html5/spec/"]
            "#,
        )
        .unwrap();
        let src = "\
// http://dev.w3.org/html5/spec/Overview.html#navigate
// https://html.spec.whatwg.org/#navigate
";
        let (records, stats) = citation_records(src, &[], &config, None, None);
        // (Both are the HTML Standard's, but the old URL's says so.)
        let sources: Vec<String> = records
            .iter()
            .filter_map(|record| match record {
                Record::Source(s) => Some(format!(
                    "{} {} {}",
                    s.loc.lineno,
                    s.data.syntax.join(","),
                    s.data.pretty
                )),
                _ => None,
            })
            .collect();
        assert_eq!(
            sources,
            vec![
                "1 use,outdated spec html #navigate (by an outdated URL)",
                "2 use spec html #navigate",
            ]
        );
        assert_eq!((stats.citations, stats.outdated), (2, 1));
    }

    #[test]
    fn test_mark_stale() {
        let src = "\
// https://html.spec.whatwg.org/#gone
void Gone() {
  // Step 1. Do it.
}
// https://html.spec.whatwg.org/#navigate
";
        let (mut records, _) = citation_records(src, &[], &config(), None, None);
        let stale = HashMap::from([(
            "SPEC_html_gone".to_string(),
            super::super::stale::Stale {
                removed: "abc".to_string(),
                suggestions: vec![super::super::stale::Suggestion {
                    sym: "SPEC_html_went".to_string(),
                    pretty: "html went".to_string(),
                    jump: "html/source#3".to_string(),
                }],
            },
        )]);
        // (The citation and its step, not the other citation.)
        assert_eq!(mark_stale(&mut records, &stale, "wubspecs"), 2);
        let lines: Vec<String> = records
            .iter()
            .map(|record| match record {
                Record::Source(s) => format!(
                    "{} source {} {}",
                    s.loc.lineno,
                    s.data.syntax.join(","),
                    s.data.pretty
                ),
                Record::Target(t) => format!("{} target {}", t.loc.lineno, t.data.pretty),
                Record::Structured(loc, value) => format!(
                    "{} structured {} {} {}",
                    loc.lineno, value["sym"], value["pretty"], value["stale"]
                ),
            })
            .collect();
        assert_eq!(
            lines,
            vec![
                "1 source use,stale spec html #gone (no longer in the spec)",
                "1 target html #gone (no longer in the spec)",
                "1 structured \"SPEC_html_gone\" \"html #gone\" {\"tree\":\"wubspecs\",\"removed\":\"abc\",\"suggestions\":[{\"sym\":\"SPEC_html_went\",\"pretty\":\"html went\",\"jump\":\"html/source#3\"}]}",
                "3 source use,stale spec html #gone (no longer in the spec)",
                "3 target html #gone (no longer in the spec)",
                "5 source use spec html #navigate",
                "5 target html #navigate",
            ]
        );
    }

    #[test]
    fn test_fallbacks() {
        let config: SpecsConfig = toml::from_str(
            r#"
            [[spec]]
            name = "css-sizing-4"
            series = "css-sizing"
            rendered = "https://drafts.csswg.org/css-sizing-4/"
            [[spec]]
            name = "css-sizing-3"
            shortname = "css-sizing"
            rendered = "https://drafts.csswg.org/css-sizing-3/"
            [[spec]]
            name = "sanitizer-api"
            successors = ["html"]
            rendered = "https://wicg.github.io/sanitizer-api/"
            [[spec]]
            name = "html"
            rendered = "https://html.spec.whatwg.org/"
            "#,
        )
        .unwrap();
        let defined = Defined::Symbols(
            [
                "SPEC_css@2Dsizing_stretch",
                "SPEC_css@2Dsizing@2D4_own",
                "SPEC_html_sanitize",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            HashSet::new(),
        );
        let src = "\
// https://drafts.csswg.org/css-sizing-4/#stretch
// https://drafts.csswg.org/css-sizing-4/#own
// https://wicg.github.io/sanitizer-api/#sanitize
// https://drafts.csswg.org/css-sizing-4/#missing
";
        let (records, stats) = citation_records(src, &[], &config, None, Some(&defined));
        assert_eq!(
            summary(&records),
            vec![
                // (Its series' current specification's.)
                "1:3-49 doc-use SPEC_css@2Dsizing_stretch |  ()",
                "2:3-45 doc-use SPEC_css@2Dsizing@2D4_own |  ()",
                // (Its successor's.)
                "3:3-49 doc-use SPEC_html_sanitize |  ()",
                // (No other spec defines it either.)
                "4:3-49 doc-use SPEC_css@2Dsizing@2D4_missing |  ()",
            ]
        );
        assert_eq!((stats.citations, stats.fallbacks), (4, 2));
    }

    #[test]
    fn test_with_nesting_ranges() {
        let src = "\
// https://html.spec.whatwg.org/multipage/browsing-the-web.html#navigate
void Navigate() {
  // Step 1. Let x be y.
  Foo();
  // https://dom.spec.whatwg.org/#concept-tree-order (step 2's)
  // Step 1.
  // Step 12.6: If z.
}

// Step 3.
void Other() {}

// https://dom.spec.whatwg.org/#concept-node
nsresult
nsNode::Do()
{
  // Step 2.
}
";
        let def = |line: u32, lines: Range<u32>, pretty: &str| Def {
            line,
            lines,
            pretty: pretty.to_string(),
            sym: format!("_{}", pretty),
        };
        let defs = vec![
            def(2, 2..9, "Navigate"),
            def(11, 11..12, "Other"),
            def(15, 15..19, "nsNode::Do"),
        ];
        let (records, stats) = citation_records(src, &defs, &config(), None, None);
        assert_eq!(
            summary(&records),
            vec![
                "1:3-72 doc-use SPEC_html_navigate | Navigate (_Navigate)",
                "3:5-11 doc-use SPEC_html_navigate | Navigate step 1 (_Navigate)",
                "5:5-52 doc-use SPEC_dom_concept@2Dtree@2Dorder | Navigate (_Navigate)",
                "6:5-11 doc-use SPEC_dom_concept@2Dtree@2Dorder | Navigate step 1 (_Navigate)",
                "7:5-14 doc-use SPEC_dom_concept@2Dtree@2Dorder | Navigate step 12.6 (_Navigate)",
                "13:3-44 doc-use SPEC_dom_concept@2Dnode | nsNode::Do (_nsNode::Do)",
                "17:5-11 doc-use SPEC_dom_concept@2Dnode | nsNode::Do step 2 (_nsNode::Do)",
            ]
        );
        assert_eq!((stats.citations, stats.steps), (3, 4));
    }

    #[test]
    fn test_example_spec_citation() {
        // (Its citation of the example spec's poke algorithm is also the
        // webtest's searchfox tree's implementation of it, in the tree's
        // diagrams with the linked tests tree's; see
        // tests/webtest/test_SpecCitations.js.)
        // https://example.spec.whatwg.org/#concept-widget-poke
        let config: SpecsConfig = toml::from_str(
            "[[spec]]\nname = \"example\"\nrendered = \"https://example.spec.whatwg.org/\"\n",
        )
        .unwrap();
        let src = "// https://example.spec.whatwg.org/multipage/#concept-widget-poke\n";
        let (records, _) = citation_records(src, &[], &config, None, None);
        assert_eq!(
            summary(&records),
            vec!["1:3-65 doc-use SPEC_example_concept@2Dwidget@2Dpoke |  ()"]
        );
    }

    #[test]
    fn test_sections() {
        let mut titles = Titles::default();
        let names = [
            (
                "Array.prototype.join",
                "sec-array.prototype.join",
                "es Array.prototype.join",
            ),
            (
                "RegExp.prototype [ %Symbol.replace% ]",
                "sec-regexp.prototype-%symbol.replace%",
                "es RegExp.prototype [ %Symbol.replace% ]",
            ),
            ("Evaluation", "sec-a-evaluation", "es Evaluation"),
            ("Evaluation", "sec-b-evaluation", "es Evaluation"),
            ("Intl.Locale", "sec-es-intl-locale", "es Intl.Locale"),
        ];
        titles.add_spec("es", names.into_iter());
        titles.add_spec(
            "intl",
            [("Intl.Locale", "sec-intl-locale", "intl Intl.Locale")].into_iter(),
        );
        titles.set_intl("intl");
        let src = "\
// ES2020 draft rev dc1e21c454bd316810be1c0e7af0131a2d7f38e9
// 22.1.3.13 Array.prototype.join ( separator )
function ArrayJoin(separator) {
    // Step 1.
}

// 22.2.5.11 RegExp.prototype [ @@replace ] ( string, replaceValue )
// 14.5.1 Runtime Semantics: Evaluation
// 9.4.2.3 Step 4.

/**
 * Intl.Locale ( tag [ , options ] )
 */
function Locale() {
    // Step 2.
}
";
        let (records, stats) = citation_records(src, &[], &config(), Some(&titles), None);
        assert_eq!(
            summary(&records),
            vec![
                "2:3-47 doc-use SPEC_es_sec@2Darray@2Eprototype@2Ejoin |  ()",
                "4:7-13 doc-use SPEC_es_sec@2Darray@2Eprototype@2Ejoin | step 1 ()",
                "7:3-68 doc-use SPEC_es_sec@2Dregexp@2Eprototype@2D@25symbol@2Ereplace@25 |  ()",
                "12:3-36 doc-use SPEC_intl_sec@2Dintl@2Dlocale |  ()",
                "15:7-13 doc-use SPEC_intl_sec@2Dintl@2Dlocale | step 2 ()",
            ]
        );
        assert_eq!((stats.sections, stats.steps), (3, 2));
    }

    #[test]
    fn test_signatures_without_nesting_ranges() {
        // (A return type and a signature on their own lines, before the
        // body's `{`, as in Gecko.)
        let src = "\
// https://html.spec.whatwg.org/#navigate
nsresult
Navigate(int aX)
{
  // Step 1.
}

// Step 2.
";
        let (records, _) = citation_records(src, &[], &config(), None, None);
        assert_eq!(
            summary(&records),
            vec![
                "1:3-41 doc-use SPEC_html_navigate |  ()",
                "5:5-11 doc-use SPEC_html_navigate | step 1 ()",
            ]
        );
    }

    #[test]
    fn test_steps_of() {
        // (StreamUtils.h's PromisifyAlgorithm's, as of 2026-10: lines saying
        // which steps of several algorithms a function implements, whose
        // steps are theirs.)
        let src = "\
// Promisification algorithm, shared among:
// Step 2 and 3 of https://html.spec.whatwg.org/#run-a
// Step 2 and 3 of
// https://html.spec.whatwg.org/#run-b
// Step 5 and 6 of https://dom.spec.whatwg.org/#run-c
int Promisify()
{
  // Step 1. Let result be the result of running it.
  int result = 0;
  // Step 6.2.
  return result;
}

// Steps 1-4 of https://html.spec.whatwg.org/#run-d are the caller's.
int Rest()
{
  // Step 5.
}
";
        let (records, _) = citation_records(src, &[], &config(), None, None);
        assert_eq!(
            summary(&records),
            vec![
                "2:19-54 doc-use SPEC_html_run@2Da | step 2 and 3 ()",
                "4:3-38 doc-use SPEC_html_run@2Db | step 2 and 3 ()",
                "5:19-53 doc-use SPEC_dom_run@2Dc | step 5 and 6 ()",
                // (Each's.)
                "8:5-11 doc-use SPEC_html_run@2Da | step 2.1 ()",
                "8:5-11 doc-use SPEC_html_run@2Db | step 2.1 ()",
                "8:5-11 doc-use SPEC_dom_run@2Dc | step 5.1 ()",
                // (One of one's steps.)
                "10:5-13 doc-use SPEC_dom_run@2Dc | step 6.2 ()",
                // (A line saying more than which steps it's of isn't only
                // that: its scope's steps are as they say.)
                "14:16-51 doc-use SPEC_html_run@2Dd | step 1-4 ()",
                "17:5-11 doc-use SPEC_html_run@2Dd | step 5 ()",
            ]
        );
        assert!(step_of("5 and 6", "6.2") && step_of("2-4", "3.a"));
        assert!(!step_of("5 and 6", "1") && !step_of("2-4", "5"));
        assert_eq!(relative_step("5 and 6", "1"), "5.1");
        assert_eq!(relative_step("3.b", "1"), "3.b.1");
    }

    #[test]
    fn test_with_indentation() {
        let src = "\
# https://html.spec.whatwg.org/#navigate
def navigate():
    # Steps 1-2.
    foo()
    if x:
        # Step 3.a
        bar()

# Step 4.
def other():
    pass

class C:
    def f(self):
        # https://dom.spec.whatwg.org/#concept-node
        # Step 1.
        if y:
            pass
        # Step 2.
    # Step 3.
";
        let (records, stats) = citation_records(src, &[], &config(), None, None);
        assert_eq!(
            summary(&records),
            vec![
                "1:2-40 doc-use SPEC_html_navigate |  ()",
                "3:6-15 doc-use SPEC_html_navigate | step 1-2 ()",
                "6:10-18 doc-use SPEC_html_navigate | step 3.a ()",
                "15:10-51 doc-use SPEC_dom_concept@2Dnode |  ()",
                "16:10-16 doc-use SPEC_dom_concept@2Dnode | step 1 ()",
                "19:10-16 doc-use SPEC_dom_concept@2Dnode | step 2 ()",
            ]
        );
        assert_eq!((stats.citations, stats.steps), (2, 4));
    }
}
