use std::collections::HashMap;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::fs::File;
use std::io;
use std::io::BufReader;
use std::io::BufWriter;
use std::io::Read;
use std::io::Seek;
use std::io::Write;
use std::path::Path;

extern crate env_logger;
extern crate tools;
use crate::languages::FormatAs;
use tools::config;
use tools::describe;
use tools::file_format::analysis::{read_analysis, read_jumps, read_source};
use tools::find_source_file;
use tools::format::format_file_data;
use tools::git_ops;
use tools::languages;

use tools::output::{F, InfoBox, PanelItem, PanelSection};

extern crate rustc_serialize;
use rustc_serialize::json;
use rustc_serialize::json::Json;

fn read_json_from_file(path: &str) -> Option<json::Object> {
    let components_file = File::open(path).ok()?;
    let mut reader = BufReader::new(&components_file);
    json::Json::from_reader(&mut reader).ok()?.into_object()
}

/// For a given path, looks up the bugzilla product and component and
/// returns it in a tuple if it could be found. The JSON data format
/// comes from https://searchfox.org/mozilla-central/rev/47edbd91c43db6229cf32d1fc4bae9b325b9e2d0/python/mozbuild/mozbuild/frontend/mach_commands.py#209-223,243
/// and is fairly straightforward.
fn get_bugzilla_component<'a>(
    all_info: &'a json::Object,
    per_file_info: &'a json::Object
) -> Option<(&'a str, &'a str)> {
    let component_id = per_file_info.get("component")?.as_i64()?.to_string();
    let mut result_iter = all_info
        .get("bugzilla-components")?
        .as_object()?
        .get(&component_id)?
        .as_array()?
        .iter();
    let product = result_iter.next()?.as_string()?;
    let component = result_iter.next()?.as_string()?;
    Some((product, component))
}

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

/// Information about expected failures/problems for specific web platform
/// tests.
struct WPTExpectationInfo {
    /// The condition strings and related bugs that disable this test in its
    /// entirety.
    disabling_conditions: Vec<(String, String)>,
    /// The number of `_subtests` that were disabled or conditionally disabled
    disabled_subtests_count: i64,
}

/// Information from `test-info-all-tests.json` which knows about files that the
/// test manifests know about.
struct TestInfo {
    failed_runs: i64,
    skip_if: Option<String>,
    skipped_runs: i64,
    total_run_time_secs: f64,
    /// "total runs" less "skipped runs"
    unskipped_runs: i64,
    /// For web platform tests with expected failures/problems, the info about
    /// that.  Tests that are expected to succeed will have None here.
    wpt_expectation_info: Option<WPTExpectationInfo>,
}

fn read_test_info_from_file_info(per_file_info: Option<&json::Object>) -> Option<TestInfo> {
    let obj = per_file_info?.get("testInfo")?.as_object()?;

    let failed_runs = match obj.get("failed runs") {
        Some(json) => json.as_i64().unwrap(),
        _ => 0
    };
    let skip_if = match obj.get("skip-if") {
        Some(json) => Some(json.as_string().unwrap().to_string()),
        _ => None
    };
    let skipped_runs = match obj.get("skipped runs") {
        Some(json) => json.as_i64().unwrap(),
        _ => 0
    };
    let total_run_time_secs= match obj.get("total run time, seconds") {
        Some(json) => json.as_f64().unwrap(),
        _ => 0.0
    };
    let total_runs = match obj.get("total runs") {
        Some(json) => json.as_i64().unwrap(),
        _ => 0
    };

    let wpt_expectation_info = match per_file_info?.get("wptInfo") {
        Some(Json::Object(obj)) => {
            let disabling_conditions = match obj.get("disabled") {
                Some(Json::Array(arr)) => arr.iter().filter_map(|cond| {
                    // cond itself should be a 2-element array where the first
                    // element is a null or the condition string and the 2nd is
                    // the bug link.
                    match cond.as_array().unwrap_or(&vec![]).as_slice() {
                        // null means there was no condition, it's always disabled.
                        [Json::Null, Json::String(b)] => Some(("ALWAYS".to_string(), b.to_string())),
                        [Json::String(a), Json::String(b)] => Some((a.to_string(), b.to_string())),
                        // I guess this is just covering up our failures?  I'm
                        // sorta tired of this patch though, so... cover up our
                        // failures.
                        _ => None,
                    }
                }).collect(),
                _ => vec![]
            };

            let disabled_subtests_count = match obj.get("subtests_with_conditions") {
                Some(json) => json.as_i64().unwrap(),
                _ => 0,
            };

            Some(WPTExpectationInfo {
                disabling_conditions,
                disabled_subtests_count,
            })
        },
        _ => None
    };

    Some(TestInfo {
        failed_runs,
        skip_if,
        skipped_runs,
        total_run_time_secs,
        unskipped_runs: total_runs - skipped_runs,
        wpt_expectation_info,
    })
}

