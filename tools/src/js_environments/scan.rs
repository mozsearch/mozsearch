//! What pages and scripts load, for their JS environments (see `discover`):
//! a page's `<script>`s (with their `src`s, or their inline text's place),
//! `<iframe>`s and `<base>` (with lol_html, whose tokenizer knows which
//! elements' contents are text, ex: an example's `<script>` in an `<xmp>`);
//! and a script's loads of other scripts, by string literals (with
//! tree-sitter): workers, service workers, `importScripts`, the subscript
//! loader's scripts, and xpcshell's `load()`.

use std::cell::RefCell;
use std::rc::Rc;

use lol_html::{HtmlRewriter, Settings, element, text};
use serde::Serialize;
use tree_sitter::{Node, Parser};

/// A page's `<script>`.
#[derive(Clone, Debug, PartialEq)]
pub struct PageScript {
    /// Its `src`, if any.
    pub src: Option<String>,
    /// Its inline text's bytes, if it has no `src`.
    pub inline: Option<std::ops::Range<usize>>,
    /// Whether it's a module script (`type="module"`).
    pub module: bool,
    /// Its start tag's line.
    pub line: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageScan {
    /// Its JS scripts, in order (not those of other types, ex: JSON).
    pub scripts: Vec<PageScript>,
    /// Its event handler attributes' (`on*`'s) values' bytes, in order.
    pub handlers: Vec<std::ops::Range<usize>>,
    /// Its `<iframe>`s' and `<frame>`s' `src`s, with their lines.
    pub frames: Vec<(String, u32)>,
    /// Its `<base>`'s `href`, if any.
    pub base: Option<String>,
}

/// The line (1-based) of a byte offset.
pub fn line_of(source: &str, offset: usize) -> u32 {
    source.as_bytes()[..offset.min(source.len())]
        .iter()
        .filter(|b| **b == b'\n')
        .count() as u32
        + 1
}

/// Whether a `<script>`'s `type` is JS (or it has none).
fn is_js_type(script_type: Option<&str>) -> (bool, bool) {
    let Some(script_type) = script_type else {
        return (true, false);
    };
    let script_type = script_type.trim().to_ascii_lowercase();
    match script_type.as_str() {
        ""
        | "text/javascript"
        | "application/javascript"
        | "application/x-javascript"
        | "text/ecmascript"
        | "application/ecmascript"
        | "text/jsx" => (true, false),
        "module" => (true, true),
        // (ex: "text/javascript;version=1.8")
        t if t.starts_with("text/javascript;") || t.starts_with("application/javascript;") => {
            (true, false)
        }
        _ => (false, false),
    }
}

fn is_html_space(b: u8) -> bool {
    matches!(b, b'\t' | b'\n' | b'\x0c' | b'\r' | b' ')
}

/// Whether an attribute's name is an event handler's (`on` and a word of 3
/// letters or more, as the shortest events' names are, ex: `oncut`, `onend`,
/// rather than ex: `one`).
fn is_handler_name(name: &[u8]) -> bool {
    name.len() >= 5
        && name[..2].eq_ignore_ascii_case(b"on")
        && name[2..].iter().all(u8::is_ascii_alphabetic)
}

/// The bytes of a start tag's event handler attributes' values (without
/// their quotes), relative to it, as the HTML tokenizer reads its attributes.
fn handler_values(tag: &[u8]) -> Vec<std::ops::Range<usize>> {
    let mut values = vec![];
    let n = tag.len();
    let mut i = 1;
    while i < n && !is_html_space(tag[i]) && tag[i] != b'/' && tag[i] != b'>' {
        i += 1;
    }
    loop {
        while i < n && (is_html_space(tag[i]) || tag[i] == b'/') {
            i += 1;
        }
        if i >= n || tag[i] == b'>' {
            break;
        }
        let name_start = i;
        // (A name's first character can be `=`.)
        i += 1;
        while i < n && !is_html_space(tag[i]) && !matches!(tag[i], b'/' | b'>' | b'=') {
            i += 1;
        }
        let name = &tag[name_start..i];
        while i < n && is_html_space(tag[i]) {
            i += 1;
        }
        if i >= n || tag[i] != b'=' {
            continue;
        }
        i += 1;
        while i < n && is_html_space(tag[i]) {
            i += 1;
        }
        let value = if i < n && (tag[i] == b'"' || tag[i] == b'\'') {
            let quote = tag[i];
            i += 1;
            let start = i;
            while i < n && tag[i] != quote {
                i += 1;
            }
            let value = start..i;
            i = (i + 1).min(n);
            value
        } else {
            let start = i;
            while i < n && !is_html_space(tag[i]) && tag[i] != b'>' {
                i += 1;
            }
            start..i
        };
        if is_handler_name(name) && !tag[value.clone()].iter().all(|b| is_html_space(*b)) {
            values.push(value);
        }
    }
    values
}

/// A `<script>`'s URL: its `src`, or an SVG one's `href` (or its older
/// `xlink:href`).
fn script_src(el: &lol_html::html_content::Element) -> Option<String> {
    el.get_attribute("src")
        .or_else(|| el.get_attribute("href"))
        .or_else(|| el.get_attribute("xlink:href"))
}

/// A page's scripts, frames and base: an HTML page's, or an SVG document's
/// (whose `<script>`s are SVG's, with `href`s, and HTML's, as prefixed
/// elements of its namespace, ex: `<h:script src="..."/>`; lol_html's
/// tokenizer has their CDATA sections' text as their text).
pub fn scan_page(source: &str) -> PageScan {
    let scan = Rc::new(RefCell::new(PageScan::default()));
    // (Whether the `<script>` whose text is next is a JS one.)
    let in_js = Rc::new(RefCell::new(false));
    let (scripts, texts, frames, bases) = (scan.clone(), scan.clone(), scan.clone(), scan.clone());
    let handlers = scan.clone();
    let (in_js_start, in_js_text) = (in_js.clone(), in_js.clone());
    let prefixed_scripts = scan.clone();
    let settings = Settings::new()
        .append_element_content_handler(element!("*", move |el| {
            // (An XML document's prefixed HTML `<script>`, with a `src`.)
            if el.tag_name().ends_with(":script")
                && let Some(src) = el.get_attribute("src")
            {
                let (js, module) = is_js_type(el.get_attribute("type").as_deref());
                if js {
                    let line = line_of(source, el.source_location().bytes().start);
                    prefixed_scripts.borrow_mut().scripts.push(PageScript {
                        src: Some(src),
                        inline: None,
                        module,
                        line,
                    });
                }
            }
            if el
                .attributes()
                .iter()
                .any(|a| is_handler_name(a.name().as_bytes()))
            {
                let tag = el.source_location().bytes();
                if let Some(text) = source.as_bytes().get(tag.clone()) {
                    handlers.borrow_mut().handlers.extend(
                        handler_values(text)
                            .into_iter()
                            .map(|v| tag.start + v.start..tag.start + v.end),
                    );
                }
            }
            Ok(())
        }))
        .append_element_content_handler(element!("script", move |el| {
            let (js, module) = is_js_type(el.get_attribute("type").as_deref());
            let src = script_src(el);
            *in_js_start.borrow_mut() = js && src.is_none();
            if js {
                let line = line_of(source, el.source_location().bytes().start);
                scripts.borrow_mut().scripts.push(PageScript {
                    src,
                    inline: None,
                    module,
                    line,
                });
            }
            Ok(())
        }))
        .append_element_content_handler(text!("script", move |t| {
            if !*in_js_text.borrow() {
                return Ok(());
            }
            let bytes = t.source_location().bytes();
            if let Some(script) = texts.borrow_mut().scripts.last_mut() {
                script.inline = Some(match &script.inline {
                    Some(range) => range.start..bytes.end,
                    None => bytes,
                });
            }
            Ok(())
        }))
        .append_element_content_handler(element!("iframe[src], frame[src]", move |el| {
            if let Some(src) = el.get_attribute("src") {
                let line = line_of(source, el.source_location().bytes().start);
                frames.borrow_mut().frames.push((src, line));
            }
            Ok(())
        }))
        .append_element_content_handler(element!("base[href]", move |el| {
            let mut scan = bases.borrow_mut();
            if scan.base.is_none() {
                scan.base = el.get_attribute("href");
            }
            Ok(())
        }));
    let mut rewriter = HtmlRewriter::new(settings, |_: &[u8]| {});
    let ok = rewriter.write(source.as_bytes()).is_ok() && rewriter.end().is_ok();
    let mut scan = Rc::try_unwrap(scan)
        .map(RefCell::into_inner)
        .unwrap_or_default();
    if !ok {
        scan.scripts
            .retain(|s| s.src.is_some() || s.inline.is_some());
    }
    scan
}

/// How a page's inline code reads: an HTML page's; an XML document's (XHTML's,
/// XUL's), whose scripts' text has entities and CDATA sections; or an SVG
/// document's, an XML one whose event handlers' event is `evt`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PageSyntax {
    Html,
    Xml,
    Svg,
}

