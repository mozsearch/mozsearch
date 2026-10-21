//! Markup's tokens: HTML (and XHTML, and XUL, whose `.xul` files were renamed
//! to `.xhtml` in 2019) and XML (SVG, XBL, XSLT, RDF), with its code as code:
//! - `<script>` elements' contents, and event handler attributes (`on*`), as
//!   JS;
//! - XBL bindings' implementations (methods' bodies, properties' getters and
//!   setters, fields, constructors, destructors, and handlers, with their
//!   `action`s) as JS, so that their code is followed into the JS classes of
//!   the Custom Elements which replaced them (2017-2019): on the conversions'
//!   commits, 85% of the bindings' JS was found moved into JS files (see
//!   blame-evolution-notes.md's "Markup tokenizer design");
//! - `<style>` elements' contents and `style` attributes as CSS.
//!
//! The rest is tokenized as spec_tokenizer tokenizes specs' markup: a start
//! tag's `<name`, its attributes' `name=` (or `name`), their values' quotes,
//! and its `>` (or `/>`) are operators, attributes' values are identifier
//! words, end tags are operators, text and comments are prose words (see
//! `prose_words`), and declarations (`<!DOCTYPE ...>`, CDATA's `<![CDATA[`
//! and `]]>`) are operators.  Every token is a verbatim slice of the source,
//! in source order, even where code was decoded for its parse (ex: `&&` from
//! an attribute's `&amp;&amp;`).
//!
//! Contexts (see `ContextMode`, from the `searchfox-contexts` attribute):
//! - By default, and for tests: the ids of the elements a token is in (ex:
//!   `#nav-bar::#back-button`), not their full paths, which wrapping an element
//!   would change; XBL bindings and their members by name (ex:
//!   `arrowscrollbox::scrollByIndex`, like the Custom Elements' classes'
//!   `MozArrowscrollbox::scrollByIndex`); and event handler attributes'
//!   code as their elements' (ex: `#back-button@oncommand`), which
//!   `inference::infer_handler_moves` relies on.  Code's own contexts (ex:
//!   a script's functions, a stylesheet's rules, and tests' names for
//!   `tests`; see typescript-tests.scm) are nested in them.
//! - For `headings` (documentation): the sections of its headings, as
//!   spec_tokenizer's for specs.

use std::collections::HashSet;
use std::ops::Range;

use crate::file_format::history::syntax_files::TokenClass;
use crate::file_format::history::syntax_files_struct::FileStructureRow;
use crate::tree_sitter_support::boilerplate::{
    FinishedTokens, RawToken, finish_tokens, prose_words,
};
use crate::tree_sitter_support::cst_tokenizer::{
    ContextMode, Embedded, TokenizeOptions, tokenize_embedded,
};
use crate::tree_sitter_support::spec_tokenizer::{Context, Item, html_contexts, lex};

/// XBL elements whose contents are JS, inside a binding (`body` only inside a
/// `method`).
const XBL_CODE: &[&str] = &[
    "getter",
    "setter",
    "field",
    "constructor",
    "destructor",
    "handler",
];

/// XBL members, which name contexts by their `name` (or `event`).
const XBL_MEMBERS: &[&str] = &[
    "method",
    "property",
    "field",
    "constructor",
    "destructor",
    "handler",
];

/// HTML's void elements, which have no contents or end tags (unless written
/// as XML, ex: XHTML's `<input/>`, which are self-closing).
const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// `<script type=...>`s whose contents are JS: no type, or a JS one.
fn is_js_type(script_type: Option<&str>) -> bool {
    match script_type.map(|t| t.trim().to_ascii_lowercase()) {
        None => true,
        Some(t) => {
            t.is_empty()
                || t == "module"
                || t.ends_with("javascript")
                || t.ends_with("ecmascript")
                || t.starts_with("application/javascript")
                || t.starts_with("text/javascript")
        }
    }
}

/// Whether an attribute is an event handler (`onclick`, XUL's `oncommand`,
/// XBL properties' `onget`).
fn is_handler(name: &str) -> bool {
    name.len() > 2 && name.starts_with("on") && name[2..].bytes().all(|b| b.is_ascii_alphabetic())
}

