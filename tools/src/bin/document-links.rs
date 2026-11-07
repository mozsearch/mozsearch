// document-links: the links of a tree's web-platform-tests' pages and
// stylesheets to its files (INDEX_ROOT/js-environments-links.json; see
// `tools::js_environments::discover`'s `wpt_links`), written into their
// analyses, after their markup's and styles' (and their inline scripts'):
// - each link a use of its file (`FILE_`, ex: a page's stylesheet, script,
//   frame, image or reftest reference, a stylesheet's `@import` or font), so
//   that its context menu goes to the file, and the file's uses are the
//   files linking it;
// - and each document's custom properties' uses (a page's, its stylesheets'
//   and theirs, `@import`ed) also uses of the definitions of the same names
//   in the document's other files (at most MAX_DEFINITIONS, by their paths),
//   as a property a page defines applies to the stylesheets it links (each
//   file's custom properties being its own; see
//   `tools::css_analyzer::document_namespace`), and a stylesheet's to the
//   pages linking it.
// A file without analysis is left as it is.
//
// A try push's (see scripts/try-analyze-web.sh) are its own: its head's
// links (LINKS, rather than INDEX_ROOT's), its analysis (ANALYSIS, rather
// than INDEX_ROOT/analysis), with the tree's (BASE, ex: INDEX_ROOT/analysis,
// gzipped or not) for the files it doesn't change, and only the files it
// changes written (ONLY, a list of paths).
//
// Usage: document-links INDEX_ROOT [--links LINKS] [--analysis ANALYSIS]
//          [--base BASE] [--only ONLY]

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde_json::{Value, json};
use tools::css_analyzer::custom_property_prefix;
use tools::file_format::analysis_manglings::mangle_file;
use tools::js_environments::discover::FileLink;
use tools::js_environments::scan::LinkKind;

/// How many other files' definitions a custom property's use has at most.
const MAX_DEFINITIONS: usize = 16;

fn is_stylesheet(path: &str) -> bool {
    path.ends_with(".css")
}

/// A file's analysis's lines, and the mangled names of the custom properties
/// it defines (in its own namespace).
struct Analysis {
    lines: Vec<String>,
    defined: BTreeSet<String>,
}

