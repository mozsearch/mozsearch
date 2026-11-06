// Annotate the C++ analysis' structured records with whether their functions
// can GC (`canGC`) and, if they can, the path to a GC (`gcPath`), from the
// hazard analysis' gcFunctions.txt (each GC function's header, `GC Function:
// MANGLED$PRETTY`, then its path to a GC, one indented function per line) and
// allFunctions.txt (every function, `MANGLED$PRETTY` per line), as
// scripts/annotate-gc.py did, for the analysis files ANALYSIS_DIR's paths on
// stdin name.  The hazard data is loaded once (firefox's is 2.9 GB; each of
// annotate-gc.py's parallel jobs loaded it into a dict) and the files are
// annotated in parallel.  A record's symbol is its `sym` or, failing that, the
// first of its variants' which the hazard analysis knows; a file without any
// is left as it is.  Only structured records are annotated (annotate-gc.py
// annotated any record mentioning "structured", ex: a target record of
// `structuredClone`), and they're written as serde_json writes them.
//
// Usage: annotate-gc ANALYSIS_DIR GC_FUNCTIONS ALL_FUNCTIONS < PATHS

use std::collections::HashMap;
use std::io::BufRead;
use std::ops::Range;
use std::path::Path;
use std::process::exit;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use rayon::prelude::*;
use serde_json::Value;

/// By mangled symbol, whether its function can GC: the byte range of its path
/// to a GC in gcFunctions.txt, if it can.
struct Functions {
    gc: &'static [u8],
    by_sym: HashMap<&'static str, Option<Range<usize>>>,
}

fn trim_end(line: &[u8]) -> &[u8] {
    let end = line
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(0, |i| i + 1);
    &line[..end]
}

fn trim_start(line: &[u8]) -> &[u8] {
    let start = line
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(line.len());
    &line[start..]
}

/// A line's symbol: what's before its first `$`.
fn line_sym(line: &[u8]) -> Option<&str> {
    let sym = line.split(|b| *b == b'$').next().unwrap_or(line);
    std::str::from_utf8(sym).ok()
}

/// The lines of `buf`, with their offsets.
fn lines(buf: &[u8]) -> impl Iterator<Item = (usize, &[u8])> {
    let mut offset = 0;
    buf.split(|b| *b == b'\n').map(move |line| {
        let start = offset;
        offset += line.len() + 1;
        (start, line)
    })
}

fn load(gc_path: &Path, all_path: &Path) -> std::io::Result<Functions> {
    // (Both files at once: they took 5.7 s one after the other.)
    let (all, gc) = rayon::join(
        || -> std::io::Result<HashMap<&'static str, Option<Range<usize>>>> {
            let all: &'static [u8] = Box::leak(std::fs::read(all_path)?.into_boxed_slice());
            let mut by_sym = HashMap::with_capacity(all.len() / 256);
            for (_, line) in lines(all) {
                if let Some(sym) = line_sym(trim_end(line)) {
                    by_sym.insert(sym, None);
                }
            }
            Ok(by_sym)
        },
        || -> std::io::Result<(&'static [u8], Vec<(&'static str, Range<usize>)>)> {
            let gc: &'static [u8] = Box::leak(std::fs::read(gc_path)?.into_boxed_slice());
            // (A GC function's path is the lines after its header, up to the
            // next.)
            let mut paths = vec![];
            let mut current: Option<(&'static str, usize)> = None;
            for (offset, raw) in lines(gc) {
                if let Some(rest) = trim_end(raw).strip_prefix(b"GC Function: ") {
                    if let Some((sym, start)) = current.take() {
                        paths.push((sym, start..offset));
                    }
                    if let Some(sym) = line_sym(rest) {
                        current = Some((sym, (offset + raw.len() + 1).min(gc.len())));
                    }
                }
            }
            if let Some((sym, start)) = current {
                paths.push((sym, start..gc.len()));
            }
            Ok((gc, paths))
        },
    );
    let mut by_sym = all?;
    let (gc, paths) = gc?;
    // (The GC functions' after all of them, the last of a function's
    // winning.)
    for (sym, range) in paths {
        by_sym.insert(sym, Some(range));
    }
    Ok(Functions { gc, by_sym })
}

impl Functions {
    /// A path to a GC: its functions (the block's indented lines), one per
    /// line.
    fn gc_path(&self, range: &Range<usize>) -> String {
        let functions: Vec<&str> = lines(&self.gc[range.clone()])
            .map(|(_, line)| trim_end(line))
            .filter(|line| line.starts_with(b" "))
            .filter_map(|line| std::str::from_utf8(trim_start(line)).ok())
            .collect();
        functions.join("\n")
    }

    /// The record's symbol which the hazard analysis knows, if any: its
    /// `sym`, or one of its variants'.
    fn known_sym<'a>(&self, datum: &'a Value) -> Option<&'a str> {
        let main = datum.get("sym").and_then(Value::as_str);
        let variants = datum
            .get("variants")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|variant| variant.get("sym").and_then(Value::as_str));
        main.into_iter()
            .chain(variants)
            .find(|sym| self.by_sym.contains_key(*sym))
    }
}

