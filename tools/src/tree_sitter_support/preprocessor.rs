//! The structure of preprocessor conditionals in C-family source (C, C++,
//! Objective-C, and Objective-C++): which branches of which `#if`s each line
//! is in, as normalized conditions (see `ConditionalStacks`), for the
//! history's structure rows and crossref's hits, so that results can say (and
//! be faceted by) what they're conditional on.  From the text alone, so for
//! every branch, whether or not a build compiles it, and for any revision.

use std::collections::HashMap;
use std::path::Path;

use super::cst_tokenizer::{default_profile_for_path, ends_in_block_comment};

/// Whether a path is C-family source (by its extension), whose conditionals
/// we know.
pub fn is_c_family(path: &str) -> bool {
    default_profile_for_path(Path::new(path))
        .is_some_and(|profile| matches!(profile.lang, "cpp" | "objc" | "objcpp"))
}

/// The stacks of preprocessor conditionals' branches that a file's lines are
/// in, outermost first, ex: `["defined(XP_WIN)", "!defined(DEBUG)"]` for a
/// line in an `#else` of an `#ifdef DEBUG` in an `#if defined(XP_WIN)`.  Not
/// a header's include guard (an `#ifndef FOO_H` first, a `#define FOO_H`
/// next, and an `#endif` ending the file).  Directives' lines are in their
/// conditionals' enclosing branches.
///
/// Branches' conditions are normalized (see `normalize_condition`): `#ifdef
/// X` is `defined(X)`, `#ifndef X` is `!defined(X)`, an `#elif`'s is its own,
/// and an `#else`'s is the negation of its conditional's other branches' (ex:
/// `!defined(DEBUG)`).
#[derive(Debug, Default)]
pub struct ConditionalStacks {
    /// Each line's start offset.
    line_starts: Vec<usize>,
    /// Each line's stack's index in `stacks`.
    line_stacks: Vec<u32>,
    /// The distinct stacks, the first of which is the empty one.
    stacks: Vec<Vec<String>>,
}

/// A directive: its lines (with continuation lines), its word (ex: "ifdef"),
/// and the rest of its text, without comments.
struct Directive {
    first_line: usize,
    last_line: usize,
    word: String,
    rest: String,
}

/// An open conditional: whether it's the include guard, its branches'
/// conditions so far, and its current branch's.
struct Frame {
    guard: bool,
    previous: Vec<String>,
    current: String,
}

impl ConditionalStacks {
    /// The stacks of `text`'s lines.
    pub fn new(text: &str) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        let lines: Vec<&str> = text.split('\n').collect();
        let directives = directives(&lines);
        let guards = guards(&directives, &lines);

        let mut stacks: Vec<Vec<String>> = vec![vec![]];
        let mut stack_ids: HashMap<Vec<String>, u32> = HashMap::new();
        stack_ids.insert(vec![], 0);
        let mut intern = |frames: &[Frame]| -> u32 {
            let stack: Vec<String> = frames
                .iter()
                .filter(|frame| !frame.guard)
                .map(|frame| frame.current.clone())
                .collect();
            *stack_ids.entry(stack.clone()).or_insert_with(|| {
                stacks.push(stack);
                (stacks.len() - 1) as u32
            })
        };

