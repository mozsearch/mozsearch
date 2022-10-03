use std::fs::{File};
use std::io::{BufReader};
use std::sync::Arc;

use lexical_sort::natural_lexical_cmp;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{from_reader, Map, Value};
use ustr::{Ustr, UstrMap, existing_ustr};

use crate::abstract_server::{FileMatch, FileMatches, Result};

use super::repo_data_ingestion::{ConcisePerFileInfo, DetailedPerFileInfo};

/// Provides access to (concise) per-file info via a pre-loaded copy of
/// `concise-per-file-info.json` and any derived indices.  This exact same
/// information is also available inside the crossref database as
/// `FILE_`-prefixed symbols.
///
/// The reasons to favor using this implementation (or growing this
/// implementation):
/// - Searching for a subset of files in the tree, including using additional
///   constraints that can be pre-computed.
///   - The crate https://github.com/lun3x/multi_index_map has tentatively
///     been identified as a way to aid in precomputation.
/// - Up-front file I/O and object allocation versus crossref-lookup which
///   loads/allocates JSON each time.  This data is able to be shared immutably.
#[derive(Clone, Debug)]
pub struct FileLookupMap {
    // We are able to safely use a UstrMap here because we ensure that in cases
    // where we're dealing with non-Ustr values that we do not create new Ustrs
    // for paths that do not exist through use of `existing_ustr`.
    concise_per_file: Arc<UstrMap<ConcisePerFileInfo<Ustr>>>,
}

impl FileLookupMap {
    pub fn new(concise_file_path: &str) -> Self {
        let components_file = File::open(concise_file_path).unwrap();
        let mut reader = BufReader::new(&components_file);
        let map: UstrMap<ConcisePerFileInfo<Ustr>> = from_reader(&mut reader).unwrap();
        FileLookupMap { concise_per_file: Arc::new(map) }
    }

    /// File lookup for when you have an existing Ustr; under no circumstances
    /// should you mint a new Ustr for a potential path from content.  If that's
    /// what you have, use `lookup_file_from_str` if it's a one-off, or use
    /// `existing_ustr` if you will be using the path multiple times.
    ///
    /// The general concern is to avoid interning a bunch of incorrect query
    /// strings.
    pub fn lookup_file_from_ustr(&self, path_ustr: &Ustr) -> Option<&ConcisePerFileInfo<Ustr>> {
        self.concise_per_file.get(&path_ustr)
    }

    /// File lookup when we don't have a Ustr already available; this is
    /// the appropriate call-site to use if you have a web-sourced potential
    /// path string which could be wrong (and therefore should not be interned).
    pub fn lookup_file_from_str(&self, path: &str) -> Option<&ConcisePerFileInfo<Ustr>> {
        if let Some(path_ustr) = existing_ustr(path) {
            self.concise_per_file.get(&path_ustr)
        } else {
            None
        }
    }

    /// Search the list of files by applying a regexp to the paths.
    pub fn search_files(&self, pathre: &str, include_dirs: bool, limit: usize) -> Result<FileMatches> {
        let re_path = Regex::new(pathre)?;
        let mut matches: Vec<FileMatch> = self.concise_per_file.iter()
            .filter(|v| {
                if !include_dirs && v.1.is_dir {
                    false
                } else {
                    re_path.is_match(v.0)
                }
            })
            .map(|v| {
                FileMatch {
                    path: v.0.clone(),
                    concise: v.1.clone(),
                }
            }).take(limit).collect();
        matches.sort_unstable_by(|a, b| natural_lexical_cmp(&a.path, &b.path));
        Ok(FileMatches {
            file_matches: matches,
        })
    }
}

/// Information about expected failures/problems for specific web platform
/// tests.
#[derive(Clone, Debug, Serialize, Deserialize)]

pub struct WPTExpectationInfo {
    /// The condition strings and related bugs that disable this test in its
    /// entirety.
    pub disabling_conditions: Vec<(String, String)>,
    /// The number of `_subtests` that were disabled or conditionally disabled
    pub disabled_subtests_count: i64,
}

/// Information from `test-info-all-tests.json` which knows about files that the
/// test manifests know about.
#[derive(Clone, Debug, Serialize, Deserialize)]
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

/// Per-file info derived from the concise and detailed info for a given file.
/// Everything in here is optional data, but this structure will be available
/// for every file to simplify control-flow.
pub struct PerFileInfo {
    pub bugzilla_component: Option<(String, String)>,
    pub test_info: Option<TestInfo>,
    pub coverage: Option<Vec<i32>>,
}

pub fn get_concise_file_info<'a>(
    all_concise_info: &'a Value,
    path: &str,
) -> Option<&'a Map<String, Value>> {
    let mut cur_obj = all_concise_info.get("root")?.as_object()?;

    for path_component in path.split('/') {
        // The current node must be a directory, get its contents.
        let dir_obj = cur_obj.get("contents")?.as_object()?;
        // And now find the next node inside the components
        cur_obj = dir_obj.get(path_component)?.as_object()?;
    }

    Some(cur_obj)
}

pub fn read_detailed_file_info(path: &str, index_path: &str) -> Option<DetailedPerFileInfo> {
    let json_fname = format!("{}/detailed-per-file-info/{}", index_path, path);
    let json_file = File::open(json_fname).ok()?;
    let mut reader = BufReader::new(&json_file);
    from_reader(&mut reader).ok()
}
