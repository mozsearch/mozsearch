//! A tree's JS environments: the globals its scripts run in, each with the
//! scripts loaded into it, in order, from its tests (by their manifests'
//! flavors; see `rules`), their pages, web-platform-tests' variants, and the
//! loads their scripts make (see `scan`): workers' globals, frames' globals,
//! and the scripts they load into their globals.  A script can be in many
//! environments (ex: dom/indexedDB's unit tests, under xpcshell and in their
//! mochitests' pages), with different scripts beside it.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use super::rules::{Rules, glob_set};
use super::scan::{Load, LoadKind, PageScan, scan_page, scan_script};
use crate::file_metadata::test_manifests::{TestEntry, join_normalized};

/// A script's role in an environment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    /// The flavor's (or testharness's) harness.
    Harness,
    /// A JS test's manifest `head` file.
    Head,
    /// The test itself (a JS test, or a worker's script).
    Test,
    /// A page's `<script src>`.
    Page,
    /// A page's inline `<script>`s (its own file, at the page's path).
    Inline,
    /// A web-platform-test's `META: script`.
    Meta,
    /// A script another loads (`importScripts`, the subscript loader, `load()`).
    Loaded,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Script {
    pub path: String,
    pub role: Role,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub module: bool,
    /// For a page's inline scripts: their lines (first and last of each).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inline: Vec<(u32, u32)>,
    /// What loaded it, if a script did: (path, line).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<(String, u32)>,
}

/// How an environment was started by another's script or page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Started {
    /// "worker", "shared-worker", "chrome-worker", "service-worker", "frame".
    pub how: String,
    pub from: (String, u32),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Environment {
    pub id: usize,
    /// Its global's kind (ex: "xpcshell", "window", "dedicatedworker").
    pub global: String,
    /// Its test flavor (ex: "xpcshell", "mochitest", "wpt").
    pub flavor: String,
    /// Its root: the test, page or worker script it runs.
    pub root: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ancestor: Option<String>,
    /// A web-platform-test's variant (ex: "dedicatedworker").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    pub scripts: Vec<Script>,
    /// The environment which started it (a worker's page, a frame's page).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started: Option<Started>,
}

/// A load it couldn't follow: not a literal, or a file the tree doesn't have.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Unresolved {
    pub path: String,
    pub line: u32,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub literal: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Discovery {
    pub environments: Vec<Environment>,
    pub unresolved: Vec<Unresolved>,
}

/// Its inputs: the tree's files, how to read them, and its test manifests'
/// entries (by test) with each manifest's moz.build variable (by path; an
/// included manifest has its including one's).
pub struct Inputs<'a> {
    pub rules: &'a Rules,
    pub files: &'a HashSet<String>,
    pub read: &'a (dyn Fn(&str) -> Option<String> + Sync),
    pub entries: &'a BTreeMap<String, Vec<TestEntry>>,
    pub manifest_variables: &'a HashMap<String, String>,
    /// The file a `chrome://` or `resource://` URL loads, if known (ex: by
    /// the tree's URL map, from the build's chrome map).
    pub url_file: &'a (dyn Fn(&str) -> Option<String> + Sync),
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

const SCRIPT_EXTENSIONS: &[&str] = &[".js", ".mjs", ".jsm", ".sjs"];
const PAGE_EXTENSIONS: &[&str] = &[".html", ".htm", ".xhtml", ".xht", ".xul"];

fn is_script(path: &str) -> bool {
    SCRIPT_EXTENSIONS.iter().any(|e| path.ends_with(e))
}

fn is_page(path: &str) -> bool {
    PAGE_EXTENSIONS.iter().any(|e| path.ends_with(e))
}

struct Discoverer<'a> {
    inputs: Inputs<'a>,
    out: Discovery,
    /// The loads not followed so far (each once, though its script is in
    /// many environments).
    unresolved_seen: HashSet<(String, u32, String, Option<String>)>,
    /// Environments' variables (see `rules::PathFlavor::variables`).
    variables: HashMap<usize, BTreeMap<String, String>>,
    /// Pages' scans, and scripts' loads, by path.
    pages: HashMap<String, Option<PageScan>>,
    loads: HashMap<String, Vec<Load>>,
}

impl<'a> Discoverer<'a> {
    fn read(&self, path: &str) -> Option<String> {
        (self.inputs.read)(path)
    }

