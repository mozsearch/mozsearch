//! What every match of a regular expression must contain, for narrowing a
//! search before running the pattern: literal strings for livegrep (a
//! prefilter of the files to run the full pattern on; see `search-text`), and
//! the tokens every match contains whole, whose journals say when matches may
//! have appeared or disappeared (see the history greps).
//!
//! Patterns are parsed with fancy-regex's parser, which takes everything the
//! `regex` crate does plus lookaround and backreferences.  The analysis is
//! like codesearch's trigram query planning (Russ Cox's "Regular Expression
//! Matching with a Trigram Index"), on literal strings rather than trigrams:
//! each node is either one of a small set of exact strings, or has sets of
//! possible prefixes and suffixes and a query its matches satisfy, and a
//! concatenation's query also has the strings spanning its parts.
//!
//! Token boundaries are kept in the literals as `BOUNDARY` characters: where
//! the pattern has `\b`, `^` or `$`, or where a literal is next to something
//! that can only match non-word characters (ex: the `\s*` in `foo\s*=`), so
//! that a token at a literal's edge counts as whole when the pattern ensures
//! nothing word-like continues it.

use std::collections::BTreeSet;

use fancy_regex::{Assertion, Expr};

use crate::file_format::history::timeline_tokens::{
    KeyRules, is_host_part, is_prose_stopword, is_trackable_token,
};

/// Marks a token boundary in a literal (see the module's comment).  It's
/// never in the text, so literals are split at it before searching.
pub const BOUNDARY: char = '\0';

/// The most strings an exact, prefix or suffix set keeps.
const MAX_SET: usize = 16;
/// The most alternatives a query's `Or` keeps (more is no constraint).
const MAX_OR: usize = 64;
/// The longest prefix or suffix kept (longer ones are cut).
const MAX_AFFIX: usize = 64;
/// The shortest useful literal piece, in characters.
const MIN_LITERAL: usize = 2;
/// How many of a repetition's required copies are spelled out (ex: `aaa` for
/// `a{300}`).
const MAX_COPIES: usize = 3;

/// A literal string, lowercased if it's matched case-insensitively.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Literal {
    pub text: String,
    pub fold_case: bool,
}

impl Literal {
    fn new(text: &str, fold_case: bool) -> Self {
        Literal {
            text: if fold_case {
                text.to_lowercase()
            } else {
                text.to_string()
            },
            fold_case,
        }
    }

    fn concat(&self, other: &Literal) -> Literal {
        let fold_case = self.fold_case || other.fold_case;
        let mut text = if fold_case && !self.fold_case {
            self.text.to_lowercase()
        } else {
            self.text.clone()
        };
        let mut rest = other.text.as_str();
        // (One boundary is as good as two.)
        if text.ends_with(BOUNDARY) {
            rest = rest.trim_start_matches(BOUNDARY);
        }
        if fold_case && !other.fold_case {
            text.push_str(&rest.to_lowercase());
        } else {
            text.push_str(rest);
        }
        Literal { text, fold_case }
    }

    /// Whether a string containing this contains `other` too.
    fn implies(&self, other: &Literal) -> bool {
        if other.fold_case {
            self.text.to_lowercase().contains(&other.text)
        } else {
            !self.fold_case && self.text.contains(&other.text)
        }
    }

    /// The pieces between boundaries.
    pub fn pieces(&self) -> impl Iterator<Item = &str> {
        self.text.split(BOUNDARY).filter(|piece| !piece.is_empty())
    }

    /// The longest piece, if any is long enough to be worth searching for.
    pub fn longest_piece(&self) -> Option<&str> {
        self.pieces()
            .max_by_key(|piece| piece.chars().count())
            .filter(|piece| piece.chars().count() >= MIN_LITERAL)
    }

    fn is_useful(&self) -> bool {
        self.longest_piece().is_some()
    }

    fn truncate_start(&self, chars: usize) -> Literal {
        let count = self.text.chars().count();
        if count <= chars {
            return self.clone();
        }
        Literal {
            text: self.text.chars().skip(count - chars).collect(),
            fold_case: self.fold_case,
        }
    }

    fn truncate_end(&self, chars: usize) -> Literal {
        Literal {
            text: self.text.chars().take(chars).collect(),
            fold_case: self.fold_case,
        }
    }
}

/// A condition every match satisfies: it contains the literals as an `And`
/// and `Or` of them say.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Query {
    /// No constraint.
    #[default]
    All,
    Literal(Literal),
    And(Vec<Query>),
    Or(Vec<Query>),
}

impl Query {
    fn and(queries: Vec<Query>) -> Query {
        let mut parts = vec![];
        for query in queries {
            match query {
                Query::All => {}
                Query::And(inner) => parts.extend(inner),
                other => parts.push(other),
            }
        }
        dedup(&mut parts);
        // (A conjunct another one implies says nothing more.)
        drop_implied(&mut parts, |kept, other| other.implies(kept));
        match parts.len() {
            0 => Query::All,
            1 => parts.pop().unwrap(),
            _ => Query::And(parts),
        }
    }

    fn or(queries: Vec<Query>) -> Query {
        let mut parts = vec![];
        for query in queries {
            match query {
                Query::All => return Query::All,
                Query::Or(inner) => parts.extend(inner),
                other => parts.push(other),
            }
        }
        dedup(&mut parts);
        // (An alternative implying another one is covered by it.)
        drop_implied(&mut parts, |kept, other| kept.implies(other));
        match parts.len() {
            0 => Query::All,
            1 => parts.pop().unwrap(),
            n if n > MAX_OR => Query::All,
            _ => Query::Or(parts),
        }
    }

    /// Any of the literals (each must be long enough to be a constraint).
    fn any_of(literals: &BTreeSet<Literal>) -> Query {
        if literals.is_empty() || literals.iter().any(|literal| !literal.is_useful()) {
            return Query::All;
        }
        Query::or(literals.iter().cloned().map(Query::Literal).collect())
    }

    pub fn is_all(&self) -> bool {
        matches!(self, Query::All)
    }

    /// The query as conjuncts of alternatives of literals, weakened where it
    /// isn't of that shape (an `And` inside an `Or` becomes its first
    /// conjunct with a literal), for searching with one alternation per
    /// conjunct.
    pub fn conjuncts(&self) -> Vec<Vec<Literal>> {
        match self {
            Query::All => vec![],
            Query::Literal(literal) => vec![vec![literal.clone()]],
            Query::And(parts) => parts.iter().flat_map(|part| part.conjuncts()).collect(),
            Query::Or(parts) => {
                let mut alternatives = vec![];
                for part in parts {
                    // (Each alternative's longest conjunct, the most selective
                    // guess.)
                    let best = part.conjuncts().into_iter().max_by_key(|conjunct| {
                        conjunct
                            .iter()
                            .map(|literal| literal.text.chars().count())
                            .min()
                            .unwrap_or(0)
                    });
                    match best {
                        Some(conjunct) => alternatives.extend(conjunct),
                        None => return vec![],
                    }
                }
                vec![alternatives]
            }
        }
    }
}

