use std::io::Write;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use file_format::analysis;
use blame;
use tokenize;
use languages;
use languages::FormatAs;

use file_format::analysis::{WithLocation, AnalysisSource, Jump};
use output::{self, F, Options, PanelItem, PanelSection};

use rustc_serialize::json::{self, Json};
use git2;
use chrono::naive::datetime::NaiveDateTime;
use chrono::offset::fixed::FixedOffset;
use chrono::datetime::DateTime;

use config;

pub fn format_code(jumps: &HashMap<String, Jump>, format: FormatAs,
                   path: &str, input: &String,
                   analysis: &Vec<WithLocation<Vec<AnalysisSource>>>) -> (Vec<String>, String)
{
    let tokens = match format {
        FormatAs::Binary => panic!("Unexpected binary file"),
        FormatAs::FormatDoc(_) => panic!("Unexpected documentation file"),
        FormatAs::Plain => tokenize::tokenize_plain(&input),
        FormatAs::FormatCLike(spec) => tokenize::tokenize_c_like(&input, spec),
        FormatAs::FormatTagLike(script_spec) => tokenize::tokenize_tag_like(&input, script_spec),
    };

    let mut output_lines = Vec::new();
    let mut output = String::new();
    let mut last = 0;

    fn fixup(s: String) -> String {
        s.replace("\r", "\u{21A9}") // U+21A9 = LEFTWARDS ARROW WITH HOOK.
    }

    let mut line_start = 0;
    let mut cur_line = 1;

    let mut cur_datum = 0;

    fn entity_replace(s: String) -> String {
        s.replace("&", "&amp;").replace("<", "&lt;")
    }

    let mut generated_json = json::Array::new();

    let mut last_pos = 0;

    for token in tokens {
        //let word = &input[token.start .. token.end];
        //println!("TOK {:?} '{}' {}", token, word, last_pos);

        assert!(last_pos <= token.start);
        assert!(token.start <= token.end);
        last_pos = token.end;

        match token.kind {
            tokenize::TokenKind::Newline => {
                output.push_str(&input[last .. token.start]);
                output_lines.push(fixup(output));
                output = String::new();

                cur_line += 1;
                line_start = token.end;
                last = token.end;

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

                let d = d.iter().filter(|item| { !item.no_crossref }).collect::<Vec<_>>();

                let mut menu_jumps : HashMap<String, Json> = HashMap::new();
                for item in d.iter() {
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
                if items.len() > 0 {
                    generated_json.push(Json::Array(vec![Json::Array(menu_jumps), Json::Array(items)]));
                    format!("data-id=\"{}\" data-i=\"{}\" ", id, index)
                } else {
                    format!("data-id=\"{}\" ", id)
                }
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
            tokenize::TokenKind::TagName => "style=\"color: blue;\" ".to_owned(),
            tokenize::TokenKind::TagAttrName => "style=\"color: blue;\" ".to_owned(),
            tokenize::TokenKind::EndTagName => "style=\"color: blue;\" ".to_owned(),
            tokenize::TokenKind::RegularExpressionLiteral => "style=\"color: #6d7b8d;\" ".to_owned(),
            _ => "".to_owned()
        };

        match token.kind {
            tokenize::TokenKind::Punctuation | tokenize::TokenKind::PlainText => {
                output.push_str(&entity_replace(input[last .. token.start].to_string()));
                output.push_str(&entity_replace(input[token.start .. token.end].to_string()));
                last = token.end;
            },
            _ => {
                if style != "" || data != "" {
                    output.push_str(&entity_replace(input[last .. token.start].to_string()));
                    output.push_str(&format!("<span {}{}>", style, data));
                    output.push_str(&entity_replace(input[token.start .. token.end].to_string()));
                    output.push_str("</span>");
                    last = token.end;
                }
            }
        }
    }

    output.push_str(&entity_replace(input[last ..].to_string()));

    if output.len() > 0 {
        output_lines.push(fixup(output));
    }

    (output_lines, json::encode(&Json::Array(generated_json)).unwrap())
}

fn latin1_to_string(bytes: Vec<u8>) -> String {
    bytes.iter().map(|&c| c as char).collect()
}

fn decode_bytes(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes.clone()) {
        Ok(s) => s,
        Err(_) => {
            latin1_to_string(bytes)
        }
    }
}

fn read_blob_entry(repo: &git2::Repository, entry: &git2::TreeEntry) -> String {
    let blob_obj = entry.to_object(repo).unwrap();
    let blob = blob_obj.as_blob().unwrap();
    let mut content = Vec::new();
    content.extend(blob.content());
    decode_bytes(content)
}

pub fn format_file_data(cfg: &config::Config,
                        tree_name: &str,
                        panel: &Vec<PanelSection>,
                        commit: Option<&git2::Commit>,
                        blame_commit: Option<&git2::Commit>,
                        path: &str,
                        data: String,
                        jumps: &HashMap<String, Jump>,
                        analysis: &Vec<WithLocation<Vec<AnalysisSource>>>,
                        writer: &mut Write) -> Result<(), &'static str>  {
    let tree_config = try!(cfg.trees.get(tree_name).ok_or("Invalid tree"));

    let format = languages::select_formatting(path);
    match format {
        FormatAs::Binary => {
            write!(writer, "Binary file").unwrap();
            return Ok(());
        },
        _ => {},
    };

    let (output_lines, analysis_json) = format_code(jumps, format, path, &data, &analysis);

    let mut _blame = String::new();
    let blame_lines = match (&tree_config.git, blame_commit) {
        (&Some(ref git_data), Some(blame_commit)) => {
            let blame_tree = try!(blame_commit.tree().map_err(|_| "Bad revision"));

            match blame_tree.get_path(Path::new(path)) {
                Ok(blame_entry) => {
                    _blame = read_blob_entry(&git_data.blame_repo, &blame_entry);
                    Some(_blame.lines().collect::<Vec<_>>())
                },
                Err(_) => None,
            }
        },
        _ => None,
    };

    let revision_owned = match commit {
        Some(commit) => {
            let rev = commit.id().to_string();
            let (header, _) = try!(blame::commit_header(commit));
            Some((rev, header))
        },
        None => None,
    };
    let revision = match revision_owned {
        Some((ref rev, ref header)) => Some((rev.as_str(), header.as_str())),
        None => None,
    };

    let filename = Path::new(path).file_name().unwrap().to_str().unwrap();
    let title = format!("{} - mozsearch", filename);
    let opt = Options {
        title: &title,
        tree_name: tree_name,
        include_date: true,
        revision: revision,
    };

    try!(output::generate_header(&opt, writer));

    try!(output::generate_breadcrumbs(&opt, writer, path));

    try!(output::generate_panel(writer, panel));

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

    output::generate_formatted(writer, &f, 0).unwrap();

    let mut last_rev = None;
    let mut last_color = false;
    for i in 0 .. output_lines.len() {
        let lineno = i + 1;

        let blame_data = if let Some(ref lines) = blame_lines {
            let blame_line = lines[i as usize];
            let pieces = blame_line.splitn(4, ':').collect::<Vec<_>>();
            let rev = pieces[0];
            let filespec = pieces[1];
            let blame_lineno = pieces[2];

            let color = if last_rev == Some(rev) { last_color } else { !last_color };
            last_rev = Some(rev);
            last_color = color;
            let class = if color { 1 } else { 2 };
            let data = format!(" class=\"blame-strip c{}\" data-blame=\"{}#{}#{}\"",
                               class, rev, filespec, blame_lineno);
            data
        } else {
            "".to_owned()
        };

        let f = F::Seq(vec![
            F::T(format!("<span id=\"l{}\" class=\"line-number\">{}", lineno, lineno)),
            F::T(format!("<div{}></div>", blame_data)),
            F::S("</span>")
        ]);

        output::generate_formatted(writer, &f, 0).unwrap();
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
    output::generate_formatted(writer, &f, 0).unwrap();

    write!(writer, "<pre>").unwrap();
    for (i, line) in output_lines.iter().enumerate() {
        write!(writer, "<code id=\"line-{}\" aria-labelledby=\"{}\">{}\n</code>",
               i + 1, i + 1, line).unwrap();
    }
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
    output::generate_formatted(writer, &f, 0).unwrap();

    write!(writer, "<script>var ANALYSIS_DATA = {};</script>\n", analysis_json).unwrap();

    output::generate_footer(&opt, tree_name, path, writer).unwrap();

    Ok(())
}

pub fn format_path(cfg: &config::Config,
                   tree_name: &str,
                   rev: &str,
                   path: &str,
                   writer: &mut Write) -> Result<(), &'static str> {
    // Get the file data.
    let tree_config = try!(cfg.trees.get(tree_name).ok_or("Invalid tree"));
    let git = try!(config::get_git(tree_config));
    let commit_obj = try!(git.repo.revparse_single(rev).map_err(|_| "Bad revision"));
    let commit = try!(commit_obj.as_commit().ok_or("Bad revision"));
    let commit_tree = try!(commit.tree().map_err(|_| "Bad revision"));
    let entry = try!(commit_tree.get_path(Path::new(path)).map_err(|_| "File not found"));

    // Get blame.
    let blame_oid = try!(git.blame_map.get(&commit.id()).ok_or("Unable to find blame for revision"));
    let blame_commit = try!(git.blame_repo.find_commit(*blame_oid).map_err(|_| "Blame is not a blob"));

    match entry.kind() {
        Some(git2::ObjectType::Blob) => {},
        _ => return Err("Invalid path; expected file"),
    }

    if entry.filemode() == 120000 {
        return Err("Path is to a symlink");
    }

    let data = read_blob_entry(&git.repo, &entry);

    let jumps : HashMap<String, analysis::Jump> = HashMap::new();
    let analysis = Vec::new();

    let panel = vec![PanelSection {
        name: "Revision control".to_owned(),
        items: vec![PanelItem {
            title: "Go to latest version".to_owned(),
            link: format!("/{}/source/{}", tree_name, path),
            update_link_lineno: true,
        }, PanelItem {
            title: "Log".to_owned(),
            link: format!("https://hg.mozilla.org/mozilla-central/log/tip/{}", path),
            update_link_lineno: false,
        }, PanelItem {
            title: "Blame".to_owned(),
            link: "javascript:alert('Hover over the gray bar on the left to see blame information.')".to_owned(),
            update_link_lineno: false,
        }],
    }];

    try!(format_file_data(cfg,
                          tree_name,
                          &panel,
                          Some(&commit),
                          Some(&blame_commit),
                          path,
                          data,
                          &jumps,
                          &analysis,
                          writer));

    Ok(())
}