    /// The file a URL (relative to `base_dir`, or one of the rules' URLs, or
    /// for a web-platform-test, path-absolute from its root) loads, if the
    /// tree has it.
    fn resolve(&self, url: &str, base_dir: &str, wpt: bool) -> Option<String> {
        let url = url.trim();
        let path = if let Some(file) = self.inputs.rules.url_file(url) {
            file
        } else if let Some(file) = (url.starts_with("chrome://") || url.starts_with("resource://"))
            .then(|| (self.inputs.url_file)(url.split(['?', '#']).next().unwrap_or(url)))
            .flatten()
        {
            file
        } else if url.contains("://") || url.starts_with("data:") || url.starts_with("blob:") {
            return None;
        } else {
            let url = url.split(['?', '#']).next().unwrap_or(url);
            match (url.strip_prefix('/'), &self.inputs.rules.wpt) {
                (Some(absolute), Some(wpt_rules)) if wpt => {
                    join_normalized(&wpt_rules.root, absolute)
                }
                (Some(_), _) => return None,
                (None, _) => join_normalized(base_dir, url),
            }
        };
        self.inputs.files.contains(&path).then_some(path)
    }

    /// Scan the tree's scripts and pages, in parallel, for `script_loads`,
    /// `page_scan` and `inline_loads`, as the discovery reaches most of them
    /// (on mozilla-central, 89,556 of its 104,009 scripts, and its pages with
    /// inline scripts), and scanning them (parsing them with tree-sitter) one
    /// at a time took most of its 81 seconds, on one core, on the 2026-11-06
    /// config8 run.
    fn scan_all(&mut self) {
        let read = self.inputs.read;
        let mut paths: Vec<&String> = self
            .inputs
            .files
            .iter()
            .filter(|path| is_script(path) || is_page(path))
            .collect();
        paths.sort();
        let scanned: Vec<(
            &String,
            Option<Vec<Load>>,
            Option<(Option<PageScan>, Vec<Load>)>,
        )> = paths
            .par_iter()
            .map(|path| {
                let source = read(path);
                let loads = is_script(path).then(|| {
                    source
                        .as_deref()
                        .map(|s| scan_script(s, 1))
                        .unwrap_or_default()
                });
                let page = is_page(path).then(|| {
                    let scan = source.as_deref().map(scan_page);
                    let mut inline = vec![];
                    if let (Some(source), Some(scan)) = (&source, &scan) {
                        for script in &scan.scripts {
                            if let Some(range) = &script.inline {
                                let first = super::scan::line_of(source, range.start);
                                inline.extend(scan_script(&source[range.clone()], first));
                            }
                        }
                    }
                    (scan, inline)
                });
                (*path, loads, page)
            })
            .collect();
        for (path, loads, page) in scanned {
            if let Some(loads) = loads {
                self.loads.insert(path.clone(), loads);
            }
            if let Some((scan, inline)) = page {
                self.loads.insert(format!("{}#inline", path), inline);
                self.pages.insert(path.clone(), scan);
            }
        }
    }

    fn page_scan(&mut self, path: &str) -> Option<&PageScan> {
        if !self.pages.contains_key(path) {
            let scan = self.read(path).map(|source| scan_page(&source));
            self.pages.insert(path.to_string(), scan);
        }
        self.pages.get(path).and_then(Option::as_ref)
    }

    fn script_loads(&mut self, path: &str) -> Vec<Load> {
        if !self.loads.contains_key(path) {
            let loads = self
                .read(path)
                .map(|s| scan_script(&s, 1))
                .unwrap_or_default();
            self.loads.insert(path.to_string(), loads);
        }
        self.loads[path].clone()
    }

    /// A page's inline scripts' loads.
    fn inline_loads(&mut self, path: &str) -> Vec<Load> {
        let key = format!("{}#inline", path);
        if !self.loads.contains_key(&key) {
            let mut loads = vec![];
            if let (Some(source), Some(scan)) = (self.read(path), self.page_scan(path).cloned()) {
                for script in &scan.scripts {
                    if let Some(range) = &script.inline {
                        let first = super::scan::line_of(&source, range.start);
                        loads.extend(scan_script(&source[range.clone()], first));
                    }
                }
            }
            self.loads.insert(key.clone(), loads);
        }
        self.loads[&key].clone()
    }

