use std::io::{self, Write};
use std::collections::HashMap;
use std::path::Path;

use analysis;
use tokenize;
use languages;
use languages::FormatAs;

use analysis::{WithLocation, AnalysisSource, Jump};
use output::{F, Options, generate_formatted, generate_breadcrumbs, generate_header, generate_footer};

use rustc_serialize::json::{self, Json};
use git2;

use config;

pub fn format_code(jumps: &HashMap<String, Jump>, format: FormatAs,
                   path: &str, input: &String,
                   analysis: &Vec<WithLocation<Vec<AnalysisSource>>>) -> (String, u64, String)
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

fn read_blob_entry(repo: &git2::Repository, entry: &git2::TreeEntry) -> String {
    let blob_obj = entry.to_object(repo).unwrap();
    let blob = blob_obj.as_blob().unwrap();
    let mut content = Vec::new();
    content.extend(blob.content());
    let data = String::from_utf8(content).unwrap();
    data
}

pub fn format_path(cfg: &config::Config,
                   tree_name: &str,
                   rev: &str,
                   path: &str,
                   writer: &mut Write) -> Result<(), &'static str> {
    let format = languages::select_formatting(path);
    match format {
        FormatAs::Binary => {
            write!(writer, "Binary file").unwrap();
            return Ok(());
        },
        _ => {},
    };

    // Get the file data.
    let tree_config = cfg.trees.get(tree_name).unwrap();
    let commit_obj = try!(tree_config.repo.revparse_single(rev).map_err(|_| "Bad revision"));
    let commit = try!(commit_obj.as_commit().ok_or("Bad revision"));
    let commit_tree = try!(commit.tree().map_err(|_| "Bad revision"));
    let entry = try!(commit_tree.get_path(Path::new(path)).map_err(|_| "File not found"));

    match entry.kind() {
        Some(git2::ObjectType::Blob) => {},
        _ => return Err("Invalid path; expected file"),
    }

    if entry.filemode() == 120000 {
        return Err("Path is to a symlink");
    }

    let data = read_blob_entry(&tree_config.repo, &entry);

    // Get the blame.
    let blame_oid = try!(tree_config.blame_map.get(&commit.id()).ok_or("Unable to find blame for revision"));
    let blame_commit = try!(tree_config.blame_repo.find_commit(*blame_oid).map_err(|_| "Blame is not a blob"));
    let blame_tree = try!(blame_commit.tree().map_err(|_| "Bad revision"));
    let blame_entry = try!(blame_tree.get_path(Path::new(path)).map_err(|_| "File not found"));

    let blame = read_blob_entry(&tree_config.blame_repo, &blame_entry);
    let blame_lines = blame.lines().collect::<Vec<_>>();

    let jumps : HashMap<String, analysis::Jump> = HashMap::new();
    let analysis = Vec::new();
    let (output, num_lines, _) = format_code(&jumps, format, path, &data, &analysis);

    let filename = Path::new(path).file_name().unwrap().to_str().unwrap();
    let title = format!("{} - mozsearch", filename);
    let opt = Options {
        title: &title,
        tree_name: tree_name,
        include_date: true,
    };

    try!(generate_header(&opt, writer));

    generate_breadcrumbs(&opt, writer, path).unwrap();

    let f = F::Seq(vec![
        F::S("<table id=\"file\" class=\"file\">"),
        F::Indent(vec![
            F::S("<thead class=\"visually-hidden\">"),
            F::Indent(vec![
                F::S("<th scope=\"col\">Line</th>"),
                F::S("<th scope=\"col\">Code</th>"),
            ]),
            F::S("</thead>"),

            F::S("<tbody>"),
            F::Indent(vec![
                F::S("<tr>"),
                F::Indent(vec![
                    F::S("<td id=\"line-numbers\">"),
                ]),
            ]),
        ]),
    ]);

    generate_formatted(writer, &f, 0).unwrap();

    let mut last_rev = None;
    let mut last_color = false;
    let mut strip_id = 0;
    for i in 0 .. num_lines-1 {
        let lineno = i + 1;

        let blame_line = blame_lines[i as usize];
        let pieces = blame_line.splitn(4, ':').collect::<Vec<_>>();
        let rev = pieces[0];
        let filespec = pieces[1];
        let blame_lineno = pieces[2];
        let filename = if filespec == "%" { &path[..] } else { filespec };

        let color = if last_rev == Some(rev) { last_color } else { !last_color };
        if color != last_color {
            strip_id += 1;
        }
        last_rev = Some(rev);
        last_color = color;
        let class = if color { 1 } else { 2 };
        let link = format!("/mozilla-central/commit/{}/{}#{}", rev, filename, blame_lineno);
        let blame_data = format!(" class=\"blame-strip c{}\" data-rev=\"{}\" data-link=\"{}\" data-strip=\"{}\"",
                                 class, rev, link, strip_id);

        let f = F::Seq(vec![
            F::T(format!("<span id=\"{}\" class=\"line-number\" unselectable=\"on\">{}", lineno, lineno)),
            F::T(format!("<div{}></div>", blame_data)),
            F::S("</span>")
        ]);

        generate_formatted(writer, &f, 0).unwrap();
    }

    let f = F::Seq(vec![
        F::Indent(vec![
            F::Indent(vec![
                F::Indent(vec![
                    F::S("</td>"),
                    F::S("<td class=\"code\">"),
                ]),
            ]),
        ]),
    ]);
    generate_formatted(writer, &f, 0).unwrap();
    
    write!(writer, "<pre>").unwrap();
    write!(writer, "{}", output).unwrap();
    write!(writer, "</pre>").unwrap();

    let f = F::Seq(vec![
        F::Indent(vec![
            F::Indent(vec![
                F::Indent(vec![
                    F::S("</td>"),
                ]),
                F::S("</tr>"),
            ]),
            F::S("</tbody>"),
        ]),
        F::S("</table>"),
    ]);
    generate_formatted(writer, &f, 0).unwrap();

    let analysis_json = "[]";
    write!(writer, "<script>var ANALYSIS_DATA = {};</script>\n", analysis_json).unwrap();

    generate_footer(&opt, writer).unwrap();
    
    Ok(())
}