impl PageSyntax {
    /// A page's, by its extension.
    pub fn of(path: &str) -> PageSyntax {
        if path.ends_with(".svg") {
            PageSyntax::Svg
        } else if [".xhtml", ".xht", ".xul", ".xml"]
            .iter()
            .any(|e| path.ends_with(e))
        {
            PageSyntax::Xml
        } else {
            PageSyntax::Html
        }
    }

    /// What an inline script file has before an event handler's value, in
    /// the blanks before it (its tag's and attribute's name): a statement of
    /// an arrow function taking the event (its `this` is the element's, which
    /// no analysis follows anyway).
    pub fn handler_prefix(self) -> &'static str {
        match self {
            PageSyntax::Svg => ";(evt)=>{",
            _ => ";(event)=>{",
        }
    }
}

/// Blanks for `text`: a space for each of its UTF-16 code units (whose
/// columns analyses count), keeping its newlines (but `\r`s, which JS but not
/// searchfox's lines would count).
fn push_blanks(out: &mut String, text: &str) {
    for c in text.chars() {
        if c == '\n' {
            out.push('\n');
        } else {
            for _ in 0..c.len_utf16() {
                out.push(' ');
            }
        }
    }
}

/// The character an entity at `text`'s start is, if it's a simple one (or a
/// numeric reference to an ASCII character), with its length.
fn entity(text: &str) -> Option<(char, usize)> {
    if !text.starts_with('&') {
        return None;
    }
    let end = text.bytes().take(10).position(|b| b == b';')?;
    let name = &text[1..end];
    let c = match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        _ => {
            let n = if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                u32::from_str_radix(hex, 16).ok()?
            } else {
                name.strip_prefix('#')?.parse().ok()?
            };
            char::from_u32(n).filter(char::is_ascii)?
        }
    };
    Some((c, end + 1))
}