fn split_lines(s: &str) -> Vec<&str> {
    let mut split = s.split('\n').collect::<Vec<_>>();
    if split[split.len() - 1].len() == 0 {
        split.pop();
    }
    split
}

pub fn format_diff(cfg: &config::Config,
                   tree_name: &str,
                   rev: &str,
                   path: &str,
                   writer: &mut Write) -> Result<(), &'static str> {
    let tree_config = try!(cfg.trees.get(tree_name).ok_or("Invalid tree"));

    let git_path = try!(config::get_git_path(tree_config));
    let output = try!(Command::new("/usr/bin/git")
                      .arg("diff-tree")
                      .arg("-p")
                      .arg("--cc")
                      .arg("--patience")
                      .arg("--full-index")
                      .arg("--no-prefix")
                      .arg("-U100000")
                      .arg(rev)
                      .arg("--")
                      .arg(path)
                      .current_dir(&git_path)
                      .output().map_err(|_| "Diff failed 1"));
    if !output.status.success() {
        println!("ERR\n{}", decode_bytes(output.stderr));
        return Err("Diff failed 2");
    }
    let difftxt = decode_bytes(output.stdout);

    if difftxt.len() == 0 {
        return format_path(cfg, tree_name, rev, path, writer);
    }

    let git = try!(config::get_git(tree_config));
    let commit_obj = try!(git.repo.revparse_single(rev).map_err(|_| "Bad revision"));
    let commit = try!(commit_obj.as_commit().ok_or("Bad revision"));

    let mut blames = Vec::new();

    for parent_oid in commit.parent_ids() {
        let blame_oid = try!(git.blame_map.get(&parent_oid).ok_or("Unable to find blame"));
        let blame_commit = try!(git.blame_repo.find_commit(*blame_oid).map_err(|_| "Blame is not a blob"));
        let blame_tree = try!(blame_commit.tree().map_err(|_| "Bad revision"));
        match blame_tree.get_path(Path::new(path)) {
            Ok(blame_entry) => {
                let blame = read_blob_entry(&git.blame_repo, &blame_entry);
                let blame_lines = blame.lines().map(|s| s.to_owned()).collect::<Vec<_>>();
                blames.push(Some(blame_lines));
            },
            Err(_) => {
                blames.push(None);
            }
        }
    }

    let mut new_lineno = 1;
    let mut old_lineno = commit.parent_ids().map(|_| 1).collect::<Vec<_>>();

    let mut lines = split_lines(&difftxt);
    for i in 0..lines.len() {
        if lines[i].starts_with('@') && i + 1 < lines.len() {
            lines = lines.split_off(i + 1);
            break;
        }
    }

    let mut new_lines = String::new();

    let mut output = Vec::new();
    for line in lines {
        if line.len() == 0 || line.starts_with('\\') {
            continue;
        }

        let num_parents = commit.parents().count();
        let (origin, content) = line.split_at(num_parents);
        let origin = origin.chars().collect::<Vec<_>>();
        let mut cur_blame = None;
        for i in 0..num_parents {
            let has_minus = origin.contains(&'-');
            if (has_minus && origin[i] == '-') ||
                (!has_minus && origin[i] != '+')
            {
                cur_blame = match blames[i] {
                    Some(ref lines) => Some(&lines[old_lineno[i] - 1]),
                    None => panic!("expected blame for '-' line, none found"),
                };
                old_lineno[i] += 1;
            }
        }

        let mut lno = -1;
        if !origin.contains(&'-') {
            new_lines.push_str(content);
            new_lines.push('\n');

            lno = new_lineno;
            new_lineno += 1;
        }

        output.push((lno, cur_blame, origin, content));
    }

    let format = languages::select_formatting(path);
    match format {
        FormatAs::Binary => {
            return Err("Cannot diff binary file");
        },
        _ => {},
    };
    let jumps : HashMap<String, analysis::Jump> = HashMap::new();
    let analysis = Vec::new();
    let (formatted_lines, _) = format_code(&jumps, format, path, &new_lines, &analysis);

    let (header, _) = try!(blame::commit_header(&commit));

    let filename = Path::new(path).file_name().unwrap().to_str().unwrap();
    let title = format!("{} - mozsearch", filename);
    let opt = Options {
        title: &title,
        tree_name: tree_name,
        include_date: true,
        revision: Some((rev, &header)),
    };

    try!(output::generate_header(&opt, writer));

    let sections = vec![PanelSection {
        name: "Revision control".to_owned(),
        items: vec![PanelItem {
            title: "Show changeset".to_owned(),
            link: format!("/{}/commit/{}", tree_name, rev),
            update_link_lineno: false,
        }, PanelItem {
            title: "Show file without diff".to_owned(),
            link: format!("/{}/rev/{}/{}", tree_name, rev, path),
            update_link_lineno: true,
        }, PanelItem {
            title: "Go to latest version".to_owned(),
            link: format!("/{}/source/{}", tree_name, path),
            update_link_lineno: true,
        }, PanelItem {
            title: "Log".to_owned(),
            link: format!("https://hg.mozilla.org/mozilla-central/log/tip/{}", path),
            update_link_lineno: false,
        }],
    }];
    try!(output::generate_panel(writer, &sections));

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

    output::generate_formatted(writer, &f, 0).unwrap();

    let mut last_rev = None;
    let mut last_color = false;
    for &(lineno, blame, ref _origin, _content) in &output {
        let blame_data = match blame {
            Some(blame) => {
                let pieces = blame.splitn(4, ':').collect::<Vec<_>>();
                let rev = pieces[0];
                let filespec = pieces[1];
                let blame_lineno = pieces[2];

                let color = if last_rev == Some(rev) { last_color } else { !last_color };
                last_rev = Some(rev);
                last_color = color;
                let class = if color { 1 } else { 2 };
                format!(" class=\"blame-strip c{}\" data-blame=\"{}#{}#{}\"",
                        class, rev, filespec, blame_lineno)
            },
            None => "".to_owned(),
        };

        let line_str = if lineno > 0 {
            format!("<span id=\"l{}\" class=\"line-number\">{}", lineno, lineno)
        } else {
            "<span class=\"line-number\">&nbsp;".to_owned()
        };
        let f = F::Seq(vec![
            F::T(line_str),
            F::T(format!("<div{}></div>", blame_data)),
            F::S("</span>")
        ]);

        output::generate_formatted(writer, &f, 0).unwrap();
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
    output::generate_formatted(writer, &f, 0).unwrap();

    fn entity_replace(s: String) -> String {
        s.replace("&", "&amp;").replace("<", "&lt;")
    }

    write!(writer, "<pre>").unwrap();
    for &(lineno, _blame, ref origin, content) in &output {
        let content = entity_replace(content.to_owned());
        let content = if lineno > 0 && (lineno as usize) < formatted_lines.len() + 1 {
            &formatted_lines[(lineno as usize) - 1]
        } else {
            &content
        };

        let origin = origin.iter().cloned().collect::<String>();

        let class = if origin.contains('-') {
            " class=\"minus-line\""
        } else if origin.contains('+') {
            " class=\"plus-line\""
        } else {
            ""
        };

        write!(writer, "<code id=\"line-{}\" aria-labelledby=\"{}\"{}>", lineno, lineno, class).unwrap();
        write!(writer, "{} {}", origin, content).unwrap();
        write!(writer, "\n</code>").unwrap();
    }
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
    output::generate_formatted(writer, &f, 0).unwrap();

    output::generate_footer(&opt, tree_name, path, writer).unwrap();

    Ok(())
}