        let mut line_stacks = vec![0; lines.len()];
        let mut frames: Vec<Frame> = vec![];
        let mut current = 0;
        let mut next_line = 0;
        for (i, directive) in directives.iter().enumerate() {
            let is_conditional = matches!(
                directive.word.as_str(),
                "if" | "ifdef" | "ifndef" | "elif" | "elifdef" | "elifndef" | "else" | "endif"
            );
            if !is_conditional {
                continue;
            }
            for stack in &mut line_stacks[next_line..directive.first_line] {
                *stack = current;
            }
            next_line = directive.last_line + 1;
            // (A directive's lines are in its conditional's enclosing branch.)
            let enclosing = match directive.word.as_str() {
                "if" | "ifdef" | "ifndef" => current,
                _ => intern(&frames[..frames.len().saturating_sub(1)]),
            };
            for stack in &mut line_stacks[directive.first_line..next_line] {
                *stack = enclosing;
            }
            match directive.word.as_str() {
                "if" | "ifdef" | "ifndef" => frames.push(Frame {
                    guard: guards[i],
                    previous: vec![],
                    current: branch_condition(&directive.word, &directive.rest),
                }),
                "elif" | "elifdef" | "elifndef" => {
                    if let Some(frame) = frames.last_mut() {
                        let previous = std::mem::take(&mut frame.current);
                        frame.previous.push(previous);
                        frame.current = branch_condition(&directive.word, &directive.rest);
                    }
                }
                "else" => {
                    if let Some(frame) = frames.last_mut() {
                        let previous = std::mem::take(&mut frame.current);
                        frame.previous.push(previous);
                        frame.current = frame
                            .previous
                            .iter()
                            .map(|condition| negate(condition))
                            .collect::<Vec<_>>()
                            .join(" && ");
                    }
                }
                _ => {
                    frames.pop();
                }
            }
            current = intern(&frames);
        }
        for stack in &mut line_stacks[next_line.min(lines.len())..] {
            *stack = current;
        }
        ConditionalStacks {
            line_starts,
            line_stacks,
            stacks,
        }
    }

    /// Whether no line is in a conditional (but an include guard).
    pub fn is_empty(&self) -> bool {
        self.stacks.len() == 1
    }

    /// The stack of a (0-based) line.
    pub fn at_line(&self, line: usize) -> &[String] {
        self.line_stacks
            .get(line)
            .map_or(&[], |id| &self.stacks[*id as usize])
    }

    /// The distinct stacks, the first of which is the empty one.
    pub fn stacks(&self) -> &[Vec<String>] {
        &self.stacks
    }

    /// The index in `stacks` of a (0-based) line's stack.
    pub fn stack_index(&self, line: usize) -> usize {
        self.line_stacks.get(line).map_or(0, |id| *id as usize)
    }

    /// The stack of the line of an offset in the text.
    pub fn at_offset(&self, offset: usize) -> &[String] {
        let line = self.line_starts.partition_point(|start| *start <= offset);
        self.at_line(line.saturating_sub(1))
    }
}

/// The directives of a file's lines: lines starting with `#` (and their
/// continuation lines), outside of block comments.
fn directives(lines: &[&str]) -> Vec<Directive> {
    let mut directives = vec![];
    let mut in_block_comment = false;
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        if in_block_comment || !trimmed.starts_with('#') {
            in_block_comment = ends_in_block_comment(line, in_block_comment);
            i += 1;
            continue;
        }
        let first_line = i;
        let mut text = trimmed[1..].to_string();
        while text.trim_end().ends_with('\\') && i + 1 < lines.len() {
            let trimmed_end = text.trim_end();
            text.truncate(trimmed_end.len() - 1);
            text.push(' ');
            i += 1;
            text.push_str(lines[i]);
        }
        in_block_comment = ends_in_block_comment(&text, false);
        let text = strip_comments(&text);
        let text = text.trim_start();
        let word_end = text
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .unwrap_or(text.len());
        directives.push(Directive {
            first_line,
            last_line: i,
            word: text[..word_end].to_string(),
            rest: text[word_end..].trim().to_string(),
        });
        i += 1;
    }
    directives
}

/// A directive's text without its comments (`//` to the end, and `/* */`,
/// including an unclosed one to the end).
fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        let line_comment = rest.find("//");
        let block_comment = rest.find("/*");
        match (line_comment, block_comment) {
            (Some(line), Some(block)) if line < block => {
                out.push_str(&rest[..line]);
                break;
            }
            (Some(line), None) => {
                out.push_str(&rest[..line]);
                break;
            }
            (_, Some(block)) => {
                out.push_str(&rest[..block]);
                out.push(' ');
                rest = match rest[block + 2..].find("*/") {
                    Some(end) => &rest[block + 2 + end + 2..],
                    None => "",
                };
            }
            (None, None) => {
                out.push_str(rest);
                break;
            }
        }
    }
    out
}

