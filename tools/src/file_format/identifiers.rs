extern crate memmap;

use self::memmap::Mmap;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufRead;
use std::process::Command;
use std::str;
use std::sync::Arc;
use ustr::{ustr, Ustr};

use serde::{Deserialize, Serialize};
use serde_json::to_string;

use super::config::Config;

fn uppercase(s: &[u8]) -> Vec<u8> {
    let mut result = vec![];
    for i in 0..s.len() {
        result.push(if s[i] >= 'a' as u8 && s[i] <= 'z' as u8 {
            s[i] - ('a' as u8) + ('A' as u8)
        } else {
            s[i]
        });
    }
    result
}

#[derive(Clone, Debug)]
pub struct IdentMap {
    mmap: Arc<Mmap>,
}

#[derive(Serialize, Deserialize)]
pub struct IdentResult {
    pub id: Ustr,
    pub symbol: Ustr,
}

// TODO: switch to https://crates.io/crates/cpp_demangle which is probably what
// pernosco uses (based on khuey being an owner) and so for consistency purposes
// is probably the right call.
fn demangle_name(name: &str) -> String {
    let output = Command::new("c++filt")
        .arg("--no-params")
        .arg(name)
        .output();
    match output {
        Err(_) => name.to_string(),
        Ok(output) => {
            if !output.status.success() {
                return name.to_string();
            }
            String::from_utf8(output.stdout)
                .unwrap_or(name.to_string())
                .trim()
                .to_string()
        }
    }
}

impl IdentMap {
    pub fn new(filename: &str) -> Option<IdentMap> {
        let file = match File::open(filename) {
            Ok(file) => { file }
            Err(e) => {
                warn!("Failed to open {}: {:?}", filename, e);
                return None;
            }
        };
        unsafe {
            match Mmap::map(&file) {
                Ok(mmap) => {
                    Some(IdentMap {
                        mmap: Arc::new(mmap)
                    })
                }
                Err(e) => {
                    warn!("Failed to mmap {}: {:?}", filename, e);
                    return None
                }
            }
        }
    }

    pub fn load(config: &Config) -> HashMap<String, IdentMap> {
        let mut result = HashMap::new();
        for (tree_name, tree_config) in &config.trees {
            println!("Loading identifiers {}", tree_name);
            let filename = format!("{}/identifiers", tree_config.paths.index_path);
            if let Some(map) = IdentMap::new(&filename) {
                result.insert(tree_name.clone(), map);
            }
        }
        result
    }

    fn get_line(&self, pos: usize) -> &[u8] {
        let mut pos = pos;
        let bytes = self.mmap.as_ref();
        if bytes[pos] == '\n' as u8 {
            pos -= 1;
        }

        let mut start = pos;
        let mut end = pos;

        while start > 0 && bytes[start - 1] != '\n' as u8 {
            start -= 1;
        }

        let size = bytes.len();
        while end < size && bytes[end] != '\n' as u8 {
            end += 1;
        }

        &bytes[start..end]
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

    pub fn lookup(
        &self,
        needle: &str,
        exact_match: bool,
        ignore_case: bool,
        max_results: usize,
    ) -> Vec<IdentResult> {
        let bytes = self.mmap.as_ref();

        let start = self.bisect(needle.as_bytes(), false);
        let end = self.bisect(needle.as_bytes(), true);

        let mut result = vec![];
        let slice = &bytes[start..end];

        for line in slice.lines() {
            let line = line.unwrap();
            let mut pieces = line.split(' ');
            let mut id = pieces.next().unwrap().to_string();
            let symbol = pieces.next().unwrap();

            // We only need to worry about suffix-related cases if the needle is
            // shorter than the identifier.
            if needle.len() < id.len() {
                let suffix = &id[needle.len()..];
                if exact_match || suffix.contains(':') || suffix.contains('.')
                {
                    continue;
                }
            }
            if !ignore_case && !id.starts_with(needle) {
                continue;
            }

            let demangled = demangle_name(&symbol);
            if demangled != symbol {
                id = demangled;
            }

            result.push(IdentResult {
                id: ustr(&id),
                symbol: ustr(symbol),
            });
            if result.len() == max_results {
                break;
            }
        }

        result
    }

    pub fn lookup_json(
        &self,
        needle: &str,
        complete: bool,
        fold_case: bool,
        max_results: usize,
    ) -> String {
        let results = self.lookup(needle, complete, fold_case, max_results);
        to_string(&results).unwrap()
    }
}
