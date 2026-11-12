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

use super::rules::{Rules, WptRules, glob_set};
use super::scan::{
    LinkKind, Load, LoadKind, PageLink, PageScan, line_and_column, scan_page, scan_script,
    scan_stylesheet,
};
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
    /// A Test262 test's frontmatter's `includes`.
    Include,
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
    /// web-platform-tests' files' links to the tree's files (see
    /// `wpt_links`), by path (written to a file of their own).
    #[serde(skip)]
    pub links: BTreeMap<String, Vec<FileLink>>,
    /// What it took from the files' sources (written to a file of their own).
    #[serde(skip)]
    pub facts: Facts,
}

/// A web-platform-tests page's or stylesheet's links, with their URLs'
/// places (the lines and columns of their starts and ends), and a page's
/// `<base>`'s `href`.
pub type LinkScan = (Vec<(PageLink, (u32, u32), (u32, u32))>, Option<String>);

/// What discovery takes from the tree's files' sources by parsing them (most
/// of its work; see `Discoverer::scan_all` and `wpt_links`), by path:
/// scripts' loads (and pages' inline scripts', by `{page}#inline`), pages'
/// scans, and web-platform-tests' pages' and stylesheets' links.  A try
/// push's discovery takes its tree's for the files it doesn't change (see
/// `Inputs::prior`), its head's other files being the tree's (its stack
/// rebased onto the indexed revision).
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Facts {
    pub loads: HashMap<String, Vec<Load>>,
    pub pages: HashMap<String, Option<PageScan>>,
    pub links: HashMap<String, LinkScan>,
}

/// A file's link to another file of the tree (see `scan::LinkKind`): its
/// URL's place (its line, and its columns, in bytes), and the file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileLink {
    pub kind: LinkKind,
    pub line: u32,
    pub start: u32,
    pub end: u32,
    pub target: String,
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
    /// Facts to take for the files but some (ex: a try push's tree's, but for
    /// the files the push changes), rather than parsing them again.
    pub prior: Option<(&'a Facts, &'a HashSet<String>)>,
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

const SCRIPT_EXTENSIONS: &[&str] = &[".js", ".mjs", ".jsm", ".sjs"];
const PAGE_EXTENSIONS: &[&str] = &[".html", ".htm", ".xhtml", ".xht", ".xul", ".svg"];

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
    /// web-platform-tests' files' link scans, by path (see `wpt_links`).
    link_scans: HashMap<String, LinkScan>,
}

impl<'a> Discoverer<'a> {
    fn read(&self, path: &str) -> Option<String> {
        (self.inputs.read)(path)
    }

