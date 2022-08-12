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
use serde_json::to_writer;
use tools::file_format::per_file_info::derive_description;
use tools::file_format::per_file_info::get_per_file_info;
use tools::file_format::per_file_info::read_json_from_file;

extern crate env_logger;
extern crate log;
extern crate tools;
use crate::languages::FormatAs;
use tools::config;
use tools::describe;
use tools::file_format::analysis::{read_analysis, read_jumps, read_source};
use tools::find_source_file;
use tools::format::{format_file_data, create_markdown_panel_section};
use tools::languages;

use tools::output::{InfoBox, PanelItem, PanelSection, F};

extern crate rustc_serialize;
use rustc_serialize::json;

extern crate flate2;
use flate2::Compression;
use flate2::write::GzEncoder;

/// Some fields support free-form indications of what bug is present.  This
/// could be a bug number or a bug URL.  For simplicity we pass-through things
/// that look like http URLs and prepend the bug id for everything else.
fn ensure_bugzilla_url(maybe_bug: &str) -> String {
    if maybe_bug.starts_with("http") {
        maybe_bug.to_string()
    } else {
        format!("https://bugzilla.mozilla.org/show_bug.cgi?id={}", maybe_bug)
    }
}

fn normalize_skip_if(skip_if: &str) -> String {
    lazy_static! {
        static ref RE_NEWLINES: Regex = Regex::new("\n+").unwrap();
    }

    RE_NEWLINES.replace_all(skip_if, " OR ").to_string()
}