    fn unresolved(&mut self, path: &str, load: &Load) {
        self.note(
            path,
            load.line,
            &format!("{:?}", load.kind),
            load.literal.clone(),
        );
    }

    fn note(&mut self, path: &str, line: u32, kind: &str, literal: Option<String>) {
        let key = (path.to_string(), line, kind.to_string(), literal.clone());
        if self.unresolved_seen.insert(key) {
            self.out.unresolved.push(Unresolved {
                path: path.to_string(),
                line,
                kind: kind.to_string(),
                literal,
            });
        }
    }

    fn push(&mut self, mut env: Environment) -> usize {
        env.id = self.out.environments.len();
        self.out.environments.push(env);
        self.out.environments.len() - 1
    }

    /// A page's environment: its scripts (and the environments of its frames
    /// and of the workers its scripts start).
    fn page_environment(
        &mut self,
        page: &str,
        global: &str,
        flavor: &str,
        parent: Option<(usize, Started)>,
        depth: usize,
    ) -> Option<usize> {
        let scan = self.page_scan(page)?.clone();
        let wpt = flavor == "wpt";
        let base_dir = match &scan.base {
            Some(base) if !base.contains("://") => {
                join_normalized(dir_of(page), base.trim_end_matches('/'))
            }
            _ => dir_of(page).to_string(),
        };
        let mut scripts: Vec<Script> = vec![];
        let mut inline: Vec<(u32, u32)> = vec![];
        let source = self.read(page).unwrap_or_default();
        for script in &scan.scripts {
            if let Some(src) = &script.src {
                match self.resolve(src, &base_dir, wpt) {
                    Some(path) => scripts.push(Script {
                        path,
                        role: Role::Page,
                        module: script.module,
                        inline: vec![],
                        from: None,
                    }),
                    None => self.note(page, script.line, "Script", Some(src.clone())),
                }
            } else if let Some(range) = &script.inline {
                let first = super::scan::line_of(&source, range.start);
                let last = super::scan::line_of(&source, range.end);
                inline.push((first, last));
                // (The page's inline scripts are one file, the page's, where
                // the first is.)
                if inline.len() == 1 {
                    scripts.push(Script {
                        path: page.to_string(),
                        role: Role::Inline,
                        module: script.module,
                        inline: vec![],
                        from: None,
                    });
                }
            }
        }
        if let Some(s) = scripts.iter_mut().find(|s| s.role == Role::Inline) {
            s.inline = inline;
        }
        let (parent_id, started) = match parent {
            Some((id, started)) => (Some(id), Some(started)),
            None => (None, None),
        };
        let id = self.push(Environment {
            id: 0,
            global: global.to_string(),
            flavor: flavor.to_string(),
            root: page.to_string(),
            manifest: None,
            ancestor: None,
            variant: None,
            scripts,
            parent: parent_id,
            started,
        });
        self.expand(id, &base_dir, depth);
        // Its frames' environments.
        if depth < MAX_DEPTH {
            for (src, line) in &scan.frames {
                match self.resolve(src, &base_dir, wpt) {
                    Some(frame) if is_page(&frame) && frame != page => {
                        let started = Started {
                            how: "frame".to_string(),
                            from: (page.to_string(), *line),
                        };
                        self.page_environment(
                            &frame,
                            global,
                            flavor,
                            Some((id, started)),
                            depth + 1,
                        );
                    }
                    Some(_) => {}
                    None => self.note(page, *line, "Frame", Some(src.clone())),
                }
            }
        }
        Some(id)
    }

