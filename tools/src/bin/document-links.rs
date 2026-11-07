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
// Usage: document-links INDEX_ROOT

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::Path;

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

fn read_analysis(path: &Path, prefix: &str) -> Option<Analysis> {
    let text = fs::read_to_string(path).ok()?;
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
    Some(Analysis { lines, defined })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: document-links INDEX_ROOT");
        std::process::exit(1);
    }
    let index_root = Path::new(&args[1]);
    let links: BTreeMap<String, Vec<FileLink>> =
        match fs::read_to_string(index_root.join("js-environments-links.json")) {
            Ok(text) => {
                serde_json::from_str(&text).expect("Couldn't parse js-environments-links.json")
            }
            Err(_) => {
                println!("document-links: no js-environments-links.json");
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
    let analyses: HashMap<&str, Analysis> = paths
        .par_iter()
        .filter_map(|path| {
            let analysis = read_analysis(
                &index_root.join("analysis").join(path),
                &custom_property_prefix(path),
            )?;
            Some((*path, analysis))
        })
        .collect();

    let written: Vec<(usize, usize)> = paths
        .par_iter()
        .filter_map(|path| {
            let analysis = analyses.get(path)?;
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
                                .is_some_and(|a| a.defined.contains(name))
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
            fs::write(index_root.join("analysis").join(path), text).ok()?;
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
