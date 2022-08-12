use std::fs::File;
use std::io::BufReader;

extern crate rustc_serialize;
use rustc_serialize::json;
use rustc_serialize::json::Json;

// TODO: Hey, this should all have been converted to serde already!
pub fn read_json_from_file(path: &str) -> Option<json::Object> {
    let components_file = File::open(path).ok()?;
    let mut reader = BufReader::new(&components_file);
    json::Json::from_reader(&mut reader).ok()?.into_object()
}

/// For a given path, looks up the bugzilla product and component and
/// returns it in a tuple if it could be found. The JSON data format
/// comes from https://searchfox.org/mozilla-central/rev/47edbd91c43db6229cf32d1fc4bae9b325b9e2d0/python/mozbuild/mozbuild/frontend/mach_commands.py#209-223,243
/// and is fairly straightforward.
pub fn get_bugzilla_component<'a>(
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

/// Information about expected failures/problems for specific web platform
/// tests.
pub struct WPTExpectationInfo {
    /// The condition strings and related bugs that disable this test in its
    /// entirety.
    pub disabling_conditions: Vec<(String, String)>,
    /// The number of `_subtests` that were disabled or conditionally disabled
    pub disabled_subtests_count: i64,
}

/// Information from `test-info-all-tests.json` which knows about files that the
/// test manifests know about.
pub struct TestInfo {
    pub failed_runs: i64,
    pub skip_if: Option<String>,
    pub skipped_runs: i64,
    pub total_run_time_secs: f64,
    /// "total runs" less "skipped runs"
    pub unskipped_runs: i64,
    /// For web platform tests with expected failures/problems, the info about
    /// that.  Tests that are expected to succeed will have None here.
    pub wpt_expectation_info: Option<WPTExpectationInfo>,
}

pub fn read_test_info_from_concise_info(concise_info: &json::Object) -> Option<TestInfo> {
    let obj = concise_info.get("testInfo")?.as_object()?;

    let failed_runs = match obj.get("failure_count") {
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
pub struct PerFileInfo {
    pub bugzilla_component: Option<(String, String)>,
    pub test_info: Option<TestInfo>,
    pub coverage: Option<Vec<i32>>,
}

pub fn get_concise_file_info<'a>(all_concise_info: &'a json::Object, path: &str) -> Option<&'a json::Object> {
    let mut cur_obj = all_concise_info.get("root")?.as_object()?;

    for path_component in path.split('/') {
        // The current node must be a directory, get its contents.
        let dir_obj = cur_obj.get("contents")?.as_object()?;
        // And now find the next node inside the components
        cur_obj = dir_obj.get(path_component)?.as_object()?;
    }

    Some(cur_obj)
}

pub fn read_detailed_file_info(path: &str, index_path: &str) -> Option<json::Object> {
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
pub fn interpolate_coverage(mut raw: Vec<i32>) -> Vec<i32> {
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
pub fn get_per_file_info(all_concise_info: &json::Object, path: &str, index_path: &str) -> PerFileInfo {
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
