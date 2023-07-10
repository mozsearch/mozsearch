use std::borrow;
use std::path::Path;

use include_dir::{include_dir, Dir};

static QUERIES_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/languages/tokenizer_queries");

fn load_language_queries(ts_lang: tree_sitter::Language, lang_str: &str) -> Result<tree_sitter::Query, String> {
    match QUERIES_DIR.get_file(format!("{}.scm", lang_str)) {
        Some(file) => {
            let maybe_contents = file.contents_utf8().map(|s| borrow::Cow::from(s));
            match maybe_contents {
                Some(contents) => {
                    tree_sitter::Query::new(ts_lang, &contents).map_err(|ts_err| {
                        ts_err.message
                    })
                }
                _ => {
                    Err(format!("No queries for lang: {}", lang_str))
                }
            }
        }
        _ => Err(format!("No queries for lang: {}", lang_str)),
    }
}

pub fn hypertokenize_source_file(filename: &str, source_contents: &str) -> Result<Vec<String>, String> {
    let ext = match Path::new(filename).extension() {
        Some(ext) => ext.to_str().unwrap(),
        None => "",
    };

    let mut tokenized = Vec::new();

    let mut parser = tree_sitter::Parser::new();
    let container_query = match ext {
        "c" | "cc" | "cpp" | "cxx" | "h" | "hh" | "hxx" | "hpp" => {
            parser
                .set_language(tree_sitter_mozcpp::language())
                .expect("Error loading Mozcpp grammar");
            load_language_queries(tree_sitter_mozcpp::language(), "cpp")?
        }
        "js" | "jsm" | "json" | "mjs" | "sjs" | "ts" => {
            parser
                .set_language(tree_sitter_typescript::language_typescript())
                .expect("Error loading Typescript grammar");
            load_language_queries(tree_sitter_typescript::language_typescript(), "typescript")?
        }
        "jsx" | "tsx" => {
            parser
                .set_language(tree_sitter_typescript::language_tsx())
                .expect("Error loading TSX grammar");
            load_language_queries(tree_sitter_typescript::language_tsx(), "typescript")?
        }
        "py" | "build" | "configure" => {
            parser
                .set_language(tree_sitter_python::language())
                .expect("Error loading Python grammar");
            load_language_queries(tree_sitter_python::language(), "python")?
        }
        "rs" => {
            parser
                .set_language(tree_sitter_rust::language())
                .expect("Error loading Rust grammar");
            load_language_queries(tree_sitter_rust::language(), "rust")?
        }
        _ => {
            return Err(format!("Unsupported file format: {}", ext));
        }
    };
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
    let mut cursor = parse_tree.walk();
    let mut _depth = 0;
    let mut visited_children = false;

    let mut query_cursor = tree_sitter::QueryCursor::new();
    let mut query_matches =
        query_cursor.matches(&container_query, parse_tree.root_node(), source_contents.as_bytes());

    let mut next_container_match = query_matches.next();
    let mut next_container_id = usize::MAX;
    if let Some(container_match) = &next_container_match {
        next_container_id = container_match.nodes_for_capture_index(container_capture_ix).next().unwrap().id();
    }

    let mut context_stack: Vec<String> = vec![];
    let mut id_stack: Vec<usize> = vec![];

    // Revised plan:
    // - Similar to our mechanism for nesting ranges in scip-indexer.rs, use tree-sitter queries
    //   in order to define queries that will match against nodes in the AST which should define
    //   a context scope.  Except now we'll store them in separate ".scm" files rather than our
    //   weird manual transformation thing.
    //   - This should provide all the extensibility we need and allow for some interesting
    //     possibilities for things like dealing with switch statements, etc.  Also, it could
    //     allow for identifying the level of strength of the context, etc.
    // - The traversal can potentially be simplified somewhat since the node identifiers should
    //   let us know to pop the context scope when we pop that node by going to the parent.

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
                let name_node = next_container_match.unwrap().nodes_for_capture_index(name_capture_ix).next().unwrap();
                let name = name_node.utf8_text(source_contents.as_bytes()).unwrap();
                context_stack.push(name.to_string());
                id_stack.push(next_container_id);

                next_container_match = query_matches.next();
                if let Some(container_match) = &next_container_match {
                    next_container_id = container_match.nodes_for_capture_index(container_capture_ix).next().unwrap().id();
                } else {
                    next_container_id = usize::MAX;
                }
            }
            if cursor.goto_first_child() {
                visited_children = false;
                _depth += 1;
            } else {
                tokenized.push(format!("{} {}", context_stack.join("::"), node.utf8_text(source_contents.as_bytes()).unwrap()));
                visited_children = true;
            }
        }
    }

    return Ok(tokenized);
}
