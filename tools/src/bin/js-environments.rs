// js-environments: a tree's JS environments (see tools::js_environments):
// the globals its scripts run in, each with the scripts loaded into it, from
// its test manifests (by the moz.build variables declaring them, as RULES's
// flavors say), its pages, web-platform-tests' variants, and the loads its
// scripts make; written to INDEX_ROOT/js-environments.json (and each file's,
// for its navigation panel, to js-environments-files.json, and the files in
// any, to js-environments-paths), with an
// inventory: how many environments of each flavor and global, how many of
// the tree's JS files are in one (and, given COVERED, a list of the files
// TypeScript projects already cover, how many neither covers, by directory),
// and the loads it couldn't follow, by kind.
//
// Usage: js-environments FILES_ROOT INDEX_ROOT RULES [COVERED]

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;

use tools::file_format::analysis_manglings::mangle_file;
use tools::file_format::url_map::read_url_map;
use tools::file_metadata::mozbuild::{is_read_mozbuild, read_mozbuild};
use tools::file_metadata::test_manifests::{Manifests, join_normalized, read_manifest};
use tools::js_environments::discover::{Inputs, Role, discover, files_json};
use tools::js_environments::rules::Rules;

const JS_EXTENSIONS: &[&str] = &[".js", ".mjs", ".jsm", ".sjs", ".cjs"];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 && args.len() != 5 {
        eprintln!("Usage: js-environments FILES_ROOT INDEX_ROOT RULES [COVERED]");
        std::process::exit(1);
    }
    let files_root = Path::new(&args[1]);
    let index_root = Path::new(&args[2]);
    let rules = Rules::parse(&fs::read_to_string(&args[3]).expect("Couldn't read RULES"))
        .unwrap_or_else(|e| panic!("{}: {}", args[3], e));
    let repo_files: Vec<String> = fs::read_to_string(index_root.join("repo-files"))
        .expect("Couldn't read repo-files")
        .lines()
        .map(str::to_string)
        .collect();
    let files: HashSet<String> = repo_files.iter().cloned().collect();
    let read = |path: &str| fs::read_to_string(files_root.join(path)).ok();

    // ## The test manifests moz.build files declare (with their variables),
    // and those they include.
    let mut manifest_variables: HashMap<String, String> = HashMap::new();
    for path in repo_files.iter().filter(|p| is_read_mozbuild(p)) {
        let Some(source) = read(path) else { continue };
        let dir = path.rsplit_once('/').map_or("", |(dir, _)| dir);
        for (manifest, variable) in read_mozbuild(&source).manifests {
            manifest_variables.insert(join_normalized(dir, &manifest), variable);
        }
    }
    let declared: BTreeSet<String> = manifest_variables.keys().cloned().collect();
    let mut manifests = Manifests::default();
    let mut ancestors: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut queue: Vec<String> = declared.iter().cloned().collect();
    let mut seen: HashSet<String> = queue.iter().cloned().collect();
    while let Some(path) = queue.pop() {
        let Some(manifest) = read(&path).and_then(|s| read_manifest(&s).ok()) else {
            continue;
        };
        for (_, child) in Manifests::includes(&path, &manifest) {
            ancestors
                .entry(child.clone())
                .or_default()
                .push(path.clone());
            if seen.insert(child.clone()) {
                queue.push(child);
            }
        }
        manifests.manifests.insert(path, manifest);
    }
    let entries = manifests.entries(&ancestors, &declared);
    println!(
        "js-environments: {} declared test manifests ({} with includes) list {} tests",
        declared.len(),
        manifests.manifests.len(),
        entries.len()
    );

    // (chrome:// and resource:// URLs' files, by the tree's URL map, if it
    // has one; see process-chrome-map.py.)
    let url_map = read_url_map(
        &index_root
            .join("aliases/url-map.json")
            .to_string_lossy()
            .to_string(),
    );
    let url_file = |url: &str| {
        url_map
            .get(&format!("URL_{}", mangle_file(url)))
            .and_then(|items| items.into_iter().next())
            .map(|item| item.pretty)
    };
    let discovery = discover(Inputs {
        rules: &rules,
        files: &files,
        read: &read,
        entries: &entries,
        manifest_variables: &manifest_variables,
        url_file: &url_file,
    });
    fs::write(
        index_root.join("js-environments.json"),
        serde_json::to_string(&discovery).unwrap(),
    )
    .expect("Couldn't write js-environments.json");
    // (Each file's, for its navigation panel; see per-file-info.toml.)
    fs::write(
        index_root.join("js-environments-files.json"),
        serde_json::to_string(&files_json(&discovery, 30, 10)).unwrap(),
    )
    .expect("Couldn't write js-environments-files.json");
    // (And every file in one, its root or one of its scripts, for diagrams of
    // a test's environments, which leave out the callers in other
    // environments' files; see `js_environments::scope`.)
    let mut paths: BTreeSet<&str> = BTreeSet::new();
    for env in &discovery.environments {
        paths.insert(&env.root);
        paths.extend(env.scripts.iter().map(|s| s.path.as_str()));
    }
    let mut paths_text = paths.into_iter().collect::<Vec<_>>().join("\n");
    paths_text.push('\n');
    fs::write(index_root.join("js-environments-paths"), paths_text)
        .expect("Couldn't write js-environments-paths");

    // ## The inventory.
    let mut kinds: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    let mut in_env: BTreeSet<&str> = BTreeSet::new();
    let mut pages_with_inline: BTreeSet<&str> = BTreeSet::new();
    for env in &discovery.environments {
        *kinds
            .entry((env.flavor.as_str(), env.global.as_str()))
            .or_default() += 1;
        for script in &env.scripts {
            if script.role == Role::Inline {
                pages_with_inline.insert(&script.path);
            } else {
                in_env.insert(&script.path);
            }
        }
    }
    println!(
        "js-environments: {} environments:",
        discovery.environments.len()
    );
    for ((flavor, global), n) in &kinds {
        println!("  {:>7} {} in {}", n, flavor, global);
    }
    let js_files: Vec<&String> = repo_files
        .iter()
        .filter(|p| JS_EXTENSIONS.iter().any(|e| p.ends_with(e)))
        .filter(|p| {
            !p.split('/')
                .any(|d| d == "third_party" || d == "node_modules")
        })
        .collect();
    let covered: HashSet<String> = args
        .get(4)
        .and_then(|p| fs::read_to_string(p).ok())
        .map(|s| s.lines().map(str::to_string).collect())
        .unwrap_or_default();
    let in_envs = js_files
        .iter()
        .filter(|p| in_env.contains(p.as_str()))
        .count();
    let by_ts = js_files
        .iter()
        .filter(|p| covered.contains(p.as_str()))
        .count();
    let neither: Vec<&&String> = js_files
        .iter()
        .filter(|p| !in_env.contains(p.as_str()) && !covered.contains(p.as_str()))
        .collect();
    println!(
        "js-environments: of {} JS files (outside third_party), {} are in an environment, {} in TypeScript projects, {} in neither; {} pages' inline scripts",
        js_files.len(),
        in_envs,
        by_ts,
        neither.len(),
        pages_with_inline.len()
    );
    let mut neither_dirs: BTreeMap<String, usize> = BTreeMap::new();
    for path in &neither {
        let dir: Vec<&str> = path.split('/').take(2).collect();
        *neither_dirs.entry(dir.join("/")).or_default() += 1;
    }
    let mut neither_dirs: Vec<(String, usize)> = neither_dirs.into_iter().collect();
    neither_dirs.sort_by(|a, b| b.1.cmp(&a.1));
    println!("js-environments: in neither, by directory:");
    for (dir, n) in neither_dirs.iter().take(25) {
        println!("  {:>7} {}", n, dir);
    }
    let mut unresolved: BTreeMap<(&str, bool), Vec<&tools::js_environments::discover::Unresolved>> =
        BTreeMap::new();
    for u in &discovery.unresolved {
        unresolved
            .entry((u.kind.as_str(), u.literal.is_some()))
            .or_default()
            .push(u);
    }
    println!(
        "js-environments: {} loads not followed:",
        discovery.unresolved.len()
    );
    for ((kind, literal), us) in &unresolved {
        println!(
            "  {:>7} {} ({})",
            us.len(),
            kind,
            if *literal {
                "a literal of a file the tree doesn't have"
            } else {
                "not a literal"
            }
        );
        for u in us.iter().take(3) {
            println!(
                "          {}:{} {}",
                u.path,
                u.line,
                u.literal.as_deref().unwrap_or("")
            );
        }
    }
}
