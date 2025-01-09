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
    css_analyzer::analyze_css(path, 1, text, &mut callback);
}