/// Drop the literals `redundant(literal, other)` says another literal makes
/// redundant.
fn drop_implied(parts: &mut Vec<Query>, redundant: impl Fn(&Literal, &Literal) -> bool) {
    let literals: Vec<Option<Literal>> = parts
        .iter()
        .map(|part| match part {
            Query::Literal(literal) => Some(literal.clone()),
            _ => None,
        })
        .collect();
    let mut i = 0;
    parts.retain(|_| {
        let index = i;
        i += 1;
        let Some(literal) = &literals[index] else {
            return true;
        };
        !literals.iter().enumerate().any(|(j, other)| {
            j != index
                && other.as_ref().is_some_and(|other| {
                    redundant(literal, other)
                        // (Of two equivalent ones, keep the first.)
                        && !(j > index && redundant(other, literal))
                })
        })
    });
}

fn dedup(parts: &mut Vec<Query>) {
    let mut seen = vec![];
    parts.retain(|part| {
        if seen.contains(part) {
            false
        } else {
            seen.push(part.clone());
            true
        }
    });
}

/// What a node of the pattern tells about its matches (see the module's
/// comment).
#[derive(Clone, Debug)]
struct Info {
    /// Whether it can match the empty string.
    can_empty: bool,
    /// The strings it matches, if they're few.
    exact: Option<BTreeSet<Literal>>,
    /// Otherwise, what its matches start and end with (the empty string if
    /// that's unknown), and what they satisfy.
    prefix: BTreeSet<Literal>,
    suffix: BTreeSet<Literal>,
    query: Query,
}

fn set_of(literal: Literal) -> BTreeSet<Literal> {
    BTreeSet::from([literal])
}

fn empty_literal() -> Literal {
    Literal::new("", false)
}

fn cross(a: &BTreeSet<Literal>, b: &BTreeSet<Literal>) -> BTreeSet<Literal> {
    let mut out = BTreeSet::new();
    for x in a {
        for y in b {
            out.insert(x.concat(y));
        }
    }
    out
}

impl Info {
    fn exact(literals: BTreeSet<Literal>) -> Info {
        Info {
            can_empty: literals.iter().any(|literal| literal.text.is_empty()),
            exact: Some(literals),
            prefix: BTreeSet::new(),
            suffix: BTreeSet::new(),
            query: Query::All,
        }
    }

    fn empty() -> Info {
        Info::exact(set_of(empty_literal()))
    }

    fn boundary() -> Info {
        Info::exact(set_of(Literal::new(&BOUNDARY.to_string(), false)))
    }

    fn any_char() -> Info {
        Info {
            can_empty: false,
            exact: None,
            prefix: set_of(empty_literal()),
            suffix: set_of(empty_literal()),
            query: Query::All,
        }
    }

    fn any_string() -> Info {
        Info {
            can_empty: true,
            ..Info::any_char()
        }
    }

    /// This without an exact set, which goes into its prefixes, suffixes and
    /// query.
    fn inexact(mut self) -> Info {
        if let Some(exact) = self.exact.take() {
            self.query = Query::and(vec![self.query, Query::any_of(&exact)]);
            self.prefix = exact.iter().map(|l| l.truncate_end(MAX_AFFIX)).collect();
            self.suffix = exact.iter().map(|l| l.truncate_start(MAX_AFFIX)).collect();
        }
        self.simplify_affixes();
        self
    }

    /// Keep the affix sets small: past `MAX_SET`, what they say goes into
    /// the query and they become unknown.
    fn simplify_affixes(&mut self) {
        for affix in [&mut self.prefix, &mut self.suffix] {
            if affix.len() > MAX_SET {
                let query = Query::any_of(affix);
                self.query = Query::and(vec![std::mem::take(&mut self.query), query]);
                *affix = set_of(empty_literal());
            }
        }
    }

    fn starts(&self) -> &BTreeSet<Literal> {
        self.exact.as_ref().unwrap_or(&self.prefix)
    }

    fn ends(&self) -> &BTreeSet<Literal> {
        self.exact.as_ref().unwrap_or(&self.suffix)
    }

    /// `starts` or `ends`, with the empty string if it can be empty.
    fn edge_or_empty(&self, edge: &BTreeSet<Literal>) -> BTreeSet<Literal> {
        let mut edge = edge.clone();
        if self.can_empty {
            edge.insert(empty_literal());
        }
        edge
    }

    fn concat(self, other: Info) -> Info {
        if let (Some(a), Some(b)) = (&self.exact, &other.exact)
            && a.len() * b.len() <= MAX_SET
        {
            return Info::exact(cross(a, b));
        }
        // (The strings spanning the two: one's end and the other's start, or
        // nothing, if either can be empty.)
        let spanning = Query::any_of(&cross(
            &self.edge_or_empty(self.ends()),
            &other.edge_or_empty(other.starts()),
        ));
        let mut prefix = match &self.exact {
            Some(exact) => cross(exact, &other.edge_or_empty(other.starts())),
            None => self.prefix.clone(),
        };
        if self.can_empty {
            prefix.extend(other.starts().iter().cloned());
        }
        let mut suffix = match &other.exact {
            Some(exact) => cross(&self.edge_or_empty(self.ends()), exact),
            None => other.suffix.clone(),
        };
        if other.can_empty {
            suffix.extend(self.ends().iter().cloned());
        }
        let can_empty = self.can_empty && other.can_empty;
        let (a, b) = (self.inexact(), other.inexact());
        let mut info = Info {
            can_empty,
            exact: None,
            prefix: prefix.iter().map(|l| l.truncate_end(MAX_AFFIX)).collect(),
            suffix: suffix.iter().map(|l| l.truncate_start(MAX_AFFIX)).collect(),
            query: Query::and(vec![a.query, b.query, spanning]),
        };
        info.simplify_affixes();
        info
    }

    fn alternate(self, other: Info) -> Info {
        if let (Some(a), Some(b)) = (&self.exact, &other.exact)
            && a.len() + b.len() <= MAX_SET
        {
            return Info::exact(a.union(b).cloned().collect());
        }
        let can_empty = self.can_empty || other.can_empty;
        let (a, b) = (self.inexact(), other.inexact());
        let mut info = Info {
            can_empty,
            exact: None,
            prefix: a.prefix.union(&b.prefix).cloned().collect(),
            suffix: a.suffix.union(&b.suffix).cloned().collect(),
            query: Query::or(vec![a.query, b.query]),
        };
        info.simplify_affixes();
        info
    }

    /// The final query: everything this says about its matches.
    fn into_query(self) -> Query {
        let info = self.inexact();
        Query::and(vec![
            info.query,
            Query::any_of(&info.prefix),
            Query::any_of(&info.suffix),
        ])
    }
}

