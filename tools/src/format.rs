use std::collections::HashMap;
use std::env;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use blame;
use file_format::analysis;
use git_ops;
use languages;
use languages::FormatAs;
use links;
use tokenize;

use config::GitData;
use file_format::analysis::{AnalysisSource, Jump, WithLocation};
use output::{self, Options, PanelItem, PanelSection, F};

use chrono::datetime::DateTime;
use chrono::naive::datetime::NaiveDateTime;
use chrono::offset::fixed::FixedOffset;
use git2;
use rustc_serialize::json::{self, Json};

use config;

pub fn format_code(
    jumps: &HashMap<String, Jump>,
    format: FormatAs,
    path: &str,
    input: &str,
    analysis: &[WithLocation<Vec<AnalysisSource>>],
) -> (Vec<String>, String) {
    let tokens = match format {
        FormatAs::Binary => panic!("Unexpected binary file"),
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
                output.push_str(&input[last..token.start]);
                output_lines.push(fixup(output));
                output = String::new();

                cur_line += 1;
                line_start = token.end;
                last = token.end;

                continue;
            }
            _ => {}
        }

        let column = (token.start - line_start) as u32;

        // Advance cur_datum as long as analysis[cur_datum] is pointing
        // to tokens we've already gone past. This effectively advances
        // cur_datum such that `analysis[cur_datum]` is the analysis data
        // for our current token (if there is any).
        while cur_datum < analysis.len() && cur_line as u32 > analysis[cur_datum].loc.lineno {
            cur_datum += 1
        }
        while cur_datum < analysis.len()
            && cur_line as u32 == analysis[cur_datum].loc.lineno
            && column > analysis[cur_datum].loc.col_start
        {
            cur_datum += 1
        }

        let datum = if cur_datum < analysis.len()
            && cur_line as u32 == analysis[cur_datum].loc.lineno
            && column == analysis[cur_datum].loc.col_start
        {
            let r = &analysis[cur_datum].data;
            cur_datum += 1;
            Some(r)
        } else {
            None
        };

        let data = match (&token.kind, datum) {
            (&tokenize::TokenKind::Identifier(None), Some(d)) => {
                // Build the list of symbols for the highlighter.
                let syms = {
                    let mut syms = String::new();
                    for (i, sym) in d.iter().flat_map(|item| item.sym.iter()).enumerate() {
                        if i != 0 {
                            syms.push_str(",");
                        }
                        syms.push_str(sym)
                    }
                    syms
                };

                let d = d
                    .iter()
                    .filter(|item| !item.no_crossref)
                    .collect::<Vec<_>>();

                let mut menu_jumps: HashMap<String, Json> = HashMap::new();
                for sym in d.iter().flat_map(|item| item.sym.iter()) {
                    let jump = match jumps.get(sym) {
                        Some(jump) => jump,
                        None => continue,
                    };

                    if jump.path == *path && jump.lineno == cur_line {
                        continue;
                    }

                    let key = format!("{}:{}", jump.path, jump.lineno);
                    let mut obj = json::Object::new();
                    obj.insert("sym".to_string(), Json::String(sym.to_string()));
                    obj.insert("pretty".to_string(), Json::String(jump.pretty.clone()));
                    menu_jumps.insert(key, Json::Object(obj));
                }

                let items = d
                    .iter()
                    .map(|item| {
                        let mut obj = json::Object::new();
                        obj.insert("pretty".to_string(), Json::String(item.pretty.clone()));
                        obj.insert("sym".to_string(), Json::String(item.sym.join(",")));
                        Json::Object(obj)
                    })
                    .collect::<Vec<_>>();

                let menu_jumps = menu_jumps.into_iter().map(|(_, v)| v).collect::<Vec<_>>();

                let index = generated_json.len();
                if items.len() > 0 {
                    generated_json.push(Json::Array(vec![
                        Json::Array(menu_jumps),
                        Json::Array(items),
                    ]));
                    format!("data-symbols=\"{}\" data-i=\"{}\" ", syms, index)
                } else {
                    format!("data-symbols=\"{}\" ", syms)
                }
            }
            _ => String::new(),
        };

        let style = match token.kind {
            tokenize::TokenKind::Identifier(None) => match datum {
                Some(d) => {
                    let classes = d.iter().flat_map(|a| {
                        a.syntax.iter().flat_map(|s| match s.as_ref() {
                            "type" => vec!["syn_type"],
                            "def" | "decl" | "idl" => vec!["syn_def"],
                            _ => vec![],
                        })
                    });
                    let classes = classes.collect::<Vec<_>>();
                    if classes.len() > 0 {
                        format!("class=\"{}\" ", classes.join(" "))
                    } else {
                        "".to_owned()
                    }
                }
                None => "".to_owned(),
            },
            tokenize::TokenKind::Identifier(Some(ref style)) => style.clone(),
            tokenize::TokenKind::StringLiteral => "class=\"syn_string\" ".to_owned(),
            tokenize::TokenKind::Comment => "class=\"syn_comment\" ".to_owned(),
            tokenize::TokenKind::TagName => "class=\"syn_tag\" ".to_owned(),
            tokenize::TokenKind::TagAttrName => "class=\"syn_tag\" ".to_owned(),
            tokenize::TokenKind::EndTagName => "class=\"syn_tag\" ".to_owned(),
            tokenize::TokenKind::RegularExpressionLiteral => "class=\"syn_regex\" ".to_owned(),
            _ => "".to_owned(),
        };

        match token.kind {
            tokenize::TokenKind::Punctuation | tokenize::TokenKind::PlainText => {
                let mut sanitized = entity_replace(input[last..token.end].to_string());
                if token.kind == tokenize::TokenKind::PlainText {
                    sanitized = links::linkify_comment(sanitized);
                }
                output.push_str(&sanitized);
                last = token.end;
            }
            _ => {
                if style != "" || data != "" {
                    output.push_str(&entity_replace(input[last..token.start].to_string()));
                    output.push_str(&format!("<span {}{}>", style, data));
                    let mut sanitized = entity_replace(input[token.start..token.end].to_string());
                    if token.kind == tokenize::TokenKind::Comment
                        || token.kind == tokenize::TokenKind::StringLiteral
                    {
                        sanitized = links::linkify_comment(sanitized);
                    }
                    output.push_str(&sanitized);
                    output.push_str("</span>");
                    last = token.end;
                }
            }
        }
    }

    output.push_str(&entity_replace(input[last..].to_string()));

    if output.len() > 0 {
        output_lines.push(fixup(output));
    }

    let analysis_json = if env::var("MOZSEARCH_DIFFABLE").is_err() {
        json::encode(&Json::Array(generated_json)).unwrap()
    } else {
        format!("{}", json::as_pretty_json(&Json::Array(generated_json)))
    };
    (output_lines, analysis_json)
}

