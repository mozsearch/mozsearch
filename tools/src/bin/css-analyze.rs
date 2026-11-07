use std::env;
use std::fs;
use tools::css_analyzer;

fn main() {
    let args: Vec<String> = env::args().collect();
    let base_path = args[1].clone();
    let path = args[2].clone();
    let full_path = format!("{base_path}/{path}");
    let text = match fs::read_to_string(full_path.clone()) {
        Ok(text) => text,
        _ => return,
    };
    let mut callback = |s| {
        println!("{}", s);
    };
    // (The tree's tests' root, if any, whose files' custom properties are
    // each document's own; see `css_analyzer::document_namespace`.)
    let tests_root = args.get(3).cloned();
    let namespace =
        css_analyzer::document_namespace(&path, tests_root.as_deref()).map(str::to_string);
    css_analyzer::analyze_css(path, 1, text, namespace.as_deref(), &mut callback);
}