    /// Follow an environment's scripts' loads: the scripts they load into it
    /// (after the loading script), and the workers they start.
    fn expand(&mut self, id: usize, base_dir: &str, depth: usize) {
        let global = self.out.environments[id].global.clone();
        let flavor = self.out.environments[id].flavor.clone();
        let wpt = flavor == "wpt";
        let is_worker = global.ends_with("worker");
        let load_global = self.inputs.rules.load_globals.contains(&global);
        let env_root = self.out.environments[id].root.clone();
        let mut i = 0;
        let mut seen: HashSet<String> = self.out.environments[id]
            .scripts
            .iter()
            .map(|s| s.path.clone())
            .collect();
        while i < self.out.environments[id].scripts.len() {
            let script = self.out.environments[id].scripts[i].clone();
            i += 1;
            // (Not the harness's own loads, which are its business.)
            if script.role == Role::Harness {
                continue;
            }
            let loads = if script.role == Role::Inline {
                self.inline_loads(&script.path)
            } else {
                self.script_loads(&script.path)
            };
            // (Scripts loaded into the environment go after the one loading them.)
            let mut inserted = 0;
            for load in loads {
                let Some(literal) = &load.literal else {
                    self.unresolved(&script.path, &load);
                    continue;
                };
                // (A variable's concatenation, ex: jit-tests' `libdir + "x.js"`,
                // by the environment's variables.)
                if let Some(rest) = literal.strip_prefix("${") {
                    let (name, tail) = rest.split_once('}').unwrap_or((rest, ""));
                    let value = self.variables.get(&id).and_then(|v| v.get(name)).cloned();
                    let resolved = value
                        .map(|v| {
                            format!(
                                "{}{}",
                                v.replace("{dir}", &format!("{}/", dir_of(&env_root))),
                                tail
                            )
                        })
                        .filter(|p| self.inputs.files.contains(p));
                    let Some(path) = resolved else {
                        self.unresolved(&script.path, &load);
                        continue;
                    };
                    if matches!(
                        load.kind,
                        LoadKind::Load | LoadKind::SubScript | LoadKind::ImportScripts
                    ) && seen.insert(path.clone())
                    {
                        self.out.environments[id].scripts.insert(
                            i + inserted,
                            Script {
                                path,
                                role: Role::Loaded,
                                module: false,
                                inline: vec![],
                                from: Some((script.path.clone(), load.line)),
                            },
                        );
                        inserted += 1;
                    }
                    continue;
                }
                // (importScripts and the subscript loader's URLs are relative
                // to the loading script's; workers', to the environment's base.)
                let resolved = match load.kind {
                    LoadKind::ImportScripts if is_worker => {
                        self.resolve(literal, dir_of(&script.path), wpt)
                    }
                    LoadKind::Load if load_global => {
                        self.resolve(literal, dir_of(&script.path), wpt)
                    }
                    LoadKind::SubScript | LoadKind::Import => {
                        self.resolve(literal, dir_of(&script.path), wpt)
                    }
                    LoadKind::Worker
                    | LoadKind::SharedWorker
                    | LoadKind::ChromeWorker
                    | LoadKind::ServiceWorker => self.resolve(literal, base_dir, wpt),
                    _ => continue,
                };
                let Some(path) = resolved else {
                    self.unresolved(&script.path, &load);
                    continue;
                };
                match load.kind {
                    LoadKind::ImportScripts
                    | LoadKind::Load
                    | LoadKind::SubScript
                    | LoadKind::Import => {
                        if seen.insert(path.clone()) {
                            self.out.environments[id].scripts.insert(
                                i + inserted,
                                Script {
                                    path,
                                    role: Role::Loaded,
                                    module: load.kind == LoadKind::Import,
                                    inline: vec![],
                                    from: Some((script.path.clone(), load.line)),
                                },
                            );
                            inserted += 1;
                        }
                    }
                    _ if depth < MAX_DEPTH => {
                        let (how, worker_global) = match load.kind {
                            LoadKind::Worker => ("worker", "dedicatedworker"),
                            LoadKind::SharedWorker => ("shared-worker", "sharedworker"),
                            LoadKind::ChromeWorker => ("chrome-worker", "chromeworker"),
                            _ => ("service-worker", "serviceworker"),
                        };
                        let started = Started {
                            how: how.to_string(),
                            from: (script.path.clone(), load.line),
                        };
                        let worker = self.push(Environment {
                            id: 0,
                            global: worker_global.to_string(),
                            flavor: flavor.clone(),
                            root: path.clone(),
                            manifest: None,
                            ancestor: None,
                            variant: None,
                            scripts: vec![Script {
                                path: path.clone(),
                                role: Role::Test,
                                module: false,
                                inline: vec![],
                                from: None,
                            }],
                            parent: Some(id),
                            started: Some(started),
                        });
                        self.expand(worker, dir_of(&path), depth + 1);
                    }
                    _ => {}
                }
            }
        }
    }