/// `text` with its entities decoded, each run of them padded with spaces to
/// its length after it (so that `&amp;&amp;` is `&&`), so that what's after
/// it stays in its column.
fn push_decoded(out: &mut String, text: &str) {
    let mut rest = text;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let mut run = 0;
        while let Some((c, len)) = entity(rest) {
            out.push(c);
            run += len - 1;
            rest = &rest[len..];
        }
        if run == 0 {
            out.push('&');
            rest = &rest[1..];
        }
        for _ in 0..run {
            out.push(' ');
        }
    }
    out.push_str(rest);
}

/// An inline script's text as JS: an XML page's (XHTML's, XUL's, SVG's) with
/// its CDATA sections' markers blanked and its entities decoded outside them
/// (as the XML parser does), and either's leading `<!--` and trailing `-->`
/// (which old pages hide their scripts in, and TypeScript doesn't take as
/// comments) blanked.
fn push_script(out: &mut String, text: &str, xml: bool) {
    let start = out.len();
    if xml {
        let mut rest = text;
        while let Some(i) = rest.find("<![CDATA[") {
            push_decoded(out, &rest[..i]);
            push_blanks(out, "<![CDATA[");
            rest = &rest[i + "<![CDATA[".len()..];
            let end = rest.find("]]>").unwrap_or(rest.len());
            out.push_str(&rest[..end]);
            if end < rest.len() {
                push_blanks(out, "]]>");
                rest = &rest[end + "]]>".len()..];
            } else {
                rest = "";
            }
        }
        push_decoded(out, rest);
    } else {
        out.push_str(text);
    }
    let script = &out[start..];
    let trimmed = script.trim_start();
    if trimmed.starts_with("<!--") {
        let at = start + script.len() - trimmed.len();
        out.replace_range(at..at + 4, "    ");
    }
    let end = start + out[start..].trim_end().len();
    if out[start..end].ends_with("-->") {
        out.replace_range(end - 3..end, "   ");
    }
}

