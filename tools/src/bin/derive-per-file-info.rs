use std::collections::BTreeMap;
use std::env;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};

extern crate env_logger;
#[macro_use]
extern crate log;
extern crate tools;
use tools::config;

extern crate rustc_serialize;
use rustc_serialize::json;
use rustc_serialize::json::{Json, ToJson};

fn read_json_from_file(path: &str) -> Option<json::Object> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(&file);
    Json::from_reader(&mut reader).ok()?.into_object()
}

fn write_json_to_file(val: Json, path: &str) -> Option<()> {
    let file = File::create(path).ok()?;
    let mut writer = BufWriter::new(file);
    write!(&mut writer, "{}", val).ok()?;
    Some(())
}

/// Helper to set the provided `key` to the provided `value` in the per-file for
/// the provided `path`, creating intermediary type="dir" nodes as we go.
///
/// This does not know how to set meta-info on directories at this time but can
/// be generalized in the future.
fn store_in_file_value(all_info: &mut json::Object, path: &str, key: &str, val: Json) {
    let mut dir_obj: &mut json::Object = all_info
        .get_mut("root")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .get_mut("contents")
        .unwrap()
        .as_object_mut()
        .unwrap();
    // Walk the path segments, creating intermediary directory nodes
    // implicitly.
    let path_pieces: Vec<String> = path.split('/').map(|s| s.to_string()).collect();
    let (file_part, dir_parts) = path_pieces.split_last().unwrap();
    for path_component in dir_parts {
        let next_val = dir_obj
            .entry(path_component.to_string())
            .or_insert_with(|| {
                let mut child = BTreeMap::new();
                child.insert("type".to_string(), "dir".to_string().to_json());
                child.insert("contents".to_string(), Json::Object(json::Object::new()));
                Json::Object(child)
            });
        dir_obj = next_val
            .as_object_mut()
            .unwrap()
            .get_mut("contents")
            .unwrap()
            .as_object_mut()
            .unwrap();
    }

    let file_val = dir_obj.entry(file_part.to_string()).or_insert_with(|| {
        let mut child = BTreeMap::new();
        child.insert("type".to_string(), "file".to_string().to_json());
        Json::Object(child)
    });
    let file_obj = file_val.as_object_mut().unwrap();
    file_obj.insert(key.to_string(), val);
}

/// Recursive helper to traverse the bugzilla component "paths" hierarchy and
/// propagate its values into the all_info structure.
///
/// - `bz_dir`: This will always be an object whose fields are filenames and
///   values will either be a recursively self-same directory object or a Number
///   which is a bugzilla components index.
/// - `all_node`: This will always be a { type: 'dir', contents } all_info
///   node.
fn traverse_and_store_bugzilla_map(bz_dir: &json::Object, all_node: &mut json::Object) {
    // We never actually want to be altering the metadata of the current
    // all_node, so just immediately access its contents.
    let all_contents = all_node
        .get_mut("contents")
        .unwrap()
        .as_object_mut()
        .unwrap();
    for (filename, value) in bz_dir {
        if value.is_object() {
            // Objects mean the child is a directory as well.
            let all_child = all_contents.entry(filename.to_string()).or_insert_with(|| {
                let mut child = BTreeMap::new();
                child.insert("type".to_string(), "dir".to_string().to_json());
                child.insert("contents".to_string(), Json::Object(json::Object::new()));
                Json::Object(child)
            });
            traverse_and_store_bugzilla_map(
                value.as_object().unwrap(),
                all_child.as_object_mut().unwrap(),
            );
        } else {
            // It's a number and therefore a file.
            let all_child = all_contents.entry(filename.to_string()).or_insert_with(|| {
                let mut child = BTreeMap::new();
                child.insert("type".to_string(), "file".to_string().to_json());
                Json::Object(child)
            });
            let child_obj = all_child.as_object_mut().unwrap();
            child_obj.insert("component".to_string(), value.clone());
        }
    }
}