/// Which of `directives` open include guards: an `#ifndef X` (or `#if
/// !defined(X)`) with a `#define X` next and no other branches, in no
/// conditionals but other guards, and either with a valueless `#define` and
/// more than it inside (so not a default, ex: `#ifndef FOO` `#define FOO`
/// `#endif`), or ending the file.  (So also guards after `#include`s, and
/// guards of sections of headers, and of amalgamations' headers.)
fn guards(directives: &[Directive], lines: &[&str]) -> Vec<bool> {
    let is_blank = |line: &&str| {
        let line = line.trim();
        line.is_empty() || line.starts_with("//") || line.starts_with("/*") || line.starts_with('*')
    };
    let mut guards = vec![false; directives.len()];
    let mut parents = vec![None; directives.len()];
    // The open conditionals: their openers' indices, and whether they're
    // candidate guards (with the `#define` and no other branches so far).
    let mut open: Vec<(usize, bool)> = vec![];
    for (i, directive) in directives.iter().enumerate() {
        match directive.word.as_str() {
            "if" | "ifdef" | "ifndef" => {
                let condition = branch_condition(&directive.word, &directive.rest);
                let name = condition
                    .strip_prefix("!defined(")
                    .and_then(|rest| rest.strip_suffix(')'));
                let defines = name.is_some()
                    && directives.get(i + 1).is_some_and(|define| {
                        define.word == "define" && define.rest.split_whitespace().next() == name
                    });
                let in_guards = open.iter().all(|(opener, _)| guards[*opener]);
                parents[i] = open.last().map(|(opener, _)| *opener);
                open.push((i, defines && in_guards));
                // (Tentatively, for its nested conditionals; settled at its
                // `#endif`.)
                guards[i] = defines && in_guards;
            }
            "elif" | "elifdef" | "elifndef" | "else" => {
                if let Some((opener, candidate)) = open.last_mut() {
                    *candidate = false;
                    guards[*opener] = false;
                }
            }
            "endif" => {
                let Some((opener, candidate)) = open.pop() else {
                    continue;
                };
                if !candidate {
                    continue;
                }
                let define = &directives[opener + 1];
                let valueless = define.rest.split_whitespace().nth(1).is_none();
                let more = i > opener + 2
                    || !lines[define.last_line + 1..directive.first_line]
                        .iter()
                        .all(is_blank);
                let ends_file = i == directives.len() - 1
                    && lines[directive.last_line + 1..].iter().all(is_blank);
                guards[opener] = (valueless && more) || (open.is_empty() && ends_file);
            }
            _ => {}
        }
    }
    // (Unclosed conditionals aren't guards, nor are guards in conditionals
    // which turned out not to be.)
    for (opener, _) in open {
        guards[opener] = false;
    }
    for i in 0..directives.len() {
        if let Some(parent) = parents[i] {
            guards[i] &= guards[parent];
        }
    }
    guards
}

/// Whether an `#if` or `#elif` directive's condition is literally false (`0`)
/// or true (`1`), if it is (ex: `#if 0 // Disabled.` is false).
pub(crate) fn literal_condition(directive: &str) -> Option<bool> {
    let text = directive.trim_start().strip_prefix('#')?.trim_start();
    let word_end = text
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .unwrap_or(text.len());
    if !matches!(&text[..word_end], "if" | "elif") {
        return None;
    }
    let rest = text[word_end..].replace("\\\n", " ");
    match normalize_condition(&strip_comments(&rest)).as_str() {
        "0" | "(0)" => Some(false),
        "1" | "(1)" => Some(true),
        _ => None,
    }
}

/// The normalized condition of an `#if`'s, `#ifdef`'s, `#ifndef`'s, or
/// `#elif`'s (or C23's `#elifdef`'s and `#elifndef`'s) branch.
fn branch_condition(word: &str, rest: &str) -> String {
    let name = || rest.split_whitespace().next().unwrap_or("");
    match word {
        "ifdef" | "elifdef" => format!("defined({})", name()),
        "ifndef" | "elifndef" => format!("!defined({})", name()),
        _ => normalize_condition(rest),
    }
}