    /// A JS test's environment: its flavor's harness, its head files, and it.
    fn script_test(&mut self, test: &str, entry: &TestEntry, flavor: &super::rules::Flavor) {
        let dir = dir_of(test);
        let mut scripts: Vec<Script> = flavor
            .harness
            .iter()
            .filter(|h| self.inputs.files.contains(*h))
            .map(|h| Script {
                path: h.clone(),
                role: Role::Harness,
                module: false,
                inline: vec![],
                from: None,
            })
            .collect();
        for implicit in &flavor.implicit_head {
            let path = join_normalized(dir, implicit);
            let listed = entry
                .values
                .get("head")
                .is_some_and(|h| h.contains(implicit));
            if !listed && self.inputs.files.contains(&path) {
                scripts.push(Script {
                    path,
                    role: Role::Head,
                    module: false,
                    inline: vec![],
                    from: None,
                });
            }
        }
        for head in entry.values.get("head").into_iter().flatten() {
            let path = join_normalized(dir, head);
            if self.inputs.files.contains(&path) {
                scripts.push(Script {
                    path,
                    role: Role::Head,
                    module: false,
                    inline: vec![],
                    from: None,
                });
            } else {
                self.note(
                    &entry.manifest,
                    entry.line as u32,
                    "Head",
                    Some(head.clone()),
                );
            }
        }
        scripts.push(Script {
            path: test.to_string(),
            role: Role::Test,
            module: test.ends_with(".mjs"),
            inline: vec![],
            from: None,
        });
        let id = self.push(Environment {
            id: 0,
            global: flavor.global.clone(),
            flavor: flavor.name.clone(),
            root: test.to_string(),
            manifest: Some(entry.manifest.clone()),
            ancestor: entry.ancestor.clone(),
            variant: None,
            scripts,
            parent: None,
            started: None,
        });
        self.expand(id, dir, 0);
    }

    fn manifest_tests(&mut self) {
        let entries = self.inputs.entries;
        let rules = self.inputs.rules;
        let manifest_variables = self.inputs.manifest_variables;
        for (test, test_entries) in entries {
            if !self.inputs.files.contains(test) {
                continue;
            }
            for entry in test_entries {
                // (An included manifest's flavor is its including one's.)
                let variable = entry
                    .ancestor
                    .as_ref()
                    .and_then(|a| manifest_variables.get(a))
                    .or_else(|| manifest_variables.get(&entry.manifest));
                let Some(flavor) = variable.and_then(|v| {
                    rules
                        .flavors
                        .iter()
                        .find(|f| f.manifest_variables.contains(v))
                }) else {
                    continue;
                };
                if is_script(test) {
                    self.script_test(test, entry, flavor);
                } else if is_page(test)
                    && let Some(id) =
                        self.page_environment(test, &flavor.global, &flavor.name, None, 0)
                {
                    let env = &mut self.out.environments[id];
                    env.manifest = Some(entry.manifest.clone());
                    env.ancestor = entry.ancestor.clone();
                }
            }
        }
    }

    /// Tests found by their paths (see `rules::PathFlavor`).
    fn path_tests(&mut self) {
        let rules = self.inputs.rules;
        let files = self.inputs.files;
        for flavor in &rules.path_flavors {
            let (include, exclude) = (glob_set(&flavor.include), glob_set(&flavor.exclude));
            let mut tests: Vec<&String> = files
                .iter()
                .filter(|p| include.is_match(p.as_str()) && !exclude.is_match(p.as_str()))
                .collect();
            tests.sort();
            for test in tests {
                let mut scripts: Vec<Script> = flavor
                    .harness
                    .iter()
                    .filter(|h| files.contains(*h))
                    .map(|h| Script {
                        path: h.clone(),
                        role: Role::Harness,
                        module: false,
                        inline: vec![],
                        from: None,
                    })
                    .collect();
                // (Each directory's heads, from the root's down to the test's.)
                if let Some(root) = &flavor.dir_heads_root {
                    let dir = dir_of(test);
                    let mut dirs = vec![];
                    let mut d = dir;
                    while d.len() >= root.len() && d.starts_with(root.as_str()) {
                        dirs.push(d);
                        if d == root {
                            break;
                        }
                        d = dir_of(d);
                    }
                    for d in dirs.into_iter().rev() {
                        for head in &flavor.dir_heads {
                            let path = format!("{}/{}", d, head);
                            if &path != test && files.contains(&path) {
                                scripts.push(Script {
                                    path,
                                    role: Role::Head,
                                    module: false,
                                    inline: vec![],
                                    from: None,
                                });
                            }
                        }
                    }
                }
                scripts.push(Script {
                    path: test.clone(),
                    role: Role::Test,
                    module: test.ends_with(".mjs"),
                    inline: vec![],
                    from: None,
                });
                let id = self.push(Environment {
                    id: 0,
                    global: flavor.global.clone(),
                    flavor: flavor.name.clone(),
                    root: test.clone(),
                    manifest: None,
                    ancestor: None,
                    variant: None,
                    scripts,
                    parent: None,
                    started: None,
                });
                if !flavor.variables.is_empty() {
                    self.variables.insert(id, flavor.variables.clone());
                }
                self.expand(id, dir_of(test), 0);
            }
        }
    }