pub fn format_file_data(
    cfg: &config::Config,
    tree_name: &str,
    panel: &[PanelSection],
    commit: &Option<git2::Commit>,
    blame_commit: &Option<git2::Commit>,
    path: &str,
    data: String,
    jumps: &HashMap<String, Jump>,
    analysis: &[WithLocation<Vec<AnalysisSource>>],
    writer: &mut Write,
    mut diff_cache: Option<&mut git_ops::TreeDiffCache>,
) -> Result<(), &'static str> {
    let tree_config = try!(cfg.trees.get(tree_name).ok_or("Invalid tree"));

    let format = languages::select_formatting(path);
    match format {
        FormatAs::Binary => {
            write!(writer, "Binary file").unwrap();
            return Ok(());
        }
        _ => {}
    };

    let (output_lines, analysis_json) = format_code(jumps, format, path, &data, &analysis);

    let blame_lines = git_ops::get_blame_lines(tree_config.git.as_ref(), blame_commit, path);

    let revision_owned = match commit {
        &Some(ref commit) => {
            let rev = commit.id().to_string();
            let (header, _) = try!(blame::commit_header(commit));
            Some((rev, header))
        }
        &None => None,
    };
    let revision = match revision_owned {
        Some((ref rev, ref header)) => Some((rev.as_str(), header.as_str())),
        None => None,
    };

    let path_wrapper = Path::new(path);
    let filename = path_wrapper.file_name().unwrap().to_str().unwrap();

    let title = format!("{} - mozsearch", filename);
    let opt = Options {
        title: &title,
        tree_name: tree_name,
        include_date: env::var("MOZSEARCH_DIFFABLE").is_err(),
        revision: revision,
    };

    try!(output::generate_header(&opt, writer));

    try!(output::generate_breadcrumbs(&opt, writer, path));

    try!(output::generate_panel(writer, panel));

    if let Some(ext) = path_wrapper.extension() {
        if ext.to_str().unwrap() == "svg" {
            if let Some(ref hg_root) = tree_config.paths.hg_root {
                let url = format!("{}/raw-file/tip/{}", hg_root, path);
                output::generate_svg_preview(writer, &url)?
            }
        }
    }

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
                F::Indent(vec![F::S("<td id=\"line-numbers\">")]),
            ]),
        ]),
    ]);

    output::generate_formatted(writer, &f, 0).unwrap();

    // Keep a cache of the "previous blame" computation across all
    // lines in the file. If we do encounter a changeset that we
    // want to ignore, chances are many lines will have that same
    // changeset as the more recent blame, and so they will all do
    // the same computation needlessly without this cache.
    let mut prev_blame_cache = git_ops::PrevBlameCache::new();

    let mut last_revs = None;
    let mut last_color = false;
    for i in 0..output_lines.len() {
        let lineno = i + 1;

        let blame_data = if let Some(ref lines) = blame_lines {
            let blame_line = &lines[i as usize];
            let pieces = blame_line.splitn(4, ':').collect::<Vec<_>>();

            // These store the final data we ship to the front-end.
            // Each of these is a comma-separated list with one element
            // for each blame entry.
            let mut revs = String::from(pieces[0]);
            let mut filespecs = String::from(pieces[1]);
            let mut blame_linenos = String::from(pieces[2]);

            if let Some(ref git) = tree_config.git {
                // These are the inputs to the find_prev_blame operation,
                // updated per iteration of the loop.
                let mut cur_rev = pieces[0].to_string();
                let mut cur_path = PathBuf::from(if pieces[1] == "%" { path } else { pieces[1] });
                let mut cur_lineno = pieces[2].parse::<u32>().unwrap();

                let mut max_ignored_allowed = 5; // chosen arbitrarily
                while git.should_ignore_for_blame(&cur_rev) {
                    if max_ignored_allowed == 0 {
                        // Push an empty entry on the end to indicate we hit the
                        // limit, but the last entry was still ignored
                        revs.push_str(",");
                        filespecs.push_str(",");
                        blame_linenos.push_str(",");
                        break;
                    }
                    max_ignored_allowed -= 1;

                    let (prev_blame_line, prev_path) = match git_ops::find_prev_blame(
                        git,
                        &cur_rev,
                        &cur_path,
                        cur_lineno,
                        &mut prev_blame_cache,
                        diff_cache.as_mut().map(|c| &mut **c),
                    ) {
                        Ok(prev) => prev,
                        Err(e) => {
                            // This can happen for many legitimate reasons, so
                            // handle it gracefully
                            info!("Unable to find prev blame: {:?}", e);
                            break;
                        }
                    };

                    let pieces = prev_blame_line.splitn(4, ':').collect::<Vec<_>>();

                    revs.push_str(",");
                    revs.push_str(pieces[0]);
                    filespecs.push_str(",");
                    filespecs.push_str(match (pieces[1], &prev_path, &cur_path) {
                        // file didn't move
                        ("%", prev, cur) if prev == cur => "%",
                        // file moved
                        ("%", prev, _) => prev.to_str().unwrap(),
                        // file moved, then moved back
                        (prevprev, _, cur) if Path::new(prevprev) == *cur => "%",
                        // file moved and moved again
                        (prevprev, _, _) => prevprev,
                    });
                    blame_linenos.push_str(",");
                    blame_linenos.push_str(pieces[2]);

                    // Update inputs to find_prev_blame for the next iteration
                    cur_rev = pieces[0].to_string();
                    cur_path = if pieces[1] == "%" {
                        prev_path
                    } else {
                        PathBuf::from(pieces[1])
                    };
                    cur_lineno = pieces[2].parse::<u32>().unwrap();
                }
            }

            let color = if last_revs.map_or(false, |last| last == revs) {
                last_color
            } else {
                !last_color
            };
            last_revs = Some(revs.clone());
            last_color = color;
            let class = if color { 1 } else { 2 };
            let data = format!(r#" class="blame-strip c{}" data-blame="{}#{}#{}" role="button" aria-label="blame" aria-expanded="false""#,
                               class, revs, filespecs, blame_linenos);
            data
        } else {
            "".to_owned()
        };

        let f = F::Seq(vec![
            F::T(format!(
                "<span id=\"l{}\" class=\"line-number\">{}",
                lineno, lineno
            )),
            F::T(format!("<div{}></div>", blame_data)),
            F::S("</span>"),
        ]);

        output::generate_formatted(writer, &f, 0).unwrap();
    }

    let f = F::Seq(vec![F::Indent(vec![F::Indent(vec![F::Indent(vec![
        F::S("</td>"),
        F::S("<td class=\"code\">"),
    ])])])]);
    output::generate_formatted(writer, &f, 0).unwrap();

    write!(writer, "<pre>").unwrap();
    for (i, line) in output_lines.iter().enumerate() {
        write!(
            writer,
            "<code id=\"line-{}\" aria-labelledby=\"{}\">{}\n</code>",
            i + 1,
            i + 1,
            line
        )
        .unwrap();
    }
    write!(writer, "</pre>").unwrap();

    let f = F::Seq(vec![
        F::Indent(vec![
            F::Indent(vec![F::Indent(vec![F::S("</td>")]), F::S("</tr>")]),
            F::S("</tbody>"),
        ]),
        F::S("</table>"),
    ]);
    output::generate_formatted(writer, &f, 0).unwrap();

    write!(
        writer,
        "<script>var ANALYSIS_DATA = {};</script>\n",
        analysis_json
    )
    .unwrap();

    output::generate_footer(&opt, tree_name, path, writer).unwrap();

    Ok(())
}

