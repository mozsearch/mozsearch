use std::env;
use std::fs::File;
use std::io::BufReader;
use std::io::Read;
use std::cell::Cell;
use std::collections::HashMap;

/*
 * Different things we handle generically:
 * - Whitespace
 * - Identifiers/reserved words
 * - String literals (either ' or ")
 * - Single-line comments
 * - Multi-line comments
 * - Multi-line string literals (''' in Python, ` in JS)
 * - Regular expression literals
 * - Punctuation
 *
 * Have a separate lexer for XML/HTML:
 * - HTML-style tags with attributes?
 * - CDATA
 * - Embedding JS inside an HTML-like language? Maybe I'll invoke
 *   the tokenizer recursively?
 * 
 * I'll just return the tokens as a list with position information.
 */

#[derive(Debug)]
enum TokenKind {
    Punctuation,
    Identifier(Option<String>),
    StringLiteral,
    Comment,
    RegularExpressionLiteral,
}

#[derive(Debug)]
struct Token {
    start: usize,
    end: usize,
    kind: TokenKind,
}

fn is_ident(ch: char) -> bool {
    (ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') || (ch == '_') || (ch >= '0' && ch <= '9')
}

fn tokenize(fname: &str, reserved_words: &HashMap<String, String>) -> (String, Vec<Token>) {
    let f = File::open(fname).unwrap();
    let mut file = BufReader::new(&f);
    let mut string = String::new();
    let size = file.read_to_string(&mut string).unwrap();
    let mut tokens = Vec::new();

    let chars : Vec<(usize, char)> = string.char_indices().collect();
    let cur_pos = Cell::new(0);

    let get_char = || {
        let p = cur_pos.get();
        cur_pos.set(p + 1);
        chars[p]
    };

    let peek_char = || {
        let (_, ch) = chars[cur_pos.get()];
        ch
    };

    let peek_pos = || {
        let (i, _) = chars[cur_pos.get()];
        i
    };

    while cur_pos.get() < chars.len() {
        let (start, ch) = get_char();
        if is_ident(ch) {
            while is_ident(peek_char()) {
                get_char();
            }

            let word = string[start .. peek_pos()].to_string();
            let class = if reserved_words.contains_key(&word) {
                Some(reserved_words.get(&word).unwrap().clone())
            } else {
                None
            };

            tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::Identifier(class)});
        } else if ch == ' ' || ch == '\t' || ch == '\r' || ch == '\n' {
            // Skip it.
        } else if ch == '/' {
            let ch = peek_char();
            if ch == '*' {
                get_char();
                loop {
                    let (_, next) = get_char();
                    if next == '*' && peek_char() == '/' {
                        break;
                    }
                }
                get_char();
                tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::Comment});
            } else if ch == '/' {
                get_char();
                loop {
                    let (_, next) = get_char();
                    if next == '\n' {
                        break;
                    }
                }
                tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::Comment});
            } else {
                tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::Punctuation});
            }
        } else if ch == '\'' || ch == '"' {
            loop {
                let (_, next) = get_char();
                if next == ch {
                    break;
                } else if next == '\\' {
                    get_char();
                }
            }
            tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::StringLiteral});
        } else {
            tokens.push(Token {start: start, end: peek_pos(), kind: TokenKind::Punctuation});
        }
    }

    (string, tokens)
}

fn main() {
    let args: Vec<_> = env::args().collect();
    let (base_args, fname_args) = args.split_at(6);

    let tree_root = &base_args[1];
    let tree_rev = &base_args[2];
    let index_root = &base_args[3];
    let mozsearch_root = &base_args[4];
    let objdir = &base_args[5];

    let mut reserved_words = HashMap::new();
    reserved_words.insert("var".to_string(), "color: blue;".to_string());
    reserved_words.insert("const".to_string(), "color: blue;".to_string());
    reserved_words.insert("let".to_string(), "color: blue;".to_string());
    reserved_words.insert("function".to_string(), "color: blue;".to_string());
    reserved_words.insert("return".to_string(), "color: blue;".to_string());
    reserved_words.insert("new".to_string(), "color: blue;".to_string());
    reserved_words.insert("null".to_string(), "color: blue;".to_string());
    reserved_words.insert("try".to_string(), "color: orange;".to_string());
    reserved_words.insert("catch".to_string(), "color: orange;".to_string());
    reserved_words.insert("if".to_string(), "color: blue;".to_string());
    reserved_words.insert("else".to_string(), "color: blue;".to_string());
    reserved_words.insert("while".to_string(), "color: blue;".to_string());
    reserved_words.insert("do".to_string(), "color: blue;".to_string());
    reserved_words.insert("for".to_string(), "color: blue;".to_string());
    reserved_words.insert("break".to_string(), "color: blue;".to_string());
    reserved_words.insert("continue".to_string(), "color: blue;".to_string());
    reserved_words.insert("this".to_string(), "color: blue;".to_string());
    reserved_words.insert("super".to_string(), "color: blue;".to_string());
    reserved_words.insert("constructor".to_string(), "color: blue;".to_string());
    reserved_words.insert("get".to_string(), "color: blue;".to_string());
    reserved_words.insert("set".to_string(), "color: blue;".to_string());
    reserved_words.insert("in".to_string(), "color: blue;".to_string());

    //println!("Hello, world! {} {} {} {} {}", tree_root, tree_rev, index_root, mozsearch_root, objdir);

    for fname in fname_args {
        //println!("FILE {}", fname);
        let (input, tokens) = tokenize(fname, &reserved_words);

        let mut output = String::new();
        let mut last = 0;

        fn entity_replace(s: String) -> String {
            s.replace("&", "&amp;").replace("<", "&lt;")
        }

        for token in tokens {
            match token.kind {
                TokenKind::Identifier(Some(class)) => {
                    output.push_str(&input[last .. token.start]);
                    output.push_str("<span style='");
                    output.push_str(&class);
                    output.push_str("'>");
                    output.push_str(&input[token.start .. token.end]);
                    output.push_str("</span>");
                    last = token.end;
                },
                TokenKind::StringLiteral => {
                    output.push_str(&input[last .. token.start]);
                    output.push_str("<span style='color: green;'>");
                    output.push_str(&entity_replace(input[token.start .. token.end].to_string()));
                    output.push_str("</span>");
                    last = token.end;
                },
                TokenKind::Comment => {
                    output.push_str(&input[last .. token.start]);
                    output.push_str("<span style='color: darkred;'>");
                    output.push_str(&entity_replace(input[token.start .. token.end].to_string()));
                    output.push_str("</span>");
                    last = token.end;
                },
                TokenKind::Punctuation => {
                    output.push_str(&input[last .. token.start]);
                    let bytes = input.as_bytes();
                    if bytes[token.start] == b'<' {
                        output.push_str("&lt;");
                    } else if bytes[token.start] == b'&' {
                        output.push_str("&amp;");
                    } else {
                        output.push_str(&input[token.start .. token.end]);
                    }
                    last = token.end;
                },
                _ => {}
            }
        }

        output.push_str(&input[last ..]);
        
        println!("<pre>");
        println!("{}", output);
        println!("</pre>");
    }
}