/// A name for a context: without whitespace (which contexts can't have; see
/// `syntax_files`), escaping `%`.
fn context_label(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join("%20")
        .replace('%', "%25")
        .replace("%2520", "%20")
}

/// `text` (at `start` in the source) with its character references decoded
/// (`&amp;`, `&lt;`, `&#38;`, ..., not DTD entities like XUL's
/// `&brandShortName;`, which stay), and the source offset of each of its bytes
/// and of its end.
fn decode_refs(text: &str, start: usize) -> (String, Vec<usize>) {
    let mut out = String::with_capacity(text.len());
    let mut map = Vec::with_capacity(text.len() + 1);
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let decoded = rest
            .starts_with('&')
            .then(|| rest.find(';').filter(|&end| end <= 10))
            .flatten()
            .and_then(|end| {
                let name = &rest[1..end];
                let c = match name {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    "nbsp" => Some('\u{a0}'),
                    _ => match name.strip_prefix('#') {
                        Some(hex) if hex.starts_with(['x', 'X']) => {
                            u32::from_str_radix(&hex[1..], 16)
                                .ok()
                                .and_then(char::from_u32)
                        }
                        Some(dec) => dec.parse().ok().and_then(char::from_u32),
                        None => None,
                    },
                };
                c.map(|c| (c, end + 1))
            });
        match decoded {
            Some((c, len)) => {
                for _ in 0..c.len_utf8() {
                    map.push(start + i);
                }
                out.push(c);
                i += len;
            }
            None => {
                let c = rest.chars().next().unwrap();
                for _ in 0..c.len_utf8() {
                    map.push(start + i);
                }
                out.push(c);
                i += c.len_utf8();
            }
        }
    }
    map.push(start + text.len());
    (out, map)
}

/// The sections of documentation's headings (see spec_tokenizer's
/// `html_contexts` and its `Emitter::move_to`), as of a position.
struct Sections {
    contexts: Vec<Context>,
    next: usize,
    sections: Vec<(u8, String)>,
    ranges: Vec<(usize, String)>,
}

impl Sections {
    fn new(mut contexts: Vec<Context>) -> Self {
        contexts.sort_by_key(|c| (c.start, std::cmp::Reverse(c.end)));
        Sections {
            contexts,
            next: 0,
            sections: vec![],
            ranges: vec![],
        }
    }

    /// The contexts at `pos` (positions only increase), and the kinds of the
    /// ones entered.
    fn at(&mut self, pos: usize) -> (Vec<String>, Vec<&'static str>) {
        let mut entered = vec![];
        while self.ranges.last().is_some_and(|(end, _)| *end <= pos) {
            self.ranges.pop();
        }
        while let Some(context) = self.contexts.get(self.next)
            && context.start <= pos
        {
            self.next += 1;
            match context.end {
                Some(end) if end <= pos => continue,
                Some(_) if self.ranges.last().is_some_and(|(_, n)| *n == context.name) => continue,
                Some(end) => self.ranges.push((end, context.name.clone())),
                None => {
                    self.ranges.clear();
                    while self
                        .sections
                        .last()
                        .is_some_and(|(r, _)| *r >= context.rank)
                    {
                        self.sections.pop();
                    }
                    self.sections.push((context.rank, context.name.clone()));
                }
            }
            entered.push(context.kind);
        }
        let names = self
            .sections
            .iter()
            .map(|(_, name)| name.clone())
            .chain(self.ranges.iter().map(|(_, name)| name.clone()))
            .collect();
        (names, entered)
    }
}

/// An open element: its name, and the context it gives its contents, if any.
struct Open {
    name: String,
    label: Option<String>,
}

struct Emitter<'s> {
    src: &'s str,
    mode: ContextMode,
    tokens: Vec<(usize, RawToken<'s>)>,
    structure: Vec<(usize, FileStructureRow)>,
    rows: HashSet<String>,
    /// For `headings`.
    sections: Option<Sections>,
    stack: Vec<Open>,
}

fn pretty(outer: &[String]) -> String {
    if outer.is_empty() {
        "%".to_string()
    } else {
        outer.join("::")
    }
}