/// A condition with consistent spacing, so that the same condition is the same
/// string: whitespace runs are single spaces, there are none inside
/// parentheses or after `!`, and `defined X` is `defined(X)`.
pub fn normalize_condition(condition: &str) -> String {
    let words: Vec<&str> = condition.split_whitespace().collect();
    let mut out = words.join(" ");
    for (from, to) in [
        ("( ", "("),
        (" )", ")"),
        ("! ", "!"),
        ("defined (", "defined("),
    ] {
        while out.contains(from) {
            out = out.replace(from, to);
        }
    }
    // `defined X` -> `defined(X)`.
    let mut result = String::with_capacity(out.len());
    let mut rest = out.as_str();
    while let Some(at) = rest.find("defined ") {
        let preceded_by_word = rest[..at]
            .chars()
            .last()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        result.push_str(&rest[..at]);
        let after = &rest[at + "defined ".len()..];
        let name_end = after
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .unwrap_or(after.len());
        if preceded_by_word || name_end == 0 {
            result.push_str("defined ");
            rest = after;
        } else {
            result.push_str(&format!("defined({})", &after[..name_end]));
            rest = &after[name_end..];
        }
    }
    result.push_str(rest);
    result
}

/// A stack's conditions as one condition, ex: `defined(XP_WIN) && (A || B)`
/// for `["defined(XP_WIN)", "A || B"]`.
pub fn joined_condition(stack: &[String]) -> String {
    if stack.len() == 1 {
        return stack[0].clone();
    }
    stack
        .iter()
        .map(|condition| {
            if has_top_level_or(condition) {
                format!("({})", condition)
            } else {
                condition.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" && ")
}

/// Whether a condition has a `||` (or `?:`) outside of parentheses, so that
/// it needs them to be `&&`ed.
fn has_top_level_or(condition: &str) -> bool {
    let mut depth = 0;
    let bytes = condition.as_bytes();
    for (i, byte) in bytes.iter().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b'?' if depth == 0 => return true,
            b'|' if depth == 0 && bytes.get(i + 1) == Some(&b'|') => return true,
            _ => {}
        }
    }
    false
}

/// The negation of a normalized condition, simplified for `defined(...)`s,
/// names, and `0` and `1`.
fn negate(condition: &str) -> String {
    let is_defined = |text: &str| {
        text.strip_prefix("defined(")
            .and_then(|rest| rest.strip_suffix(')'))
            .is_some_and(is_identifier)
    };
    if let Some(positive) = condition.strip_prefix('!')
        && (is_defined(positive) || is_identifier(positive))
    {
        return positive.to_string();
    }
    match condition {
        "0" => "1".to_string(),
        "1" => "0".to_string(),
        _ if is_defined(condition) || is_identifier(condition) => format!("!{}", condition),
        _ => format!("!({})", condition),
    }
}