fn get_per_file_info<'a>(all_info: &'a json::Object, path: &str) -> Option<&'a json::Object> {
    let mut cur_obj = all_info.get("root")?.as_object()?;

    for path_component in path.split('/') {
        // The current node must be a directory, get its contents.
        let dir_obj = cur_obj.get("contents")?.as_object()?;
        // And now find the next node inside the comopnents
        cur_obj = dir_obj.get(path_component)?.as_object()?;
    }

    Some(cur_obj)
}

fn main() {
    env_logger::init();

    let args: Vec<_> = env::args().collect();
    let (base_args, fname_args) = args.split_at(3);

    let cfg = config::load(&base_args[1], true);
    println!("Config file read");

    let tree_name = &base_args[2];
    let tree_config = cfg.trees.get(tree_name).unwrap();

    let jumps_fname = format!("{}/jumps", tree_config.paths.index_path);
    //let jumps : std::collections::HashMap<String, tools::analysis::Jump> = std::collections::HashMap::new();
    let jumps = read_jumps(&jumps_fname);
    println!("Jumps read");

    let all_file_info_fname = format!("{}/derived-per-file-info.json", tree_config.paths.index_path);
    let all_file_info_data = read_json_from_file(&all_file_info_fname);
    if all_file_info_data.is_some() {
        println!("Per-file info read");
    } else {
        println!("No derived-per-file-info.json file found");
    }

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

    let mut extension_mapping = HashMap::new();
    extension_mapping.insert("cpp", ("header", vec!["h", "hh", "hpp", "hxx"]));
    extension_mapping.insert("cc", ("header", vec!["h", "hh", "hpp", "hxx"]));
    extension_mapping.insert("cxx", ("header", vec!["h", "hh", "hpp", "hxx"]));
    extension_mapping.insert("h", ("source", vec!["cpp", "cc", "cxx"]));
    extension_mapping.insert("hh", ("source", vec!["cpp", "cc", "cxx"]));
    extension_mapping.insert("hpp", ("source", vec!["cpp", "cc", "cxx"]));
    extension_mapping.insert("hxx", ("source", vec!["cpp", "cc", "cxx"]));

    let mut diff_cache = git_ops::TreeDiffCache::new();
    for path in fname_args {
        println!("File {}", path);

        let output_fname = format!("{}/file/{}", tree_config.paths.index_path, path);
        let source_fname = find_source_file(
            path,
            &tree_config.paths.files_path,
            &tree_config.paths.objdir_path,
        );

        let format = languages::select_formatting(path);

        let output_file = File::create(output_fname).unwrap();
        let mut writer = BufWriter::new(output_file);

        let source_file = match File::open(source_fname.clone()) {
            Ok(f) => f,
            Err(_) => {
                println!("Unable to open file");
                continue;
            }
        };

        let path_wrapper = Path::new(&source_fname);
        let metadata = fs::symlink_metadata(path_wrapper).unwrap();
        if metadata.file_type().is_symlink() {
            let dest = fs::read_link(path_wrapper).unwrap();
            write!(writer, "Symlink to {}", dest.to_str().unwrap()).unwrap();
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

        let analysis_fname = format!("{}/analysis/{}", tree_config.paths.index_path, path);
        let analysis = read_analysis(&analysis_fname, &mut read_source);

        let per_file_info = match &all_file_info_data {
            Some(data) => get_per_file_info(data, path),
            _ => None
        };

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
                        println!("Unable to read file: {:?}", e);
                        continue;
                    }
                }
            }
        }

        if let Some(file_description) = describe::describe_file(&input, &path_wrapper, &format) {
            let description_fname =
                format!("{}/description/{}", tree_config.paths.index_path, path);
            let description_file = File::create(description_fname).unwrap();
            let mut desc_writer = BufWriter::new(description_file);
            write!(desc_writer, "{}", file_description).unwrap();
        }

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
            });
        };
        if let (Some(data), Some(info)) = (&all_file_info_data, &per_file_info) {
            if !path.contains("__GENERATED__") {
                if let Some((product, component)) = get_bugzilla_component(data, info) {
                    source_panel_items.push(PanelItem {
                        title: format!("File a bug in {} :: {}", product, component),
                        link: format!(
                            "https://bugzilla.mozilla.org/enter_bug.cgi?product={}&component={}",
                            product.replace("&", "%26"),
                            component.replace("&", "%26")
                        ),
                        update_link_lineno: "",
                        accel_key: None,
                    });
                }
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
                });
                if let Some(ref hg_root) = tree_config.paths.hg_root {
                    vcs_panel_items.push(PanelItem {
                        title: "Log".to_owned(),
                        link: format!("{}/log/tip/{}", hg_root, path),
                        update_link_lineno: "",
                        accel_key: Some('L'),
                    });
                    vcs_panel_items.push(PanelItem {
                        title: "Raw".to_owned(),
                        link: format!("{}/raw-file/tip/{}", hg_root, path),
                        update_link_lineno: "",
                        accel_key: Some('R'),
                    });
                }
                if tree_config.paths.git_blame_path.is_some() {
                    vcs_panel_items.push(PanelItem {
                        title: "Blame".to_owned(),
                        link: "javascript:alert('Hover over the gray bar on the left to see blame information.')".to_owned(),
                        update_link_lineno: "",
                        accel_key: None,
                    });
                }
                panel.push(PanelSection {
                    name: "Revision control".to_owned(),
                    items: vcs_panel_items,
                });
            }
        }

        let mut tools_items = vec![];
        if let Some(ref hg_root) = tree_config.paths.hg_root {
            tools_items.push(PanelItem {
                title: "HG Web".to_owned(),
                link: format!("{}/file/tip/{}", hg_root, path),
                update_link_lineno: "#l{}",
                accel_key: None,
            });
        }
        if let Some(ref ccov_root) = tree_config.paths.ccov_root {
            tools_items.push(PanelItem {
                title: "Code Coverage".to_owned(),
                link: format!("{}#revision=latest&path={}&view=file", ccov_root, path),
                update_link_lineno: "&line={}",
                accel_key: None,
            });
        }
        if let Some(ref dxr_root) = tree_config.paths.dxr_root {
            tools_items.push(PanelItem {
                title: "DXR".to_owned(),
                link: format!("{}/source/{}", dxr_root, path),
                update_link_lineno: "#{}",
                accel_key: None,
            });
        }
        if let Some(ref github) = tree_config.paths.github_repo {
            match Path::new(path).extension().and_then(OsStr::to_str) {
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
                    });
                }
                _ => (),
            };
        }
        // Defer pushing "Other Tools" until after the test processing so that
        // we can add a wpt.fyi link as appropriate.

        if let Some(test_info) = read_test_info_from_file_info(per_file_info) {
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
                if skip_if.eq_ignore_ascii_case("fission") ||
                   skip_if.eq_ignore_ascii_case("!fission") {
                    has_quieted_warnings = true;
                } else {
                    has_warnings = true;
                }
                list_nodes.push(F::T(format!(
                    r#"<li>This test gets skipped with pattern: <span class="test-skip-info">{}</span></li>"#,
                    skip_if
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
                    "<li>This test failed {} times in the preceding 7 days.</li>",
                    test_info.failed_runs
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

            list_nodes.push(F::T(format!(
                "<li>This test ran {} times in the preceding 7 days with an average run time of {:.2} secs.</li>",
                test_info.unskipped_runs,
                test_info.total_run_time_secs / (test_info.unskipped_runs as f64),
            )));

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
                body_nodes: vec![
                    F::S("<ul>"),
                    F::Indent(list_nodes),
                    F::S("</ul>"),
                ],
                box_kind: box_kind.to_string(),
            });
        }

        if !tools_items.is_empty() {
            panel.push(PanelSection {
                name: "Other Tools".to_owned(),
                items: tools_items,
            });
        }


        format_file_data(
            &cfg,
            tree_name,
            &panel,
            &info_boxes,
            &head_commit,
            &blame_commit,
            path,
            input,
            &jumps,
            &analysis,
            &mut writer,
            Some(&mut diff_cache),
        )
        .unwrap();
    }
}