    /// web-platform-tests: a `.any.js` test's variants (`META: global=`, by
    /// default a window and a dedicated worker), `.window.js` and
    /// `.worker.js` tests, and pages loading testharness.js.
    fn wpt_tests(&mut self) {
        let rules = self.inputs.rules;
        let Some(wpt) = &rules.wpt else {
            return;
        };
        let root = format!("{}/", wpt.root);
        let window_harness: Vec<String> = wpt
            .window_harness
            .iter()
            .map(|h| join_normalized(&wpt.root, h))
            .collect();
        let worker_harness: Vec<String> = wpt
            .worker_harness
            .iter()
            .map(|h| join_normalized(&wpt.root, h))
            .collect();
        let files = self.inputs.files;
        let mut tests: Vec<&String> = files.iter().filter(|p| p.starts_with(&root)).collect();
        tests.sort();
        for test in tests {
            let name = test.rsplit('/').next().unwrap_or(test);
            if is_wpt_support(&test[root.len()..]) {
                continue;
            }
            let variants: Vec<&str> = if name.ends_with(".any.js") {
                let Some(source) = self.read(test) else {
                    continue;
                };
                any_variants(&source)
            } else if name.ends_with(".window.js") {
                vec!["window"]
            } else if name.ends_with(".worker.js") {
                vec!["dedicatedworker"]
            } else if is_page(test) {
                let loads_harness = self.page_scan(test).is_some_and(|scan| {
                    scan.scripts.iter().any(|s| {
                        s.src
                            .as_deref()
                            .is_some_and(|s| s.ends_with("/resources/testharness.js"))
                    })
                });
                if loads_harness {
                    self.page_environment(test, "window", "wpt", None, 0);
                }
                continue;
            } else {
                continue;
            };
            let source = self.read(test).unwrap_or_default();
            let metas = meta_scripts(&source);
            for variant in variants {
                let worker = variant != "window";
                let harness = if worker {
                    &worker_harness
                } else {
                    &window_harness
                };
                let mut scripts: Vec<Script> = harness
                    .iter()
                    .filter(|h| self.inputs.files.contains(*h))
                    .map(|h| Script {
                        path: h.clone(),
                        role: Role::Harness,
                        module: false,
                        inline: vec![],
                        from: None,
                    })
                    .collect();
                for (meta, line) in &metas {
                    match self.resolve(meta, dir_of(test), true) {
                        Some(path) => scripts.push(Script {
                            path,
                            role: Role::Meta,
                            module: false,
                            inline: vec![],
                            from: Some((test.clone(), *line)),
                        }),
                        None => self.note(test, *line, "Meta", Some(meta.clone())),
                    }
                }
                scripts.push(Script {
                    path: test.clone(),
                    role: Role::Test,
                    module: variant.ends_with("-module"),
                    inline: vec![],
                    from: None,
                });
                let id = self.push(Environment {
                    id: 0,
                    global: variant.trim_end_matches("-module").to_string(),
                    flavor: "wpt".to_string(),
                    root: test.clone(),
                    manifest: None,
                    ancestor: None,
                    variant: Some(variant.to_string()),
                    scripts,
                    parent: None,
                    started: None,
                });
                self.expand(id, dir_of(test), 0);
            }
        }
    }
}

