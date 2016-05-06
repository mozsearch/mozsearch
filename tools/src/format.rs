use std::collections::HashMap;

use tokenize;
use languages::FormatAs;

use analysis::{WithLocation, AnalysisSource, Jump};

extern crate rustc_serialize;
use self::rustc_serialize::json;
use self::rustc_serialize::json::Json;

pub fn format_text(jumps: &HashMap<String, Jump>, format: FormatAs,
                   path: &str, input: &String, analysis: &Vec<WithLocation<Vec<AnalysisSource>>>) -> (String, u64, String)
{
    let tokens = match format {
        FormatAs::Binary => panic!("Unexpected binary file"),
        FormatAs::Plain => {
            let lines = input.split('\n');
            let mut tokens = Vec::new();
            let mut start = 0;
            for line in lines {
                if line.len() > 0 {
                    tokens.push(tokenize::Token {
                        start: start,
                        end: start + line.len(),
                        kind: tokenize::TokenKind::PlainText,
                    });
                }
                start += line.len();
                if start == input.len() {
                    break;
                }
                tokens.push(tokenize::Token {
                    start: start,
                    end: start + 1,
                    kind: tokenize::TokenKind::Newline,
                });
                start += 1;
            }
            tokens
        },
        FormatAs::Formatted(spec) => tokenize::tokenize_c_like(&input, spec),
    };

    let mut output = String::new();
    let mut last = 0;

    let mut line_start = 0;
    let mut cur_line = 1;

    let mut cur_datum = 0;

    fn entity_replace(s: String) -> String {
        s.replace("&", "&amp;").replace("<", "&lt;")
    }

    output.push_str(&format!("<code id=\"line-{}\" aria-labelledby=\"{}\">", cur_line, cur_line));

    let mut generated_json = json::Array::new();

    for token in tokens {
        match token.kind {
            tokenize::TokenKind::Newline => {
                output.push_str(&input[last .. token.start]);
                output.push_str("\n</code>");

                cur_line += 1;
                line_start = token.end;
                last = token.end;
                output.push_str(&format!("<code id=\"line-{}\" aria-labelledby=\"{}\">", cur_line, cur_line));

                continue;
            },
            _ => {}
        }

        let column = (token.start - line_start) as u32;

        // This should never happen, but sometimes the analysis
        // has bugs in it. This works around them.
        while cur_datum < analysis.len() &&
            cur_line as u32 > analysis[cur_datum].loc.lineno
        {
            cur_datum += 1
        }

        let datum =
            if cur_datum < analysis.len() &&
            cur_line as u32 == analysis[cur_datum].loc.lineno &&
            column == analysis[cur_datum].loc.col_start
        {
            let r = &analysis[cur_datum].data;
            cur_datum += 1;
            Some(r)
        } else {
            None
        };

        let data = match (&token.kind, datum) {
            (&tokenize::TokenKind::Identifier(None), Some(d)) => {
                let ref id = d[0].sym;

                let mut menu_jumps : HashMap<String, Json> = HashMap::new();
                for item in d {
                    let syms = item.sym.split(',');
                    for sym in syms {
                        match jumps.get(sym) {
                            Some(jump) => {
                                if !(&jump.path == path && jump.lineno == cur_line) {
                                    let key = format!("{}:{}", jump.path, jump.lineno);
                                    let mut obj = json::Object::new();
                                    obj.insert("sym".to_string(), Json::String(sym.to_string()));
                                    obj.insert("pretty".to_string(), Json::String(jump.pretty.clone()));
                                    menu_jumps.insert(key, Json::Object(obj));
                                }
                            },
                            None => {}
                        }
                    }
                }

                let items = d.iter().map(|item| {
                    let mut obj = json::Object::new();
                    obj.insert("pretty".to_string(), Json::String(item.pretty.clone()));
                    obj.insert("sym".to_string(), Json::String(item.sym.clone()));
                    Json::Object(obj)
                }).collect::<Vec<_>>();

                let menu_jumps = menu_jumps.into_iter().map(|(_, v)| v).collect::<Vec<_>>();

                let index = generated_json.len();
                generated_json.push(Json::Array(vec![Json::Array(menu_jumps), Json::Array(items)]));

                format!("data-id=\"{}\" data-i=\"{}\" ", id, index)
            },
            _ => "".to_string()
        };

        let style = match token.kind {
            tokenize::TokenKind::Identifier(None) => {
                match datum {
                    Some(d) => {
                        let styles = d.iter().flat_map(|a| a.syntax.iter().flat_map(|s| match s.as_ref() {
                            "type" => vec!["color: teal;"],
                            "def" | "decl" | "idl" => vec!["font-weight: 600;"],
                            _ => vec![],
                        }));
                        let styles = styles.collect::<Vec<_>>();
                        if styles.len() > 0 {
                            format!("style=\"{}\" ", styles.join(" "))
                        } else {
                            "".to_owned()
                        }
                    },
                    None => "".to_owned()
                }
            },
            tokenize::TokenKind::Identifier(Some(ref style)) => style.clone(),
            tokenize::TokenKind::StringLiteral => "style=\"color: green;\" ".to_owned(),
            tokenize::TokenKind::Comment => "style=\"color: darkred;\" ".to_owned(),
            tokenize::TokenKind::RegularExpressionLiteral => "style=\"color: #6d7b8d;\" ".to_owned(),
            _ => "".to_owned()
        };

        match token.kind {
            tokenize::TokenKind::Punctuation => {
                output.push_str(&input[last .. token.start]);
                output.push_str(&entity_replace(input[token.start .. token.end].to_string()));
                last = token.end;
            },
            _ => {
                if style != "" || data != "" {
                    output.push_str(&input[last .. token.start]);
                    output.push_str(&format!("<span {}{}>", style, data));
                    output.push_str(&entity_replace(input[token.start .. token.end].to_string()));
                    output.push_str("</span>");
                    last = token.end;
                }
            }
        }
    }

    output.push_str(&input[last ..]);
    output.push_str("</code>\n");

    let output = output.replace("\r", "\u{240D}"); // U+240D = CR symbol.
    (output, cur_line, json::encode(&Json::Array(generated_json)).unwrap())
}