fn entry_to_blob(repo: &git2::Repository, entry: &git2::TreeEntry) -> Result<String, &'static str> {
    match entry.kind() {
        Some(git2::ObjectType::Blob) => {}
        _ => return Err("Invalid path; expected file"),
    }

    if entry.filemode() == 120000 {
        return Err("Path is to a symlink");
    }

    Ok(git_ops::read_blob_entry(repo, entry))
}

pub fn format_path(
    cfg: &config::Config,
    tree_name: &str,
    rev: &str,
    path: &str,
    writer: &mut Write,
) -> Result<(), &'static str> {
    // Get the file data.
    let tree_config = try!(cfg.trees.get(tree_name).ok_or("Invalid tree"));
    let git = try!(config::get_git(tree_config));
    let commit_obj = try!(git.repo.revparse_single(rev).map_err(|_| "Bad revision"));
    let commit = try!(commit_obj.into_commit().map_err(|_| "Bad revision"));
    let commit_tree = try!(commit.tree().map_err(|_| "Bad revision"));
    let path_obj = Path::new(path);
    let data = match commit_tree.get_path(path_obj) {
        Ok(entry) => entry_to_blob(&git.repo, &entry)?,
        Err(_) => {
            // Check to see if this path is inside a submodule
            let mut test_path = path_obj.parent();
            loop {
                let subrepo_path = match test_path {
                    Some(path) => path,
                    None => return Err("File not found"),
                };
                let entry = match commit_tree.get_path(subrepo_path) {
                    Ok(e) => e,
                    Err(_) => {
                        test_path = subrepo_path.parent();
                        continue;
                    }
                };
                if entry.kind() != Some(git2::ObjectType::Commit) {
                    return Err("File not found");
                }

                // If we get here, the path is inside a submodule
                let subrepo_path = subrepo_path.to_str().ok_or("UTF-8 error")?;
                let subrepo = git
                    .repo
                    .find_submodule(subrepo_path)
                    .map_err(|_| "Can't find submodule")?;
                let subrepo = subrepo.open().map_err(|_| "Can't open submodule")?;
                let path_in_subrepo = path_obj
                    .strip_prefix(subrepo_path)
                    .map_err(|_| "Submodule path error")?;
                let subentry = subrepo
                    .find_commit(entry.id())
                    .and_then(|commit| commit.tree())
                    .and_then(|tree| tree.get_path(path_in_subrepo))
                    .map_err(|_| "File not found in submodule")?;
                break entry_to_blob(&subrepo, &subentry)?;
            }
        }
    };

    // Get blame.
    let blame_commit = if let Some(ref blame_repo) = git.blame_repo {
        let blame_oid = try!(git
            .blame_map
            .get(&commit.id())
            .ok_or("Unable to find blame for revision"));
        Some(try!(blame_repo
            .find_commit(*blame_oid)
            .map_err(|_| "Blame is not a blob")))
    } else {
        None
    };

    let jumps: HashMap<String, analysis::Jump> = HashMap::new();
    let analysis = Vec::new();

    let hg_rev: &str = tree_config
        .git
        .as_ref()
        .and_then(|git| git.hg_map.get(&commit.id()))
        .and_then(|rev| Some(rev.as_ref())) // &String to &str conversion
        .unwrap_or(&"tip");

    let mut vcs_panel_items = vec![];
    vcs_panel_items.push(PanelItem {
        title: "Go to latest version".to_owned(),
        link: format!("/{}/source/{}", tree_name, path),
        update_link_lineno: true,
        accel_key: None,
    });
    if let Some(ref hg_root) = tree_config.paths.hg_root {
        vcs_panel_items.push(PanelItem {
            title: "Log".to_owned(),
            link: format!("{}/log/{}/{}", hg_root, hg_rev, path),
            update_link_lineno: false,
            accel_key: Some('L'),
        });
        vcs_panel_items.push(PanelItem {
            title: "Raw".to_owned(),
            link: format!("{}/raw-file/{}/{}", hg_root, hg_rev, path),
            update_link_lineno: false,
            accel_key: Some('R'),
        });
    }
    if tree_config.paths.git_blame_path.is_some() {
        vcs_panel_items.push(PanelItem {
            title: "Blame".to_owned(),
            link:
                "javascript:alert('Hover over the gray bar on the left to see blame information.')"
                    .to_owned(),
            update_link_lineno: false,
            accel_key: None,
        });
    }
    let panel = vec![PanelSection {
        name: "Revision control".to_owned(),
        items: vcs_panel_items,
    }];

    try!(format_file_data(
        cfg,
        tree_name,
        &panel,
        &Some(commit),
        &blame_commit,
        path,
        data,
        &jumps,
        &analysis,
        writer,
        None
    ));

    Ok(())
}