/// Whether a character can be part of a token (see
/// `timeline_tokens::is_trackable_token`).
pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// Whether an expression's first (or last, if `last`) character, when it
/// matches anything, is always a non-word character or a boundary, and
/// whether it can match the empty string.
fn edge_is_nonword(expr: &Expr, last: bool) -> (bool, bool) {
    match expr {
        Expr::Empty => (true, true),
        Expr::Literal { val, .. } => {
            let c = if last {
                val.chars().next_back()
            } else {
                val.chars().next()
            };
            match c {
                Some(c) => (!is_word_char(c), false),
                None => (true, true),
            }
        }
        Expr::Delegate { inner, .. } => (class_is_nonword(inner), false),
        Expr::Any { .. } | Expr::LiteralBytes { .. } => (false, false),
        Expr::Assertion(assertion) => match assertion {
            Assertion::NotWordBoundary => (true, true),
            // (A boundary itself.)
            _ => (true, false),
        },
        Expr::GeneralNewline { .. } => (true, false),
        Expr::Concat(children) => {
            let ordered: Box<dyn Iterator<Item = &Expr>> = if last {
                Box::new(children.iter().rev())
            } else {
                Box::new(children.iter())
            };
            for child in ordered {
                let (nonword, can_empty) = edge_is_nonword(child, last);
                if !nonword {
                    return (false, false);
                }
                if !can_empty {
                    return (true, false);
                }
            }
            (true, true)
        }
        Expr::Alt(children) => {
            let mut all_nonword = true;
            let mut any_empty = false;
            for child in children {
                let (nonword, can_empty) = edge_is_nonword(child, last);
                all_nonword &= nonword;
                any_empty |= can_empty;
            }
            (all_nonword, any_empty)
        }
        Expr::Group(child) => edge_is_nonword(child, last),
        Expr::AtomicGroup(child) => edge_is_nonword(child, last),
        Expr::Repeat { child, lo, .. } => {
            let (nonword, can_empty) = edge_is_nonword(child, last);
            (nonword, can_empty || *lo == 0)
        }
        // (Zero-width, so whatever follows decides.)
        Expr::LookAround(..) | Expr::KeepOut => (true, true),
        _ => (false, true),
    }
}

/// Whether a one-character class (a delegate's regex, ex: `\s`, `[(\[]`) only
/// matches non-word characters.
fn class_is_nonword(class: &str) -> bool {
    let Ok(hir) = regex_syntax::Parser::new().parse(class) else {
        return false;
    };
    match hir.kind() {
        regex_syntax::hir::HirKind::Class(regex_syntax::hir::Class::Unicode(class)) => {
            class.iter().all(|range| {
                let (start, end) = (range.start() as u32, range.end() as u32);
                // (Small ranges are checked character by character; a big one
                // is assumed to have word characters.)
                end - start < 256
                    && (start..=end)
                        .filter_map(char::from_u32)
                        .all(|c| !is_word_char(c))
            })
        }
        regex_syntax::hir::HirKind::Class(regex_syntax::hir::Class::Bytes(class)) => class
            .iter()
            .all(|range| (range.start()..=range.end()).all(|b| !is_word_char(b as char))),
        regex_syntax::hir::HirKind::Literal(lit) => std::str::from_utf8(&lit.0)
            .map(|s| s.chars().all(|c| !is_word_char(c)))
            .unwrap_or(false),
        _ => false,
    }
}

/// The characters a one-character class matches, if they're few (ex: `[xy]`,
/// `\.`).
fn class_chars(class: &str) -> Option<Vec<char>> {
    let hir = regex_syntax::Parser::new().parse(class).ok()?;
    match hir.kind() {
        regex_syntax::hir::HirKind::Class(regex_syntax::hir::Class::Unicode(class)) => {
            let mut chars = vec![];
            for range in class.iter() {
                for c in range.start()..=range.end() {
                    chars.push(c);
                    if chars.len() > 4 {
                        return None;
                    }
                }
            }
            Some(chars)
        }
        regex_syntax::hir::HirKind::Literal(lit) => {
            let s = std::str::from_utf8(&lit.0).ok()?;
            (s.chars().count() == 1).then(|| s.chars().collect())
        }
        _ => None,
    }
}

/// Analyze `expr`, whose neighbors ensure a token boundary before it if
/// `boundary_before`, and after it if `boundary_after`.
fn analyze(expr: &Expr, boundary_before: bool, boundary_after: bool) -> Info {
    match expr {
        Expr::Empty => Info::empty(),
        Expr::Literal { val, casei } => {
            let mut text = String::new();
            if boundary_before {
                text.push(BOUNDARY);
            }
            text.push_str(val);
            if boundary_after {
                text.push(BOUNDARY);
            }
            Info::exact(set_of(Literal::new(&text, *casei)))
        }
        Expr::LiteralBytes { .. } | Expr::Any { .. } => Info::any_char(),
        Expr::Delegate { inner, casei } => match class_chars(inner) {
            Some(chars) => Info::exact(
                chars
                    .into_iter()
                    .map(|c| Literal::new(&c.to_string(), *casei))
                    .collect(),
            ),
            None => Info::any_char(),
        },
        Expr::Assertion(assertion) => match assertion {
            Assertion::NotWordBoundary => Info::empty(),
            _ => Info::boundary(),
        },
        Expr::GeneralNewline { .. } => Info::any_char(),
        Expr::Concat(children) => {
            let mut info = Info::empty();
            for (i, child) in children.iter().enumerate() {
                // (Whether what's before and after the child ensures a
                // boundary: the nearest neighbor that can't be empty only
                // matches non-word characters there, with any empty-able
                // ones between doing so too, or the concatenation's own edge
                // does.)
                let before = neighbors_nonword(children[..i].iter().rev(), true, boundary_before);
                let after = neighbors_nonword(children[i + 1..].iter(), false, boundary_after);
                info = info.concat(analyze(child, before, after));
            }
            info
        }
        Expr::Alt(children) => {
            let mut iter = children.iter();
            let Some(first) = iter.next() else {
                return Info::empty();
            };
            let mut info = analyze(first, boundary_before, boundary_after);
            for child in iter {
                info = info.alternate(analyze(child, boundary_before, boundary_after));
            }
            info
        }
        Expr::Group(child) => analyze(child, boundary_before, boundary_after),
        Expr::AtomicGroup(child) => analyze(child, boundary_before, boundary_after),
        Expr::Repeat { child, lo, hi, .. } => {
            if *hi == 0 {
                return Info::empty();
            }
            if *lo == 0 && *hi == 1 {
                return Info::empty().alternate(analyze(child, boundary_before, boundary_after));
            }
            if *lo == 0 {
                return Info::any_string();
            }
            if *lo == 1 && *hi == 1 {
                return analyze(child, boundary_before, boundary_after);
            }
            // (Its first few required copies, ex: `aaa` for `a{300}`, then
            // what any more copies say.)
            let one = analyze(child, false, false);
            let copies = (*lo).min(MAX_COPIES);
            let mut info = one.clone();
            for _ in 1..copies {
                info = info.concat(one.clone());
            }
            if *hi > copies {
                // (Any more copies, which may be none: they start and end as
                // a copy does, or not at all, and require nothing.)
                let one = one.inexact();
                let more = Info {
                    can_empty: true,
                    exact: None,
                    prefix: one.edge_or_empty(&one.prefix),
                    suffix: one.edge_or_empty(&one.suffix),
                    query: Query::All,
                };
                info = info.concat(more);
            }
            info
        }
        // (Zero-width.)
        Expr::LookAround(..) | Expr::KeepOut | Expr::ContinueFromPreviousMatchEnd => Info::empty(),
        _ => Info::any_string(),
    }
}

