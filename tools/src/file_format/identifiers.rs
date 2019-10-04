extern crate memmap;

use self::memmap::{Mmap, Protection};
use std::str;
use std::io::BufRead;
use std::collections::HashMap;
use std::process::Command;

use rustc_serialize::json;

use crate::config;

fn uppercase(s: &[u8]) -> Vec<u8> {
    let mut result = vec![];
    for i in 0 .. s.len() {
        result.push(if s[i] >= 'a' as u8 && s[i] <= 'z' as u8 { s[i] - ('a' as u8) + ('A' as u8) } else { s[i] });
    }
    result
}

pub struct IdentMap {
    mmap: Mmap,
}

#[derive(RustcDecodable, RustcEncodable)]
pub struct IdentResult {
    pub id: String,
    pub symbol: String,
}

fn demangle_name(name: &str) -> String {
    let output = Command::new("c++filt").arg("--no-params").arg(name).output();
    match output {
        Err(_) => name.to_string(),
        Ok(output) => {
            if !output.status.success() {
                return name.to_string();
            }
            String::from_utf8(output.stdout).unwrap_or(name.to_string()).trim().to_string()
        }
    }
}

impl IdentMap {
    fn new(filename: &str) -> IdentMap {
        let file_mmap = Mmap::open_path(filename, Protection::Read).unwrap();
        IdentMap { mmap: file_mmap }
    }

    pub fn load(config: &config::Config) -> HashMap<String, IdentMap> {
        let mut result = HashMap::new();
        for (tree_name, tree_config) in &config.trees {
            println!("Loading identifiers {}", tree_name);
            let filename = format!("{}/identifiers", tree_config.paths.index_path);
            let map = IdentMap::new(&filename);
            result.insert(tree_name.clone(), map);
        }
        result
    }

    fn get_line(&self, pos: usize) -> &[u8] {
        let mut pos = pos;
        let bytes: &[u8] = unsafe { self.mmap.as_slice() };
        if bytes[pos] == '\n' as u8 {
            pos -= 1;
        }

        let mut start = pos;
        let mut end = pos;

        while start > 0 && bytes[start - 1] != '\n' as u8 {
            start -= 1;
        }

        let size = self.mmap.len();
        while end < size && bytes[end] != '\n' as u8 {
            end += 1;
        }

        &bytes[start .. end]
    }

    fn bisect(&self, needle: &[u8], upper_bound: bool) -> usize {
        let mut needle = uppercase(needle);
        if upper_bound {
            needle.push('~' as u8);
        }

        let mut first = 0;
        let mut count = self.mmap.len();

        while count > 0 {
            let step = count / 2;
            let pos = first + step;

            let line = self.get_line(pos);
            let line_upper = uppercase(line);
            if line_upper < needle || (upper_bound && line_upper == needle) {
                first = pos + 1;
                count -= step + 1;
            } else {
                count = step;
            }
        }

        first
    }

    // NEED A WAY TO LIMIT NUMBER OF RESULTS RETURNED TO 6
    // Also need to apply c++filt to the ident as a better human-readable name
    pub fn lookup(&self, needle: &str, complete: bool, fold_case: bool, max_results: usize) -> Vec<IdentResult> {
        let start = self.bisect(needle.as_bytes(), false);
        let end = self.bisect(needle.as_bytes(), true);

        let mut result = vec![];
        let bytes: &[u8] = unsafe { self.mmap.as_slice() };
        let slice = &bytes[start .. end];

        for line in slice.lines() {
            let line = line.unwrap();
            let mut pieces = line.split(' ');
            let mut id = pieces.next().unwrap().to_string();
            let symbol = pieces.next().unwrap();

            {
                let suffix = &id[needle.len() ..];
                if suffix.contains(':') || suffix.contains('.') || (complete && suffix.len() > 0) {
                    continue;
                }
            }
            if !fold_case && !id.starts_with(needle) {
                continue;
            }

            let demangled = demangle_name(&symbol);
            if demangled != symbol {
                id = demangled;
            }

            result.push(IdentResult { id: id, symbol: symbol.to_string() });
            if result.len() == max_results {
                break;
            }
        }

        result
    }

    pub fn lookup_json(&self, needle: &str, complete: bool, fold_case: bool, max_results: usize) -> String {
        let results = self.lookup(needle, complete, fold_case, max_results);
        json::encode(&results).unwrap()
    }
}