fn main() {
    env_logger::init();

    let args: Vec<_> = env::args().collect();
    // TODO: refactor _fname_args out of existence; we now require paths to come via stdin.
    let (base_args, _fname_args) = args.split_at(3);

    let mut stdout = io::stdout().lock();

    let pre_config = Instant::now();
    let tree_name = &base_args[2];
    let cfg = config::load(&base_args[1], true, Some(&tree_name));
    writeln!(stdout, "Config file read, duration: {}us", pre_config.elapsed().as_micros() as u64).unwrap();

    let tree_config = cfg.trees.get(tree_name).unwrap();

    let pre_jumps = Instant::now();
    let jumps_fname = format!("{}/jumps", tree_config.paths.index_path);
    //let jumps : std::collections::HashMap<String, tools::analysis::Jump> = std::collections::HashMap::new();
    let jumps = read_jumps(&jumps_fname);
    writeln!(stdout, "Jumps read, duration: {}us", pre_jumps.elapsed().as_micros() as u64).unwrap();

    let pre_per_file = Instant::now();
    let all_file_info_fname = format!(
        "{}/concise-per-file-info.json",
        tree_config.paths.index_path
    );
    let all_file_info_data = match read_json_from_file(&all_file_info_fname) {
        Some(data) => {
            writeln!(stdout, "Per-file info read, duration: {}us", pre_per_file.elapsed().as_micros() as u64).unwrap();
            data
        },
        None => {
            writeln!(stdout, "No concise-per-file-info.json file found").unwrap();
            json::Object::new()
        }
    };

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

    writeln!(stdout, "Blame prep done, duration: {}us", pre_blame_prep.elapsed().as_micros() as u64).unwrap();

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
        let path = path_buf.trim_end();
        let file_start = Instant::now();
        writeln!(stdout, "File '{}'", path).unwrap();

        let output_fname = format!("{}/file/{}", tree_config.paths.index_path, path);
        let gzip_output_fname = format!("{}.gz", output_fname);
        let source_fname = find_source_file(
            &path,
            &tree_config.paths.files_path,
            &tree_config.paths.objdir_path,
        );

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
        writeln!(stdout, "  Analysis load duration: {}us", pre_analysis_load.elapsed().as_micros() as u64).unwrap();

        let pre_per_file_info = Instant::now();
        let per_file_info = get_per_file_info(
            &all_file_info_data,
            &path,
            &tree_config.paths.index_path);
        writeln!(stdout, "  Per-file info load duration: {}us", pre_per_file_info.elapsed().as_micros() as u64).unwrap();

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
                        writeln!(stdout, "Unable to read source file '{}': {:?}", source_fname, e).unwrap();
                        continue;
                    }
                }
            }
        }
        writeln!(stdout, "  File contents read duration: {}us", pre_file_read.elapsed().as_micros() as u64).unwrap();

        let pre_describe_file = Instant::now();
        if let Some(str_description) = describe::describe_file(&input, &path_wrapper, &format) {
            let description_fname =
                format!("{}/description/{}", tree_config.paths.index_path, path);
            let description_file = File::create(description_fname).unwrap();
            let desc_writer = BufWriter::new(description_file);
            let file_description = derive_description(str_description, &metadata, &per_file_info);
            to_writer(desc_writer, &file_description).unwrap();
        }
        writeln!(stdout, "  File described duration: {}us", pre_describe_file.elapsed().as_micros() as u64).unwrap();

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
        let mut info_boxes = vec![];

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
            if let Some((product, component)) = per_file_info.bugzilla_component {
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
            match Path::new(&path).extension().and_then(OsStr::to_str) {
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
        // Defer pushing "Other Tools" until after the test processing so that
        // we can add a wpt.fyi link as appropriate.

        if let Some(test_info) = per_file_info.test_info {
            let mut list_nodes: Vec<F> = vec![];

            let mut has_quieted_warnings = false;
            let mut has_warnings = false;
            let mut has_errors = false;

            // TODO: Add commas to numbers.  This is a localization issue, but
            // we're also hard-coding English in here so my pragmatic solution
            // is to punt.  https://crates.io/crates/fluent is the most correct
            // (Mozilla project) answer but... I think we're still planning to
            // hard-code en-US/en-CA.

            if let Some(skip_if) = test_info.skip_if {
                // Don't be as dramatic about the fission case because there are
                // frequently test cases that intentionally cover the fission
                // and non-fission cases AND the fission team has been VERY
                // proactive about tracking and fixing these issues, so there's
                // no need to be loud about it.
                if skip_if.eq_ignore_ascii_case("fission")
                    || skip_if.eq_ignore_ascii_case("!fission")
                {
                    has_quieted_warnings = true;
                } else {
                    has_warnings = true;
                }
                list_nodes.push(F::T(format!(
                    r#"<li>This test gets skipped with pattern: <span class="test-skip-info">{}</span></li>"#,
                    normalize_skip_if(&skip_if)
                )));
            }

            if test_info.skipped_runs > 0 {
                // We leave the warning logic to the skip_if check because it
                // can avoid escalating the "!fission" case.
                list_nodes.push(F::T(format!(
                    "<li>This test was skipped {} times in the preceding 7 days.</li>",
                    test_info.skipped_runs
                )));
            }

            if test_info.failed_runs > 0 {
                has_errors = true;
                list_nodes.push(F::T(format!(
                    r#"<li>This test failed {} times in the preceding 30 days. <a href="https://bugzilla.mozilla.org/buglist.cgi?quicksearch={}">quicksearch this test</a></li>"#,
                    test_info.failed_runs,
                    &path
                )));
            }

            // ### WPT cases (happens regardless of existence of meta .ini)
            if let Some(wpt_root) = &tree_config.paths.wpt_root {
                let wpt_test_root = format!("{}/tests/", wpt_root);
                if let Some(wpt_test_path) = path.strip_prefix(&wpt_test_root) {
                    tools_items.push(PanelItem {
                        title: "Web Platform Tests Dashboard".to_owned(),
                        link: format!("https://wpt.fyi/results/{}", wpt_test_path),
                        update_link_lineno: "",
                        accel_key: None,
                        copyable: true,
                    });
                }
            }

            // ### WPT Expectation Info (only happens when there's a meta .ini)
            if let Some(wpt_info) = test_info.wpt_expectation_info {
                // Meta files do not exist for good reasons, this counts as a
                // warning.
                has_warnings = true;

                // The existence of this structure means that a meta file
                // exists, and we know that the first incidence of /tests/
                // when replaced with /meta/ is that path.  We don't need to
                // bother with strip_prefix and rebuilding paths, although if we
                // were creating URLs to services, we would want that.
                let meta_ini_path = path.replacen("/tests/", "/meta/", 1);
                let meta_url = format!(
                    // The .ini extension gets appended onto the path, retaining
                    // the existing file extension.
                    r#"/{}/source/{}.ini"#,
                    tree_name,
                    meta_ini_path,
                );

                // Track whether we emitted something with the meta URL.
                let mut linked_meta = false;

                if wpt_info.disabling_conditions.len() > 0 {
                    linked_meta = true;
                    list_nodes.push(F::Seq(vec![
                        F::S("<li>"),
                        F::Indent(vec![
                            F::T(format!(
                                r#"This test has a <a href="{}">WPT meta file</a> that disables it given conditions:"#,
                                meta_url
                            )),
                            F::S("<ul>"),
                            F::Indent(wpt_info.disabling_conditions.iter().map(|(cond, bug)| {
                                F::T(format!(
                                    r#"<li><span class="test-skip-info">{}</span>&nbsp; : <a href="{}">{}</a></li>"#,
                                    // The condition text can embed newlines at the end.
                                    cond.trim(),
                                    ensure_bugzilla_url(bug),
                                    bug,
                                ))
                            }).collect()),
                            F::S("</ul>"),
                        ]),
                        F::S("</li>"),
                    ]));
                }

                if wpt_info.disabled_subtests_count > 0 {
                    linked_meta = true;
                    list_nodes.push(F::T(format!(
                        r#"<li>This test has a <a href="{}">WPT meta file</a> that expects {} subtest issues."#,
                        meta_url,
                        wpt_info.disabled_subtests_count,
                    )));
                }

                // If we didn't emit bullets above that have the link, then emit a vague message
                // with the link.
                if !linked_meta {
                    list_nodes.push(F::T(format!(
                        r#"<li>This test has a <a href="{}">WPT meta file</a> for some reason."#,
                        meta_url,
                    )));
                }
            }

            if test_info.unskipped_runs > 0 {
                list_nodes.push(F::T(format!(
                    "<li>This test ran {} times in the preceding 7 days with an average run time of {:.2} secs.</li>",
                    test_info.unskipped_runs,
                    test_info.total_run_time_secs / (test_info.unskipped_runs as f64),
                )));
            }

            // box_kind is used for styling, currently naive red-green
            // color-blindness unfriendly background colors, but hopefully with
            // distinct shape icon badges in the future.  (The fancy branch has
            // icons available.)
            //
            // heading_html changes in parallel for screen readers and to
            // address the red-green color-blindness issue above.
            let (heading_html, box_kind) = if has_errors {
                ("Test Info: Errors", "error")
            } else if has_warnings {
                ("Test Info: Warnings", "warning")
            } else if has_quieted_warnings {
                ("Test Info: FYI", "info")
            } else {
                ("Test Info:", "info")
            };

            info_boxes.push(InfoBox {
                heading_html: heading_html.to_string(),
                body_nodes: vec![F::S("<ul>"), F::Indent(list_nodes), F::S("</ul>")],
                box_kind: box_kind.to_string(),
            });
        }

        if !tools_items.is_empty() {
            panel.push(PanelSection {
                name: "Other Tools".to_owned(),
                items: tools_items,
            });
        }

        match format_file_data(
            &cfg,
            tree_name,
            &panel,
            &info_boxes,
            &head_commit,
            &blame_commit,
            &path,
            input,
            &jumps,
            &analysis,
            &per_file_info.coverage,
            &mut writer,
        ) {
            Ok(perf_info) => {
                writeln!(stdout, "  Format code duration: {}us", perf_info.format_code_duration_us).unwrap();
                writeln!(stdout, "  Blame lines duration: {}us", perf_info.blame_lines_duration_us).unwrap();
                writeln!(stdout, "  Commit info duration: {}us", perf_info.commit_info_duration_us).unwrap();
                writeln!(stdout, "  Format mixing duration: {}us", perf_info.format_mixing_duration_us).unwrap();
            }
            Err(err) => {
                // Make sure our output log file indicates what happened.
                writeln!(stdout, "  warning: format_file_data failed: {}", err).unwrap();
                // Also embed the error into the output file
                writeln!(writer, "<h3>format_file_data failed: {}</h3>", err).unwrap();
            }
        }

        writer.finish().unwrap();
        writeln!(stdout, "  Total writing duration: {}us", file_start.elapsed().as_micros() as u64).unwrap();
    }
    writeln!(stdout, "Done writing files.").unwrap();
}