/// Whether `neighbors` (nearest first) ensure a boundary: see `analyze`.
fn neighbors_nonword<'a>(
    neighbors: impl Iterator<Item = &'a Expr>,
    looking_back: bool,
    edge: bool,
) -> bool {
    for neighbor in neighbors {
        // (What's before ends next to the child; what's after starts there.)
        let (nonword, can_empty) = edge_is_nonword(neighbor, looking_back);
        if !nonword {
            return false;
        }
        if !can_empty {
            return true;
        }
    }
    edge
}

/// Whether an expression uses features only fancy-regex has (lookaround,
/// backreferences, atomic groups, ...), which livegrep's RE2 doesn't.
fn needs_fancy(expr: &Expr) -> bool {
    match expr {
        Expr::LookAround(..)
        | Expr::Backref { .. }
        | Expr::BackrefWithRelativeRecursionLevel { .. }
        | Expr::AtomicGroup(_)
        | Expr::KeepOut
        | Expr::ContinueFromPreviousMatchEnd
        | Expr::BackrefExistsCondition { .. }
        | Expr::Conditional { .. }
        | Expr::SubroutineCall(_)
        | Expr::BacktrackingControlVerb(_)
        | Expr::Absent(_)
        | Expr::DefineGroup { .. } => true,
        _ => children(expr).any(needs_fancy),
    }
}

/// Whether an expression asks to match across lines: a newline in a literal
/// or class (`\n`, `[\s\S]` aside from `\s` alone), `.` matching newlines
/// (`(?s)`), or `\R`.
fn spans_lines(expr: &Expr) -> bool {
    match expr {
        Expr::Literal { val, .. } => val.contains('\n'),
        Expr::Any { newline, .. } => *newline,
        Expr::GeneralNewline { .. } => true,
        Expr::Delegate { inner, .. } => {
            // (`\s` and the like match newlines too, but mean spaces on a
            // line; a class naming `\n` asks for newlines.)
            inner.contains("\\n") || inner.contains("\\S") && inner.contains("\\s")
        }
        _ => children(expr).any(spans_lines),
    }
}

fn children(expr: &Expr) -> Box<dyn Iterator<Item = &Expr> + '_> {
    match expr {
        Expr::Concat(children) | Expr::Alt(children) => Box::new(children.iter()),
        Expr::Group(child) => Box::new(std::iter::once(&**child)),
        Expr::LookAround(child, _) | Expr::AtomicGroup(child) => {
            Box::new(std::iter::once(&**child))
        }
        Expr::Repeat { child, .. } => Box::new(std::iter::once(&**child)),
        Expr::Conditional {
            condition,
            true_branch,
            false_branch,
        } => Box::new([&**condition, &**true_branch, &**false_branch].into_iter()),
        Expr::DefineGroup { definitions } => Box::new(std::iter::once(&**definitions)),
        _ => Box::new(std::iter::empty()),
    }
}

/// A pattern's analysis.
#[derive(Clone, Debug)]
pub struct RegexPlan {
    /// What every match contains.
    pub query: Query,
    /// Whether the pattern needs fancy-regex (lookaround, backreferences, ...).
    pub needs_fancy: bool,
    /// Whether the pattern asks to match across lines (so it runs on whole
    /// files rather than line by line).
    pub spans_lines: bool,
}

impl RegexPlan {
    /// Whether livegrep (RE2, a line at a time) can run the pattern itself.
    /// (Its other limits, ex: the width of what a pattern can match, are only
    /// known from its answer.)
    pub fn livegrep_can_run(&self) -> bool {
        !self.needs_fancy && !self.spans_lines
    }
}

/// Analyze `pattern`, matched case-insensitively if `fold_case`.
pub fn plan(pattern: &str, fold_case: bool) -> Result<RegexPlan, String> {
    let source = if fold_case {
        format!("(?i){}", pattern)
    } else {
        pattern.to_string()
    };
    let tree = Expr::parse_tree(&source).map_err(|e| e.to_string())?;
    Ok(RegexPlan {
        query: analyze(&tree.expr, false, false).into_query(),
        needs_fancy: needs_fancy(&tree.expr),
        spans_lines: spans_lines(&tree.expr),
    })
}

/// A token that every match contains, or a piece of one (see `tokens`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TokenTerm {
    /// This whole token.
    Whole(String),
    /// A token starting with this (the literal's end cut it).
    Prefix(String),
    /// A token ending with this.
    Suffix(String),
    /// A token containing this.
    Contains(String),
}

impl TokenTerm {
    pub fn text(&self) -> &str {
        match self {
            TokenTerm::Whole(s)
            | TokenTerm::Prefix(s)
            | TokenTerm::Suffix(s)
            | TokenTerm::Contains(s) => s,
        }
    }
}

/// A condition on the tokens every match contains: the tokens `Query`'s
/// literals contain, as `And`s and `Or`s of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenQuery {
    /// No constraint.
    All,
    Term {
        term: TokenTerm,
        fold_case: bool,
    },
    And(Vec<TokenQuery>),
    Or(Vec<TokenQuery>),
}

impl TokenQuery {
    fn and(queries: Vec<TokenQuery>) -> TokenQuery {
        let mut parts: Vec<TokenQuery> = vec![];
        for query in queries {
            match query {
                TokenQuery::All => {}
                TokenQuery::And(inner) => parts.extend(inner),
                other if !parts.contains(&other) => parts.push(other),
                _ => {}
            }
        }
        match parts.len() {
            0 => TokenQuery::All,
            1 => parts.pop().unwrap(),
            _ => TokenQuery::And(parts),
        }
    }

    fn or(queries: Vec<TokenQuery>) -> TokenQuery {
        let mut parts: Vec<TokenQuery> = vec![];
        for query in queries {
            match query {
                TokenQuery::All => return TokenQuery::All,
                TokenQuery::Or(inner) => parts.extend(inner),
                other if !parts.contains(&other) => parts.push(other),
                _ => {}
            }
        }
        match parts.len() {
            0 => TokenQuery::All,
            1 => parts.pop().unwrap(),
            _ => TokenQuery::Or(parts),
        }
    }

    pub fn is_all(&self) -> bool {
        matches!(self, TokenQuery::All)
    }

    /// The cheapest way to satisfy this, by `cost` of each term: an `And`'s
    /// cheapest conjunct, an `Or`'s alternatives all together.  None if
    /// there's no constraint (or a term's cost is unknown, as None).
    pub fn cheapest<'a>(
        &'a self,
        cost: &mut impl FnMut(&TokenTerm, bool) -> Option<u64>,
    ) -> Option<(u64, Vec<(&'a TokenTerm, bool)>)> {
        match self {
            TokenQuery::All => None,
            TokenQuery::Term { term, fold_case } => {
                cost(term, *fold_case).map(|c| (c, vec![(term, *fold_case)]))
            }
            TokenQuery::And(parts) => parts
                .iter()
                .filter_map(|part| part.cheapest(cost))
                .min_by_key(|(c, _)| *c),
            TokenQuery::Or(parts) => {
                let mut total = 0u64;
                let mut terms = vec![];
                for part in parts {
                    let (c, part_terms) = part.cheapest(cost)?;
                    total = total.saturating_add(c);
                    terms.extend(part_terms);
                }
                Some((total, terms))
            }
        }
    }
}