fn split_lines(s: &str) -> Vec<&str> {
    let mut split = s.split('\n').collect::<Vec<_>>();
    if split[split.len() - 1].len() == 0 {
        split.pop();
    }
    split
}

pub fn format_diff(
    cfg: &config::Config,
    tree_name: &str,
    rev: &str,
    path: &str,
    writer: &mut Write,
) -> Result<(), &'static str> {
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
        .output()
        .map_err(|_| "Diff failed 1"));
    if !output.status.success() {
        println!("ERR\n{}", git_ops::decode_bytes(output.stderr));
        return Err("Diff failed 2");
    }
    let difftxt = git_ops::decode_bytes(output.stdout);

    if difftxt.len() == 0 {
        return format_path(cfg, tree_name, rev, path, writer);
    }

    let git = try!(config::get_git(tree_config));
    let commit_obj = try!(git.repo.revparse_single(rev).map_err(|_| "Bad revision"));
    let commit = try!(commit_obj.as_commit().ok_or("Bad revision"));

    let mut blames = Vec::new();

    for parent_oid in commit.parent_ids() {
        if git.blame_repo.is_none() {
            blames.push(None);
            continue;
        }
        let blame_oid = try!(git.blame_map.get(&parent_oid).ok_or("Unable to find blame"));
        let blame_commit = try!(git
            .blame_repo
            .as_ref()
            .unwrap()
            .find_commit(*blame_oid)
            .map_err(|_| "Blame is not a blob"));
        let blame_tree = try!(blame_commit.tree().map_err(|_| "Bad revision"));
        match blame_tree.get_path(Path::new(path)) {
            Ok(blame_entry) => {
                let blame =
                    git_ops::read_blob_entry(git.blame_repo.as_ref().unwrap(), &blame_entry);
                let blame_lines = blame.lines().map(|s| s.to_owned()).collect::<Vec<_>>();
                blames.push(Some(blame_lines));
            }
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
            if (has_minus && origin[i] == '-') || (!has_minus && origin[i] != '+') {
                cur_blame = match blames[i] {
                    Some(ref lines) => Some(&lines[old_lineno[i] - 1]),
                    None => return Err("expected blame for '-' line, none found"),
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
        }
        _ => {}
    };
    let jumps: HashMap<String, analysis::Jump> = HashMap::new();
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

    let mut vcs_panel_items = vec![
        PanelItem {
            title: "Show changeset".to_owned(),
            link: format!("/{}/commit/{}", tree_name, rev),
            update_link_lineno: false,
            accel_key: None,
        },
        PanelItem {
            title: "Show file without diff".to_owned(),
            link: format!("/{}/rev/{}/{}", tree_name, rev, path),
            update_link_lineno: true,
            accel_key: None,
        },
        PanelItem {
            title: "Go to latest version".to_owned(),
            link: format!("/{}/source/{}", tree_name, path),
            update_link_lineno: true,
            accel_key: None,
        },
    ];
    if let Some(ref hg_root) = tree_config.paths.hg_root {
        vcs_panel_items.push(PanelItem {
            title: "Log".to_owned(),
            link: format!("{}/log/tip/{}", hg_root, path),
            update_link_lineno: false,
            accel_key: Some('L'),
        });
    }
    let sections = vec![PanelSection {
        name: "Revision control".to_owned(),
        items: vcs_panel_items,
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
                F::Indent(vec![F::S("<td id=\"line-numbers\">")]),
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

                let color = if last_rev == Some(rev) {
                    last_color
                } else {
                    !last_color
                };
                last_rev = Some(rev);
                last_color = color;
                let class = if color { 1 } else { 2 };
                format!(r#" class="blame-strip c{}" data-blame="{}#{}#{}" role="button" aria-label="blame" aria-expanded="false""#,
                        class, rev, filespec, blame_lineno)
            }
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
            F::S("</span>"),
        ]);

        output::generate_formatted(writer, &f, 0).unwrap();
    }

    let f = F::Seq(vec![F::Indent(vec![F::Indent(vec![F::Indent(vec![
        F::S("</td>"),
        F::S("<td class=\"code\">"),
    ])])])]);
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

        write!(
            writer,
            "<code id=\"line-{}\" aria-labelledby=\"{}\"{}>",
            lineno, lineno, class
        )
        .unwrap();
        write!(writer, "{} {}", origin, content).unwrap();
        write!(writer, "\n</code>").unwrap();
    }
    write!(writer, "</pre>").unwrap();

    let f = F::Seq(vec![
        F::Indent(vec![
            F::Indent(vec![F::Indent(vec![F::S("</td>")]), F::S("</tr>")]),
            F::S("</tbody>"),
        ]),
        F::S("</table>"),
    ]);
    output::generate_formatted(writer, &f, 0).unwrap();

    output::generate_footer(&opt, tree_name, path, writer).unwrap();

    Ok(())
}