impl<'s> Emitter<'s> {
    /// The contexts at `pos`.
    fn outer(&mut self, pos: usize) -> Vec<String> {
        if let Some(sections) = &mut self.sections {
            let (names, entered) = sections.at(pos);
            if let Some(kind) = entered.last() {
                let context = pretty(&names);
                if self.rows.insert(context.clone()) {
                    self.structure.push((
                        pos,
                        FileStructureRow {
                            pretty: context,
                            is_def: true,
                            kind: kind.to_string(),
                            pp: vec![],
                        },
                    ));
                }
            }
            return names;
        }
        self.stack
            .iter()
            .filter_map(|open| open.label.clone())
            .collect()
    }

    fn push(&mut self, class: TokenClass, range: Range<usize>) {
        let text = &self.src[range.clone()];
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        let start = range.start + (trimmed.as_ptr() as usize - text.as_ptr() as usize);
        let context = pretty(&self.outer(start));
        self.tokens.push((
            start,
            RawToken {
                context,
                class,
                text: &self.src[start..start + trimmed.len()],
            },
        ));
    }

    /// Words of `range`: prose's for text and comments, else split on
    /// whitespace.
    fn words(&mut self, class: TokenClass, range: Range<usize>) {
        let src = self.src;
        let text = &src[range.clone()];
        let words: Vec<&'s str> = if matches!(class, TokenClass::Text | TokenClass::Comment) {
            prose_words(text).collect()
        } else {
            text.split_whitespace().collect()
        };
        for word in words {
            let start = word.as_ptr() as usize - src.as_ptr() as usize;
            self.push(class, start..start + word.len());
        }
    }

    /// Code (decoded from `range`, for its parse; see `decode_refs`), in
    /// `outer`'s contexts, or for CSS declarations (a `style` attribute's),
    /// in a rule of its own.
    fn code(
        &mut self,
        embedded: Embedded,
        range: Range<usize>,
        outer: Vec<String>,
        declarations: bool,
    ) {
        let src = self.src;
        let (decoded, map) = decode_refs(&src[range.clone()], range.start);
        let (prefix, suffix) = if declarations { ("a{", "}") } else { ("", "") };
        let text = format!("{}{}{}", prefix, decoded, suffix);
        let to_source = |offset: usize| -> Option<usize> {
            offset
                .checked_sub(prefix.len())
                .filter(|o| *o <= decoded.len())
                .map(|o| map[o])
        };
        let mode = if self.sections.is_some() {
            ContextMode::Default
        } else {
            self.mode
        };
        match tokenize_embedded(embedded, mode, &text, src, &to_source, &outer) {
            Ok((tokens, structure)) => {
                // (A `style` attribute's rule isn't a context, and its `}`
                // maps to where the code ends, which isn't the code's.)
                for (offset, mut token) in tokens {
                    if offset >= range.end {
                        continue;
                    }
                    if declarations {
                        token.context = pretty(&outer);
                    }
                    self.tokens.push((offset, token));
                }
                if !declarations {
                    for (offset, row) in structure {
                        if self.rows.insert(row.pretty.clone()) {
                            self.structure.push((offset, row));
                        }
                    }
                }
            }
            Err(_) => self.words(TokenClass::Text, range),
        }
    }

    /// Code in an element's contents, with any CDATA section's markers as
    /// operators (ex: XHTML's `<script><![CDATA[ ... ]]></script>`).
    fn contents_code(&mut self, embedded: Embedded, range: Range<usize>) {
        let text = &self.src[range.clone()];
        let lead = text.len() - text.trim_start().len();
        let start = range.start + lead;
        let mut code = start..range.end;
        if text[lead..].starts_with("<![CDATA[") {
            self.push(TokenClass::Operator, start..start + 9);
            code.start = start + 9;
            if let Some(end) = self.src[code.clone()].rfind("]]>") {
                let end = code.start + end;
                self.push(TokenClass::Operator, end..end + 3);
                code.end = end;
            }
        }
        let outer = self.outer(code.start);
        self.code(embedded, code, outer, false);
    }

    fn attr_value(
        &self,
        attrs: &[(Range<usize>, Option<Range<usize>>)],
        wanted: &str,
    ) -> Option<&'s str> {
        attrs.iter().find_map(|(name, value)| {
            let name =
                self.src[name.clone()].trim_end_matches(|c: char| c == '=' || c.is_whitespace());
            if name.eq_ignore_ascii_case(wanted) {
                value.as_ref().map(|v| &self.src[v.clone()])
            } else {
                None
            }
        })
    }

    fn in_binding(&self) -> bool {
        self.stack.iter().any(|open| open.name == "binding")
    }

    fn start_tag(
        &mut self,
        range: &Range<usize>,
        name: &str,
        attrs: &[(Range<usize>, Option<Range<usize>>)],
    ) {
        let src = self.src;
        let name_end = src[range.start + 1..range.end]
            .find(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/')
            .map_or(range.end, |i| range.start + 1 + i);
        self.push(TokenClass::Operator, range.start..name_end);
        let in_binding = self.in_binding();
        // (The context this element gives its contents.)
        let label = if self.sections.is_some() {
            None
        } else if name == "binding" {
            self.attr_value(attrs, "id").map(context_label)
        } else if in_binding && XBL_MEMBERS.contains(&name) {
            match name {
                "constructor" | "destructor" => Some(name.to_string()),
                "handler" => self
                    .attr_value(attrs, "event")
                    .map(|e| format!("on{}", context_label(e))),
                _ => self.attr_value(attrs, "name").map(context_label),
            }
        } else {
            self.attr_value(attrs, "id")
                .map(|id| format!("#{}", context_label(id)))
        };
        let mut pos = name_end;
        for (attr, value) in attrs {
            self.words(TokenClass::Operator, attr.clone());
            pos = attr.end;
            let Some(value) = value else {
                continue;
            };
            // (Quotes are operators of their own.)
            self.push(TokenClass::Operator, pos..value.start);
            let attr_name = src[attr.clone()]
                .trim_end_matches(|c: char| c == '=' || c.is_whitespace())
                .to_ascii_lowercase();
            let xbl_action = in_binding && name == "handler" && attr_name == "action";
            if is_handler(&attr_name) || xbl_action {
                // (A handler's code is its element's, ex: `#back-button@oncommand`.)
                let mut outer = self.outer(value.start);
                outer.push(format!(
                    "{}@{}",
                    label.clone().unwrap_or_default(),
                    attr_name
                ));
                self.code(Embedded::Js, value.clone(), outer, false);
            } else if attr_name == "style" {
                let outer = self.outer(value.start);
                self.code(Embedded::Css, value.clone(), outer, true);
            } else {
                self.words(TokenClass::Identifier, value.clone());
            }
            pos = value.end;
            if src[pos..range.end].starts_with(['"', '\'']) {
                self.push(TokenClass::Operator, pos..pos + 1);
                pos += 1;
            }
        }
        // `>` or `/>`.
        self.push(TokenClass::Operator, pos..range.end);
        let self_closing = src[range.clone()].ends_with("/>");
        if !self_closing && !VOID.contains(&name) {
            self.stack.push(Open {
                name: name.to_string(),
                label: label.clone(),
            });
            if label.is_some() {
                let context = pretty(&self.outer(range.end));
                let kind = match name {
                    "binding" => "class",
                    "method" | "handler" | "constructor" | "destructor" if in_binding => "method",
                    "property" | "field" if in_binding => "field",
                    _ => "element",
                };
                if self.rows.insert(context.clone()) {
                    self.structure.push((
                        range.start,
                        FileStructureRow {
                            pretty: context,
                            is_def: true,
                            kind: kind.to_string(),
                            pp: vec![],
                        },
                    ));
                }
            }
        }
    }

    fn end_tag(&mut self, range: &Range<usize>, name: &str) {
        if let Some(at) = self.stack.iter().rposition(|open| open.name == name) {
            self.stack.truncate(at);
        }
        self.push(TokenClass::Operator, range.clone());
    }

    /// An element's text: code, for scripts, styles, and XBL's code, or else
    /// prose.
    fn text(&mut self, range: Range<usize>, script_is_js: bool) {
        let innermost = self.stack.last().map(|open| open.name.as_str());
        let in_binding = self.in_binding();
        let in_method = self.stack.iter().any(|open| open.name == "method");
        match innermost {
            Some("script") if script_is_js => self.contents_code(Embedded::Js, range),
            Some("style") => self.contents_code(Embedded::Css, range),
            Some("body") if in_binding && in_method => self.contents_code(Embedded::Js, range),
            Some(name) if in_binding && XBL_CODE.contains(&name) => {
                self.contents_code(Embedded::Js, range)
            }
            _ => self.words(TokenClass::Text, range),
        }
    }
}

