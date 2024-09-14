use std::borrow;
use std::path::Path;

use include_dir::{include_dir, Dir};

use crate::file_format::history::syntax_files_struct::FileStructureRow;

static QUERIES_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/languages/tokenizer_queries");

fn load_language_queries(
    ts_lang: &tree_sitter::Language,
    lang_str: &str,
) -> Result<tree_sitter::Query, String> {
    match QUERIES_DIR.get_file(format!("{}.scm", lang_str)) {
        Some(file) => {
            let maybe_contents = file.contents_utf8().map(|s| borrow::Cow::from(s));
            match maybe_contents {
                Some(contents) => {
                    tree_sitter::Query::new(&ts_lang, &contents).map_err(|ts_err| ts_err.message)
                }
                _ => Err(format!("No queries for lang: {}", lang_str)),
            }
        }
        _ => Err(format!("No queries for lang: {}", lang_str)),
    }
}

pub struct HyperTokenized {
    pub lang: String,
    pub tokenized: Vec<String>,
    pub structure: Vec<FileStructureRow>,
}

/// Process a source file with tree-sitter to derive the structurally-bound
/// syntax tokens and an outline of the structure of the file.
pub fn hypertokenize_source_file(
    filename: &str,
    source_contents: &str,
) -> Result<HyperTokenized, String> {
    let ext = match Path::new(filename).extension() {
        Some(ext) => ext.to_str().unwrap(),
        None => "",
    };

    let mut tokenized = Vec::new();
    let mut structure = Vec::new();

    let mut parser = tree_sitter::Parser::new();
    // ### atom_nodes ###
    //
    // We borrow difftastic's terminology to deal with awkward tree-sitter nodes
    // like tree-sitter-cpp's `string_literal` where we want to use the contents
    // of the node and ignore the fact that it has children because there are
    // only children for the opening and closing `"` characters but no node for
    // the actual contents of the string.
    //
    // See https://github.com/tree-sitter/tree-sitter/issues/1156 for more
    // information on the underlying tree-sitter issue.
    //
    // Specific example details:
    // - `#include "big_header.h"` has 3 children:
    //   - `#include"`: 0 children
    //   - `"big_header.h"`: 2 children, both of which are the quotes?!  This
    //     differs from `<stdlib.h>` which is just a single monolithic string
    //     with no children.
    //   - `\n`: 0 children
    //
    // ### ignore_nodes
    //
    // As noted in https://github.com/tree-sitter/tree-sitter-c/issues/97 the
    // C preprocessor nodes currently are weird and include the trailing
    // newline.  For our purposes, we never actually want to emit a newline
    // token, so it's easy enough for us to just forbid that node.
    let (lang, ts_lang, ts_query_filename, atom_nodes, ignore_nodes) = match ext {
        "c" | "cc" | "cpp" | "cxx" | "h" | "hh" | "hxx" | "hpp" => {
            let ts_lang: tree_sitter::Language = tree_sitter_cpp::LANGUAGE.into();
            let string_literal = ts_lang.id_for_node_kind("string_literal", true);
            let char_literal = ts_lang.id_for_node_kind("char_literal", true);
            let newline = ts_lang.id_for_node_kind("\n", false);
            (
                "cpp",
                ts_lang,
                "cpp",
                vec![string_literal, char_literal],
                vec![newline],
            )
        }
        "js" | "jsm" | "json" | "mjs" | "sjs" | "ts" => (
            "js",
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            "typescript",
            vec![],
            vec![],
        ),
        "jsx" | "tsx" => (
            "js",
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            "typescript",
            vec![],
            vec![],
        ),
        "py" | "build" | "configure" => (
            "py",
            tree_sitter_python::LANGUAGE.into(),
            "python",
            vec![],
            vec![],
        ),
        "rs" => (
            "rust",
            tree_sitter_rust::LANGUAGE.into(),
            "rust",
            vec![],
            vec![],
        ),
        // Explicitly skip things we know are binary; this list copied from "langauages.rs"
        "ogg" | "ttf" | "xpi" | "png" | "bcmap" | "gif" | "ogv" | "jpg" | "jpeg" | "bmp"
        | "icns" | "ico" | "mp4" | "sqlite" | "jar" | "webm" | "webp" | "woff" | "class"
        | "m4s" | "mgif" | "wav" | "opus" | "mp3" | "otf" => {
            return Err("Binary files can't be tokenized".to_string());
        }
        _ => {
            return Ok(HyperTokenized {
                lang: "none".to_string(),
                tokenized: source_contents
                    .split_whitespace()
                    .map(|s| format!("% {}", s))
                    .collect(),
                structure: vec![],
            });
        }
    };
    parser
        .set_language(&ts_lang)
        .expect("Error loading grammar");
    let container_query = load_language_queries(&ts_lang, ts_query_filename)?;

    let name_capture_ix = container_query.capture_index_for_name("name").unwrap();
    let container_capture_ix = container_query.capture_index_for_name("container").unwrap();

    let parse_tree = match parser.parse(source_contents.as_bytes(), None) {
        Some(t) => t,
        _ => {
            return Err("Parse failed!".to_string());
        }
    };

    // The cursor traversal logic here is derived from the tree-sitter-cli
    // parse_file_at_path logic: https://github.com/tree-sitter/tree-sitter/blob/master/cli/src/parse.rs
    //
    // A good resource if you are interested in what this class is doing is to instead look at
    // https://github.com/Wilfred/difftastic/blob/master/src/parse/tree_sitter_parser.rs which
    // I discovered after running into problems with the node modeling of tree-sitter-cpp's
    // `string_literal` node and found https://github.com/tree-sitter/tree-sitter/issues/1156
    // and related issues and discussion.  Note that it is explicitly mapping tree-sitter's
    // pseudo-CST to its own tree rep, whereas we are just linearizing tokens here, but the
    // general desire to have all tokens remains.
    let mut cursor = parse_tree.walk();
    let mut _depth = 0;
    let mut visited_children = false;

    let mut query_cursor = tree_sitter::QueryCursor::new();
    let mut query_matches = query_cursor.matches(
        &container_query,
        parse_tree.root_node(),
        source_contents.as_bytes(),
    );

    let mut next_container_match = query_matches.next();
    let mut next_container_id = usize::MAX;
    if let Some(container_match) = &next_container_match {
        next_container_id = container_match
            .nodes_for_capture_index(container_capture_ix)
            .next()
            .unwrap()
            .id();
    }

    let mut context_stack: Vec<String> = vec![];
    let empty_context = "%".to_string();
    let mut context_pretty = empty_context.clone();
    let mut id_stack: Vec<usize> = vec![];

    loop {
        let node = cursor.node();
        if visited_children {
            if cursor.goto_next_sibling() {
                visited_children = false;
            } else if cursor.goto_parent() {
                visited_children = true;
                _depth -= 1;

                if let Some(container_id) = id_stack.last() {
                    if cursor.node().id() == *container_id {
                        context_stack.pop();
                        context_pretty = if context_stack.is_empty() {
                            empty_context.clone()
                        } else {
                            context_stack.join("::")
                        };
                        id_stack.pop();
                    }
                }
            } else {
                break;
            }
        } else {
            // We are considering this node for the first time and before any of
            // its children.

            // Handle if this is our next container.
            if node.id() == next_container_id {
                let pattern_index = next_container_match.as_ref().unwrap().pattern_index;
                let name_node = next_container_match
                    .as_ref()
                    .unwrap()
                    .nodes_for_capture_index(name_capture_ix)
                    .next()
                    .unwrap();
                let name = name_node.utf8_text(source_contents.as_bytes()).unwrap();
                context_stack.push(name.to_string());
                context_pretty = if context_stack.is_empty() {
                    empty_context.clone()
                } else {
                    context_stack.join("::")
                };
                // We're assuming there's only one `#set!` directive right now and that it's
                // "structure.kind" and that it exists.  We do require it to exist, but...
                // TODO: It likely makes sense to preprocess the query by iterating over
                // its patterns and explicitly mapping based on the key so that we can
                // have the kind already available as a string we can clone.
                let structure_kind = container_query
                    .property_settings(pattern_index)
                    .first()
                    .unwrap()
                    .value
                    .as_ref()
                    .unwrap()
                    .to_string();
                structure.push(FileStructureRow {
                    pretty: context_pretty.clone(),
                    // TODO: This should come from a `#set!` directive too but this nuance
                    // won't matter for a bit, so I'm punting because there's a potential
                    // the SCM queries would need to get a little more complex in order to
                    // differentiate between decl and def and when making the change it
                    // would probably be ideal to add more test coverage.
                    is_def: true,
                    kind: structure_kind.to_string(),
                });
                id_stack.push(next_container_id);

                next_container_match = query_matches.next();
                if let Some(container_match) = &next_container_match {
                    next_container_id = container_match
                        .nodes_for_capture_index(container_capture_ix)
                        .next()
                        .unwrap()
                        .id();
                } else {
                    next_container_id = usize::MAX;
                }
            }
            let node_kind_id = node.kind_id();
            if ignore_nodes.contains(&node_kind_id) {
                // ignore this node!
                visited_children = true;
            } else if !atom_nodes.contains(&node_kind_id) && cursor.goto_first_child() {
                visited_children = false;
                _depth += 1;
            } else {
                let token = node.utf8_text(source_contents.as_bytes()).unwrap().trim();
                // Comments don't get further tokenized and are marked as extra, so for now we
                // only perform additional whitespace tokenization for "extra" nodes.  This
                // may turn out to be wrong.
                if token.is_empty() {
                    // ignore empty tokens!
                } else if node.is_extra() {
                    // TODO: probably better to use the regex crate here to avoid a bunch of empty
                    // matches for consecutive whitespace.
                    for piece in token.split(char::is_whitespace) {
                        if piece.is_empty() {
                            continue;
                        }
                        tokenized.push(format!("{} {}", context_pretty, piece));
                    }
                } else {
                    tokenized.push(format!("{} {}", context_pretty, token));
                }
                visited_children = true;
            }
        }
    }

    return Ok(HyperTokenized {
        lang: lang.to_string(),
        tokenized,
        structure,
    });
}