/// A page's inline scripts and event handlers as a JS file whose positions
/// are the page's (see `push_blanks`): the rest of the page blanked, each
/// script after a `;` (its start tag's `>`'s place), so that it's a
/// statement of its own, and each handler as a statement of a function (see
/// `PageSyntax::handler_prefix`; a handler without room for it, too far left on its line,
/// is left out), with its value's entities decoded; or None if it has none.
pub fn inline_file(source: &str, scan: &PageScan, syntax: PageSyntax) -> Option<String> {
    let xml = syntax != PageSyntax::Html;
    let prefix = syntax.handler_prefix();
    let mut pieces: Vec<(std::ops::Range<usize>, bool)> = scan
        .scripts
        .iter()
        .filter_map(|s| s.inline.clone())
        .map(|r| (r, false))
        .chain(scan.handlers.iter().map(|r| (r.clone(), true)))
        .filter(|(r, _)| {
            r.end <= source.len()
                && source.is_char_boundary(r.start)
                && source.is_char_boundary(r.end)
        })
        .collect();
    pieces.sort_by_key(|(r, _)| r.start);
    let mut out = String::with_capacity(source.len());
    let mut at = 0;
    let mut code = false;
    for (range, handler) in pieces {
        if range.start < at {
            continue;
        }
        push_blanks(&mut out, &source[at..range.start]);
        at = range.end;
        let text = &source[range];
        if !handler {
            if out.ends_with(' ') {
                out.pop();
                out.push(';');
            }
            push_script(&mut out, text, xml);
            code |= !text.trim().is_empty();
            continue;
        }
        let room = out.len() - out.trim_end_matches(' ').len();
        if room < prefix.len() {
            push_blanks(&mut out, text);
            continue;
        }
        out.truncate(out.len() - prefix.len());
        out.push_str(prefix);
        push_decoded(&mut out, text);
        code = true;
        // (Its end, in its quote's place, or that of what ends it unquoted;
        // or before the newline ending it.)
        out.push('}');
        if let Some(c) = source[at..].chars().next().filter(|c| *c != '\n') {
            for _ in 1..c.len_utf16() {
                out.push(' ');
            }
            at += c.len_utf8();
        }
    }
    push_blanks(&mut out, &source[at..]);
    code.then_some(out)
}

/// How a script loads another.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LoadKind {
    /// `new Worker(url)`: a dedicated worker's script.
    Worker,
    /// `new SharedWorker(url)`.
    SharedWorker,
    /// `new ChromeWorker(url)`.
    ChromeWorker,
    /// `navigator.serviceWorker.register(url)` (and any `*.serviceWorker.register`).
    ServiceWorker,
    /// `importScripts(url, ...)`, in a worker.
    ImportScripts,
    /// `*.loadSubScript(url, ...)` (the subscript loader's).
    SubScript,
    /// `load(path)` (xpcshell's, and the JS shell's), and the JS shell's
    /// `loadRelativeToScript(path)`.
    Load,
    /// A module's `import`s (static, re-exports, and `import()`s).
    Import,
}

