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

use lazy_static::lazy_static;
use regex::Regex;
use tools::file_format::config;
use tools::file_format::per_file_info::read_detailed_file_info;
use tools::file_format::per_file_info::FileLookupMap;
use tools::templating::builder::build_and_parse;
use ustr::ustr;

extern crate env_logger;
extern crate log;
extern crate tools;
use crate::languages::FormatAs;
use tools::file_format::analysis::{read_analysis, read_source};
use tools::format::{create_markdown_panel_section, format_file_data};
use tools::file_format::crossref_lookup::CrossrefLookupMap;
use tools::languages;

use tools::output::{PanelItem, PanelSection};

extern crate flate2;
use flate2::write::GzEncoder;
use flate2::Compression;

fn main() {
    env_logger::init();

    let args: Vec<_> = env::args().collect();
    // TODO: refactor _fname_args out of existence; we now require paths to come via stdin.
    let (base_args, _fname_args) = args.split_at(4);

    let mut stdout = io::stdout().lock();

    let pre_config = Instant::now();
    let tree_name = &base_args[2];
    let cfg = config::load(&base_args[1], true, Some(&tree_name), Some(base_args[3].to_string()));
    writeln!(
        stdout,
        "Config file read, duration: {}us",
        pre_config.elapsed().as_micros() as u64
    )
    .unwrap();

    let tree_config = cfg.trees.get(tree_name).unwrap();

    let pre_templates = Instant::now();
    let source_file_info_boxes_liquid_str = cfg
        .read_tree_config_file_with_default("source_file_info_boxes.liquid")
        .unwrap();
    let source_file_info_boxes_template = build_and_parse(&source_file_info_boxes_liquid_str);
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

    let jumpref_path = format!("{}/jumpref", tree_config.paths.index_path);
    let jumpref_extra_path = format!("{}/jumpref-extra", tree_config.paths.index_path);

    let pre_jumpref = Instant::now();
    let jumpref_lookup_map = CrossrefLookupMap::new(&jumpref_path, &jumpref_extra_path);

    writeln!(stdout, "Jumpref opened, duration: {}us", pre_jumpref.elapsed().as_micros() as u64).unwrap();

    let pre_lookup_map = Instant::now();
    let file_lookup_path = format!(
        "{}/concise-per-file-info.json",
        tree_config.paths.index_path
    );
    let file_lookup_map = FileLookupMap::new(&file_lookup_path);
    writeln!(
        stdout,
        "FileLookupMap loadded, duration: {}us",
        pre_lookup_map.elapsed().as_micros() as u64
    )
    .unwrap();

    let pre_blame_prep = Instant::now();
    let (blame_commit, head_oid) = match &tree_config.git {
        &Some(ref git) => {
            let head_oid = git.repo.refname_to_id("HEAD").unwrap();
            let blame_commit = if let Some(ref blame_repo) = git.blame_repo {
                let blame_oid = blame_repo.refname_to_id("HEAD").unwrap();
                Some(blame_repo.find_commit(blame_oid).unwrap())
            } else {
                None
            };
            (blame_commit, Some(head_oid))
        }
        &None => (None, None),
    };

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

    let mut stdin = io::stdin().lock();

    let mut path_buf = String::new();
    while () == path_buf.clear() && stdin.read_line(&mut path_buf).unwrap() > 0 {
        let path = ustr(path_buf.trim_end());
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
        let mut writer = GzEncoder::new(raw_writer, Compression::default());

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

        match format {
            FormatAs::Binary => {
                let _ = io::copy(&mut reader, &mut writer);
                continue;
            }
            _ => {}
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
            .unwrap_or(&OsStr::new(""))
            .to_str()
            .unwrap();
        let show_header = match extension_mapping.get(extension) {
            Some(&(ref description, ref try_extensions)) => {
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
                title: format!("Go to {} file", other_desc),
                link: other_path,
                update_link_lineno: "",
                accel_key: None,
                copyable: false,
            });
        };

        if !path.contains("__GENERATED__") {
            if let Some((product, component)) = concise_info.bugzilla_component {
                source_panel_items.push(PanelItem {
                    title: format!("File a bug in {} :: {}", product, component),
                    link: format!(
                        "https://bugzilla.mozilla.org/enter_bug.cgi?product={}&component={}",
                        product.replace("&", "%26"),
                        component.replace("&", "%26")
                    ),
                    update_link_lineno: "",
                    accel_key: None,
                    copyable: true,
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

        if let Some(oid) = head_oid {
            if !path.contains("__GENERATED__") {
                let mut vcs_panel_items = vec![];
                vcs_panel_items.push(PanelItem {
                    title: "Permalink".to_owned(),
                    link: format!("/{}/rev/{}/{}", tree_name, oid, path),
                    update_link_lineno: "#{}",
                    accel_key: Some('Y'),
                    copyable: true,
                });
                vcs_panel_items.push(PanelItem {
                    title: "Remove the Permalink".to_owned(),
                    link: format!("/{}/source/{}", tree_name, path),
                    update_link_lineno: "#{}",
                    accel_key: None,
                    copyable: false,
                });
                if let Some(ref hg_root) = tree_config.paths.hg_root {
                    vcs_panel_items.push(PanelItem {
                        title: "Log".to_owned(),
                        link: format!("{}/log/tip/{}", hg_root, path),
                        update_link_lineno: "",
                        accel_key: Some('L'),
                        copyable: true,
                    });
                    vcs_panel_items.push(PanelItem {
                        title: "Raw".to_owned(),
                        link: format!("{}/raw-file/tip/{}", hg_root, path),
                        update_link_lineno: "",
                        accel_key: Some('R'),
                        copyable: true,
                    });
                }
                if tree_config.paths.git_blame_path.is_some() {
                    vcs_panel_items.push(PanelItem {
                        title: "Blame".to_owned(),
                        link: "javascript:alert('Hover over the gray bar on the left to see blame information.')".to_owned(),
                        update_link_lineno: "",
                        accel_key: None,
                        copyable: false,
                    });
                }
                panel.push(PanelSection {
                    name: "Revision control".to_owned(),
                    items: vcs_panel_items,
                    raw_items: vec![],
                });
            }
        }

        panel.push(create_markdown_panel_section(true));

        let mut tools_items = vec![];
        if let Some(ref hg_root) = tree_config.paths.hg_root {
            tools_items.push(PanelItem {
                title: "HG Web".to_owned(),
                link: format!("{}/file/tip/{}", hg_root, path),
                update_link_lineno: "#l{}",
                accel_key: None,
                copyable: false,
            });
        }
        if let Some(ref ccov_root) = tree_config.paths.ccov_root {
            tools_items.push(PanelItem {
                title: "Code Coverage".to_owned(),
                link: format!("{}#revision=latest&path={}&view=file", ccov_root, path),
                update_link_lineno: "&line={}",
                accel_key: None,
                copyable: false,
            });
        }
        if let Some(ref github) = tree_config.paths.github_repo {
            match Path::new(path.as_str()).extension().and_then(OsStr::to_str) {
                Some("md") | Some("rst") => {
                    tools_items.push(PanelItem {
                        title: "Rendered view".to_owned(),
                        link: format!(
                            "{}/blob/{}/{}",
                            github,
                            head_oid.map_or("master".to_string(), |x| x.to_string()),
                            path
                        ),
                        update_link_lineno: "",
                        accel_key: None,
                        copyable: false,
                    });
                }
                _ => (),
            };
        }

        let liquid_globals = liquid::object!({
            "tree": tree_name,
            "path": &path,
            // Propagate config settings that aren't absolute paths.  We do some
            // renaming here compared to `TreeConfigPaths` for clarity.
            "config": {
                "coverage_url": &tree_config.paths.ccov_root.as_deref().unwrap_or(""),
                "github_repo_url": &tree_config.paths.github_repo.as_deref().unwrap_or(""),
                "hg_repo_url": &tree_config.paths.hg_root.as_deref().unwrap_or(""),
                "wpt_root": &tree_config.paths.wpt_root.as_deref().unwrap_or(""),
            },
            "concise": &concise_info,
            "detailed": &detailed_info,
        });

        lazy_static! {
            static ref RE_WHITESPACE_CLEANUP: Regex = Regex::new(r#"(\n *)+\n"#).unwrap();
        }

        let mut source_file_info_boxes = source_file_info_boxes_template
            .render(&liquid_globals)
            .unwrap();
        // It's really difficult to get whitespace right in the templates right now.
        // While it probably makes sense to just pass what we get from this through
        // a formatter in general, for now let's at least just use this exciting
        // regex to clean things up.
        source_file_info_boxes = RE_WHITESPACE_CLEANUP
            .replace_all(&source_file_info_boxes, "\n")
            .to_string();
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

        match format_file_data(
            &cfg,
            tree_name,
            &panel,
            source_file_info_boxes,
            &head_commit,
            &blame_commit,
            &path,
            input,
            &jumpref_lookup_map,
            &analysis,
            &detailed_info.coverage_lines,
            &mut writer,
        ) {
            Ok(perf_info) => {
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
            }
            Err(err) => {
                // Make sure our output log file indicates what happened.
                writeln!(stdout, "  warning: format_file_data failed: {}", err).unwrap();
                // Also embed the error into the output file
                writeln!(writer, "<h3>format_file_data failed: {}</h3>", err).unwrap();
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
