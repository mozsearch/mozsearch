use std::collections::HashMap;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::fs::File;
use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::io::BufWriter;
use std::io::Read;
use std::io::Seek;
use std::io::Write;
use std::path::Path;
use std::time::Instant;

use tools::file_format::config;
use tools::file_format::per_file_info::FileLookupMap;
use tools::file_format::per_file_info::read_detailed_file_info;
use tools::output::BreadcrumbsLinksTo;
use tools::output::PanelItemLabel;
use tools::templating::builder::build_and_parse;
use ustr::{Ustr, ustr};

extern crate env_logger;
extern crate log;
extern crate tools;
use crate::languages::FormatAs;
use tools::doc_trees_handler::find_doc_url;
use tools::file_format::analysis::{read_analysis, read_source};
use tools::file_format::jumpref::TreeJumprefs;
use tools::format::{
    create_markdown_panel_section, file_extras, format_file_data, has_rendered_spec_view,
    hyperblame_files, imported_spec_files, rendered_url, spec_globals,
    tests_firefox_dev_panel_item, tree_specs,
};
use tools::languages;
use tools::templating::info_boxes::InfoBoxes;
use tools::url_encode_path::url_encode_path;

use tools::output::{PanelItem, PanelItemExtras, PanelItemLink, PanelSection};

extern crate flate2;
use flate2::Compression;
use flate2::write::GzEncoder;
use tools::file_format::chunked_gzip::LineChunkedGzipWriter;

/// Write a page's hyperblame data files (see `hyperblame_files`) gzipped into
/// `dir`, along with the empty files nginx's `try_files` needs to find them
/// (like our `file/` outputs; see below).
fn write_hyperblame_files(dir: &str, files: Vec<(String, String)>) {
    fs::create_dir_all(dir).unwrap();
    for (name, contents) in files {
        File::create(format!("{}/{}", dir, name)).unwrap();
        let file = File::create(format!("{}/{}.gz", dir, name)).unwrap();
        let mut writer = GzEncoder::new(BufWriter::new(file), Compression::default());
        writer.write_all(contents.as_bytes()).unwrap();
        writer.finish().unwrap();
    }
}

/// A review-context.toml rule's load's link (see `metadata-analyze`), what
/// its panel item's copy button copies (a file's path, else the link), and
/// its name: a file of the tree's (its rendered view, for a document), of
/// another GitHub repository's, or another revision's diff.
fn review_load_link(tree_name: &str, load: &serde_json::Value) -> Option<(String, String, String)> {
    match load["type"].as_str().unwrap_or("file") {
        "file" => {
            let path = load["path"].as_str()?;
            match load["repo"].as_str() {
                Some(repo) => {
                    let url = format!(
                        "https://github.com/{}/blob/{}/{}",
                        repo,
                        load["branch"].as_str().unwrap_or("main"),
                        path
                    );
                    Some((url.clone(), url, format!("{} {}", repo, path)))
                }
                None => {
                    let is_doc = [".md", ".markdown", ".rst"]
                        .iter()
                        .any(|ext| path.ends_with(ext));
                    let view = if is_doc { "rendered" } else { "source" };
                    Some((
                        format!("/{}/{}/{}", tree_name, view, url_encode_path(path)),
                        path.to_string(),
                        path.to_string(),
                    ))
                }
            }
        }
        "fetch_revision" => {
            let (url, name) = match (
                load["revision"].as_str(),
                load["repo"].as_str(),
                load["hash"].as_str(),
            ) {
                (Some(revision), _, _) => (
                    format!("https://phabricator.services.mozilla.com/{}", revision),
                    revision.to_string(),
                ),
                (None, Some(repo), Some(hash)) => (
                    format!("https://github.com/{}/commit/{}", repo, hash),
                    format!("{} {}", repo, hash),
                ),
                _ => return None,
            };
            Some((url.clone(), url, name))
        }
        _ => None,
    }
}

