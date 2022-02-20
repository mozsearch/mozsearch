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
#[macro_use]
extern crate log;
extern crate tools;
use crate::languages::FormatAs;
use tools::config;
use tools::describe;
use tools::file_format::analysis::{read_analysis, read_jumps, read_source};
use tools::find_source_file;
use tools::format::format_file_data;
use tools::git_ops;
use tools::languages;

use tools::output::{InfoBox, PanelItem, PanelSection, F};

extern crate rustc_serialize;
use rustc_serialize::json;
use rustc_serialize::json::Json;

extern crate flate2;
use flate2::Compression;
use flate2::write::GzEncoder;

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
    per_file_info: &'a json::Object,
) -> Option<(String, String)> {
    let component_id = per_file_info.get("component")?.as_i64()?.to_string();
    let mut result_iter = all_info
        .get("bugzilla-components")?
        .as_object()?
        .get(&component_id)?
        .as_array()?
        .iter();
    let product = result_iter.next()?.as_string()?;
    let component = result_iter.next()?.as_string()?;
    Some((product.to_string(), component.to_string()))
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

fn read_test_info_from_concise_info(concise_info: &json::Object) -> Option<TestInfo> {
    let obj = concise_info.get("testInfo")?.as_object()?;

    let failed_runs = match obj.get("failed runs") {
        Some(json) => json.as_i64().unwrap(),
        _ => 0,
    };
    let skip_if = match obj.get("skip-if") {
        Some(json) => Some(json.as_string().unwrap().to_string()),
        _ => None,
    };
    let skipped_runs = match obj.get("skipped runs") {
        Some(json) => json.as_i64().unwrap(),
        _ => 0,
    };
    let total_run_time_secs = match obj.get("total run time, seconds") {
        Some(json) => json.as_f64().unwrap(),
        _ => 0.0,
    };
    let total_runs = match obj.get("total runs") {
        Some(json) => json.as_i64().unwrap(),
        _ => 0,
    };

    let wpt_expectation_info = match concise_info.get("wptInfo") {
        Some(Json::Object(obj)) => {
            let disabling_conditions = match obj.get("disabled") {
                Some(Json::Array(arr)) => arr
                    .iter()
                    .filter_map(|cond| {
                        // cond itself should be a 2-element array where the first
                        // element is a null or the condition string and the 2nd is
                        // the bug link.
                        match cond.as_array().unwrap_or(&vec![]).as_slice() {
                            // null means there was no condition, it's always disabled.
                            [Json::Null, Json::String(b)] => {
                                Some(("ALWAYS".to_string(), b.to_string()))
                            }
                            [Json::String(a), Json::String(b)] => {
                                Some((a.to_string(), b.to_string()))
                            }
                            // I guess this is just covering up our failures?  I'm
                            // sorta tired of this patch though, so... cover up our
                            // failures.
                            _ => {
                                warn!("Unhandled disabled condition JSON branch! {:?}", cond);
                                None
                            }
                        }
                    })
                    .collect(),
                _ => vec![],
            };

            let disabled_subtests_count = match obj.get("subtests_with_conditions") {
                Some(json) => json.as_i64().unwrap(),
                _ => 0,
            };

            Some(WPTExpectationInfo {
                disabling_conditions,
                disabled_subtests_count,
            })
        }
        _ => None,
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

/// Per-file info derived from the concise and detailed info for a given file.
/// Everything in here is optional data, but this structure will be available
/// for every file to simplify control-flow.
struct PerFileInfo {
    bugzilla_component: Option<(String, String)>,
    test_info: Option<TestInfo>,
    coverage: Option<Vec<i32>>,
}

fn get_concise_file_info<'a>(all_concise_info: &'a json::Object, path: &str) -> Option<&'a json::Object> {
    let mut cur_obj = all_concise_info.get("root")?.as_object()?;

    for path_component in path.split('/') {
        // The current node must be a directory, get its contents.
        let dir_obj = cur_obj.get("contents")?.as_object()?;
        // And now find the next node inside the components
        cur_obj = dir_obj.get(path_component)?.as_object()?;
    }

    Some(cur_obj)
}

fn read_detailed_file_info(path: &str, index_path: &str) -> Option<json::Object> {
    let detailed_file_info_fname = format!(
        "{}/detailed-per-file-info/{}",
        index_path,
        path
    );
    read_json_from_file(&detailed_file_info_fname)
}

/// Interpolate coverage hits/misses for lines that didn't have coverage data,
/// as indicated by a -1.
///
/// Given coverage data where values are either -1 indicating no coverage or are
/// a coverage value >= 0, replace the -1 values with explicit interpolated hit
/// miss values:
/// * `-3`: Interpolated miss.
/// * `-2`: Interpolated hit.
///
/// The choice of using these additional values is because this might be
/// something that the upstream generator of the coverage data might do in the
/// future, and it already uses -1 as a magic value.
///
/// The goal of this interpolation is to minimize visual noise in the coverage
/// data.  Transitions to and from the uncovered (-1) state are informative but
/// are distracting and limit the ability to use preattentive processing
/// (see https://www.csc2.ncsu.edu/faculty/healey/PP/) to pick the more relevant
/// transitions between covered/uncovered.
///
/// It's straightforward to interpolate hits when there's an uncovered gap
/// between hits and likewise miss when there's an uncovered gap between misses.
/// The interesting questions are:
/// - What to do the start and end of the file.
/// - What to do when the uncovered gap is between a hit and a miss.  Extra
///   information about the AST or nesting contexts might help
///
/// Our arbitrary decisions here are:
/// - Leave the starts and ends of file uncovered.  This is more realistic but
///   at the cost of this realism making it less obvious that interpolation is
///   present in the rest of the file, potentially leading to bad inferences.
///   - We attempt to mitigate this by making sure the hover information makes
///     it clear when interpolation is at play so if someone looks into what's
///     going on they at least aren't misled for too long.
/// - Maximally interpolate hits over misses.  Our goal is that people's eyes
///   are drawn to misses.  This interpolation strategy makes sure that the
///   start and end of a run of misses are lines that are explicitly detected
///   as misses.
///
fn interpolate_coverage(mut raw: Vec<i32>) -> Vec<i32> {
    // We don't interpolate at the start or end of files, so start with already
    // having a valid -1 interpolation value.
    let mut have_interp_val = true;
    let mut interp_val = -1;
    // This value will never be used because we set have_interp_val to true
    // above which means we won't calculate an interpretation with this value.
    let mut last_noninterp_val = -1;
    for i in 0..raw.len() {
        let val = raw[i];
        // If we have a valid coverage value (=0 is miss, >0 is hit) then leave
        // the value as is, remember this value for interpolation and note that
        // we'll need to compute our next interpolation value.
        if val >= 0 {
            last_noninterp_val = val;
            have_interp_val = false;
            continue;
        }
        // Not a valid value, so we need to interpolate.

        // Did we already calculate our interpolation value?  If so, keep using
        // it.  (Note that at the start of the file we start our overwriting -1
        // with -1.)
        if have_interp_val {
            raw[i] = interp_val;
            continue;
        }

        // Check the next lines until we find a value that's >= 0.  If we don't
        // find any, then our end-of-file logic wants us to maintain a -1, so
        // configure for that base-case.
        have_interp_val = true;
        interp_val = -1;
        for j in (i + 1)..raw.len() {
            let next_val = raw[j];
            if next_val == -1 {
                continue;
            }
            // We've found a value which means that both last_noninterp_val and
            // next_val are >= 0.  (last_noninterp_val can never be -1 because
            // we start the loop with have_interp_val=true.)

            // Favor hits over misses (see func doc block for rationale).
            if next_val > 0 || last_noninterp_val > 0 {
                interp_val = -2;
            } else {
                interp_val = -3;
            }
            break;
        }
        raw[i] = interp_val;
    }
    raw
}

#[test]
fn test_interpolate_coverage() {
    let cases = vec![
        // empty
        vec![
            vec![],
            vec![]
        ],
        // interpolate a hit between two hits
        vec![
            vec![1, -1, 1],
            vec![1, -2, 1]
        ],
        // interpolate a miss between two misses
        vec![
            vec![0, -1, 0],
            vec![0, -3, 0]
        ],
        // interpolate a hit if there's a hit on either side
        vec![
            vec![1, -1, 0, -1, 1],
            vec![1, -2, 0, -2, 1]
        ],
        // don't interpolate ends
        vec![
            vec![-1, 1, -1, 1, -1],
            vec![-1, 1, -2, 1, -1]
        ],
        // don't interpolate if the whole file is uncovered
        vec![
            vec![-1, -1, -1, -1, -1],
            vec![-1, -1, -1, -1, -1]
        ],
        // combine all of the above (except for whole file), single interp each.
        vec![
            vec![-1, -1, 0, -1, 0, -1, 1, -1, 1, -1, 1, -1, 0, -1],
            vec![-1, -1, 0, -3, 0, -2, 1, -2, 1, -2, 1, -2, 0, -1]
        ],
        // now double the length of the interpolation runs
        vec![
            vec![-1, -1, 0, -1, -1, 0, -1, -1, 1, -1, -1, 1, -1, -1, 1, -1, -1, 0, -1],
            vec![-1, -1, 0, -3, -3, 0, -2, -2, 1, -2, -2, 1, -2, -2, 1, -2, -2, 0, -1]
        ],
        // now triple!
        vec![
            vec![-1, -1, 0, -1, -1, -1, 0, -1, -1, -1, 1, -1, -1, -1, 1, -1, -1, -1, 1, -1, -1, -1, 0, -1],
            vec![-1, -1, 0, -3, -3, -3, 0, -2, -2, -2, 1, -2, -2, -2, 1, -2, -2, -2, 1, -2, -2, -2, 0, -1]
        ],
        // add some runs of non-interpolated values to make sure we don't randomly clobber data.
        vec![
            vec![1, 2, 4, -1, 8, 16, 32, -1, -1, 64, 0, 0, -1, 0, 128, 256, -1, 512],
            vec![1, 2, 4, -2, 8, 16, 32, -2, -2, 64, 0, 0, -3, 0, 128, 256, -2, 512]
        ],
    ];

    for pair in cases {
        assert_eq!(
            interpolate_coverage(pair[0].clone()),
            pair[1]
        );
    }

}

/// Extract any per-file info from the concise info aggregate object plus
/// anything in the individual detailed file if it exists.
fn get_per_file_info(all_concise_info: &json::Object, path: &str, index_path: &str) -> PerFileInfo {
    let (bugzilla_component, test_info) = match get_concise_file_info(all_concise_info, path) {
        Some(concise_info) => (
            get_bugzilla_component(all_concise_info, concise_info),
            read_test_info_from_concise_info(concise_info)
        ),
        None => (None, None),
    };

    let coverage = match read_detailed_file_info(path, index_path) {
        Some(mut detailed_obj) => {
            match detailed_obj.remove("lineCoverage") {
                Some(Json::Array(arr)) => Some(interpolate_coverage(arr.iter().map(|x| x.as_i64().unwrap_or(-1) as i32).collect())),
                _ => None,
            }
        },
        None => None,
    };

    PerFileInfo {
        bugzilla_component,
        test_info,
        coverage,
    }
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

    let all_file_info_fname = format!(
        "{}/concise-per-file-info.json",
        tree_config.paths.index_path
    );
    let all_file_info_data = match read_json_from_file(&all_file_info_fname) {
        Some(data) => {
            println!("Per-file info read");
            data
        },
        None => {
            println!("No concise-per-file-info.json file found");
            json::Object::new()
        }
    };

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
        let gzip_output_fname = format!("{}.gz", output_fname);
        let source_fname = find_source_file(
            path,
            &tree_config.paths.files_path,
            &tree_config.paths.objdir_path,
        );

        let format = languages::select_formatting(path);

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

        let per_file_info = get_per_file_info(
            &all_file_info_data,
            path,
            &tree_config.paths.index_path);

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
            &per_file_info.coverage,
            &mut writer,
            Some(&mut diff_cache),
        )
        .unwrap();

        writer.finish().unwrap();
    }
}
