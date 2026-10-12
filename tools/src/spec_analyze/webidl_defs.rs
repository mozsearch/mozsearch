//! The definitions in a block of WebIDL (ex: a ReSpec spec's `<pre
//! class=idl>`), by tree-sitter-webidl: their types (as webidl2's, which
//! ReSpec uses, ex: "interface mixin", "operation", "field", "enum-value"),
//! names, what they're for (the definition they're members of), and where
//! their names are.

use std::ops::Range;

use tree_sitter::{Node, Parser};

/// A definition in a block of WebIDL.
#[derive(Clone, Debug, PartialEq)]
pub struct IdlDef {
    /// Its type, as webidl2's, ex: "interface", "operation", "field".
    pub idl_type: &'static str,
    /// Its name: an enum value's without its quotes, a constructor's
    /// "constructor", and a special operation without one its special
    /// keyword's (ex: "getter").
    pub name: String,
    /// The name of the definition it's a member of, if it's a member.
    pub parent: Option<String>,
    /// Whether it's a partial definition (not a member of one).
    pub partial: bool,
    /// The bytes of its name (or keyword) in the block.
    pub range: Range<usize>,
}

impl IdlDef {
    /// Its dfn type (ReSpec's `getDfnType`), ex: "method", "dict-member".
    pub fn dfn_type(&self) -> &'static str {
        match self.idl_type {
            "operation" => "method",
            "field" => "dict-member",
            "callback interface" | "interface mixin" => "interface",
            other => other,
        }
    }
}

/// webidl2's type of a definition's or member's node kind, if it's one.
fn idl_type(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "interface" | "partial_interface" => "interface",
        "mixin" | "partial_mixin" => "interface mixin",
        "callback_interface" => "callback interface",
        "dictionary" | "partial_dictionary" => "dictionary",
        "namespace" | "partial_namespace" => "namespace",
        "enum" => "enum",
        "callback" => "callback",
        "typedef" => "typedef",
        "attribute" => "attribute",
        "operation" | "legacycaller" => "operation",
        "constructor" => "constructor",
        "const" => "const",
        "field" => "field",
        _ => return None,
    })
}

/// The definitions in `text`, a block of WebIDL, in order.
pub fn idl_defs(text: &str) -> Vec<IdlDef> {
    let mut parser = Parser::new();
    let language: tree_sitter::Language = tree_sitter_webidl::LANGUAGE.into();
    if parser.set_language(&language).is_err() {
        return vec![];
    }
    let Some(tree) = parser.parse(text, None) else {
        return vec![];
    };
    let mut defs = vec![];
    walk(tree.root_node(), text, None, &mut defs);
    defs
}

/// The references in `text`, a block of WebIDL, to definitions by their
/// names (ex: `GizmoOptions` in `optional GizmoOptions options`, and the
/// interfaces of inheritances and `includes` statements): their bytes and
/// the names, in order.
pub fn idl_references(text: &str) -> Vec<(Range<usize>, String)> {
    let mut parser = Parser::new();
    let language: tree_sitter::Language = tree_sitter_webidl::LANGUAGE.into();
    if parser.set_language(&language).is_err() {
        return vec![];
    }
    let Some(tree) = parser.parse(text, None) else {
        return vec![];
    };
    let mut references = vec![];
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if node.kind() == "identifier"
            && node
                .parent()
                .is_some_and(|p| matches!(p.kind(), "type_base" | "inheritance" | "includes"))
        {
            let range = node.byte_range();
            references.push((range.clone(), unescape(&text[range]).to_string()));
        }
        let mut cursor = node.walk();
        let children: Vec<Node> = node.named_children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
    references
}

/// An identifier without the underscore which escapes it, if it has one (ex:
/// "object" for `_object`).
fn unescape(identifier: &str) -> &str {
    identifier.strip_prefix('_').unwrap_or(identifier)
}

/// The first anonymous child of `node` which is one of `keywords`.
fn keyword<'a>(node: Node<'a>, keywords: &[&str]) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| !child.is_named() && keywords.contains(&child.kind()))
}