/// Annotate a file's structured records, rewriting it if any are; whether
/// any were.
fn annotate(path: &Path, functions: &Functions) -> std::io::Result<bool> {
    let text = std::fs::read_to_string(path)?;
    let mut out = String::with_capacity(text.len() + 4096);
    let mut annotated = false;
    for line in text.lines() {
        let line = line.trim_end();
        if line.contains("structured")
            && let Ok(mut datum) = serde_json::from_str::<Value>(line)
            && datum.get("structured").is_some()
            && let Some(sym) = functions.known_sym(&datum).map(str::to_string)
            && let Some(record) = datum.as_object_mut()
        {
            match &functions.by_sym[sym.as_str()] {
                None => {
                    record.insert("canGC".to_string(), Value::Bool(false));
                }
                Some(range) => {
                    record.insert("canGC".to_string(), Value::Bool(true));
                    record.insert(
                        "gcPath".to_string(),
                        Value::String(functions.gc_path(range)),
                    );
                }
            }
            out.push_str(&serde_json::to_string(&datum).unwrap());
            out.push('\n');
            annotated = true;
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if annotated {
        std::fs::write(path, out)?;
    }
    Ok(annotated)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("Usage: annotate-gc ANALYSIS_DIR GC_FUNCTIONS ALL_FUNCTIONS < PATHS");
        exit(1);
    }
    let analysis_dir = Path::new(&args[1]);
    let (gc_path, all_path) = (Path::new(&args[2]), Path::new(&args[3]));
    if !gc_path.exists() || !all_path.exists() {
        return;
    }
    let started = Instant::now();
    let functions = load(gc_path, all_path).unwrap_or_else(|e| {
        eprintln!(
            "annotate-gc: couldn't read the hazard analysis' data: {}",
            e
        );
        exit(1);
    });
    let loaded = started.elapsed();
    let paths: Vec<String> = std::io::stdin()
        .lock()
        .lines()
        .map_while(Result::ok)
        .map(|line| line.trim_end().to_string())
        .filter(|line| !line.is_empty())
        .collect();
    let annotated = AtomicUsize::new(0);
    let failed = AtomicUsize::new(0);
    paths.par_iter().for_each(
        |path| match annotate(&analysis_dir.join(path), &functions) {
            Ok(true) => {
                annotated.fetch_add(1, Ordering::Relaxed);
            }
            Ok(false) => {}
            Err(e) => {
                eprintln!("annotate-gc: {}: {}", path, e);
                failed.fetch_add(1, Ordering::Relaxed);
            }
        },
    );
    println!(
        "annotate-gc: {} functions known (loaded in {:.1}s); annotated {} of {} files in {:.1}s",
        functions.by_sym.len(),
        loaded.as_secs_f64(),
        annotated.load(Ordering::Relaxed),
        paths.len(),
        started.elapsed().as_secs_f64(),
    );
    if failed.load(Ordering::Relaxed) > 0 {
        exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_annotate() {
        let dir = std::env::temp_dir().join(format!("annotate-gc-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let gc = dir.join("gcFunctions.txt");
        let all = dir.join("allFunctions.txt");
        std::fs::write(
            &gc,
            "GC Function: _Z4DoGCv$void DoGC()\n  (GC)\n\nGC Function: _Z5CanGCv$void CanGC()\n  void DoGC()\n  (GC)\n",
        )
        .unwrap();
        std::fs::write(
            &all,
            "_Z4DoGCv$void DoGC()\n_Z5CanGCv$void CanGC()\n_Z6NoGCv$void NoGC()\n",
        )
        .unwrap();
        let functions = load(&gc, &all).unwrap();
        let file = dir.join("file.cpp");
        let lines = [
            r#"{"loc":"00001:5-9","target":1,"kind":"use","pretty":"structuredClone","sym":"_Z6NoGCv"}"#,
            r#"{"loc":"00002:5-9","structured":1,"pretty":"CanGC","sym":"_Z5CanGCv","kind":"function"}"#,
            r#"{"loc":"00003:5-9","structured":1,"pretty":"NoGC","sym":"_Z6NoGCv","kind":"function"}"#,
            r#"{"loc":"00004:5-9","structured":1,"pretty":"V","sym":"_Z1Vv","variants":[{"sym":"_Z4DoGCv"}]}"#,
            r#"{"loc":"00005:5-9","structured":1,"pretty":"Unknown","sym":"_Z7Unknownv"}"#,
        ];
        std::fs::write(&file, lines.join("\n") + "\n").unwrap();
        assert!(annotate(&file, &functions).unwrap());
        let out = std::fs::read_to_string(&file).unwrap();
        let out: Vec<&str> = out.lines().collect();
        // (Not structured records, nor those the hazard analysis doesn't know.)
        assert_eq!(out[0], lines[0]);
        assert_eq!(out[4], lines[4]);
        assert_eq!(
            out[1],
            r#"{"loc":"00002:5-9","structured":1,"pretty":"CanGC","sym":"_Z5CanGCv","kind":"function","canGC":true,"gcPath":"void DoGC()\n(GC)"}"#
        );
        assert_eq!(
            out[2],
            r#"{"loc":"00003:5-9","structured":1,"pretty":"NoGC","sym":"_Z6NoGCv","kind":"function","canGC":false}"#
        );
        assert_eq!(
            out[3],
            r#"{"loc":"00004:5-9","structured":1,"pretty":"V","sym":"_Z1Vv","variants":[{"sym":"_Z4DoGCv"}],"canGC":true,"gcPath":"(GC)"}"#
        );
        // (A file without any is left as it is.)
        let other = dir.join("other.cpp");
        std::fs::write(&other, lines[4]).unwrap();
        assert!(!annotate(&other, &functions).unwrap());
        assert_eq!(std::fs::read_to_string(&other).unwrap(), lines[4]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