/// How deep frames and workers are followed (a worker's worker's ...).
const MAX_DEPTH: usize = 3;

/// web-platform-tests' support files, which aren't tests (see its manifest's
/// sourcefile.py): those in its resources, support, tools and common
/// directories, and those whose names start with `_` or contain `-ref`.
fn is_wpt_support(rel: &str) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    let name = parts.last().copied().unwrap_or("");
    parts[..parts.len() - 1]
        .iter()
        .any(|d| matches!(*d, "resources" | "support" | "tools" | "common" | "_venv3"))
        || name.starts_with('_')
        || name.contains("-ref.")
        || name.contains("-ref-")
        || rel.starts_with("tools/")
}

/// A `.any.js` test's variants, by its `META: global=` (wpt's sourcefile.py's
/// `global_suffixes`): window and dedicatedworker by default, `worker` being
/// dedicated, shared and service workers, and `-module` ones; not the
/// shadowrealm ones (no global type for them yet).
fn any_variants(source: &str) -> Vec<&'static str> {
    let mut globals: BTreeSet<&'static str> = BTreeSet::new();
    let mut seen_meta = false;
    for line in source
        .lines()
        .take_while(|l| l.starts_with("//") || l.trim().is_empty())
    {
        let Some(meta) = line.strip_prefix("// META:") else {
            continue;
        };
        let Some(value) = meta.trim().strip_prefix("global=") else {
            continue;
        };
        seen_meta = true;
        for global in value.split(',') {
            let global = global.trim();
            let (negated, global) = match global.strip_prefix('!') {
                Some(g) => (true, g),
                None => (false, global),
            };
            let expanded: &[&'static str] = match global {
                "window" => &["window"],
                "worker" => &["dedicatedworker", "sharedworker", "serviceworker"],
                "dedicatedworker" => &["dedicatedworker"],
                "sharedworker" => &["sharedworker"],
                "serviceworker" => &["serviceworker"],
                "dedicatedworker-module" => &["dedicatedworker-module"],
                "sharedworker-module" => &["sharedworker-module"],
                "serviceworker-module" => &["serviceworker-module"],
                "worker-module" => &[
                    "dedicatedworker-module",
                    "sharedworker-module",
                    "serviceworker-module",
                ],
                "default" => &["window", "dedicatedworker"],
                _ => &[],
            };
            for g in expanded {
                if negated {
                    globals.remove(g);
                } else {
                    globals.insert(g);
                }
            }
        }
    }
    if !seen_meta {
        return vec!["window", "dedicatedworker"];
    }
    globals.into_iter().collect()
}

/// A web-platform-test's `META: script=` URLs, with their lines.
fn meta_scripts(source: &str) -> Vec<(String, u32)> {
    source
        .lines()
        .enumerate()
        .take_while(|(_, l)| l.starts_with("//") || l.trim().is_empty())
        .filter_map(|(i, l)| {
            let meta = l.strip_prefix("// META:")?.trim();
            meta.strip_prefix("script=")
                .map(|s| (s.trim().to_string(), i as u32 + 1))
        })
        .collect()
}

/// Discover a tree's environments.
pub fn discover(inputs: Inputs) -> Discovery {
    let mut d = Discoverer {
        inputs,
        out: Discovery::default(),
        pages: HashMap::new(),
        loads: HashMap::new(),
        unresolved_seen: HashSet::new(),
        variables: HashMap::new(),
    };
    d.scan_all();
    d.manifest_tests();
    d.path_tests();
    d.wpt_tests();
    d.out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_any_variants() {
        assert_eq!(any_variants("test();\n"), vec!["window", "dedicatedworker"]);
        assert_eq!(
            any_variants(
                "// META: global=window,worker\n// META: script=/common/utils.js\ntest();\n"
            ),
            vec!["dedicatedworker", "serviceworker", "sharedworker", "window"]
        );
        assert_eq!(
            any_variants("// META: global=worker,!serviceworker\n"),
            vec!["dedicatedworker", "sharedworker"]
        );
        assert_eq!(
            meta_scripts(
                "// META: global=window\n// META: script=/common/utils.js\n// META: script=resources/x.js\n"
            ),
            vec![
                ("/common/utils.js".to_string(), 2),
                ("resources/x.js".to_string(), 3)
            ]
        );
    }

    #[test]
    fn test_wpt_support() {
        assert!(is_wpt_support("fetch/api/resources/utils.js"));
        assert!(is_wpt_support("css/foo-ref.html"));
        assert!(!is_wpt_support("fetch/api/basic/request.any.js"));
    }
}