/// Tokenize markup (see the module's documentation).
pub fn tokenize_markup(
    source: &str,
    options: TokenizeOptions,
) -> Result<(FinishedTokens, Vec<FileStructureRow>), String> {
    let items = lex(source, false);
    let sections = (options.contexts == ContextMode::Headings)
        .then(|| Sections::new(html_contexts(source, &items)));
    let mut out = Emitter {
        src: source,
        mode: options.contexts,
        tokens: vec![],
        structure: vec![],
        rows: HashSet::new(),
        sections,
        stack: vec![],
    };
    // (Whether the open `<script>`'s contents are JS.)
    let mut script_is_js = true;
    for item in &items {
        match item {
            Item::Start { range, name, attrs } => {
                if name == "script" {
                    script_is_js = is_js_type(out.attr_value(attrs, "type"));
                }
                out.start_tag(range, name, attrs);
            }
            Item::End { range, name } => out.end_tag(range, name),
            Item::Comment(range) => {
                let inner_start = (range.start + 4).min(range.end);
                let inner_end =
                    if source[range.clone()].ends_with("-->") && range.end - 3 >= inner_start {
                        range.end - 3
                    } else {
                        range.end
                    };
                out.push(TokenClass::Operator, range.start..inner_start);
                out.words(TokenClass::Comment, inner_start..inner_end);
                out.push(TokenClass::Operator, inner_end..range.end);
            }
            Item::Declaration(range) => out.words(TokenClass::Operator, range.clone()),
            Item::Text(range) => out.text(range.clone(), script_is_js),
        }
    }
    // (Code's tokens were added as it was parsed, after their elements'
    // earlier tokens; stable, so tokens at the same offset keep their order.)
    out.tokens.sort_by_key(|(offset, _)| *offset);
    out.structure.sort_by_key(|(offset, _)| *offset);
    Ok((
        finish_tokens(
            source,
            out.tokens.into_iter().map(|(_, token)| token).collect(),
        ),
        out.structure.into_iter().map(|(_, row)| row).collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_format::history::syntax_files::split_token_line;

    /// `(context, class, token)` of each token, checking that every token is
    /// its source's slice.
    fn tokens(source: &str, contexts: ContextMode) -> Vec<(String, char, String)> {
        let (finished, _) = tokenize_markup(source, TokenizeOptions { contexts }).unwrap();
        finished
            .lines
            .iter()
            .zip(&finished.offsets)
            .map(|(line, offset)| {
                let token = split_token_line(line);
                let offset = offset.expect("a token without an offset") as usize;
                assert_eq!(&source[offset..offset + token.token.len()], token.token);
                (
                    token.context.to_string(),
                    token.class.as_char(),
                    token.token.to_string(),
                )
            })
            .collect()
    }

    fn context_of(tokens: &[(String, char, String)], token: &str) -> String {
        tokens
            .iter()
            .find(|(_, _, t)| t == token)
            .unwrap_or_else(|| panic!("no {:?} in {:?}", token, tokens))
            .0
            .clone()
    }

    #[test]
    fn test_xul_and_xbl() {
        let source = concat!(
            "<window id=\"main\">\n",
            "  <toolbarbutton id=\"back-button\" oncommand=\"if (a &amp;&amp; b) goBack(event);\"/>\n",
            "  <script><![CDATA[ function init() { if (a < b) start(); } ]]></script>\n",
            "  <label value=\"&brandShortName;\">Some text.</label>\n",
            "</window>\n",
            "<bindings><binding id=\"scrollbox\"><implementation>\n",
            "  <method name=\"scrollByIndex\"><parameter name=\"index\"/>\n",
            "    <body><![CDATA[ this.scrollBy(index); ]]></body></method>\n",
            "  <property name=\"size\" onget=\"return this._size;\"/>\n",
            "  <field name=\"count\">0</field>\n",
            "</implementation><handlers><handler event=\"click\" action=\"this.go();\"/></handlers></binding></bindings>\n",
        );
        let t = tokens(source, ContextMode::Default);
        // (A handler's code is JS, decoded for its parse, but verbatim.)
        assert_eq!(context_of(&t, "goBack"), "#main::#back-button@oncommand");
        assert!(t.contains(&(
            "#main::#back-button@oncommand".to_string(),
            'o',
            "&amp;&amp;".to_string()
        )));
        // (A script's, in a CDATA section, whose markers are operators.)
        assert_eq!(context_of(&t, "start"), "#main::init");
        assert!(t.iter().any(|(_, c, tok)| *c == 'o' && tok == "<![CDATA["));
        // (DTD entities aren't code, and text is prose.)
        assert!(t.contains(&("#main".to_string(), 'i', "&brandShortName;".to_string())));
        assert!(t.contains(&("#main".to_string(), 't', "text".to_string())));
        // (XBL's members, by name, with their code as JS.)
        assert_eq!(context_of(&t, "scrollBy"), "scrollbox::scrollByIndex");
        assert_eq!(context_of(&t, "_size"), "scrollbox::size@onget");
        assert_eq!(context_of(&t, "go"), "scrollbox::onclick@action");
        let field = t
            .iter()
            .find(|(c, _, tok)| c == "scrollbox::count" && tok == "0")
            .unwrap();
        assert_eq!(field.1, 'n');
    }

    #[test]
    fn test_html_styles_and_scripts() {
        let source = concat!(
            "<!DOCTYPE html><title>A test</title>\n",
            "<style>#box > p:hover { color: #fff; }</style>\n",
            "<input id=\"field\" value=\"x\"><p id=\"para\" style=\"margin: 0\">Hello</p>\n",
            "<script type=\"text/plain\">not code</script>\n",
            "<script>promise_test(async t => { await check(); }, \"Paragraph works\");</script>\n",
            "<h2>Results</h2><p>More</p>\n",
        );
        let t = tokens(source, ContextMode::Default);
        assert_eq!(context_of(&t, "color"), "#box>p:hover");
        // (A `style` attribute's declarations are the element's, and its
        // quotes are only the markup's.)
        assert_eq!(context_of(&t, "margin"), "%");
        assert_eq!(
            t.iter()
                .filter(|(_, c, tok)| *c == 'o' && tok == "\"")
                .count(),
            10
        );
        // (A void element doesn't contain what follows it.)
        assert_eq!(context_of(&t, "Hello"), "#para");
        // (Scripts which aren't JS are text.)
        assert!(t.contains(&("%".to_string(), 't', "code".to_string())));
        assert_eq!(context_of(&t, "check"), "%");
        // (Tests' names for `tests`, and headings' sections for `headings`,
        // with the same tokens.)
        let tests = tokens(source, ContextMode::Tests);
        assert_eq!(context_of(&tests, "check"), "Paragraph%20works");
        let headings = tokens(source, ContextMode::Headings);
        assert_eq!(context_of(&headings, "More"), "Results");
        assert_eq!(context_of(&headings, "Hello"), "%");
        let words = |t: &[(String, char, String)]| -> Vec<String> {
            t.iter()
                .map(|(_, c, tok)| format!("{}{}", c, tok))
                .collect()
        };
        assert_eq!(words(&tests), words(&t));
        assert_eq!(words(&headings), words(&t));
    }

    #[test]
    fn test_decode_refs() {
        let (text, map) = decode_refs("a &amp;&amp; &#x3c;b&#62; &foo;", 10);
        assert_eq!(text, "a && <b> &foo;");
        assert_eq!(map[2], 12);
        assert_eq!(map[3], 17);
        assert_eq!(map[4], 22);
        assert_eq!(*map.last().unwrap(), 10 + 31);
    }
}