/// Words that are keywords in a language the history tokenizes, whose
/// journals only have their uses as words in comments and the like (code's
/// keywords aren't tracked), so they don't say when a match appeared.
const KEYWORDS: &[&str] = &[
    "abstract",
    "alignas",
    "alignof",
    "and",
    "as",
    "asm",
    "assert",
    "async",
    "auto",
    "await",
    "bool",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "constexpr",
    "continue",
    "crate",
    "def",
    "default",
    "del",
    "delete",
    "do",
    "double",
    "dyn",
    "elif",
    "else",
    "enum",
    "explicit",
    "export",
    "extends",
    "extern",
    "final",
    "finally",
    "float",
    "fn",
    "for",
    "friend",
    "from",
    "fun",
    "function",
    "global",
    "goto",
    "if",
    "impl",
    "implements",
    "import",
    "in",
    "inline",
    "instanceof",
    "int",
    "interface",
    "is",
    "lambda",
    "let",
    "long",
    "loop",
    "match",
    "mod",
    "move",
    "mut",
    "mutable",
    "namespace",
    "native",
    "new",
    "noexcept",
    "not",
    "object",
    "of",
    "operator",
    "or",
    "override",
    "package",
    "pass",
    "private",
    "protected",
    "pub",
    "public",
    "raise",
    "ref",
    "register",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "struct",
    "super",
    "switch",
    "synchronized",
    "template",
    "throw",
    "throws",
    "trait",
    "try",
    "type",
    "typedef",
    "typeid",
    "typename",
    "typeof",
    "union",
    "unsafe",
    "unsigned",
    "use",
    "using",
    "val",
    "var",
    "virtual",
    "void",
    "volatile",
    "when",
    "where",
    "while",
    "with",
    "yield",
];

/// Words that some language's tokens don't track as identifiers (see
/// `timeline_tokens::namespace_value_words`).
const VALUE_WORDS: &[&str] = &[
    "this",
    "nullptr",
    "NULL",
    "true",
    "false",
    "super",
    "null",
    "undefined",
    "self",
    "None",
    "True",
    "False",
    "Self",
    "crate",
    "Some",
    "Ok",
    "Err",
];

/// Whether `word`, as a whole token, has a journal that covers its uses in
/// code (and prose: not the stopwords which comments' words leave out, nor,
/// with V2's keys, the parts which compounds leave out, ex: `www` and `org`).
fn journaled(word: &str, rules: KeyRules) -> bool {
    is_trackable_token(word)
        && !KEYWORDS.contains(&word)
        && !VALUE_WORDS.contains(&word)
        && !is_prose_stopword(word)
        && !(rules >= KeyRules::V2 && is_host_part(word))
}

fn is_quote(c: char) -> bool {
    matches!(c, '"' | '\'' | '`')
}

/// The terms of a literal's compound words next to a quote (see
/// `timeline_tokens::KeyRules::V2`, whose histories track them whole and by
/// their parts): a quote next to a word starts or ends a string (or quotes it
/// in prose), where they're tracked whole, but in code they aren't
/// (`moz-icon` there is the tokens `moz`, `-` and `icon`, whose words are the
/// literal's other terms).  Each compound of hyphens of the literal's runs of
/// words joined by `-` or `.` (ex: `moz-icon` in `"moz-icon://`, and
/// `moz-document` in `"layout.css.moz-document`), and each run of 3 or more
/// parts joined by dots which the literal starts (`"network.proxy.type`), is
/// whole where the literal ends it on both sides (with a character which
/// isn't a word's, and doesn't join one), else a piece.  Not compounds which
/// could be a bug's key (`bug-NNNNNNN`, lowercase whatever the text's case),
/// nor those without a whole word which could be a token, which a compound's
/// key needs (ex: `x-1`; see `timeline_tokens::compound_keys`).
fn compound_terms(literal: &Literal) -> Vec<TokenQuery> {
    // (A boundary between two characters says nothing more about them, ex:
    // `"\0moz\0-\0icon\0"`.)
    let text: Vec<char> = literal.text.chars().collect();
    let chars: Vec<char> = (0..text.len())
        .filter(|&i| {
            text[i] != BOUNDARY
                || i == 0
                || i + 1 == text.len()
                || text[i - 1] == BOUNDARY
                || text[i + 1] == BOUNDARY
        })
        .map(|i| text[i])
        .collect();
    let at = |i: Option<usize>| i.and_then(|i| chars.get(i).copied());
    // (Whether the character at `i` ends a compound, with `beyond` the one
    // past it.)
    let ends = |i: Option<usize>, beyond: Option<usize>| match at(i) {
        None | Some(BOUNDARY) => false,
        Some('-' | '.') => at(beyond).is_some_and(|c| c != BOUNDARY && !is_word_char(c)),
        Some(_) => true,
    };
    let mut terms = vec![];
    let mut i = 0;
    while i < chars.len() {
        if !is_word_char(chars[i]) {
            i += 1;
            continue;
        }
        let start = i;
        let mut joined = false;
        loop {
            while i < chars.len() && is_word_char(chars[i]) {
                i += 1;
            }
            if i + 1 < chars.len() && matches!(chars[i], '-' | '.') && is_word_char(chars[i + 1]) {
                joined = true;
                i += 1;
            } else {
                break;
            }
        }
        let end = i;
        let quoted =
            at(start.checked_sub(1)).is_some_and(is_quote) || at(Some(end)).is_some_and(is_quote);
        if !joined || !quoted {
            continue;
        }
        let bounded_before = ends(start.checked_sub(1), start.checked_sub(2));
        let bounded_after = ends(Some(end), Some(end + 1));
        let mut push = |text: &[char], before: bool, after: bool| {
            let text: String = text.iter().collect();
            let term = match (before, after) {
                (true, true) => TokenTerm::Whole(text),
                (true, false) => TokenTerm::Prefix(text),
                (false, true) => TokenTerm::Suffix(text),
                (false, false) => TokenTerm::Contains(text),
            };
            terms.push(TokenQuery::Term {
                term,
                fold_case: literal.fold_case,
            });
        };
        // (Keys are at most 128 bytes long.)
        let too_long = |text: &[char]| text.iter().map(|c| c.len_utf8()).sum::<usize>() > 128;
        let run = &chars[start..end];
        let dotted: Vec<&[char]> = run.split(|&c| c == '.').collect();
        if dotted.len() >= 3
            && bounded_before
            && (run[0].is_alphabetic() || run[0] == '_' || run[0] == '$')
            && !(bounded_after && too_long(run))
        {
            push(run, true, bounded_after);
        }
        for (n, part) in dotted.iter().enumerate() {
            let before = n > 0 || bounded_before;
            let after = n + 1 < dotted.len() || bounded_after;
            if !part.contains(&'-') {
                continue;
            }
            let words: Vec<String> = part
                .split(|&c| c == '-')
                .map(|word| word.iter().collect())
                .collect();
            let maybe_bug = words.len() == 2
                && words[1].chars().all(|c| c.is_ascii_digit())
                && if before {
                    words[0].eq_ignore_ascii_case("bug")
                } else {
                    "bug".ends_with(&words[0].to_ascii_lowercase())
                };
            let whole_word = words.iter().enumerate().any(|(w, word)| {
                (w > 0 || before) && (w + 1 < words.len() || after) && is_trackable_token(word)
            });
            if !maybe_bug && whole_word && !(before && after && too_long(part)) {
                push(part, before, after);
            }
        }
    }
    terms
}