fn generate_commit_info(tree_name: &str,
                        tree_config: &config::TreeConfig,
                        writer: &mut Write,
                        commit: &git2::Commit)  -> Result<(), &'static str> {
    let (header, remainder) = try!(blame::commit_header(&commit));

    fn format_rev(tree_name: &str, oid: git2::Oid) -> String {
        format!("<a href=\"/{}/commit/{}\">{}</a>", tree_name, oid, oid)
    }

    fn format_sig(sig: git2::Signature) -> String {
        format!("{} &lt;{}>", sig.name().unwrap(), sig.email().unwrap())
    }

    let parents = commit.parent_ids().map(|p| {
        F::T(format!("<tr><td>parent</td><td>{}</td></tr>", format_rev(tree_name, p)))
    }).collect::<Vec<_>>();

    let git = try!(config::get_git(tree_config));
    let hg = match git.hg_map.get(&commit.id()) {
        Some(hg_id) => {
            let hg_link = format!("<a href=\"https://hg.mozilla.org/mozilla-central/rev/{}\">{}</a>", hg_id, hg_id);
            vec![F::T(format!("<tr><td>hg</td><td>{}</td></tr>", hg_link))]
        },

        None => vec![]
    };

    let git = format!("<a href=\"https://github.com/mozilla/gecko-dev/commit/{}\">{}</a>",
                      commit.id(), commit.id());

    let naive_t = NaiveDateTime::from_timestamp(commit.time().seconds(), 0);
    let tz = FixedOffset::east(commit.time().offset_minutes() * 60);
    let t : DateTime<FixedOffset> = DateTime::from_utc(naive_t, tz);
    let t = t.to_rfc2822();

    let f = F::Seq(vec![
        F::T(format!("<h3>{}</h3>", header)),
        F::T(format!("<pre><code>{}</code></pre>", remainder)),

        F::S("<table>"),
        F::Indent(vec![
            F::T(format!("<tr><td>commit</td><td>{}</td></tr>", format_rev(tree_name, commit.id()))),
            F::Seq(parents),
            F::Seq(hg),
            F::T(format!("<tr><td>git</td><td>{}</td></tr>", git)),
            F::T(format!("<tr><td>author</td><td>{}</td></tr>", format_sig(commit.author()))),
            F::T(format!("<tr><td>committer</td><td>{}</td></tr>", format_sig(commit.committer()))),
            F::T(format!("<tr><td>commit time</td><td>{}</td></tr>", t)),
        ]),
        F::S("</table>"),
    ]);

    try!(output::generate_formatted(writer, &f, 0));

    let git_path = try!(config::get_git_path(tree_config));
    let output = try!(Command::new("/usr/bin/git")
                      .arg("show")
                      .arg("--cc")
                      .arg("--pretty=format:")
                      .arg("--raw")
                      .arg(format!("{}", commit.id()))
                      .current_dir(&git_path)
                      .output()
                      .map_err(|_| "Diff failed 1"));
    if !output.status.success() {
        println!("ERR\n{}", decode_bytes(output.stderr));
        return Err("Diff failed 2");
    }
    let difftxt = decode_bytes(output.stdout);

    let lines = split_lines(&difftxt);
    let mut changes = Vec::new();
    for line in lines {
        if line.len() == 0 {
            continue;
        }

        let suffix = &line[commit.parents().count() ..];
        let prefix_size = 2 * (commit.parents().count() + 1);
        let mut data = suffix.splitn(prefix_size + 1, ' ');
        let data = try!(data.nth(prefix_size).ok_or("Invalid diff output 3"));
        let file_info = data.split('\t').take(2).collect::<Vec<_>>();

        let f = F::T(format!("<li>{} <a href=\"/{}/diff/{}/{}\">{}</a>",
                             file_info[0], tree_name, commit.id(), file_info[1], file_info[1]));
        changes.push(f);
    }

    let f = F::Seq(vec![
        F::S("<ul>"),
        F::Indent(changes),
        F::S("</ul>"),
    ]);
    try!(output::generate_formatted(writer, &f, 0));

    Ok(())
}

pub fn format_commit(cfg: &config::Config,
                     tree_name: &str,
                     rev: &str,
                     writer: &mut Write) -> Result<(), &'static str> {
    let tree_config = try!(cfg.trees.get(tree_name).ok_or("Invalid tree"));

    let git = try!(config::get_git(tree_config));
    let commit_obj = try!(git.repo.revparse_single(rev).map_err(|_| "Bad revision"));
    let commit = try!(commit_obj.as_commit().ok_or("Bad revision"));

    let title = format!("{} - mozsearch", rev);
    let opt = Options {
        title: &title,
        tree_name: tree_name,
        include_date: true,
        revision: None,
    };

    try!(output::generate_header(&opt, writer));

    try!(generate_commit_info(tree_name, &tree_config, writer, commit));

    output::generate_footer(&opt, tree_name, "", writer).unwrap();

    Ok(())
}