fn is_identifier(text: &str) -> bool {
    !text.is_empty()
        && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !text.starts_with(|c: char| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stacks(text: &str) -> Vec<String> {
        let stacks = ConditionalStacks::new(text);
        (0..text.split('\n').count())
            .map(|line| stacks.at_line(line).join(" | "))
            .collect()
    }

    #[test]
    fn test_conditional_stacks() {
        let text = "#ifndef FOO_H\n\
                    #define FOO_H\n\
                    a;\n\
                    #ifdef DEBUG\n\
                    b;\n\
                    #  if defined XP_WIN && !defined( FOO ) // comment\n\
                    c;\n\
                    #  elif MOZ_WIDGET_GTK\n\
                    d;\n\
                    #  else\n\
                    e;\n\
                    #  endif\n\
                    #else\n\
                    f;\n\
                    #endif\n\
                    #if 0\n\
                    g;\n\
                    #endif\n\
                    #endif  // FOO_H\n";
        assert_eq!(
            stacks(text),
            vec![
                "",
                "",
                "",
                "",
                "defined(DEBUG)",
                "defined(DEBUG)",
                "defined(DEBUG) | defined(XP_WIN) && !defined(FOO)",
                "defined(DEBUG)",
                "defined(DEBUG) | MOZ_WIDGET_GTK",
                "defined(DEBUG)",
                "defined(DEBUG) | !(defined(XP_WIN) && !defined(FOO)) && !MOZ_WIDGET_GTK",
                "defined(DEBUG)",
                "",
                "!defined(DEBUG)",
                "",
                "",
                "0",
                "",
                "",
                "",
            ]
        );
    }

    #[test]
    fn test_not_include_guards() {
        // (A default.)
        assert_eq!(
            stacks("#ifndef FOO\n#define FOO 1\n#endif\nb;\n"),
            vec!["", "!defined(FOO)", "", "", ""]
        );
        assert_eq!(
            stacks("#ifndef FOO\n#define FOO\n#endif\nb;\n"),
            vec!["", "!defined(FOO)", "", "", ""]
        );
        // (No `#define` of its macro next.)
        assert_eq!(
            stacks("#ifndef FOO\n#define BAR\n#endif\n"),
            vec!["", "!defined(FOO)", "", ""]
        );
        // (Another branch.)
        assert_eq!(
            stacks("#ifndef FOO_H\n#define FOO_H\na;\n#else\nb;\n#endif\n"),
            vec![
                "",
                "!defined(FOO_H)",
                "!defined(FOO_H)",
                "",
                "defined(FOO_H)",
                "",
                ""
            ]
        );
        // Guards after `#include`s, of sections, and in guards are guards.
        let text = "#include \"a.h\"\n#ifndef FOO_H\n#define FOO_H\n#ifndef BAR_H\n#define BAR_H\n\
                    a;\n#endif\n#endif\n#ifndef BAZ_H\n#define BAZ_H\nb;\n#endif\nc;\n";
        assert!(ConditionalStacks::new(text).is_empty());
        // `#if !defined(...)` guards are guards.
        assert!(
            ConditionalStacks::new("#if !defined(FOO_H)\n#define FOO_H\na;\n#endif\n").is_empty()
        );
    }

    #[test]
    fn test_directives_in_comments_and_continuations() {
        let text = "/*\n#ifdef NOT\n*/\n#if defined(A) && \\\n    defined(B)\nx;\n#endif\n";
        assert_eq!(
            stacks(text),
            vec!["", "", "", "", "", "defined(A) && defined(B)", "", ""]
        );
        let stacks = ConditionalStacks::new(text);
        assert_eq!(
            stacks.at_offset(text.find("x;").unwrap()),
            ["defined(A) && defined(B)"]
        );
    }

    #[test]
    fn test_literal_condition() {
        assert_eq!(literal_condition("#if 0"), Some(false));
        assert_eq!(literal_condition("#  if (0) // Disabled."), Some(false));
        assert_eq!(literal_condition("#elif 1"), Some(true));
        assert_eq!(literal_condition("#if 0 && \\\n  defined(A)"), None);
        assert_eq!(literal_condition("#ifdef DEBUG"), None);
        assert_eq!(literal_condition("#if 01"), None);
    }

    #[test]
    fn test_negate() {
        assert_eq!(negate("defined(DEBUG)"), "!defined(DEBUG)");
        assert_eq!(negate("!defined(DEBUG)"), "defined(DEBUG)");
        assert_eq!(negate("FOO"), "!FOO");
        assert_eq!(negate("!FOO"), "FOO");
        assert_eq!(negate("0"), "1");
        assert_eq!(negate("A || B"), "!(A || B)");
        assert_eq!(
            joined_condition(&["defined(A)".to_string(), "B || C".to_string()]),
            "defined(A) && (B || C)"
        );
        assert_eq!(
            joined_condition(&["!(B || C)".to_string(), "D".to_string()]),
            "!(B || C) && D"
        );
        assert_eq!(
            normalize_condition("defined  FOO&&BAR >  1"),
            "defined(FOO)&&BAR > 1"
        );
    }
}