/// The token terms of a literal: its runs of word characters, whole where a
/// boundary or non-word character is on both sides within the literal, and
/// otherwise pieces of tokens; and with V2's keys, its compound words' (see
/// `compound_terms`).
fn literal_terms(literal: &Literal, rules: KeyRules) -> TokenQuery {
    let chars: Vec<char> = literal.text.chars().collect();
    let mut terms = vec![];
    let mut i = 0;
    while i < chars.len() {
        if !is_word_char(chars[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && is_word_char(chars[i]) {
            i += 1;
        }
        let word: String = chars[start..i].iter().collect();
        let bounded_before = start > 0;
        let bounded_after = i < chars.len();
        // (A piece can be the whole token too, so it must be one whose
        // journal covers code, ex: not `case`, whose matches are often the
        // keyword; and a piece must be long enough to narrow the tokens
        // down.)
        if !journaled(&word, rules)
            || !(bounded_before && bounded_after) && word.chars().count() < 3
        {
            continue;
        }
        let term = match (bounded_before, bounded_after) {
            (true, true) => TokenTerm::Whole(word),
            (true, false) => TokenTerm::Prefix(word),
            (false, true) => TokenTerm::Suffix(word),
            (false, false) => TokenTerm::Contains(word),
        };
        terms.push(TokenQuery::Term {
            term,
            fold_case: literal.fold_case,
        });
    }
    if rules >= KeyRules::V2 {
        terms.extend(compound_terms(literal));
    }
    TokenQuery::and(terms)
}

impl Query {
    /// The tokens every match contains (see `TokenQuery`), as a history with
    /// the key `rules` tracks them.
    pub fn tokens(&self, rules: KeyRules) -> TokenQuery {
        match self {
            Query::All => TokenQuery::All,
            Query::Literal(literal) => literal_terms(literal, rules),
            Query::And(parts) => {
                TokenQuery::and(parts.iter().map(|part| part.tokens(rules)).collect())
            }
            Query::Or(parts) => {
                TokenQuery::or(parts.iter().map(|part| part.tokens(rules)).collect())
            }
        }
    }
}

/// The RE2 pattern (as livegrep takes) matching any of `literals`' longest
/// pieces, and whether to search for it case-insensitively (if any of them
/// is: a superset of the others' matches; livegrep's RE2 doesn't take
/// `(?i:...)`).
pub fn livegrep_alternation(literals: &[Literal]) -> Option<(String, bool)> {
    let mut alternatives = vec![];
    for literal in literals {
        alternatives.push(regex_syntax::escape(literal.longest_piece()?));
    }
    alternatives.sort();
    alternatives.dedup();
    let fold_case = literals.iter().any(|literal| literal.fold_case);
    Some((alternatives.join("|"), fold_case))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(text: &str) -> Query {
        Query::Literal(Literal::new(text, false))
    }

    fn query(pattern: &str) -> Query {
        plan(pattern, false).unwrap().query
    }

    fn b(text: &str) -> String {
        text.replace('|', &BOUNDARY.to_string())
    }

    #[test]
    fn test_literals() {
        assert_eq!(query("foo"), lit("foo"));
        assert_eq!(query(r"\bfoo\b"), lit(&b("|foo|")));
        // (`\s*` and `=` only match non-word characters, so `kMax` ends at a
        // boundary.)
        assert_eq!(query(r"kMax\s*=\s*(\d+)"), lit(&b("kMax|")));
        assert_eq!(query(r"foo.*bar"), Query::And(vec![lit("foo"), lit("bar")]));
        assert_eq!(
            query(r"(foo|bar)baz"),
            Query::Or(vec![lit("barbaz"), lit("foobaz")])
        );
        assert_eq!(query(r"(foo)?bar"), lit("bar"));
        assert_eq!(query(r"\w+"), Query::All);
        assert_eq!(query(r"a{300}"), lit("aaa"));
        assert_eq!(
            query(r"(?:ab){2,}c"),
            Query::And(vec![lit("abab"), lit("abc")])
        );
        assert_eq!(query(r"a.b"), Query::All);
        // (Lookaround and backreferences don't constrain, but the rest does.)
        assert_eq!(query(r"(?<=foo)bar(\w)\1"), lit("bar"));
        // (Case-insensitive literals are lowercased.)
        assert_eq!(
            plan("FooBar", true).unwrap().query,
            Query::Literal(Literal::new("foobar", true))
        );
        assert!(plan(r"(?<=foo)bar", false).unwrap().needs_fancy);
        assert!(!plan(r"foo\s+bar", false).unwrap().spans_lines);
        assert!(plan(r"foo\n\s*bar", false).unwrap().spans_lines);
        assert!(plan(r"(?s)foo.*bar", false).unwrap().spans_lines);
        assert!(plan(r"foo\s+bar", false).unwrap().livegrep_can_run());
    }

    fn whole(text: &str) -> TokenQuery {
        TokenQuery::Term {
            term: TokenTerm::Whole(text.to_string()),
            fold_case: false,
        }
    }

    fn term(term: TokenTerm) -> TokenQuery {
        TokenQuery::Term {
            term,
            fold_case: false,
        }
    }

    #[test]
    fn test_tokens() {
        let tokens = |pattern: &str| query(pattern).tokens(KeyRules::V1);
        // (A pref name: its parts are whole tokens inside the literal, and the
        // ends are pieces.)
        assert_eq!(
            tokens(&regex::escape("dom.clients.openwindow_favors_same_process")),
            TokenQuery::And(vec![
                term(TokenTerm::Suffix("dom".to_string())),
                whole("clients"),
                term(TokenTerm::Prefix(
                    "openwindow_favors_same_process".to_string()
                )),
            ])
        );
        assert_eq!(
            tokens(&regex::escape(
                r#""dom.clients.openwindow_favors_same_process""#
            )),
            TokenQuery::And(vec![
                whole("dom"),
                whole("clients"),
                whole("openwindow_favors_same_process"),
            ])
        );
        assert_eq!(tokens(r"\bkMaxWorkers\b"), whole("kMaxWorkers"));
        assert_eq!(
            tokens(r"kMaxWorkers\s*=\s*(\d+)"),
            term(TokenTerm::Suffix("kMaxWorkers".to_string()))
        );
        assert_eq!(tokens(r"\bkMaxWorkers\s*=\s*(\d+)"), whole("kMaxWorkers"));
        // (Keywords' journals don't cover code, and numbers aren't tracked.)
        assert_eq!(tokens(r"\breturn 42;"), TokenQuery::All);
        assert_eq!(tokens(r"case\s+JSOp::(\w+)\s*:"), whole("JSOp"));
        assert_eq!(
            tokens(r"\b(Foo|Bar)Baz\b"),
            TokenQuery::Or(vec![whole("BarBaz"), whole("FooBaz")])
        );
        assert_eq!(tokens(r"\w+\(\)"), TokenQuery::All);
    }

    #[test]
    fn test_compound_tokens() {
        let tokens = |text: &str| query(&regex::escape(text)).tokens(KeyRules::V2);
        let and = |terms: Vec<TokenQuery>| TokenQuery::And(terms);
        let prefix = |text: &str| term(TokenTerm::Prefix(text.to_string()));
        let suffix = |text: &str| term(TokenTerm::Suffix(text.to_string()));
        // (Next to a quote, compounds are whole tokens too.)
        assert_eq!(
            tokens(r#""moz-icon""#),
            and(vec![whole("moz"), whole("icon"), whole("moz-icon")])
        );
        assert_eq!(
            tokens(r#"SchemeIs("moz-icon")"#),
            and(vec![
                suffix("SchemeIs"),
                whole("moz"),
                whole("icon"),
                whole("moz-icon")
            ])
        );
        assert_eq!(
            tokens(r#""moz-icon://"#),
            and(vec![whole("moz"), whole("icon"), whole("moz-icon")])
        );
        assert_eq!(
            tokens(r#"-moz-box""#),
            and(vec![whole("moz"), whole("box"), suffix("moz-box")])
        );
        assert_eq!(
            tokens(r#""dom.clients.openwindow_favors_same_process""#),
            and(vec![
                whole("dom"),
                whole("clients"),
                whole("openwindow_favors_same_process"),
                whole("dom.clients.openwindow_favors_same_process"),
            ])
        );
        assert_eq!(
            tokens(r#""layout.css.moz-doc"#),
            and(vec![
                whole("layout"),
                whole("css"),
                whole("moz"),
                prefix("doc"),
                prefix("layout.css.moz-doc"),
                prefix("moz-doc"),
            ])
        );
        // (In code, `moz-icon` is `moz`, `-` and `icon`.)
        assert_eq!(tokens("moz-icon"), and(vec![suffix("moz"), prefix("icon")]));
        assert_eq!(
            query(&regex::escape(r#""moz-icon""#)).tokens(KeyRules::V1),
            and(vec![whole("moz"), whole("icon")])
        );
        // (Hosts' `www` and `org` aren't compounds' parts.)
        assert_eq!(
            tokens(r#""www.mozilla.org""#),
            and(vec![whole("mozilla"), whole("www.mozilla.org")])
        );
        // (Not compounds without a word, or which could be a bug's key.)
        assert_eq!(tokens("'x-1'"), TokenQuery::All);
        assert_eq!(tokens(r#""bug-1234""#), TokenQuery::All);
        assert_eq!(tokens(r#"ug-1234""#), TokenQuery::All);
        assert_eq!(
            tokens(r#""ug-1234""#),
            and(vec![whole("ug"), whole("ug-1234")])
        );
    }

    #[test]
    fn test_cheapest() {
        let q = query(&regex::escape(
            r#""dom.clients.openwindow_favors_same_process""#,
        ))
        .tokens(KeyRules::V1);
        let mut cost = |term: &TokenTerm, _fold: bool| Some(1_000_000 / term.text().len() as u64);
        let (_, terms) = q.cheapest(&mut cost).unwrap();
        assert_eq!(
            terms,
            vec![(
                &TokenTerm::Whole("openwindow_favors_same_process".to_string()),
                false
            )]
        );
        let q = query(r"\b(Foo|Bar)Baz\b").tokens(KeyRules::V1);
        let (total, terms) = q.cheapest(&mut cost).unwrap();
        assert_eq!(terms.len(), 2);
        assert_eq!(total, 2 * (1_000_000 / 6));
        assert!(TokenQuery::All.cheapest(&mut cost).is_none());
    }

    /// Whether `query` holds for a match at `range` of `text`.
    fn holds(query: &Query, text: &str, range: std::ops::Range<usize>) -> bool {
        let found = &text[range];
        match query {
            Query::All => true,
            Query::Literal(literal) => literal.pieces().all(|piece| {
                if literal.fold_case {
                    found.to_lowercase().contains(piece)
                } else {
                    found.contains(piece)
                }
            }),
            Query::And(parts) => parts
                .iter()
                .all(|part| holds(part, text, range_of(text, found))),
            Query::Or(parts) => parts
                .iter()
                .any(|part| holds(part, text, range_of(text, found))),
        }
    }

    fn range_of(text: &str, found: &str) -> std::ops::Range<usize> {
        let start = found.as_ptr() as usize - text.as_ptr() as usize;
        start..start + found.len()
    }

    /// A text's tokens, as a history with the key `rules` has them (as
    /// ranges): its runs of word characters (identifiers in code, and parts of
    /// compounds in strings), and with V2's, the compounds whose keys its
    /// strings (between `"`s) have (see `timeline_tokens::compound_keys`).
    fn model_tokens(text: &str, rules: KeyRules) -> Vec<(usize, usize)> {
        let mut tokens = vec![];
        let mut start = None;
        for (i, c) in text
            .char_indices()
            .chain(std::iter::once((text.len(), ' ')))
        {
            match (is_word_char(c), start) {
                (true, None) => start = Some(i),
                (false, Some(s)) => {
                    tokens.push((s, i));
                    start = None;
                }
                _ => {}
            }
        }
        if rules < KeyRules::V2 {
            return tokens;
        }
        let quotes: Vec<usize> = text.match_indices('"').map(|(i, _)| i).collect();
        for pair in quotes.chunks(2) {
            let (open, close) = (pair[0] + 1, pair.get(1).copied().unwrap_or(text.len()));
            let string = &text[open..close];
            let mut keys = vec![];
            let line = crate::file_format::history::syntax_files::TokenLine {
                context: "%",
                class: crate::file_format::history::syntax_files::TokenClass::String,
                token: string,
            };
            crate::file_format::history::timeline_tokens::tracked_token_keys(
                rules,
                "js",
                &line,
                None,
                &mut |key| keys.push(key.into_owned()),
            );
            // (The runs of words joined by `-` or `.`, their parts joined by
            // hyphens, and where they are, if the string has their keys.)
            let joined = regex::Regex::new(r"[\w$]+(?:[-.][\w$]+)+").unwrap();
            for run in joined.find_iter(string) {
                let mut candidates = vec![(run.start(), run.end())];
                let mut at = run.start();
                for part in run.as_str().split('.') {
                    candidates.push((at, at + part.len()));
                    at += part.len() + 1;
                }
                for (s, e) in candidates {
                    if keys.iter().any(|key| key == &string[s..e])
                        && string[s..e].contains(['-', '.'])
                    {
                        tokens.push((open + s, open + e));
                    }
                }
            }
        }
        tokens
    }

    /// Whether `query` holds for a match at `range` of `text`, whose `tokens`
    /// overlapping the match are its tokens (see `model_tokens`).
    fn tokens_hold(
        query: &TokenQuery,
        text: &str,
        tokens: &[(usize, usize)],
        range: &std::ops::Range<usize>,
    ) -> bool {
        let overlapping = tokens
            .iter()
            .filter(|&&(s, e)| s < range.end && range.start < e);
        match query {
            TokenQuery::All => true,
            TokenQuery::Term { term, fold_case } => overlapping.into_iter().any(|&(s, e)| {
                let fold = |t: &str| {
                    if *fold_case {
                        t.to_lowercase()
                    } else {
                        t.to_string()
                    }
                };
                let run = fold(&text[s..e]);
                let piece = fold(term.text());
                match term {
                    // (A whole token is inside the match.)
                    TokenTerm::Whole(_) => range.start <= s && e <= range.end && run == piece,
                    TokenTerm::Prefix(_) => run.starts_with(&piece),
                    TokenTerm::Suffix(_) => run.ends_with(&piece),
                    TokenTerm::Contains(_) => run.contains(&piece),
                }
            }),
            TokenQuery::And(parts) => parts
                .iter()
                .all(|part| tokens_hold(part, text, tokens, range)),
            TokenQuery::Or(parts) => parts
                .iter()
                .any(|part| tokens_hold(part, text, tokens, range)),
        }
    }

    /// Random patterns of `words` (and `quoted`) and `punct` over random texts
    /// of them (and `strings`' words and punctuation, quoted, with spaces
    /// around them, as a quote next to a word starts or ends a string; see
    /// `compound_terms`): every match satisfies the plan's query and tokens,
    /// by `rules`.  Returns how many matches' plans had compounds' terms.
    fn check_plans(
        rules: KeyRules,
        words: &[&str],
        punct: &[&str],
        strings: &[&str],
        quoted: &[&str],
    ) -> usize {
        fn has_compound(query: &TokenQuery) -> bool {
            match query {
                TokenQuery::All => false,
                TokenQuery::Term { term, .. } => term.text().contains(['-', '.']),
                TokenQuery::And(parts) | TokenQuery::Or(parts) => parts.iter().any(has_compound),
            }
        }
        let mut compounds = 0;
        let mut state: u64 = 0x9E3779B97F4A7C15;
        let mut next = move |n: usize| -> usize {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % n as u64) as usize
        };
        let mut checked = 0;
        for round in 0..3000 {
            // (A pattern: a sequence of pieces, some repeated, some
            // alternatives.)
            fn piece(
                next: &mut dyn FnMut(usize) -> usize,
                depth: usize,
                words: &[&str],
                punct: &[&str],
            ) -> String {
                let atom = match next(if depth > 1 { 8 } else { 10 }) {
                    0..=2 => regex::escape(words[next(words.len())]),
                    3 => regex::escape(punct[next(punct.len())].trim_matches('\\')),
                    4 => [r"\s", r"\w", r"\d", ".", r"\b", r"[(=]"][next(6)].to_string(),
                    5 => r"\s*".to_string(),
                    6 | 7 => regex::escape(&format!(
                        "{}{}",
                        words[next(words.len())],
                        punct[next(punct.len())].trim_matches('\\')
                    )),
                    8 => format!(
                        "({}|{})",
                        piece(next, depth + 1, words, punct),
                        piece(next, depth + 1, words, punct)
                    ),
                    _ => format!(
                        "(?:{}{})",
                        piece(next, depth + 1, words, punct),
                        piece(next, depth + 1, words, punct)
                    ),
                };
                match next(8) {
                    0 => format!("(?:{})?", atom),
                    1 => format!("(?:{})*", atom),
                    2 => format!("(?:{})+", atom),
                    _ => atom,
                }
            }
            let pattern_words: Vec<&str> = words.iter().chain(quoted).copied().collect();
            let pattern: String = (0..1 + next(4))
                .map(|_| piece(&mut next, 0, &pattern_words, punct))
                .collect();
            let fold_case = next(2) == 0;
            let Ok(re) = regex::RegexBuilder::new(&pattern)
                .case_insensitive(fold_case)
                .build()
            else {
                continue;
            };
            let plan = plan(&pattern, fold_case).unwrap();
            let tokens = plan.query.tokens(rules);
            // (Only strings have quotes in their texts.)
            let code_punct: Vec<&str> = punct
                .iter()
                .copied()
                .filter(|p| strings.is_empty() || *p != "\"")
                .collect();
            for _ in 0..20 {
                let mut text = String::new();
                for _ in 0..next(12) {
                    match next(if strings.is_empty() { 3 } else { 4 }) {
                        0 => {
                            text.push_str(&code_punct[next(code_punct.len())].replace("\\n", "\n"))
                        }
                        3 => {
                            text.push_str(" \"");
                            for _ in 0..1 + next(6) {
                                text.push_str(match next(2) {
                                    0 => strings[next(strings.len())],
                                    _ => words[next(words.len())],
                                });
                            }
                            text.push_str("\" ");
                        }
                        _ => text.push_str(words[next(words.len())]),
                    }
                }
                let text_tokens = model_tokens(&text, rules);
                for found in re.find_iter(&text) {
                    if found.is_empty() {
                        continue;
                    }
                    checked += 1;
                    compounds += has_compound(&tokens) as usize;
                    assert!(
                        holds(&plan.query, &text, found.range()),
                        "round {}: {:?} (fold {}) matched {:?} in {:?}, against {:?}",
                        round,
                        pattern,
                        fold_case,
                        found.as_str(),
                        text,
                        plan.query
                    );
                    assert!(
                        tokens_hold(&tokens, &text, &text_tokens, &found.range()),
                        "round {}: {:?} (fold {}) matched {:?} in {:?}, against {:?}",
                        round,
                        pattern,
                        fold_case,
                        found.as_str(),
                        text,
                        tokens
                    );
                }
            }
        }
        assert!(checked > 1000, "{}", checked);
        compounds
    }

    #[test]
    fn test_plans_are_sound() {
        check_plans(
            KeyRules::V1,
            &[
                "foo", "Bar", "kMax", "case", "JSOp", "barBaz", "return", "x1", "the", "Foo",
            ],
            &[" ", "=", "(", ")", "::", ".", ";", ", ", "\"", "\n"],
            &[],
            &[],
        );
        // (Compounds, in strings and code.)
        let compounds = check_plans(
            KeyRules::V2,
            &[
                "foo", "Bar", "moz", "icon", "x1", "a", "12", "bug", "1234", "Foo",
            ],
            &[" ", "-", ".", "(", ")", "=", ";", "\"", "\n"],
            &[
                "-",
                ".",
                " ",
                ":",
                "/",
                "://",
                "moz-icon",
                "foo.Bar.moz",
                "icon-x1",
                "bug-1234",
            ],
            &[
                "\"moz-icon",
                "moz-icon\"",
                "icon-x1\"",
                "\"foo.Bar.moz",
                "\"Foo.bar-moz.",
                "\"bug-",
                "-a\"",
            ],
        );
        assert!(compounds > 150, "{}", compounds);
    }

    #[test]
    fn test_conjuncts() {
        assert_eq!(
            livegrep_alternation(&query(r"foo.*bar").conjuncts()[0]).unwrap(),
            ("foo".to_string(), false)
        );
        let q = query(r"(foo|bar)baz");
        let conjuncts = q.conjuncts();
        assert_eq!(conjuncts.len(), 1);
        assert_eq!(
            livegrep_alternation(&conjuncts[0]).unwrap(),
            ("barbaz|foobaz".to_string(), false)
        );
        assert_eq!(
            livegrep_alternation(&plan("Foo.Bar", true).unwrap().query.conjuncts()[0]).unwrap(),
            ("foo".to_string(), true)
        );
        assert!(query(r"\w+").conjuncts().is_empty());
    }
}