/// A script's load of another, by a string literal (or not: `literal` is
/// None, for the inventory of the loads it can't follow).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Load {
    pub kind: LoadKind,
    pub literal: Option<String>,
    pub line: u32,
}

/// A string literal's or a template's (without substitutions) value; or a
/// variable's concatenation with one, as `${variable}value` (ex: jit-tests'
/// `libdir + "asserts.js"`; see `rules::PathFlavor::variables`).
fn literal_value(node: Node, source: &str) -> Option<String> {
    if node.kind() == "binary_expression"
        && node
            .child_by_field_name("operator")
            .is_some_and(|o| &source[o.byte_range()] == "+")
        && let (Some(left), Some(right)) = (
            node.child_by_field_name("left"),
            node.child_by_field_name("right"),
        )
        && left.kind() == "identifier"
    {
        return literal_value(right, source)
            .map(|value| format!("${{{}}}{}", &source[left.byte_range()], value));
    }
    match node.kind() {
        "string" | "template_string" => {
            if node.kind() == "template_string"
                && (0..node.named_child_count()).any(|i| {
                    node.named_child(i as u32)
                        .is_some_and(|c| c.kind() == "template_substitution")
                })
            {
                return None;
            }
            let text = &source[node.byte_range()];
            Some(text[1..text.len().saturating_sub(1)].to_string())
        }
        _ => None,
    }
}

fn callee_name<'a>(node: Node, source: &'a str) -> (&'a str, Option<&'a str>) {
    // (A call's or construction's function: its name, and its object's text
    // if it's a member.)
    match node.kind() {
        "identifier" | "import" => (&source[node.byte_range()], None),
        "member_expression" => {
            let property = node
                .child_by_field_name("property")
                .map_or("", |p| &source[p.byte_range()]);
            let object = node
                .child_by_field_name("object")
                .map(|o| &source[o.byte_range()]);
            (property, object)
        }
        _ => ("", None),
    }
}