fn main() {
    env_logger::init();

    let args: Vec<_> = env::args().collect();
    // TODO: refactor _fname_args out of existence; we now require paths to come via stdin.
    let (base_args, _fname_args) = args.split_at(5);

    let mut stdout = io::stdout().lock();

    let pre_config = Instant::now();
    let tree_name = &base_args[2];
    let cfg = config::load(
        &base_args[1],
        true,
        Some(tree_name),
        Some(base_args[3].to_string()),
        Some(base_args[4].to_string()),
    );
    writeln!(
        stdout,
        "Config file read, duration: {}us",
        pre_config.elapsed().as_micros() as u64
    )
    .unwrap();

    let tree_config = cfg.trees.get(tree_name).unwrap();
    let tree_specs = tree_specs(tree_name, tree_config);
    // (The files the tree's ecmarkup specs import and its Bikeshed specs
    // include, which have rendered views as their parts of the specs.)
    let imported_spec_files = tree_specs
        .as_deref()
        .map_or_else(Default::default, |specs| {
            let files_path = Path::new(&tree_config.paths.files_path);
            imported_spec_files(tree_config, specs, &|file: &str| {
                std::fs::read_to_string(files_path.join(file)).ok()
            })
        });

    let pre_templates = Instant::now();
    let info_boxes = InfoBoxes::load(&cfg);
    let panel_section_templates = InfoBoxes::load_dir(&cfg, "panel_sections");
    let source_file_other_tools_panel_liquid_str = cfg
        .read_tree_config_file_with_default("source_file_other_tools_panels.liquid")
        .unwrap();
    let source_file_other_tools_panel_template =
        build_and_parse(&source_file_other_tools_panel_liquid_str);
    writeln!(
        stdout,
        "Tree templates read, duration: {}us",
        pre_templates.elapsed().as_micros() as u64
    )
    .unwrap();

    let pre_jumpref = Instant::now();
    // (With the linked trees', which were indexed first, for their symbols'
    // jumps, ex: firefox-disco's citations' of wubspecs' definitions.)
    let jumprefs = TreeJumprefs::open(&tree_config.paths, &cfg.tree_index_paths);

    writeln!(
        stdout,
        "Jumpref opened, duration: {}us",
        pre_jumpref.elapsed().as_micros() as u64
    )
    .unwrap();

    // The paths to output (all of them first, so that the file lookup map
    // has only theirs; see `FileLookupMap::new_for_paths`).
    let paths: Vec<Ustr> = io::stdin()
        .lock()
        .lines()
        .map_while(|line| line.ok())
        .map(|line| ustr(line.trim_end()))
        .collect();

    let pre_lookup_map = Instant::now();
    let file_lookup_path = format!(
        "{}/concise-per-file-info.json",
        tree_config.paths.index_path
    );
    let file_lookup_map =
        FileLookupMap::new_for_paths(&file_lookup_path, &paths.iter().copied().collect());
    writeln!(
        stdout,
        "FileLookupMap loadded, duration: {}us",
        pre_lookup_map.elapsed().as_micros() as u64
    )
    .unwrap();

    let pre_blame_prep = Instant::now();
    let (blame_commit, head_oid) = match tree_config.git {
        Some(ref git) => {
            // Because our git tree (possibly a worktree) must explicitly be checked out to the
            // specific revision of interest, HEAD is the right choice here.  (And note that this
            // can potentially be a detached HEAD for the firefox-* trees where we inevitably are
            // using an older revision in order to get coverage data.)
            let head_oid = git.repo.refname_to_id("HEAD").unwrap();
            // However, we support using a bare repo for blame for firefox-shared and so we must
            // not directly rely on our blame tree's HEAD.  `config::load` already handles using the
            // config "git_branch" if it's present or falling back to HEAD if it's not.  So it'
            // sufficient for us to just use the blame-map here to find the right blame commit
            // for the head_oid above.
            let blame_commit = git.blame_repo.as_ref().and_then(|blame_repo| {
                git.blame_rev(head_oid)
                    .map(|blame_oid| blame_repo.find_commit(blame_oid).unwrap())
            });
            (blame_commit, Some(head_oid))
        }
        None => (None, None),
    };

    let coverage_commit = tree_config
        .git
        .as_ref()
        .and_then(|git| git.coverage_repo.as_ref())
        .zip(head_oid)
        .and_then(|(repo, commit)| {
            repo.revparse_single(&format!("refs/tags/reverse/all/all/{}", commit))
                .ok()
                .and_then(|object| object.peel_to_commit().ok())
        });

    let head_commit =
        head_oid.and_then(|oid| tree_config.git.as_ref().unwrap().repo.find_commit(oid).ok());

    writeln!(
        stdout,
        "Blame prep done, duration: {}us",
        pre_blame_prep.elapsed().as_micros() as u64
    )
    .unwrap();

    let mut extension_mapping = HashMap::new();
    extension_mapping.insert("cpp", ("header", vec!["h", "hh", "hpp", "hxx"]));
    extension_mapping.insert("cc", ("header", vec!["h", "hh", "hpp", "hxx"]));
    extension_mapping.insert("cxx", ("header", vec!["h", "hh", "hpp", "hxx"]));
    extension_mapping.insert("h", ("source", vec!["cpp", "cc", "cxx"]));
    extension_mapping.insert("hh", ("source", vec!["cpp", "cc", "cxx"]));
    extension_mapping.insert("hpp", ("source", vec!["cpp", "cc", "cxx"]));
    extension_mapping.insert("hxx", ("source", vec!["cpp", "cc", "cxx"]));

    for path in paths {
        let file_start = Instant::now();
        writeln!(stdout, "File '{}'", path).unwrap();

        let output_fname = format!("{}/file/{}", tree_config.paths.index_path, path);
        let gzip_output_fname = format!("{}.gz", output_fname);
        let source_fname = tree_config.find_source_file(&path);

        let format = languages::select_formatting(&path);

        // Create a zero length output file with the normal name for nginx
        // try_files reasons... UNLESS the file we're dealing with already has
        // a ".gz" suffix.  (try_files isn't aware of the gzip_static magic and
        // so looks for a file with the exact non-.gz suffix, which means it has
        // to exist and so we normally need to create one.)
        //
        // The general problem scenario are tests where there's a file "FOO" and
        // its gzipped variant "FOO.gz" in the tree.  In that case there's an
        // overlap for "FOO.gz".  When processing "FOO.gz" we will write to
        // "FOO.gz.gz" but also want the file "FOO.gz" to exist.  And for "FOO"
        // we will write to "FOO.gz" and want "FOO" to exist.  If our heuristic
        // is to not create the zero-length file for "FOO.gz", we win and don't
        // have to worry about pathological races as long as the source file
        // "FOO" exists.  But if it doesn't our try_files logic will never allow
        // the user to view the (gibberish for humans) "FOO.gz" source file.
        //
        // So we use that heuristic.  Because I'm a human.  A lazy, lazy human.
        // Robots or non-lazy humans are welcome to contribute better fixes for
        // this and will be showered with praise.
        if !output_fname.ends_with(".gz") {
            File::create(output_fname).unwrap();
        }
        let output_file = File::create(gzip_output_fname).unwrap();
        let raw_writer = BufWriter::new(output_file);
        // A gzip with chunks of lines, so that /query/'s results can read just
        // the lines they show (see `chunked_gzip`).  (Written when it's
        // finished below, or dropped by the early `continue`s.)
        let mut writer = LineChunkedGzipWriter::new(raw_writer);

        let source_file = match File::open(source_fname.clone()) {
            Ok(f) => f,
            Err(_) => {
                writeln!(stdout, "Unable to open source file '{}'", source_fname).unwrap();
                continue;
            }
        };

        let path_wrapper = Path::new(&source_fname);
        let metadata = fs::symlink_metadata(path_wrapper).unwrap();
        if metadata.file_type().is_symlink() {
            let dest = fs::read_link(path_wrapper).unwrap();
            write!(writer, "Symlink to '{}'", dest.to_str().unwrap()).unwrap();
            continue;
        }

        let mut reader = BufReader::new(&source_file);

        if let FormatAs::Binary = format {
            let _ = io::copy(&mut reader, &mut writer);
            continue;
        };

        let pre_analysis_load = Instant::now();
        let analysis_fname = format!("{}/analysis/{}", tree_config.paths.index_path, path);
        let analysis = read_analysis(&analysis_fname, &mut read_source);
        writeln!(
            stdout,
            "  Analysis load duration: {}us",
            pre_analysis_load.elapsed().as_micros() as u64
        )
        .unwrap();

        let pre_per_file_info = Instant::now();
        let concise_info = file_lookup_map.lookup_file_from_ustr(&path).unwrap();
        let detailed_info = read_detailed_file_info(&path, &tree_config.paths.index_path).unwrap();
        writeln!(
            stdout,
            "  Per-file info load duration: {}us",
            pre_per_file_info.elapsed().as_micros() as u64
        )
        .unwrap();

        let pre_file_read = Instant::now();
        let mut input = String::new();
        match reader.read_to_string(&mut input) {
            Ok(_) => {}
            Err(_) => {
                let mut bytes = Vec::new();
                reader.seek(std::io::SeekFrom::Start(0)).unwrap();
                match reader.read_to_end(&mut bytes) {
                    Ok(_) => {
                        input.push_str(&bytes.iter().map(|c| *c as char).collect::<String>());
                    }
                    Err(e) => {
                        writeln!(
                            stdout,
                            "Unable to read source file '{}': {:?}",
                            source_fname, e
                        )
                        .unwrap();
                        continue;
                    }
                }
            }
        }
        writeln!(
            stdout,
            "  File contents read duration: {}us",
            pre_file_read.elapsed().as_micros() as u64
        )
        .unwrap();

        let extension = path_wrapper
            .extension()
            .unwrap_or(OsStr::new(""))
            .to_str()
            .unwrap();
        let show_header = match extension_mapping.get(extension) {
            Some((description, try_extensions)) => {
                let mut result = None;
                for try_ext in try_extensions {
                    let try_buf = path_wrapper.with_extension(try_ext);
                    let try_path = try_buf.as_path();
                    if try_path.exists() {
                        let (path_base, _) = path.split_at(path.len() - extension.len() - 1);
                        result = Some((
                            description.to_owned(),
                            format!("/{}/source/{}.{}", tree_name, path_base, try_ext),
                        ));
                        break;
                    }
                }
                result
            }
            None => None,
        };

        let mut panel = vec![];

        let mut source_panel_items = vec![];
        if let Some((other_desc, other_path)) = show_header {
            source_panel_items.push(PanelItem {
                label: PanelItemLabel::Plaintext(format!("Go to {} file", other_desc)),
                tooltip: format!(
                    "Open the corresponding {} file of the current file",
                    other_desc
                ),
                id: "panel-goto-other-file",
                link: url_encode_path(other_path.as_str()),
                update_link_lineno: "",
                accel_key: None,
                copyable: false,
                extras: Default::default(),
            });
        };

        if !path.contains("__GENERATED__")
            && let Some((product, component)) = concise_info.bugzilla_component
        {
            // (The moz.build rule giving it the component, which is where the
            // component comes from, unless a config's per-file-info.toml has
            // it from elsewhere.)
            let rule = &detailed_info.info["bugzilla_rule"];
            let rule_link = (rule["product"].as_str() == Some(product.as_str())
                && rule["component"].as_str() == Some(component.as_str()))
            .then(|| rule["path"].as_str().zip(rule["line"].as_u64()))
            .flatten()
            .map(|(rule_path, line)| PanelItemLink {
                label: "moz.build".to_string(),
                tooltip: format!(
                    "Go to the rule giving this file its component, in {} on line {}",
                    rule_path, line
                ),
                link: format!(
                    "/{}/source/{}#{}",
                    tree_name,
                    url_encode_path(rule_path),
                    line
                ),
            });
            source_panel_items.push(PanelItem {
                label: PanelItemLabel::Plaintext(format!(
                    "File a bug in {} :: {}",
                    product, component
                )),
                tooltip: format!(
                    "Open the new bug form for the {} :: {} component in bugzilla",
                    product, component
                ),
                id: "panel-file-bug",
                link: format!(
                    "https://bugzilla.mozilla.org/enter_bug.cgi?product={}&component={}",
                    product.replace("&", "%26"),
                    component.replace("&", "%26")
                ),
                update_link_lineno: "",
                accel_key: None,
                copyable: true,
                extras: PanelItemExtras {
                    copy_text: None,
                    link: rule_link,
                },
            });
        }

        // The review guidance (see review-context.toml) for patches changing
        // the file, by the rules' loads, with links to the rules.
        for rule in detailed_info.info["review_rules"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let (Some(name), Some(rules_path), Some(line)) = (
                rule["name"].as_str(),
                rule["path"].as_str(),
                rule["line"].as_u64(),
            ) else {
                continue;
            };
            let partial = rule["applies"].as_str() == Some("partially");
            let loads: Vec<&serde_json::Value> =
                rule["loads"].as_array().into_iter().flatten().collect();
            for load in &loads {
                let Some((link, copy_text, what)) = review_load_link(&tree_name, load) else {
                    continue;
                };
                let mut label = format!("Review skill: {}", name);
                if loads.len() > 1 {
                    label = format!("{} ({})", label, what);
                }
                if partial {
                    label = format!("{}, with other changes", label);
                }
                source_panel_items.push(PanelItem {
                    label: PanelItemLabel::Plaintext(label),
                    tooltip: format!(
                        "{}{}",
                        rule["description"].as_str().unwrap_or(name),
                        if partial {
                            " (for patches changing this file and others the rule needs)"
                        } else {
                            ""
                        }
                    ),
                    id: "",
                    link,
                    update_link_lineno: "",
                    accel_key: None,
                    copyable: true,
                    extras: PanelItemExtras {
                        copy_text: Some(copy_text),
                        link: Some(PanelItemLink {
                            label: "rule".to_string(),
                            tooltip: format!("Go to the rule, in {} on line {}", rules_path, line),
                            link: format!("/{}/source/{}#{}", tree_name, rules_path, line),
                        }),
                    },
                });
            }
        }

        if !source_panel_items.is_empty() {
            panel.push(PanelSection {
                name: "Source code".to_owned(),
                items: source_panel_items,
                raw_items: vec![],
            });
        };

        let encoded_path = url_encode_path(path.as_str());

        if let Some(oid) = head_oid {
            if !path.contains("__GENERATED__") {
                let mut vcs_panel_items = vec![];
                vcs_panel_items.push(PanelItem {
                    label: PanelItemLabel::Plaintext("Permalink".to_owned()),
                    tooltip: "Create a revision-specific link of the current file".to_owned(),
                    id: "panel-permalink",
                    link: format!("/{}/rev/{}/{}", tree_name, oid, encoded_path),
                    update_link_lineno: "#{}",
                    accel_key: Some('Y'),
                    copyable: true,
                    extras: Default::default(),
                });
                vcs_panel_items.push(PanelItem {
                    label: PanelItemLabel::Plaintext("Remove the Permalink".to_owned()),
                    tooltip: "Create a revision-agnostic link of the current file".to_owned(),
                    id: "panel-remove-permalink",
                    link: format!("/{}/source/{}", tree_name, encoded_path),
                    update_link_lineno: "#{}",
                    accel_key: None,
                    copyable: false,
                    extras: Default::default(),
                });

                let gh_log_link = tree_config.paths.github_repo.as_ref().map(|gh_root| {
                    format!(
                        "{}/commits/{}/{}",
                        gh_root,
                        tree_config.paths.git_branch.as_deref().unwrap_or("HEAD"),
                        encoded_path
                    )
                });
                let hg_log_link = tree_config
                    .paths
                    .hg_root
                    .as_ref()
                    .map(|hg_root| format!("{}/log/default/{}", hg_root, encoded_path));
                if let Some(link) = gh_log_link {
                    vcs_panel_items.push(PanelItem {
                        label: PanelItemLabel::Plaintext("Git log".to_owned()),
                        tooltip: "Open git log of the current file".to_owned(),
                        id: "panel-log-git",
                        link,
                        update_link_lineno: "",
                        accel_key: hg_log_link.is_none().then_some('L'),
                        copyable: true,
                        extras: Default::default(),
                    });
                }
                if let Some(link) = hg_log_link {
                    vcs_panel_items.push(PanelItem {
                        label: PanelItemLabel::Plaintext("Mercurial log".to_owned()),
                        tooltip: "Open mercurial log of the current file".to_owned(),
                        id: "panel-log-hg",
                        link,
                        update_link_lineno: "",
                        accel_key: Some('L'),
                        copyable: true,
                        extras: Default::default(),
                    });
                }

                panel.push(PanelSection {
                    name: "Revision control".to_owned(),
                    items: vcs_panel_items,
                    raw_items: vec![],
                });
            } else {
                let mut gen_files_panel_items = vec![];
                gen_files_panel_items.push(PanelItem {
                    label: PanelItemLabel::Plaintext("Raw".to_owned()),
                    tooltip: "Open a raw file of the current file".to_owned(),
                    id: "panel-raw",
                    link: format!("/{}/raw/{}", tree_name, path),
                    update_link_lineno: "",
                    accel_key: Some('R'),
                    copyable: true,
                    extras: Default::default(),
                });

                panel.push(PanelSection {
                    name: "Generated file".to_owned(),
                    items: gen_files_panel_items,
                    raw_items: vec![],
                });
            }
        }

        // (The raw file is in its "Copy as..." popup.)
        let raw_url = if path.contains("__GENERATED__") {
            None
        } else {
            tree_config.paths.make_raw_resource_branch_url(&path)
        };
        panel.push(create_markdown_panel_section(true, raw_url.as_deref()));

        let mut tools_items = vec![];

        if concise_info.info.get("test_results").is_some() {
            tools_items.push(tests_firefox_dev_panel_item(&path, false));
        }

        match Path::new(path.as_str()).extension().and_then(OsStr::to_str) {
            Some("md") | Some("rst") => {
                // (A file a Bikeshed spec includes is rendered as its part.)
                let included = imported_spec_files.contains_key(path.as_str());
                tools_items.push(PanelItem {
                    label: PanelItemLabel::Plaintext("Rendered view".to_owned()),
                    tooltip: if included {
                        "Show the rendered spec, with the blame of its blocks"
                    } else {
                        "Show the rendered document, with the blame of its blocks"
                    }
                    .to_owned(),
                    id: "panel-rendered-view",
                    link: rendered_url(tree_name, None, &path),
                    update_link_lineno: if included { "#{}" } else { "" },
                    accel_key: None,
                    copyable: true,
                    extras: Default::default(),
                });

                if let Some(url) = find_doc_url(&cfg, path.as_str()) {
                    tools_items.push(PanelItem {
                        label: PanelItemLabel::Plaintext("Source Docs".to_owned()),
                        tooltip: "Open the rendered documentation on Source Docs".to_owned(),
                        id: "panel-source-docs",
                        link: url,
                        update_link_lineno: "",
                        accel_key: None,
                        copyable: false,
                        extras: Default::default(),
                    });
                }

                if let Some(ref github) = tree_config.paths.github_repo {
                    tools_items.push(PanelItem {
                        label: PanelItemLabel::Plaintext("GitHub Rendered view".to_owned()),
                        tooltip: "Open the rendered documentation on GitHub".to_owned(),
                        id: "panel-github-rendered-view",
                        link: format!(
                            "{}/blob/{}/{}",
                            github,
                            head_oid.map_or("master".to_string(), |x| x.to_string()),
                            encoded_path
                        ),
                        update_link_lineno: "",
                        accel_key: None,
                        copyable: false,
                        extras: Default::default(),
                    });
                }
            }
            _ => {
                // (A spec's source, ex: the HTML Standard's, whose rendered
                // view can be split into pages, where the line is.)
                let imported = imported_spec_files.contains_key(path.as_str());
                if let Some(specs) = &tree_specs
                    && (has_rendered_spec_view(tree_config, specs, &path) || imported)
                {
                    tools_items.push(PanelItem {
                        label: PanelItemLabel::Plaintext("Rendered view".to_owned()),
                        tooltip: "Show the rendered spec, with the blame of its blocks".to_owned(),
                        id: "panel-rendered-view",
                        link: rendered_url(tree_name, None, &path),
                        // (An imported file's view is one page.)
                        update_link_lineno: if imported { "#{}" } else { "/line/{}" },
                        accel_key: None,
                        copyable: true,
                        extras: Default::default(),
                    });
                }
            }
        }

        let mut liquid_globals = liquid::object!({
            "tree": tree_name,
            "path": &path,
            // Propagate config settings that aren't absolute paths.  We do some
            // renaming here compared to `TreeConfigPaths` for clarity.
            "config": {
                "github_repo_url": &tree_config.paths.github_repo.as_deref().unwrap_or(""),
                "hg_repo_url": &tree_config.paths.hg_root.as_deref().unwrap_or(""),
                "wpt_root": &tree_config.paths.wpt_root.as_deref().unwrap_or(""),
            },
            "concise": &concise_info,
            "detailed": &detailed_info,
        });

        // (A spec's source, or a file it imports or includes, has the spec's
        // info, ex: for an obsolete spec's info box.)
        if let Some(specs) = &tree_specs {
            let spec = specs.specs.iter().find(|spec| {
                spec.source_path().as_deref() == Some(path.as_str())
                    || imported_spec_files.get(path.as_str()) == Some(&spec.name)
            });
            if let Some(spec) = spec {
                liquid_globals.insert(
                    "spec".into(),
                    spec_globals(tree_name, tree_config, specs, spec),
                );
            }
        }
        liquid_globals
            .entry("spec")
            .or_insert(liquid::model::Value::Nil);
        let source_file_info_boxes = info_boxes.render(&liquid_globals);
        // (The templates' panel sections, after the "Source code" one.)
        let at = panel
            .iter()
            .position(|section| section.name == "Source code")
            .map_or(0, |i| i + 1);
        for (i, html) in panel_section_templates
            .render_each(&liquid_globals)
            .into_iter()
            .enumerate()
        {
            panel.insert(
                at + i,
                PanelSection {
                    name: String::new(),
                    items: vec![],
                    raw_items: vec![html],
                },
            );
        }
        let source_file_other_tools_panels = source_file_other_tools_panel_template
            .render(&liquid_globals)
            .unwrap()
            .trim()
            .to_string();

        if !tools_items.is_empty() || !source_file_other_tools_panels.is_empty() {
            panel.push(PanelSection {
                name: "Other Tools".to_owned(),
                items: tools_items,
                raw_items: vec![source_file_other_tools_panels],
            });
        }

        // A panic formatting the file (ex: on source a tokenizer misreads)
        // only loses its page, which says so, rather than the tree's output,
        // since output.sh's parallel stops every job at the first which fails.
        // (The panic's message and backtrace go to stderr, as usual.)
        let formatted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let formatted = file_extras(
                &cfg,
                tree_name,
                &head_commit,
                BreadcrumbsLinksTo::Latest,
                &blame_commit,
                coverage_commit.as_ref(),
                &path,
                &input,
            )
            .and_then(|extras| {
                format_file_data(
                    &cfg,
                    tree_name,
                    panel,
                    source_file_info_boxes,
                    extras,
                    BreadcrumbsLinksTo::Latest,
                    &path,
                    input,
                    &jumprefs,
                    &analysis,
                    &mut writer,
                )
            });
            match formatted {
                Ok(formatted) => {
                    let perf_info = formatted.perf;
                    writeln!(
                        stdout,
                        "  Format code duration: {}us",
                        perf_info.format_code_duration_us
                    )
                    .unwrap();
                    writeln!(
                        stdout,
                        "  Blame lines duration: {}us",
                        perf_info.blame_lines_duration_us
                    )
                    .unwrap();
                    writeln!(
                        stdout,
                        "  Commit info duration: {}us",
                        perf_info.commit_info_duration_us
                    )
                    .unwrap();
                    writeln!(
                        stdout,
                        "  Format mixing duration: {}us",
                        perf_info.format_mixing_duration_us
                    )
                    .unwrap();

                    if let (Some(page), Some(git)) = (&formatted.token_blame, &tree_config.git) {
                        let pre_hyperblame = Instant::now();
                        write_hyperblame_files(
                            &format!("{}/hyperblame/{}", tree_config.paths.index_path, path),
                            hyperblame_files(tree_config, git, page),
                        );
                        writeln!(
                            stdout,
                            "  Hyperblame files duration: {}us",
                            pre_hyperblame.elapsed().as_micros() as u64
                        )
                        .unwrap();
                    }
                }
                Err(err) => {
                    // Make sure our output log file indicates what happened.
                    writeln!(stdout, "  warning: format_file_data failed: {}", err).unwrap();
                    // Also embed the error into the output file
                    writeln!(writer, "<h3>format_file_data failed: {}</h3>", err).unwrap();
                }
            }
        }));
        if let Err(panic) = formatted {
            let message = panic
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| panic.downcast_ref::<&str>().copied())
                .unwrap_or("(no message)");
            writeln!(
                stdout,
                "  warning: formatting the file panicked: {}",
                message
            )
            .unwrap();
            writer.clear();
            writeln!(
                writer,
                "<h3>Searchfox failed to render this file: {}</h3>",
                tools::rendered_docs::escape(message)
            )
            .unwrap();
            // (For finding which files did, besides the logs.)
            let diags = format!("{}/diags/output", tree_config.paths.index_path);
            let _ = std::fs::create_dir_all(&diags);
            if let Ok(mut panicked) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(format!("{}/panicked.tsv", diags))
            {
                let _ = writeln!(panicked, "{}\t{}", path, message.replace(['\t', '\n'], " "));
            }
        }

        writer.finish().unwrap();
        writeln!(
            stdout,
            "  Total writing duration: {}us",
            file_start.elapsed().as_micros() as u64
        )
        .unwrap();
    }
    writeln!(stdout, "Done writing files.").unwrap();
}