    /// The prior facts to take for `path`, if any (see `Inputs::prior`).
    fn prior(&self, path: &str) -> Option<&'a Facts> {
        self.inputs
            .prior
            .filter(|(_, changed)| !changed.contains(path))
            .map(|(facts, _)| facts)
    }

    /// The file a URL (relative to `base_dir`, or one of the rules' URLs, or
    /// for a web-platform-test, path-absolute from its root, or from an extra
    /// root's, by its prefix) loads, if the tree has it.
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
                    match wpt_rules
                        .extra_roots
                        .iter()
                        .find(|(prefix, _)| url.starts_with(prefix.as_str()))
                    {
                        Some((prefix, dir)) => join_normalized(dir, &url[prefix.len()..]),
                        None => join_normalized(&wpt_rules.root, absolute),
                    }
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
    /// config8 run.  (But those it has prior facts for; see `Inputs::prior`.)
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
                if let Some(facts) = self.prior(path) {
                    let loads = is_script(path)
                        .then(|| facts.loads.get(path.as_str()).cloned())
                        .flatten();
                    let page = is_page(path)
                        .then(|| {
                            let scan = facts.pages.get(path.as_str())?;
                            let inline = facts.loads.get(&format!("{}#inline", path))?;
                            Some((scan.clone(), inline.clone()))
                        })
                        .flatten();
                    if loads.is_some() || page.is_some() {
                        return (*path, loads, page);
                    }
                }
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
        } else if !scan.handlers.is_empty() {
            // (Its event handlers are its inline code too.)
            scripts.push(Script {
                path: page.to_string(),
                role: Role::Inline,
                module: false,
                inline: vec![],
                from: None,
            });
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

    /// web-platform-tests, in each of its roots: a `.any.js` test's variants
    /// (`META: global=`, by default a window and a dedicated worker),
    /// `.worker.js`, `.window.js` and `.extension.js` tests (by its manifest's
    /// rules: tools/manifest/sourcefile.py's, by its name's flags, in that
    /// order), and its pages with scripts or event handlers (tests, as its
    /// manifest has those loading testharness.js, references, crash tests,
    /// manual tests; and its support pages which no test loads as a frame),
    /// each in a window of its own; and its Test262 tests (see `wpt_test262`).
    fn wpt_tests(&mut self) {
        let rules = self.inputs.rules;
        let Some(wpt) = &rules.wpt else {
            return;
        };
        let harness_of = |list: &[String]| -> Vec<String> {
            list.iter().map(|h| join_normalized(&wpt.root, h)).collect()
        };
        let window_harness = harness_of(&wpt.window_harness);
        let worker_harness = harness_of(&wpt.worker_harness);
        let extension_harness = if wpt.extension_harness.is_empty() {
            window_harness.clone()
        } else {
            harness_of(&wpt.extension_harness)
        };
        let test262_dirs: Vec<String> = wpt
            .test262
            .iter()
            .flat_map(|t| &t.tests)
            .map(|d| format!("{}/", join_normalized(&wpt.root, d)))
            .collect();
        let files = self.inputs.files;
        let roots: Vec<String> = std::iter::once(format!("{}/", wpt.root))
            .chain(wpt.extra_roots.values().map(|r| format!("{}/", r)))
            .collect();
        let no_harness: Vec<String> = vec![];
        let mut support_pages: Vec<&String> = vec![];
        for root in &roots {
            let mut tests: Vec<&String> = files
                .iter()
                .filter(|p| p.starts_with(root.as_str()))
                .collect();
            tests.sort();
            for test in tests {
                let name = test.rsplit('/').next().unwrap_or(test);
                if test262_dirs.iter().any(|d| test.starts_with(d.as_str())) {
                    continue;
                }
                if is_wpt_support(&test[root.len()..]) {
                    if is_page(test) {
                        support_pages.push(test);
                    }
                    continue;
                }
                let flags: Vec<&str> = name.split('.').skip(1).collect();
                let js = name.ends_with(".js");
                let (variants, harness, metas): (Vec<&str>, Option<&Vec<String>>, bool) =
                    if js && flags.contains(&"any") {
                        let Some(source) = self.read(test) else {
                            continue;
                        };
                        (any_variants(&source), None, true)
                    } else if js && flags.contains(&"worker") {
                        // (Its page starts it as it is: it loads testharness.js
                        // itself, and its META scripts aren't loaded.)
                        (vec!["dedicatedworker"], Some(&no_harness), false)
                    } else if js && flags.contains(&"window") {
                        (vec!["window"], Some(&window_harness), true)
                    } else if js && flags.contains(&"extension") {
                        (vec!["window"], Some(&extension_harness), true)
                    } else {
                        if is_page(test)
                            && self
                                .page_scan(test)
                                .is_some_and(|s| !s.scripts.is_empty() || !s.handlers.is_empty())
                        {
                            self.page_environment(test, "window", "wpt", None, 0);
                        }
                        continue;
                    };
                let source = self.read(test).unwrap_or_default();
                let metas = if metas { meta_scripts(&source) } else { vec![] };
                for variant in variants {
                    let worker = variant.trim_end_matches("-module") != "window";
                    let harness = harness.unwrap_or(if worker {
                        &worker_harness
                    } else {
                        &window_harness
                    });
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
        // (Support pages, but those an environment has already: ex, a test's
        // frame.)
        let mut have: HashSet<String> = self
            .out
            .environments
            .iter()
            .map(|e| e.root.clone())
            .collect();
        for page in support_pages {
            if have.contains(page)
                || !self
                    .page_scan(page)
                    .is_some_and(|s| !s.scripts.is_empty() || !s.handlers.is_empty())
            {
                continue;
            }
            let first = self.out.environments.len();
            self.page_environment(page, "window", "wpt", None, 0);
            have.extend(
                self.out.environments[first..]
                    .iter()
                    .map(|e| e.root.clone()),
            );
        }
        self.wpt_test262(wpt);
        self.out.links = self.wpt_links(&roots);
    }

    /// web-platform-tests' files' links to the tree's files: its pages'
    /// (relative to their `<base>`, if any) and its stylesheets', by their
    /// URLs as web-platform-tests resolves them (see `resolve`).
    fn wpt_links(&mut self, roots: &[String]) -> BTreeMap<String, Vec<FileLink>> {
        let files = self.inputs.files;
        let read = self.inputs.read;
        let under_roots = |p: &String| roots.iter().any(|r| p.starts_with(r.as_str()));
        let mut paths: Vec<&String> = files
            .iter()
            .filter(|p| under_roots(p) && (is_page(p) || p.ends_with(".css")))
            .collect();
        paths.sort();
        // (Each file's links, with their sources' positions, in parallel; but
        // those it has prior facts for.)
        let scanned: Vec<(
            &String,
            Vec<(PageLink, (u32, u32), (u32, u32))>,
            Option<String>,
        )> = paths
            .par_iter()
            .map(|path| {
                if let Some((links, base)) = self
                    .prior(path)
                    .and_then(|facts| facts.links.get(path.as_str()))
                {
                    return (*path, links.clone(), base.clone());
                }
                let Some(source) = read(path) else {
                    return (*path, vec![], None);
                };
                let (links, base) = if path.ends_with(".css") {
                    (scan_stylesheet(&source), None)
                } else {
                    let scan = scan_page(&source);
                    (scan.links, scan.base)
                };
                let links = links
                    .into_iter()
                    .map(|link| {
                        let start = line_and_column(&source, link.value.start);
                        let end = line_and_column(&source, link.value.end);
                        (link, start, end)
                    })
                    .collect();
                (*path, links, base)
            })
            .collect();
        let mut out: BTreeMap<String, Vec<FileLink>> = BTreeMap::new();
        for (path, links, base) in scanned {
            self.link_scans
                .insert(path.clone(), (links.clone(), base.clone()));
            let base_dir = match &base {
                Some(base) if !base.contains("://") => {
                    join_normalized(dir_of(path), base.trim_end_matches('/'))
                }
                _ => dir_of(path).to_string(),
            };
            for (link, (line, start), (end_line, end)) in links {
                // (A link's URL on one line; and one to another file.)
                if end_line != line {
                    continue;
                }
                let Some(target) = self.resolve(&link.url, &base_dir, true) else {
                    continue;
                };
                if &target == path {
                    continue;
                }
                out.entry(path.clone()).or_default().push(FileLink {
                    kind: link.kind,
                    line,
                    start,
                    end,
                    target,
                });
            }
        }
        out
    }

    /// web-platform-tests' Test262 tests (see `rules::Test262Rules`), each in
    /// a window (its variant `test262`, or `test262-module` for a module).
    fn wpt_test262(&mut self, wpt: &WptRules) {
        let Some(test262) = &wpt.test262 else {
            return;
        };
        let files = self.inputs.files;
        let dirs: Vec<String> = test262
            .tests
            .iter()
            .map(|d| format!("{}/", join_normalized(&wpt.root, d)))
            .collect();
        let in_tree = |list: &[String]| -> Vec<String> {
            list.iter()
                .map(|h| join_normalized(&wpt.root, h))
                .filter(|h| files.contains(h))
                .collect()
        };
        let harness = in_tree(&test262.harness);
        let async_harness = in_tree(&test262.async_harness);
        let includes_dir = join_normalized(&wpt.root, &test262.includes_dir);
        let mut tests: Vec<&String> = files
            .iter()
            .filter(|p| {
                p.ends_with(".js")
                    && !p.ends_with("_FIXTURE.js")
                    && dirs.iter().any(|d| p.starts_with(d.as_str()))
            })
            .collect();
        tests.sort();
        let read = self.inputs.read;
        let records: Vec<Option<Test262Record>> = tests
            .par_iter()
            .map(|test| read(test).and_then(|source| test262_record(&source)))
            .collect();
        for (test, record) in tests.into_iter().zip(records) {
            // (Without frontmatter, it's a support file.)
            let Some(record) = record else {
                continue;
            };
            let harness_script = |path: &String| Script {
                path: path.clone(),
                role: Role::Harness,
                module: false,
                inline: vec![],
                from: None,
            };
            let mut scripts: Vec<Script> = harness.iter().map(harness_script).collect();
            for include in &record.includes {
                let path = join_normalized(&includes_dir, include);
                if files.contains(&path) {
                    scripts.push(Script {
                        path,
                        role: Role::Include,
                        module: false,
                        inline: vec![],
                        from: Some((test.clone(), record.includes_line)),
                    });
                } else {
                    self.note(test, record.includes_line, "Include", Some(include.clone()));
                }
            }
            if record.is_async {
                scripts.extend(async_harness.iter().map(harness_script));
            }
            scripts.push(Script {
                path: test.clone(),
                role: Role::Test,
                module: record.module,
                inline: vec![],
                from: None,
            });
            let id = self.push(Environment {
                id: 0,
                global: "window".to_string(),
                flavor: "wpt".to_string(),
                root: test.clone(),
                manifest: None,
                ancestor: None,
                variant: Some(
                    if record.module {
                        "test262-module"
                    } else {
                        "test262"
                    }
                    .to_string(),
                ),
                scripts,
                parent: None,
                started: None,
            });
            self.expand(id, dir_of(test), 0);
        }
    }
}

/// A Test262 test's frontmatter's (`/*--- ... ---*/`, YAML) `includes`, with
/// their line, and whether its `flags` have `module` and `async`.
#[derive(Debug, Default, PartialEq)]
struct Test262Record {
    includes: Vec<String>,
    includes_line: u32,
    module: bool,
    is_async: bool,
}

/// A Test262 test's record, if it has frontmatter (as wpt's
/// tools/manifest/test262.py reads it: only its `includes` and `flags`, each
/// a flow list, `[a, b]`, or a block one, of `- a` lines).
fn test262_record(source: &str) -> Option<Test262Record> {
    let start = source.find("/*---")? + "/*---".len();
    let end = start + source[start..].find("---*/")?;
    let first_line = super::scan::line_of(source, start);
    let mut record = Test262Record::default();
    let lines: Vec<&str> = source[start..end].lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let key_line = first_line + i as u32;
        i += 1;
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.starts_with(char::is_whitespace) || !matches!(key, "includes" | "flags") {
            continue;
        }
        let value = value.split(" #").next().unwrap_or("").trim();
        let mut items: Vec<String> = vec![];
        if let Some(list) = value.strip_prefix('[') {
            items.extend(
                list.trim_end_matches(']')
                    .split(',')
                    .map(|v| v.trim().trim_matches(['"', '\'']).to_string())
                    .filter(|v| !v.is_empty()),
            );
        } else if value.is_empty() {
            while i < lines.len() {
                let Some(item) = lines[i].trim_start().strip_prefix("- ") else {
                    break;
                };
                if !lines[i].starts_with(char::is_whitespace) {
                    break;
                }
                items.push(
                    item.split(" #")
                        .next()
                        .unwrap_or("")
                        .trim()
                        .trim_matches(['"', '\''])
                        .to_string(),
                );
                i += 1;
            }
        }
        if key == "includes" {
            record.includes = items;
            record.includes_line = key_line;
        } else {
            record.module = items.iter().any(|f| f == "module");
            record.is_async = items.iter().any(|f| f == "async");
        }
    }
    Some(record)
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

/// A web-platform-test's `META:` lines, as wpt's sourcefile.py reads them
/// (`read_script_metadata`, `//\s*META:\s*(\w*)=(.*)$`): its first lines, up
/// to the first that isn't one, each as (key, value, line).
fn meta_lines(source: &str) -> impl Iterator<Item = (&str, &str, u32)> {
    source.lines().enumerate().map_while(|(i, line)| {
        let meta = line
            .strip_prefix("//")?
            .trim_start()
            .strip_prefix("META:")?
            .trim_start();
        let (key, value) = meta.split_once('=')?;
        key.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            .then_some((key, value, i as u32 + 1))
    })
}

/// A `.any.js` test's variants, by its first `META: global=` (wpt's
/// sourcefile.py's `parse_variants`): window and dedicatedworker without one
/// (or with an empty one), `worker` being dedicated, shared and service
/// workers, and `-module` ones (`worker-module` a dedicated worker's, as
/// serve.py's WorkerModulesHandler runs it); not the shadowrealm ones or
/// jsshell (no global type for them yet), or keywords wpt doesn't have.
fn any_variants(source: &str) -> Vec<&'static str> {
    let Some((_, value, _)) = meta_lines(source).find(|(key, _, _)| *key == "global") else {
        return vec!["window", "dedicatedworker"];
    };
    if value.is_empty() {
        return vec!["window", "dedicatedworker"];
    }
    let mut globals: BTreeSet<&'static str> = BTreeSet::new();
    for global in value.split(',') {
        let expanded: &[&'static str] = match global.trim() {
            "window" => &["window"],
            "window-module" => &["window-module"],
            "worker" => &["dedicatedworker", "sharedworker", "serviceworker"],
            "dedicatedworker" => &["dedicatedworker"],
            "sharedworker" => &["sharedworker"],
            "serviceworker" => &["serviceworker"],
            "worker-module" | "dedicatedworker-module" => &["dedicatedworker-module"],
            "sharedworker-module" => &["sharedworker-module"],
            "serviceworker-module" => &["serviceworker-module"],
            _ => &[],
        };
        globals.extend(expanded);
    }
    globals.into_iter().collect()
}

/// A web-platform-test's `META: script=` URLs, with their lines.
fn meta_scripts(source: &str) -> Vec<(String, u32)> {
    meta_lines(source)
        .filter(|(key, _, _)| *key == "script")
        .map(|(_, value, line)| (value.trim().to_string(), line))
        .collect()
}

/// The places (lines and columns) of what discovery takes from a page's scan,
/// or a stylesheet's links (`ranges`, their bytes in `source`).
fn places(source: &str, ranges: impl Iterator<Item = std::ops::Range<usize>>) -> Vec<(u32, u32)> {
    ranges
        .flat_map(|range| [range.start, range.end])
        .map(|offset| line_and_column(source, offset))
        .collect()
}

/// What discovery takes from a version of a page: its scan, its inline
/// scripts' loads, and their places.
fn page_facts(source: &str) -> (PageScan, Vec<Load>, Vec<(u32, u32)>) {
    let scan = scan_page(source);
    let mut loads = vec![];
    for script in &scan.scripts {
        if let Some(range) = &script.inline {
            let first = super::scan::line_of(source, range.start);
            loads.extend(scan_script(&source[range.clone()], first));
        }
    }
    let ranges = scan
        .scripts
        .iter()
        .filter_map(|script| script.inline.clone())
        .chain(scan.handlers.iter().cloned())
        .chain(scan.links.iter().map(|link| link.value.clone()));
    let places = places(source, ranges);
    (scan, loads, places)
}

/// Whether discovery finds the same in two versions of the file at `path`
/// (`old` and `new`, or None, for a file added or removed; ex: by a try push,
/// whose environments are its tree's if it changes nothing that discovery
/// reads; see js-environments' --unchanged): what it takes from a page (its
/// scan, its inline scripts' loads, and their places), a script (its loads,
/// and web-platform-tests' META lines), a web-platform-tests stylesheet (its
/// links, and their places), a moz.build file (its test manifests), or a
/// test manifest (the tree's `manifests`: all of it); and, for a file added
/// or removed, whether it's one discovery could read (a script, page,
/// stylesheet, moz.build file or test manifest), or any file under
/// web-platform-tests' roots (`wpt_roots`), which their pages' links can name.
pub fn same_discovery_facts(
    path: &str,
    old: Option<&str>,
    new: Option<&str>,
    manifests: &HashSet<String>,
    wpt_roots: &[String],
) -> bool {
    use crate::file_metadata::mozbuild::{is_read_mozbuild, read_mozbuild};
    let under_wpt = wpt_roots.iter().any(|root| path.starts_with(root.as_str()));
    let (old, new) = match (old, new) {
        (Some(old), Some(new)) => (old, new),
        (None, None) => return true,
        _ => {
            return !(is_script(path)
                || is_page(path)
                || path.ends_with(".css")
                || is_read_mozbuild(path)
                || manifests.contains(path)
                || under_wpt);
        }
    };
    if old == new {
        return true;
    }
    if is_read_mozbuild(path) {
        return read_mozbuild(old).manifests == read_mozbuild(new).manifests;
    }
    if manifests.contains(path) {
        return false;
    }
    if is_page(path) {
        return page_facts(old) == page_facts(new);
    }
    if is_script(path) {
        return scan_script(old, 1) == scan_script(new, 1) && meta_lines(old).eq(meta_lines(new));
    }
    if path.ends_with(".css") && under_wpt {
        let facts = |source: &str| {
            let links = scan_stylesheet(source);
            let places = places(source, links.iter().map(|link| link.value.clone()));
            (links, places)
        };
        return facts(old) == facts(new);
    }
    true
}

/// The roots of a tree's web-platform-tests (by its rules), with slashes, for
/// `same_discovery_facts`.
pub fn wpt_roots(rules: &Rules) -> Vec<String> {
    rules
        .wpt
        .iter()
        .flat_map(|wpt| {
            std::iter::once(format!("{}/", wpt.root))
                .chain(wpt.extra_roots.values().map(|root| format!("{}/", root)))
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
        link_scans: HashMap::new(),
    };
    d.scan_all();
    d.manifest_tests();
    d.path_tests();
    d.wpt_tests();
    d.out.facts = Facts {
        loads: d.loads,
        pages: d.pages,
        links: d.link_scans,
    };
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
        // (ex: push-api/push-event.https.any.js, a window's module.)
        assert_eq!(
            any_variants(
                "// META: global=window-module\n// META: script=/resources/testdriver.js\n"
            ),
            vec!["window-module"]
        );
        assert_eq!(
            any_variants("// META: global=worker-module, sharedworker-module\n"),
            vec!["dedicatedworker-module", "sharedworker-module"]
        );
        // (ex: workers/Worker-constructor-proto.any.js.)
        assert_eq!(
            any_variants("//META: global=worker\n"),
            vec!["dedicatedworker", "serviceworker", "sharedworker"]
        );
        // The first `global=` is the test's; shadowrealm and jsshell have no
        // global type yet, and an empty one is the defaults.
        assert_eq!(
            any_variants("// META: global=window,shadowrealm,jsshell\n// META: global=worker\n"),
            vec!["window"]
        );
        assert_eq!(
            any_variants("// META: global=\n"),
            vec!["window", "dedicatedworker"]
        );
        // META lines are a test's first lines.
        assert_eq!(
            any_variants("// Copyright\n// META: global=window\n"),
            vec!["window", "dedicatedworker"]
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
        assert_eq!(
            meta_scripts("// META: script=/common/utils.js\n\n// META: script=resources/x.js\n"),
            vec![("/common/utils.js".to_string(), 1)]
        );
    }

    #[test]
    fn test_test262_record() {
        assert_eq!(
            test262_record(
                "// Copyright\n/*---\ndescription: x\nincludes: [propertyHelper.js, compareArray.js]\nflags: [module, async]\n---*/\nverifyProperty();\n"
            ),
            Some(Test262Record {
                includes: vec![
                    "propertyHelper.js".to_string(),
                    "compareArray.js".to_string()
                ],
                includes_line: 4,
                module: true,
                is_async: true,
            })
        );
        // (Block lists, and other keys' nested lines.)
        assert_eq!(
            test262_record(
                "/*---\nnegative:\n  phase: parse\n  type: SyntaxError\nincludes:\n  - a.js\n  - \"b.js\" # why\nflags:\n  - onlyStrict\n---*/\n"
            ),
            Some(Test262Record {
                includes: vec!["a.js".to_string(), "b.js".to_string()],
                includes_line: 5,
                module: false,
                is_async: false,
            })
        );
        // (A fixture, or a harness's support file.)
        assert_eq!(test262_record("export var x = 1;\n"), None);
    }

    /// A tree of files (by path) and its discovery, with prior facts (see
    /// `Inputs::prior`), if any.
    fn discover_tree(
        rules: &Rules,
        tree: &BTreeMap<&str, &str>,
        prior: Option<(&Facts, &HashSet<String>)>,
    ) -> Discovery {
        let files: HashSet<String> = tree.keys().map(|p| p.to_string()).collect();
        let read = |path: &str| tree.get(path).map(|s| s.to_string());
        let entries = BTreeMap::new();
        let manifest_variables = HashMap::new();
        let url_file = |_: &str| None;
        discover(Inputs {
            rules,
            files: &files,
            read: &read,
            entries: &entries,
            manifest_variables: &manifest_variables,
            url_file: &url_file,
            prior,
        })
    }

    /// With prior facts (ex: a try push's tree's), but for the files which
    /// changed (ex: the push's), discovery finds what it finds without them:
    /// what changed files load or link to, and what unchanged ones do that
    /// files added or removed make resolve or not.
    #[test]
    fn test_prior_facts() {
        let rules = Rules::parse(
            r#"
load_globals = ["xpcshell"]
[[path_flavors]]
name = "xpcshell"
include = ["tests/**/test_*.js"]
global = "xpcshell"
[wpt]
root = "wpt"
window_harness = ["resources/testharness.js"]
worker_harness = ["resources/testharness.js"]
"#,
        )
        .unwrap();
        let mut tree: BTreeMap<&str, &str> = BTreeMap::from([
            (
                "tests/a/test_one.js",
                "load(\"helper.js\");\nload(\"missing.js\");\n",
            ),
            ("tests/a/helper.js", "// a helper\n"),
            (
                "tests/b/test_two.js",
                "load(\"../a/helper.js\");\nload(\"../a/later.js\");\n",
            ),
            ("wpt/resources/testharness.js", ""),
            (
                "wpt/dom/page.html",
                "<script src=\"/resources/testharness.js\"></script>\n<script src=\"support/x.js\"></script>\n<link rel=stylesheet href=\"style.css\">\n<script>new Worker(\"worker.js\");</script>\n",
            ),
            ("wpt/dom/support/x.js", ""),
            ("wpt/dom/style.css", "@import url(\"other.css\");\n"),
            ("wpt/dom/other.css", ""),
            ("wpt/dom/worker.js", "importScripts(\"support/x.js\");\n"),
            (
                "wpt/dom/test.any.js",
                "// META: script=support/x.js\ntest();\n",
            ),
        ]);
        let before = discover_tree(&rules, &tree, None);

        tree.insert(
            "tests/a/test_one.js",
            "load(\"helper.js\");\nload(\"missing.js\");\nload(\"new.js\");\n",
        );
        tree.insert("tests/a/new.js", "");
        // (Which unchanged test_two.js loads.)
        tree.insert("tests/a/later.js", "");
        tree.insert(
            "wpt/dom/page.html",
            "<script src=\"/resources/testharness.js\"></script>\n<link rel=stylesheet href=\"style.css\">\n<script>new Worker(\"worker.js\");</script>\n",
        );
        tree.insert(
            "wpt/dom/style.css",
            "@import url(\"other.css\");\n.x { background: url(img.png) }\n",
        );
        tree.insert("wpt/dom/img.png", "");
        tree.remove("wpt/dom/other.css");
        tree.insert("wpt/dom/worker.js", "importScripts(\"support/z.js\");\n");
        tree.insert("wpt/dom/support/z.js", "");
        let changed: HashSet<String> = [
            "tests/a/test_one.js",
            "tests/a/new.js",
            "tests/a/later.js",
            "wpt/dom/page.html",
            "wpt/dom/style.css",
            "wpt/dom/img.png",
            "wpt/dom/other.css",
            "wpt/dom/worker.js",
            "wpt/dom/support/z.js",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        let without = discover_tree(&rules, &tree, None);
        let with = discover_tree(&rules, &tree, Some((&before.facts, &changed)));
        let value = |d: &Discovery| {
            (
                serde_json::to_value(d).unwrap(),
                serde_json::to_value(&d.links).unwrap(),
            )
        };
        assert_eq!(value(&with), value(&without));
        // (And the changes changed what it finds.)
        assert_ne!(value(&without), value(&before));
        let roots: Vec<&str> = without
            .environments
            .iter()
            .flat_map(|e| e.scripts.iter().map(|s| s.path.as_str()))
            .collect();
        assert!(roots.contains(&"tests/a/later.js"), "{:?}", roots);
        assert!(roots.contains(&"wpt/dom/support/z.js"), "{:?}", roots);
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