/// A file's analysis, in `dir`, or else in `base` (gzipped or not; see
/// compress-outputs.sh), with whether it's `dir`'s.
fn read_analysis(
    dir: &Path,
    base: Option<&Path>,
    path: &str,
    prefix: &str,
) -> Option<(Analysis, bool)> {
    let (text, own) = match fs::read_to_string(dir.join(path)) {
        Ok(text) => (text, true),
        Err(_) => {
            let base = base?;
            let text = match fs::read_to_string(base.join(path))
                .ok()
                .filter(|t| !t.is_empty())
            {
                Some(text) => text,
                None => {
                    let mut text = String::new();
                    flate2::read::GzDecoder::new(
                        fs::File::open(base.join(format!("{}.gz", path))).ok()?,
                    )
                    .read_to_string(&mut text)
                    .ok()?;
                    text
                }
            };
            (text, false)
        }
    };
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut defined = BTreeSet::new();
    for line in lines
        .iter()
        .filter(|l| l.contains(prefix) && l.contains("\"target\""))
    {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if matches!(record["kind"].as_str(), Some("def" | "decl"))
            && let Some(name) = record["sym"].as_str().and_then(|s| s.strip_prefix(prefix))
        {
            defined.insert(name.to_string());
        }
    }
    Some((Analysis { lines, defined }, own))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let usage = "Usage: document-links INDEX_ROOT [--links LINKS] [--analysis ANALYSIS] [--base BASE] [--only ONLY]";
    if args.len() < 2 || args.len() % 2 != 0 {
        eprintln!("{}", usage);
        std::process::exit(1);
    }
    let index_root = Path::new(&args[1]);
    let mut links_path = index_root.join("js-environments-links.json");
    let mut analysis_dir = index_root.join("analysis");
    let mut base: Option<PathBuf> = None;
    let mut only: Option<HashSet<String>> = None;
    for pair in args[2..].chunks(2) {
        match pair[0].as_str() {
            "--links" => links_path = PathBuf::from(&pair[1]),
            "--analysis" => analysis_dir = PathBuf::from(&pair[1]),
            "--base" => base = Some(PathBuf::from(&pair[1])),
            "--only" => {
                only = Some(
                    fs::read_to_string(&pair[1])
                        .expect("Couldn't read ONLY")
                        .lines()
                        .map(str::to_string)
                        .collect(),
                )
            }
            _ => {
                eprintln!("{}", usage);
                std::process::exit(1);
            }
        }
    }
    let links: BTreeMap<String, Vec<FileLink>> = match fs::read_to_string(&links_path) {
        Ok(text) => serde_json::from_str(&text).expect("Couldn't parse the links"),
        Err(_) => {
            println!("document-links: no {}", links_path.display());
            return;
        }
    };

    // ## Documents: each page with its stylesheets, and theirs (`@import`ed).
    let mut documents: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for page in links.keys().filter(|p| !is_stylesheet(p)) {
        let mut files: BTreeSet<&str> = BTreeSet::new();
        files.insert(page);
        let mut todo: Vec<&str> = vec![page];
        while let Some(file) = todo.pop() {
            for link in links.get(file).into_iter().flatten() {
                let follows = match link.kind {
                    LinkKind::Stylesheet => file == page.as_str(),
                    LinkKind::Import => true,
                    _ => false,
                };
                if follows && is_stylesheet(&link.target) && files.insert(&link.target) {
                    todo.push(&link.target);
                }
            }
        }
        if files.len() > 1 {
            documents.insert(page, files);
        }
    }
    let mut documents_of: HashMap<&str, Vec<&str>> = HashMap::new();
    for (page, files) in &documents {
        for file in files {
            documents_of.entry(file).or_default().push(page);
        }
    }

    // ## The files' analyses (those with links, or in a document).
    let mut paths: BTreeSet<&str> = links.keys().map(String::as_str).collect();
    paths.extend(documents.values().flatten().copied());
    let paths: Vec<&str> = paths.into_iter().collect();
    let analyses: HashMap<&str, (Analysis, bool)> = paths
        .par_iter()
        .filter_map(|path| {
            let analysis = read_analysis(
                &analysis_dir,
                base.as_deref(),
                path,
                &custom_property_prefix(path),
            )?;
            Some((*path, analysis))
        })
        .collect();

    let written: Vec<(usize, usize)> = paths
        .par_iter()
        .filter_map(|path| {
            // (Only the analysis's own, and those ONLY lists.)
            let (analysis, own) = analyses.get(path)?;
            if !own || only.as_ref().is_some_and(|only| !only.contains(*path)) {
                return None;
            }
            let mut lines = analysis.lines.clone();
            let mut added_links = 0;
            for link in links.get(*path).into_iter().flatten() {
                let loc = format!("{:05}:{}-{}", link.line, link.start, link.end);
                let sym = format!("FILE_{}", mangle_file(&link.target));
                lines.push(
                    json!({"loc": loc, "source": 1, "syntax": "use,file",
                           "pretty": format!("file {}", link.target), "sym": sym})
                    .to_string(),
                );
                lines.push(
                    json!({"loc": loc, "target": 1, "kind": "use",
                           "pretty": link.target, "sym": sym})
                    .to_string(),
                );
                added_links += 1;
            }

            // (Its custom properties' uses' other definitions: the other
            // files of its documents defining their names.)
            let prefix = custom_property_prefix(path);
            let others: BTreeSet<&str> = documents_of
                .get(path)
                .into_iter()
                .flatten()
                .flat_map(|page| documents[page].iter().copied())
                .filter(|file| file != path)
                .collect();
            let mut cross = 0;
            if !others.is_empty() {
                let definitions_of = |name: &str| -> Vec<String> {
                    others
                        .iter()
                        .filter(|file| {
                            analyses
                                .get(*file)
                                .is_some_and(|(a, _)| a.defined.contains(name))
                        })
                        .take(MAX_DEFINITIONS)
                        .map(|file| format!("{}{}", custom_property_prefix(file), name))
                        .collect()
                };
                let mut extra_targets = vec![];
                for line in lines.iter_mut() {
                    if !line.contains(&prefix) {
                        continue;
                    }
                    let Ok(mut record) = serde_json::from_str::<Value>(line) else {
                        continue;
                    };
                    let syms: Vec<String> = record["sym"]
                        .as_str()
                        .unwrap_or_default()
                        .split(',')
                        .map(str::to_string)
                        .collect();
                    let Some(name) = syms
                        .iter()
                        .find_map(|s| s.strip_prefix(&prefix).map(str::to_string))
                    else {
                        continue;
                    };
                    if record.get("target").is_some() {
                        if record["kind"].as_str() != Some("use") {
                            continue;
                        }
                        for sym in definitions_of(&name) {
                            let mut other = record.clone();
                            other["sym"] = json!(sym);
                            extra_targets.push(other.to_string());
                            cross += 1;
                        }
                    } else if record.get("source").is_some()
                        && record["syntax"].as_str().is_some_and(|s| s.contains("use"))
                    {
                        let more: Vec<String> = definitions_of(&name)
                            .into_iter()
                            .filter(|s| !syms.contains(s))
                            .collect();
                        if !more.is_empty() {
                            record["sym"] =
                                json!(syms.into_iter().chain(more).collect::<Vec<_>>().join(","));
                            *line = record.to_string();
                        }
                    }
                }
                lines.extend(extra_targets);
            }
            if added_links == 0 && cross == 0 {
                return None;
            }
            let mut text = lines.join("\n");
            text.push('\n');
            fs::write(analysis_dir.join(path), text).ok()?;
            Some((added_links, cross))
        })
        .collect();
    println!(
        "document-links: {} files' analyses: {} links, {} custom properties' uses of other files' definitions; {} documents",
        written.len(),
        written.iter().map(|w| w.0).sum::<usize>(),
        written.iter().map(|w| w.1).sum::<usize>(),
        documents.len()
    );
}
