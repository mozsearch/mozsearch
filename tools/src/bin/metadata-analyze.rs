// Derive files' metadata from the rules a tree declares it by, with the rules'
// positions (see `tools::file_metadata`), for `per-file-info.toml`'s
// ingestion into the detailed per-file info.
//
// Usage:
//   metadata-analyze FILES_ROOT INDEX_ROOT
//
// It reads INDEX_ROOT/repo-files (the tree's files) and writes, in
// INDEX_ROOT, each only if the tree has its rules:
// - mozbuild-bug-components.json: each file's Bugzilla component by its
//   moz.build files' `Files` rules, with the moz.build line giving it, as
//   {"rules": {"N": {"product", "component", "path", "line"}}, "paths": {..}}
//   where "paths" nests directories as objects with files' values the rules'
//   numbers (like bugzilla-components.json's).
// - review-context-rules.json: the `review-context.toml` rules applying to
//   each file (as if a patch changed only it), as {"sets": {"N": [rule..]},
//   "paths": {..}}, a rule being {"name", "description", "path", "line",
//   "applies": "fully"/"partially", "loads": [..]}.
// - test-manifest-entries.json: each test's entries in the test manifests
//   the moz.build files declare (and those they include), with their
//   conditions' lines, as {"tests": {"all": [{"test", "manifests": [..]}]}}.
//
// It also compares the components and the conditions with
// bugzilla-components.json's and test-info-all-tests.json's, if INDEX_ROOT
// has them (Taskcluster's), and says how many agree.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;

use serde_json::{Map, Value, json};
use tools::file_metadata::mozbuild::{FilesRules, Origin, is_read_mozbuild, read_mozbuild};
use tools::file_metadata::review_context::{Applies, read_rules};
use tools::file_metadata::test_manifests::{
    CONDITION_KEYS, Manifests, join_normalized, read_manifest,
};

/// Nest a path's value into directories' objects.
fn nest(root: &mut Map<String, Value>, path: &str, value: Value) {
    let mut node = root;
    let mut parts = path.split('/').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            node.insert(part.to_string(), value);
            return;
        }
        let entry = node
            .entry(part.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            return;
        }
        node = entry.as_object_mut().unwrap();
    }
}

fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_string(value).unwrap())
        .unwrap_or_else(|e| panic!("Couldn't write {}: {}", path.display(), e));
}

/// bugzilla-components.json's component for each file.
fn artifact_components(path: &Path) -> Option<HashMap<String, (String, String)>> {
    let json: Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    let components: HashMap<String, (String, String)> = json
        .get("components")?
        .as_object()?
        .iter()
        .filter_map(|(id, c)| {
            Some((
                id.clone(),
                (c.get(0)?.as_str()?.to_string(), c.get(1)?.as_str()?.to_string()),
            ))
        })
        .collect();
    let mut out = HashMap::new();
    let mut stack = vec![(String::new(), json.get("paths")?)];
    while let Some((prefix, node)) = stack.pop() {
        for (name, value) in node.as_object()? {
            let path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", prefix, name)
            };
            match value {
                Value::Object(_) => stack.push((path, value)),
                Value::Number(n) => {
                    if let Some(c) = components.get(&n.to_string()) {
                        out.insert(path, c.clone());
                    }
                }
                _ => {}
            }
        }
    }
    Some(out)
}