/// Process a number of JSON input files that provide per-file data into a
/// single aggregated JSON file.  The schema is taken from the bugzilla
/// `components-normalized.json` representation which is a compact/normalized
/// representation.
///
/// The intent of the resulting file is to provide a place for small amounts of
/// meta-information about a file, such as would be appropriate to display and
/// facet search results that match files or display in the directory listing.
///
/// Information that is verbose in nature and specific to the contents of a
/// file, such as code coverage information should instead be stored in separate
/// files-per-file.  This tool could still be responsible for performing the
/// aggregation of that information from disparate sources, however!
///
/// ## Input Files
///
/// ### bugzilla-components.json
///
/// - "components": A dictionary mapping from stringified numeric values to list
///   tuples of the form [product, component].
/// - "paths": A tree where internal nodes are dictionaries corresponding to
///   directories.  Each key is a filename and each value is either another
///   directory dictionary or a non-stringified number corresponding to an entry
///   in the `components` top-level dictionary.
///
/// ### test-info-all-tests.json
///
/// - "description": A string which conveys the date range and tree that this
///   data corresponds to.
/// - "summary": A dictionary with the following keys:
///   - "components"
///   - "failed tests"
///   - "manifests"
///   - "skipped tests"
///   - "tests"
/// - "tests": A dictionary whose keys are bugzilla "Product::Component" strings
///   and values are list of objects with the following keys:
///   - "failed runs": Number
///   - "skip-if" (optional): String excerpt of the manifest's skip-if clause.
///   - "skipped runs": Number
///   - "test": Repository-relative path of the test file.
///   - "total run time, seconds": Floating point number.
///   - "total runs": Number
///
/// ### WPT wpt-metadata-summary.json
///
/// Consult
/// https://searchfox.org/mozilla-central/source/testing/web-platform/tests/tools/wptrunner/wptrunner/manifestexpected.py
/// for
///
/// - [directory]: A WPT-root (testing/web-platform/tests) string identifying a
///   directory containing tests.  Value is an object.
///   - "bug": Corresponds to a `bug: NNN` line in a meta-dir `__dir__.ini` file
///     with value payload `[null, "NNN"]`.
///   - "lsan-allowed": Corresponds to a `__dir__.ini`
///     `lsan-allowed: [Alloc, Create, ...]` line and results in `["Alloc",
///     "Create", ...]`.
///   - "_tests": An object whose keys are test file names.
///     - [test file name]: Value is an object which may contain any of the
///       following keys:
///       - "disabled": An array of 2-tuple arrays, where each 2-tuple is of the
///         form [if-predicate contents, bug URL].  So the line
///         `if (os == "win"): https://bugzilla.mozilla.org/show_bug.cgi?id=NNN`
///         under a "disabled" mochitest ini-format header would result in
///         `["os == \"win\"\n", "https://bugzilla.mozilla.org/show_bug.cgi?id=NNN"]
///         and a line line the following directly under the test name
///         `disabled: https://bugzilla.mozilla.org/show_bug.cgi?id=NNN` gives
///         `[null, "https://bugzilla.mozilla.org/show_bug.cgi?id=NNN"]`.
///         - It appears the bug URL's can just be straight bug numbers or
///           string bug aliases.
///       - "_subtests":
///         - [assertion string]: Payload is an object with optional keys:
///           - "intermittent": An array of nested tuples of the form
///             [condition clause, [ expected values ]].  For example, given
///             `if (processor == "x86") and debug: ["PASS", "FAIL"]` indented
///             beneath an `expected:` header results in
///             `["(processor == \"x86\") and debug\n", ["PASS", "FAIL"]]`.
///             - If this key is not present, then it appears this corresponds
///               to an ini entry of `expected: FAIL`, which would be equivalent
///               to `[null, ["FAIL"]]` I guess.
///       - "max-asserts": [condition?, max-asserts value]
///
/// ## Output File
///
/// For the time being we imitate the bugzilla-components.json representation.
/// A hierarchical tree representation is retained because the ability to
/// perform local lookups, walking up the tree as needed, is useful
/// functionality, as gecko is large enough that global coordination is
/// impractical, but component level coordination is feasible.
///
/// - "bugzilla-components": Directly from bugzilla-components.json, a
///   dictionary mapping from stringified numeric values to list tuples of the
///   form [product, component].
/// - "root": More explicit version of bugzilla-components.json's rep.  Each
///   node in the tree structure is an object with one of the following forms,
///   with the root having type "dir":
///   - { type: "dir", contents } where:
///     - "contents": An object directionary whose keys are filenames and value
///        nodes.
///   - { type: "file", component, testInfo } where:
///     - "component": Value is the numeric bugzilla component to be looked up.
///     - "testInfo": The value nodes from `test-info-alltests.json` verbatim.
///       In the future this may gain more data.
///     - "wptInfo": A digested version of the per-test info.  The presence of
///       this field means that a per-test metadata file exists which is itself
///       an indication of there being some kind of notable manipulation going
///       on even if we don't understand it specifically.
///       - "disabled": Directly propagated from the test-level "disabled".
fn main() {
    env_logger::init();

    let args: Vec<_> = env::args().collect();

    let cfg = config::load(&args[1], true);
    println!("Config file read");

    let tree_name = &args[2];
    let tree_config = cfg.trees.get(tree_name).unwrap();

    // ## Build empty derived info structure
    let mut all_info: json::Object = BTreeMap::new();
    {
        let contents = json::Object::new();
        let mut root = json::Object::new();
        root.insert("type".to_string(), "dir".to_string().to_json());
        root.insert("contents".to_string(), Json::Object(contents));
        all_info.insert("root".to_string(), Json::Object(root));
    }

    // ## Load bugzilla data and merge it in to the derived info structure
    let bugzilla_fname = format!("{}/bugzilla-components.json", tree_config.paths.index_path);
    let bugzilla_data = read_json_from_file(&bugzilla_fname);
    if let Some(mut data) = bugzilla_data {
        info!("Bugzilla components read");

        all_info.insert(
            "bugzilla-components".to_string(),
            data.remove("components").unwrap(),
        );

        if let Some(bz_root) = data.get("paths") {
            traverse_and_store_bugzilla_map(
                bz_root.as_object().unwrap(),
                all_info.get_mut("root").unwrap().as_object_mut().unwrap(),
            );
        }
    } else {
        warn!("No bugzilla-components.json file found");
    }

    // ## Load test info and merge it in to the derived info structure
    let test_info_fname = format!("{}/test-info-all-tests.json", tree_config.paths.index_path);
    let test_info_data = read_json_from_file(&test_info_fname);
    if let Some(mut data) = test_info_data {
        info!("Test info data read");

        if let Some(Json::Object(tests_obj)) = data.remove("tests") {
            for (_, component_tests_value) in tests_obj.into_iter() {
                if let Json::Array(tests_arr) = component_tests_value {
                    for test_info_value in tests_arr.into_iter() {
                        let mut test_info_obj = match test_info_value {
                            Json::Object(obj) => obj,
                            _ => panic!("Test value should be an object."),
                        };
                        let test_path = match test_info_obj.remove("test") {
                            Some(Json::String(str)) => str,
                            _ => panic!("Test `test` field should be present and a string."),
                        };
                        store_in_file_value(
                            &mut all_info,
                            &test_path,
                            "testInfo",
                            Json::Object(test_info_obj),
                        );
                    }
                }
            }
        }
    } else {
        warn!("No test-info-all-tests.json file found");
    }

    // ## Load WPT meta info and merge it in to the derived info structure
    let wpt_info_fname = format!("{}/wpt-metadata-summary.json", tree_config.paths.index_path);
    let wpt_info_data = read_json_from_file(&wpt_info_fname);
    if let (Some(wpt_root), Some(data)) = (tree_config.paths.wpt_root.clone(), wpt_info_data) {
        info!("WPT info read");

        for (dir_path, dir_info) in data.into_iter() {
            // Process only the tests info.  There may be some notable stuff
            // here at the directory's `__dir__.ini` level, but we're not doing
            // anything with it yet.
            if let Some(Json::Object(tests_obj)) = dir_info.into_object().unwrap().remove("_tests")
            {
                for (test_filename, test_info) in tests_obj.into_iter() {
                    let mut propagate = BTreeMap::new();
                    // Process "disabled" which indicates there were file-level
                    // failure disablings.

                    let mut test_obj = test_info.into_object().unwrap();
                    if let Some(conditions) = test_obj.remove("disabled") {
                        propagate.insert("disabled".to_string(), conditions);
                    }

                    if let Some(Json::Object(subtests_obj)) = test_obj.remove("_subtests") {
                        propagate.insert(
                            "subtests_with_conditions".to_string(),
                            Json::U64(subtests_obj.len() as u64),
                        );
                    }

                    store_in_file_value(
                        &mut all_info,
                        &format!("{}/tests/{}/{}", wpt_root, dir_path, test_filename),
                        "wptInfo",
                        Json::Object(propagate),
                    );
                }
            }
        }
    } else {
        warn!("No wpt-metadata-summary.json file found");
    }

    // ## Write out the derived info structure
    let output_fname = format!(
        "{}/derived-per-file-info.json",
        tree_config.paths.index_path
    );
    if write_json_to_file(Json::Object(all_info), &output_fname).is_some() {
        info!("Per-file info written to disk");
    } else {
        warn!("Unable to write per-file info to disk");
    }
}