fn walk(node: Node, text: &str, parent: Option<&str>, defs: &mut Vec<IdlDef>) {
    let kind = node.kind();
    // (A stringifier without an attribute or operation is an operation.)
    if kind == "stringifier" {
        let mut cursor = node.walk();
        let has_member = node
            .named_children(&mut cursor)
            .any(|child| matches!(child.kind(), "attribute" | "operation"));
        if !has_member && let Some(token) = keyword(node, &["stringifier"]) {
            defs.push(IdlDef {
                idl_type: "operation",
                name: "stringifier".to_string(),
                parent: parent.map(str::to_string),
                partial: false,
                range: token.byte_range(),
            });
            return;
        }
    }
    let mut here = parent.map(str::to_string);
    if let Some(idl_type) = idl_type(kind) {
        let is_member = matches!(
            idl_type,
            "attribute" | "operation" | "constructor" | "const" | "field"
        );
        let name_node = node.child_by_field_name("name").or_else(|| match kind {
            "constructor" => keyword(node, &["constructor"]),
            "operation" => keyword(node, &["getter", "setter", "deleter"]),
            _ => None,
        });
        if let Some(name_node) = name_node {
            let range = name_node.byte_range();
            let name_text = unescape(&text[range.clone()]).to_string();
            defs.push(IdlDef {
                idl_type,
                name: name_text.clone(),
                parent: if is_member {
                    parent.map(str::to_string)
                } else {
                    None
                },
                partial: kind.starts_with("partial_"),
                range,
            });
            if !is_member {
                here = Some(name_text);
            }
            // An enum's values (in its `string_list`).
            if kind == "enum" {
                let mut cursor = node.walk();
                let lists: Vec<Node> = node
                    .named_children(&mut cursor)
                    .filter(|child| child.kind() == "string_list")
                    .collect();
                for list in lists {
                    let mut cursor = list.walk();
                    for child in list.named_children(&mut cursor) {
                        if child.kind() != "string" {
                            continue;
                        }
                        let range = child.byte_range();
                        let quoted = &text[range.clone()];
                        let value = quoted.trim_matches('"');
                        // (The value without its quotes, but for the empty
                        // string's.)
                        let range = if value.is_empty() {
                            range
                        } else {
                            range.start + 1..range.end - 1
                        };
                        defs.push(IdlDef {
                            idl_type: "enum-value",
                            name: value.to_string(),
                            parent: here.clone(),
                            partial: false,
                            range,
                        });
                    }
                }
                return;
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk(child, text, here.as_deref(), defs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_idl_defs() {
        let text = r#"[Exposed=Window]
interface Gadget : EventTarget {
  constructor(DOMString name);
  readonly attribute DOMString name;
  Promise<undefined> poke(optional GadgetOptions options = {});
  getter DOMString (unsigned long index);
  stringifier;
  const unsigned short READY = 1;
};
dictionary GadgetOptions {
  boolean force = false;
};
enum GadgetState { "idle", "" };
partial interface Navigator {
  readonly attribute Gadget gadget;
};
interface mixin Pokeable {};
callback GadgetCallback = undefined (Gadget gadget);
"#;
        let defs: Vec<(String, String, Option<String>, bool, String)> = idl_defs(text)
            .into_iter()
            .map(|d| {
                (
                    d.idl_type.to_string(),
                    d.name.clone(),
                    d.parent.clone(),
                    d.partial,
                    text[d.range.clone()].to_string(),
                )
            })
            .collect();
        let expected: Vec<(String, String, Option<String>, bool, String)> = [
            ("interface", "Gadget", None, false, "Gadget"),
            (
                "constructor",
                "constructor",
                Some("Gadget"),
                false,
                "constructor",
            ),
            ("attribute", "name", Some("Gadget"), false, "name"),
            ("operation", "poke", Some("Gadget"), false, "poke"),
            ("operation", "getter", Some("Gadget"), false, "getter"),
            (
                "operation",
                "stringifier",
                Some("Gadget"),
                false,
                "stringifier",
            ),
            ("const", "READY", Some("Gadget"), false, "READY"),
            ("dictionary", "GadgetOptions", None, false, "GadgetOptions"),
            ("field", "force", Some("GadgetOptions"), false, "force"),
            ("enum", "GadgetState", None, false, "GadgetState"),
            ("enum-value", "idle", Some("GadgetState"), false, "idle"),
            ("enum-value", "", Some("GadgetState"), false, "\"\""),
            ("interface", "Navigator", None, true, "Navigator"),
            ("attribute", "gadget", Some("Navigator"), false, "gadget"),
            ("interface mixin", "Pokeable", None, false, "Pokeable"),
            ("callback", "GadgetCallback", None, false, "GadgetCallback"),
        ]
        .into_iter()
        .map(|(k, n, p, partial, t)| {
            (
                k.to_string(),
                n.to_string(),
                p.map(str::to_string),
                partial,
                t.to_string(),
            )
        })
        .collect();
        assert_eq!(defs, expected);
        let references: Vec<String> = idl_references(text)
            .into_iter()
            .map(|(range, name)| format!("{} {}", &text[range], name))
            .collect();
        assert_eq!(
            references,
            [
                "EventTarget EventTarget",
                "GadgetOptions GadgetOptions",
                "Gadget Gadget",
                "Gadget Gadget",
            ]
        );
    }
}