/// Each file's environments, for its navigation panel (as
/// `{"files": {"all": [{"path", "roots": [tree], "in": [..], "in_count"}]}}`,
/// which per-file-info.toml ingests): the environments it's the root of (a
/// test, or a page), each a tree of its scripts and of the environments it
/// starts (its workers and frames, with theirs); and the others it's in (at
/// most `max_in`, of `in_count`), with their roots, flavors, globals and its
/// role, the first `max_trees` with their trees.
pub fn files_json(discovery: &Discovery, max_in: usize, max_trees: usize) -> serde_json::Value {
    use serde_json::{Value, json};
    let envs = &discovery.environments;
    let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
    for env in envs {
        if let Some(parent) = env.parent {
            children.entry(parent).or_default().push(env.id);
        }
    }
    fn tree(
        envs: &[Environment],
        children: &HashMap<usize, Vec<usize>>,
        id: usize,
        depth: usize,
    ) -> Value {
        let env = &envs[id];
        let mut value = json!({
            "flavor": env.flavor,
            "global": env.global,
            "root": env.root,
            "scripts": env.scripts,
        });
        for (key, field) in [
            ("manifest", &env.manifest),
            ("ancestor", &env.ancestor),
            ("variant", &env.variant),
        ] {
            if let Some(field) = field {
                value[key] = json!(field);
            }
        }
        if let Some(started) = &env.started {
            value["started"] = json!(started);
        }
        if depth < MAX_DEPTH + 1 {
            let kids: Vec<Value> = children
                .get(&id)
                .into_iter()
                .flatten()
                .map(|child| tree(envs, children, *child, depth + 1))
                .collect();
            if !kids.is_empty() {
                value["children"] = json!(kids);
            }
        }
        value
    }
    // (By file: the environments it's the root of, and those it's in.)
    let mut roots: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    let mut ins: BTreeMap<&str, Vec<(usize, Role)>> = BTreeMap::new();
    for env in envs {
        if env.parent.is_none() {
            roots.entry(&env.root).or_default().push(env.id);
        }
        // (A worker's or frame's environment's root is in it as its root.)
        for script in &env.scripts {
            if env.parent.is_none() && script.path == env.root {
                continue;
            }
            let entry = ins.entry(&script.path).or_default();
            if !entry.iter().any(|(id, _)| *id == env.id) {
                entry.push((env.id, script.role));
            }
        }
    }
    let top = |id: usize| {
        let mut id = id;
        while let Some(parent) = envs[id].parent {
            id = parent;
        }
        id
    };
    let paths: BTreeSet<&str> = roots.keys().chain(ins.keys()).copied().collect();
    let mut all = vec![];
    for path in paths {
        let mut file = json!({ "path": path });
        if let Some(ids) = roots.get(path) {
            file["roots"] = json!(
                ids.iter()
                    .map(|id| tree(envs, &children, *id, 0))
                    .collect::<Vec<_>>()
            );
        }
        if let Some(entries) = ins.get(path) {
            let listed: Vec<Value> = entries
                .iter()
                .take(max_in)
                .enumerate()
                .map(|(i, (id, role))| {
                    let env = &envs[*id];
                    let top_env = &envs[top(*id)];
                    let mut value = json!({
                        "root": top_env.root,
                        "env_root": env.root,
                        "flavor": env.flavor,
                        "global": env.global,
                        "role": role,
                    });
                    // (Its test's manifest, which tells apart its runs, ex: of
                    // a manifest which two others include.)
                    for (key, field) in [
                        ("manifest", &top_env.manifest),
                        ("ancestor", &top_env.ancestor),
                        ("variant", &env.variant),
                    ] {
                        if let Some(field) = field {
                            value[key] = json!(field);
                        }
                    }
                    if i < max_trees {
                        value["tree"] = tree(envs, &children, top(*id), 0);
                    }
                    value
                })
                .collect();
            file["in"] = json!(listed);
            file["in_count"] = json!(entries.len());
        }
        all.push(file);
    }
    json!({ "files": { "all": all } })
}
