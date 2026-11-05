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

/// A page's scripts, frames and base.
pub fn scan_page(source: &str) -> PageScan {
    let scan = Rc::new(RefCell::new(PageScan::default()));
    // (Whether the `<script>` whose text is next is a JS one.)
    let in_js = Rc::new(RefCell::new(false));
    let (scripts, texts, frames, bases) = (scan.clone(), scan.clone(), scan.clone(), scan.clone());
    let (in_js_start, in_js_text) = (in_js.clone(), in_js.clone());
    let settings = Settings::new()
        .append_element_content_handler(element!("script", move |el| {
            let (js, module) = is_js_type(el.get_attribute("type").as_deref());
            *in_js_start.borrow_mut() = js && el.get_attribute("src").is_none();
            if js {
                let line = line_of(source, el.source_location().bytes().start);
                scripts.borrow_mut().scripts.push(PageScript {
                    src: el.get_attribute("src"),
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
    /// xpcshell's `load(path)`.
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
            ("call_expression", "load") if object.is_none() => LoadKind::Load,
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
                (LoadKind::Import, lit("./fixture_FIXTURE.js"), 10),
                (LoadKind::Import, lit("../reexported.mjs"), 11),
                (LoadKind::Import, lit("./lazy.mjs"), 12),
            ]
        );
    }
}