fn generate_commit_info(
    tree_name: &str,
    tree_config: &config::TreeConfig,
    writer: &mut Write,
    commit: &git2::Commit,
) -> Result<(), &'static str> {
    let (header, remainder) = try!(blame::commit_header(&commit));

    fn format_rev(tree_name: &str, oid: git2::Oid) -> String {
        format!("<a href=\"/{}/commit/{}\">{}</a>", tree_name, oid, oid)
    }

    fn format_sig(sig: git2::Signature, git: &GitData) -> String {
        let (name, email) = git
            .mailmap
            .lookup(sig.name().unwrap(), sig.email().unwrap());
        format!("{} &lt;{}>", name, email)
    }

    let parents = commit
        .parent_ids()
        .map(|p| {
            F::T(format!(
                "<tr><td>parent</td><td>{}</td></tr>",
                format_rev(tree_name, p)
            ))
        })
        .collect::<Vec<_>>();

    let git = try!(config::get_git(tree_config));
    let hg = match git.hg_map.get(&commit.id()) {
        Some(hg_id) => {
            let hg_link = format!(
                "<a href=\"{}/rev/{}\">{}</a>",
                tree_config.paths.hg_root.as_ref().unwrap(),
                hg_id,
                hg_id
            );
            vec![F::T(format!("<tr><td>hg</td><td>{}</td></tr>", hg_link))]
        }

        None => vec![],
    };

    let id_string = format!("{}", commit.id());
    let gitstr = tree_config.paths.github_repo.as_ref().map(|ref ghurl| {
        format!(
            "<a href=\"{}/commit/{}\">{}</a>",
            ghurl, id_string, id_string
        )
    });

    let naive_t = NaiveDateTime::from_timestamp(commit.time().seconds(), 0);
    let tz = FixedOffset::east(commit.time().offset_minutes() * 60);
    let t: DateTime<FixedOffset> = DateTime::from_utc(naive_t, tz);
    let t = t.to_rfc2822();

    let f = F::Seq(vec![
        F::T(format!("<h3>{}</h3>", header)),
        F::T(format!("<pre><code>{}</code></pre>", remainder)),
        F::S("<table>"),
        F::Indent(vec![
            F::T(format!(
                "<tr><td>commit</td><td>{}</td></tr>",
                format_rev(tree_name, commit.id())
            )),
            F::Seq(parents),
            F::Seq(hg),
            F::T(gitstr.map_or(String::new(), |g| {
                format!("<tr><td>git</td><td>{}</td></tr>", g)
            })),
            F::T(format!(
                "<tr><td>author</td><td>{}</td></tr>",
                format_sig(commit.author(), git)
            )),
            F::T(format!(
                "<tr><td>committer</td><td>{}</td></tr>",
                format_sig(commit.committer(), git)
            )),
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
        .arg(id_string)
        .current_dir(&git_path)
        .output()
        .map_err(|_| "Diff failed 1"));
    if !output.status.success() {
        println!("ERR\n{}", git_ops::decode_bytes(output.stderr));
        return Err("Diff failed 2");
    }
    let difftxt = git_ops::decode_bytes(output.stdout);

    let lines = split_lines(&difftxt);
    let mut changes = Vec::new();
    for line in lines {
        if line.len() == 0 {
            continue;
        }

        let suffix = &line[commit.parents().count()..];
        let prefix_size = 2 * (commit.parents().count() + 1);
        let mut data = suffix.splitn(prefix_size + 1, ' ');
        let data = try!(data.nth(prefix_size).ok_or("Invalid diff output 3"));
        let file_info = data.split('\t').take(2).collect::<Vec<_>>();

        let f = F::T(format!(
            "<li>{} <a href=\"/{}/diff/{}/{}\">{}</a>",
            file_info[0],
            tree_name,
            commit.id(),
            file_info[1],
            file_info[1]
        ));
        changes.push(f);
    }

    let f = F::Seq(vec![F::S("<ul>"), F::Indent(changes), F::S("</ul>")]);
    try!(output::generate_formatted(writer, &f, 0));

    Ok(())
}

pub fn format_commit(
    cfg: &config::Config,
    tree_name: &str,
    rev: &str,
    writer: &mut Write,
) -> Result<(), &'static str> {
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

    try!(generate_commit_info(
        tree_name,
        &tree_config,
        writer,
        commit
    ));

    output::generate_footer(&opt, tree_name, "", writer).unwrap();

    Ok(())
}