fn main() {
    env_logger::init();
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: metadata-analyze FILES_ROOT INDEX_ROOT");
        std::process::exit(1);
    }
    let files_root = Path::new(&args[1]);
    let index_root = Path::new(&args[2]);
    let repo_files: Vec<String> = fs::read_to_string(index_root.join("repo-files"))
        .expect("Couldn't read repo-files")
        .lines()
        .map(str::to_string)
        .collect();
    let file_set: HashSet<&str> = repo_files.iter().map(String::as_str).collect();
    let read = |path: &str| fs::read_to_string(files_root.join(path)).ok();

    // ## moz.build files: their Files rules, and the test manifests they declare.
    let mut rules = FilesRules::default();
    let mut declared: BTreeSet<String> = BTreeSet::new();
    let mut unresolved = 0;
    for path in repo_files.iter().filter(|p| is_read_mozbuild(p)) {
        let Some(source) = read(path) else {
            continue;
        };
        let mozbuild = read_mozbuild(&source);
        for what in &mozbuild.unresolved {
            println!("metadata-analyze: {}: couldn't evaluate {}", path, what);
        }
        unresolved += mozbuild.unresolved.len();
        let dir = path.rsplit_once('/').map_or("", |(dir, _)| dir);
        declared.extend(mozbuild.manifests.iter().map(|(m, _)| join_normalized(dir, m)));
        rules.add(path, mozbuild.rules);
    }

    // ## Each file's component, and the rule giving it.
    let mut components: HashMap<String, (String, String)> = HashMap::new();
    if !rules.is_empty() {
        let mut rule_ids: HashMap<((String, String), Origin), usize> = HashMap::new();
        let mut rule_values = Map::new();
        let mut paths = Map::new();
        for path in &repo_files {
            let Some((component, origin)) = rules.component(path) else {
                continue;
            };
            components.insert(path.clone(), component.clone());
            let next = rule_ids.len();
            let id = *rule_ids.entry((component.clone(), origin.clone())).or_insert_with(|| {
                rule_values.insert(
                    next.to_string(),
                    json!({
                        "product": component.0,
                        "component": component.1,
                        "path": origin.path,
                        "line": origin.line,
                    }),
                );
                next
            });
            nest(&mut paths, path, json!(id));
        }
        write_json(
            &index_root.join("mozbuild-bug-components.json"),
            &json!({ "rules": rule_values, "paths": paths }),
        );
        println!(
            "metadata-analyze: {} files' components by {} moz.build rules ({} constructs not evaluated)",
            components.len(),
            rule_ids.len(),
            unresolved
        );
        if let Some(artifact) = artifact_components(&index_root.join("bugzilla-components.json")) {
            let (mut agree, mut differ) = (0, vec![]);
            for path in &repo_files {
                match (components.get(path), artifact.get(path)) {
                    (a, b) if a == b => agree += 1,
                    (a, b) => differ.push((path, a, b)),
                }
            }
            println!(
                "metadata-analyze: components agree with bugzilla-components.json for {} files, differ for {}",
                agree,
                differ.len()
            );
            for (path, ours, theirs) in differ.iter().take(10) {
                println!("  {}: {:?} vs {:?}", path, ours, theirs);
            }
        }
    }

    // ## review-context.toml's rules applying to each file.
    if let Some(source) = read("review-context.toml") {
        match read_rules(&source) {
            Ok(review_rules) => {
                let mut set_ids: HashMap<Vec<(usize, bool)>, usize> = HashMap::new();
                let mut sets = Map::new();
                let mut paths = Map::new();
                let mut matched = 0;
                for path in &repo_files {
                    let component = components.get(path).map(|(p, c)| format!("{}::{}", p, c));
                    let applying = review_rules.applying(path, component.as_deref());
                    if applying.is_empty() {
                        continue;
                    }
                    matched += 1;
                    let key: Vec<(usize, bool)> = applying
                        .iter()
                        .map(|(i, a)| (*i, *a == Applies::Fully))
                        .collect();
                    let next = set_ids.len();
                    let id = *set_ids.entry(key).or_insert_with(|| {
                        let set: Vec<Value> = applying
                            .iter()
                            .map(|(i, applies)| {
                                let rule = &review_rules.rules[*i];
                                json!({
                                    "name": rule.name,
                                    "description": rule.description,
                                    "path": "review-context.toml",
                                    "line": rule.line,
                                    "applies": applies,
                                    "loads": rule.loads,
                                })
                            })
                            .collect();
                        sets.insert(next.to_string(), Value::Array(set));
                        next
                    });
                    nest(&mut paths, path, json!(id));
                }
                write_json(
                    &index_root.join("review-context-rules.json"),
                    &json!({ "sets": sets, "paths": paths }),
                );
                println!(
                    "metadata-analyze: {} review-context.toml rules apply to {} files",
                    review_rules.rules.len(),
                    matched
                );
            }
            Err(e) => println!("metadata-analyze: review-context.toml: {}", e),
        }
    }

    // ## The declared test manifests, and those they include.
    if !declared.is_empty() {
        let mut manifests = Manifests::default();
        let mut ancestors: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut queue: Vec<String> = declared.iter().cloned().collect();
        let mut seen: HashSet<String> = queue.iter().cloned().collect();
        while let Some(path) = queue.pop() {
            let Some(source) = read(&path) else {
                println!("metadata-analyze: missing test manifest {}", path);
                continue;
            };
            let manifest = match read_manifest(&source) {
                Ok(manifest) => manifest,
                Err(e) => {
                    println!("metadata-analyze: {}: {}", path, e.lines().next().unwrap_or(""));
                    continue;
                }
            };
            for (_, child) in Manifests::includes(&path, &manifest) {
                ancestors.entry(child.clone()).or_default().push(path.clone());
                if seen.insert(child.clone()) {
                    queue.push(child);
                }
            }
            manifests.manifests.insert(path, manifest);
        }
        let entries = manifests.entries(&ancestors, &declared);
        let tests: Vec<Value> = entries
            .iter()
            .filter(|(test, _)| file_set.contains(test.as_str()))
            .map(|(test, entries)| json!({ "test": test, "manifests": entries }))
            .collect();
        println!(
            "metadata-analyze: {} test manifests list {} tests",
            manifests.manifests.len(),
            tests.len()
        );
        write_json(
            &index_root.join("test-manifest-entries.json"),
            &json!({ "tests": { "all": tests } }),
        );

        // (Compared with test-info-all-tests.json's TOML manifests' entries.)
        if let Some(info) = fs::read_to_string(index_root.join("test-info-all-tests.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        {
            let (mut agree, mut differ) = (0, vec![]);
            for e in info["tests"].as_object().into_iter().flat_map(|o| o.values()).flat_map(|a| a.as_array().into_iter().flatten()) {
                let (Some(test), Some(manifest)) = (
                    e["test"].as_str(),
                    e["manifest"].get(0).and_then(Value::as_str),
                ) else {
                    continue;
                };
                if !manifest.ends_with(".toml") || manifest.starts_with('/') {
                    continue;
                }
                let (ancestor, manifest) = match manifest.split_once(':') {
                    Some((a, m)) => (Some(a), m),
                    None => (None, manifest),
                };
                let ours = entries.get(test).and_then(|es| {
                    es.iter()
                        .find(|e| e.manifest == manifest && e.ancestor.as_deref() == ancestor)
                });
                let same = ours.is_some_and(|ours| {
                    CONDITION_KEYS
                        .iter()
                        .all(|key| ours.manifestparser_value(key) == e[*key].as_str().unwrap_or(""))
                });
                if same {
                    agree += 1;
                } else {
                    differ.push(format!("{} in {}", test, manifest));
                }
            }
            println!(
                "metadata-analyze: conditions agree with test-info-all-tests.json for {} tests, differ for {}",
                agree,
                differ.len()
            );
            for d in differ.iter().take(10) {
                println!("  {}", d);
            }
        }
    }
}