/// A script's loads of other scripts (lines offset by `first_line - 1`, for
/// a page's inline script).
pub fn scan_script(source: &str, first_line: u32) -> Vec<Load> {
    let mut parser = Parser::new();
    let language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TSX.into();
    if parser.set_language(&language).is_err() {
        return vec![];
    }
    let Some(tree) = parser.parse(source, None) else {
        return vec![];
    };
    let mut loads = vec![];
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        for i in (0..node.child_count()).rev() {
            if let Some(child) = node.child(i as u32) {
                stack.push(child);
            }
        }
        // (A module's static imports and re-exports.)
        if matches!(node.kind(), "import_statement" | "export_statement")
            && let Some(source_node) = node.child_by_field_name("source")
        {
            loads.push(Load {
                kind: LoadKind::Import,
                literal: literal_value(source_node, source),
                line: node.start_position().row as u32 + first_line,
            });
            continue;
        }
        let (function, arguments) = match node.kind() {
            "new_expression" => (
                node.child_by_field_name("constructor"),
                node.child_by_field_name("arguments"),
            ),
            "call_expression" => (
                node.child_by_field_name("function"),
                node.child_by_field_name("arguments"),
            ),
            _ => continue,
        };
        let (Some(function), Some(arguments)) = (function, arguments) else {
            continue;
        };
        let (name, object) = callee_name(function, source);
        let kind = match (node.kind(), name) {
            ("new_expression", "Worker") => LoadKind::Worker,
            ("new_expression", "SharedWorker") => LoadKind::SharedWorker,
            ("new_expression", "ChromeWorker") => LoadKind::ChromeWorker,
            ("call_expression", "register")
                if object.is_some_and(|o| o.ends_with("serviceWorker")) =>
            {
                LoadKind::ServiceWorker
            }
            ("call_expression", "importScripts") if object.is_none() => LoadKind::ImportScripts,
            ("call_expression", "loadSubScript" | "loadSubScriptWithOptions")
                if object.is_some() =>
            {
                LoadKind::SubScript
            }
            ("call_expression", "load" | "loadRelativeToScript") if object.is_none() => {
                LoadKind::Load
            }
            ("call_expression", "import") if object.is_none() => LoadKind::Import,
            _ => continue,
        };
        let line = node.start_position().row as u32 + first_line;
        let args: Vec<Node> = (0..arguments.named_child_count())
            .filter_map(|i| arguments.named_child(i as u32))
            .filter(|a| a.kind() != "comment")
            .collect();
        // (importScripts' every argument; the others' first.)
        let taken = if kind == LoadKind::ImportScripts {
            args.len()
        } else {
            args.len().min(1)
        };
        if taken == 0 {
            continue;
        }
        for arg in &args[..taken] {
            loads.push(Load {
                kind,
                literal: literal_value(*arg, source),
                line,
            });
        }
    }
    loads
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_page() {
        let page = r#"<html><head><base href="sub/">
<script src="/tests/SimpleTest/SimpleTest.js"></script>
<script type="application/json">{"a": 1}</script>
<script type="module" src="mod.mjs"></script>
</head><body>
<pre><xmp><script src="example.js"></script></xmp></pre>
<iframe src="file_frame.html"></iframe>
<script>
  var x = 1;
</script>
</body></html>"#;
        let scan = scan_page(page);
        assert_eq!(scan.base.as_deref(), Some("sub/"));
        assert_eq!(scan.frames, vec![("file_frame.html".to_string(), 7)]);
        let srcs: Vec<(Option<&str>, bool, u32)> = scan
            .scripts
            .iter()
            .map(|s| (s.src.as_deref(), s.module, s.line))
            .collect();
        assert_eq!(
            srcs,
            vec![
                (Some("/tests/SimpleTest/SimpleTest.js"), false, 2),
                (Some("mod.mjs"), true, 4),
                (None, false, 8)
            ]
        );
        let inline = scan.scripts[2].inline.clone().unwrap();
        assert_eq!(page[inline].trim(), "var x = 1;");
    }

    #[test]
    fn test_handler_values() {
        let tag =
            br#"<button id=b onclick="go(1)" ONMOUSEOVER='x()' onkeyup=y() one="no" onblur="  ">"#;
        let values: Vec<&[u8]> = handler_values(tag).into_iter().map(|r| &tag[r]).collect();
        assert_eq!(values, vec![&b"go(1)"[..], b"x()", b"y()"]);
    }

    #[test]
    fn test_inline_file() {
        let page = "<html><body onload=\"init()\">\n<script>\nvar x = 1;\n</script><script src=\"a.js\"></script>\n<button onclick='go(x, &quot;\u{e9}&quot;)'>Go</button><b oncut=a()>\u{1f600}\n<script><!--\nfunction init() {}\n//--></script><p onclick=\"z()\"></p>\n</body></html>";
        let scan = scan_page(page);
        let file = inline_file(page, &scan, PageSyntax::Html).unwrap();
        // (The same lines, of the same UTF-16 lengths.)
        let lengths = |text: &str| -> Vec<usize> {
            text.split('\n').map(|l| l.encode_utf16().count()).collect()
        };
        assert_eq!(lengths(&file), lengths(page));
        let lines: Vec<&str> = file.split('\n').map(str::trim_end).collect();
        assert_eq!(
            lines,
            vec![
                "         ;(event)=>{init()}",
                "       ;",
                "var x = 1;",
                "",
                "      ;(event)=>{go(x, \"     \u{e9}\"     )}          ;(event)=>{a()}",
                "       ;",
                "function init() {}",
                "//             ;(event)=>{z()}",
                "",
            ]
        );
        // (An XHTML page's CDATA and entities, a run of them decoded together;
        // a page without code.)
        let xhtml =
            "<script>//<![CDATA[\nif (a < b) {}\n//]]></script><script>c &amp;&amp; d</script>";
        let file = inline_file(xhtml, &scan_page(xhtml), PageSyntax::Xml).unwrap();
        assert_eq!(
            file,
            "       ;//         \nif (a < b) {}\n//                   ;c &&         d         "
        );
        assert_eq!(
            inline_file("<p>hi</p>", &scan_page("<p>hi</p>"), PageSyntax::Html),
            None
        );
    }

    #[test]
    fn test_scan_svg() {
        let svg = r#"<?xml version="1.0"?>
<svg xmlns="http://www.w3.org/2000/svg" xmlns:h="http://www.w3.org/1999/xhtml" onload="init()">
<h:script src="/resources/testharness.js"/>
<script href="helper.js"/>
<script type="text/ecmascript"><![CDATA[
  function init() { return 1 < 2; }
]]></script>
</svg>
"#;
        let scan = scan_page(svg);
        let srcs: Vec<Option<&str>> = scan.scripts.iter().map(|s| s.src.as_deref()).collect();
        assert_eq!(
            srcs,
            vec![Some("/resources/testharness.js"), Some("helper.js"), None]
        );
        assert_eq!(
            scan.handlers
                .iter()
                .map(|r| &svg[r.clone()])
                .collect::<Vec<_>>(),
            vec!["init()"]
        );
        // (Its CDATA section's text, whose markers its file blanks.)
        let file = inline_file(svg, &scan, PageSyntax::Svg).unwrap();
        let lines: Vec<&str> = file.split('\n').map(str::trim_end).collect();
        assert_eq!(
            lines[1],
            "                                                                              ;(evt)=>{init()}"
        );
        assert_eq!(lines[4], "                                       ;");
        assert_eq!(lines[5], "  function init() { return 1 < 2; }");
        assert_eq!(lines[6], "");
    }

    #[test]
    fn test_scan_script() {
        let script = r#"
let w = new Worker("worker_a.js");
navigator.serviceWorker.register(`sw.js`, { scope: "./" });
importScripts("a.js", "b.js", dynamic);
Services.scriptloader.loadSubScript("chrome://mochitests/content/browser/x/head2.js", this);
load("head_extra.js");
new SharedWorker(url);
load(libdir + "asserts.js");
obj.load("not-xpcshell.js");
loadRelativeToScript("../../tests/non262/shell.js");
import { a } from "./fixture_FIXTURE.js";
export * from "../reexported.mjs";
const m = await import("./lazy.mjs");
"#;
        let loads: Vec<(LoadKind, Option<String>, u32)> = scan_script(script, 1)
            .into_iter()
            .map(|l| (l.kind, l.literal, l.line))
            .collect();
        let lit = |s: &str| Some(s.to_string());
        assert_eq!(
            loads,
            vec![
                (LoadKind::Worker, lit("worker_a.js"), 2),
                (LoadKind::ServiceWorker, lit("sw.js"), 3),
                (LoadKind::ImportScripts, lit("a.js"), 4),
                (LoadKind::ImportScripts, lit("b.js"), 4),
                (LoadKind::ImportScripts, None, 4),
                (
                    LoadKind::SubScript,
                    lit("chrome://mochitests/content/browser/x/head2.js"),
                    5
                ),
                (LoadKind::Load, lit("head_extra.js"), 6),
                (LoadKind::SharedWorker, None, 7),
                (LoadKind::Load, lit("${libdir}asserts.js"), 8),
                (LoadKind::Load, lit("../../tests/non262/shell.js"), 10),
                (LoadKind::Import, lit("./fixture_FIXTURE.js"), 11),
                (LoadKind::Import, lit("../reexported.mjs"), 12),
                (LoadKind::Import, lit("./lazy.mjs"), 13),
            ]
        );
    }
}
