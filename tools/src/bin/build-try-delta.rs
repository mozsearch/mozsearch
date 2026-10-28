//! Build a try push's semantic delta (see `tools::try_delta`) from its
//! analysis: the analysis files that compiling the push's stack rebased onto
//! the tree's indexed revision (the head `HEAD`, in the tree's git repo)
//! wrote, for the files it changes and the files of the translation units
//! including them.
//!
//! The delta has the analysis of the files whose records the push changes:
//! the files it changes, and the others whose records differ from the base's
//! (ex: a caller of a method whose signature it changes), with their
//! citations of web specs, as indexing's (see `cite_specs`).  Files that not
//! every translation unit including them was compiled for (`--incomplete`)
//! keep the base's records too.  Their crossref data is from crossref's
//! subset mode, for the files' base records and for their records in the
//! push: each symbol whose data differs gets a `CrossrefPatch` replacing its
//! results in those files, and a jumpref from its patched data.
//!
//! It says how long each of its phases took as they end, and all of them at
//! its end (see `Phases`), for the try import's log.
//!
//! Usage: build-try-delta CONFIG TREE PUSH HEAD TIP_ANALYSIS
//!          [--incomplete FILE] [--objdir DIR [--changed-generated FILE]]
//!          [--threads N]
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Instant;

use clap::Parser;
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use git2::{Delta, Repository};
use ustr::{Ustr, ustr};

use tools::file_format::bisectable_mmap::{BisectableMmap, write_bisectable};
use tools::file_format::config;
use tools::file_format::crossref::{CrossrefData, CrossrefPatch};
use tools::file_format::jumpref::{JumprefData, convert_crossref_value_to_sym_info_rep};
use tools::file_format::rollup::RollupDelta;
use tools::try_delta::{AnalyzedFile, FileChange, TryDeltaMeta, delta_dir};

#[derive(Parser)]
struct Cli {
    /// The config file.
    config_file: String,
    /// The tree.
    tree_name: String,
    /// The try push's name (in the history's `try-pushes/`).
    push: String,
    /// The head of the push's stack rebased onto the indexed revision.
    head: String,
    /// The analysis files the push's compilation wrote, by path.
    tip_analysis: String,
    /// A file listing the analysis files of `tip_analysis` that not every
    /// translation unit including them was compiled for.
    #[clap(long)]
    incomplete: Option<String>,
    /// The objdir of the push's compilation, for generated files' sources.
    #[clap(long)]
    objdir: Option<String>,
    /// A file listing the generated files (`__GENERATED__/...`) whose
    /// contents the push changes (relative to the base's build, not
    /// necessarily the tree's copies, which may differ trivially, ex: in
    /// paths in comments), whose versions in `--objdir` the delta keeps.
    #[clap(long)]
    changed_generated: Option<String>,
    #[clap(long, default_value_t = 4)]
    threads: usize,
}

/// How long the delta's phases took, each said as it ends (so that a
/// failure's log says which phase it was in, after the last one said), and all
/// of them at the end.
struct Phases {
    start: Instant,
    phase_start: Instant,
    ended: Vec<(&'static str, f64)>,
}

impl Phases {
    fn new() -> Self {
        let now = Instant::now();
        Phases {
            start: now,
            phase_start: now,
            ended: vec![],
        }
    }

    /// End the phase `name`, saying what it did if `detail` isn't empty.
    fn end(&mut self, name: &'static str, detail: &str) {
        let seconds = self.phase_start.elapsed().as_secs_f64();
        if detail.is_empty() {
            println!("build-try-delta: {} in {:.1} s", name, seconds);
        } else {
            println!("build-try-delta: {} in {:.1} s ({})", name, seconds, detail);
        }
        self.ended.push((name, seconds));
        self.phase_start = Instant::now();
    }

    fn summary(&self) {
        let phases: Vec<String> = self
            .ended
            .iter()
            .map(|(name, seconds)| format!("{} {:.1} s", name, seconds))
            .collect();
        println!(
            "build-try-delta: {:.1} s in all: {}",
            self.start.elapsed().as_secs_f64(),
            phases.join(", ")
        );
    }
}

/// The lines of an analysis file (gzipped or not), if it exists.
fn read_records(path: &Path) -> Option<Vec<String>> {
    let gz = PathBuf::from(format!("{}.gz", path.display()));
    let text = if gz.exists() {
        let mut text = String::new();
        GzDecoder::new(File::open(&gz).ok()?)
            .read_to_string(&mut text)
            .ok()?;
        text
    } else {
        fs::read_to_string(path).ok()?
    };
    Some(
        text.lines()
            .filter(|line| !line.is_empty())
            .map(String::from)
            .collect(),
    )
}

fn write_lines(path: &Path, lines: &[String]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut out = std::io::BufWriter::new(File::create(path).unwrap());
    for line in lines {
        writeln!(out, "{}", line).unwrap();
    }
    out.flush().unwrap();
}

/// The files under `root`, by path relative to it.
fn walk(root: &Path, dir: &Path, files: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, files);
        } else if let Ok(rel) = path.strip_prefix(root) {
            files.push(rel.to_string_lossy().into_owned());
        }
    }
}

/// The values of a bisectable map's entries, by id.
fn read_map<T>(dir: &Path, name: &str) -> BTreeMap<String, T>
where
    T: for<'de> serde::Deserialize<'de>,
{
    let inline = dir.join(name);
    let extra = dir.join(format!("{}-extra", name));
    if fs::metadata(&inline).map_or(true, |m| m.len() == 0) {
        return BTreeMap::new();
    }
    let ids: Vec<String> = BufReader::new(File::open(&inline).unwrap())
        .lines()
        .map_while(|line| line.ok())
        .filter_map(|line| line.strip_prefix('!').map(String::from))
        .collect();
    let map: BisectableMmap<T> =
        BisectableMmap::new(inline.to_str().unwrap(), extra.to_str().unwrap()).unwrap();
    ids.into_iter()
        .filter_map(|id| Some((id.clone(), map.lookup(&id).ok()??)))
        .collect()
}

/// The paths of the symbol's own results, in every kind.
fn result_paths(data: &CrossrefData) -> BTreeSet<Ustr> {
    [
        &data.uses,
        &data.definitions,
        &data.assignments,
        &data.declarations,
        &data.forwards,
        &data.idl,
        &data.idl_partial,
        &data.glean,
        &data.aliases,
        &data.doc_defs,
        &data.doc_uses,
    ]
    .into_iter()
    .flatten()
    .flatten()
    .filter(|result| result.tree.is_none())
    .map(|result| result.path)
    .collect()
}

/// The records of the files with web spec citations (see
/// `spec_analyze::citations`) at the head, of those of `paths` the push
/// analyzes or changes, by path: their records in `tip_root` (if any), with
/// the citations, if the tree's `SPEC_` symbols are a linked tree's.  As
/// indexing adds them to the files' analysis after the compilers' (see
/// scripts/spec-analyze.sh), so that the files' records are the base's if
/// the push doesn't change them, and the push's files have their citations.
fn cite_specs<'a>(
    cli: &Cli,
    tree_config: &config::TreeConfig,
    repo: &Repository,
    head: &git2::Tree,
    tip_root: &Path,
    paths: impl Iterator<Item = &'a String>,
    scratch: &Path,
) -> BTreeMap<String, Vec<String>> {
    let mut cited = BTreeMap::new();
    let Some(spec_tree) = tree_config.paths.linked_trees.get("SPEC_") else {
        return cited;
    };
    let spec_cfg = config::load(&cli.config_file, false, Some(spec_tree), None, None);
    let Some(spec_paths) = spec_cfg.trees.get(spec_tree).map(|tree| &tree.paths) else {
        return cited;
    };
    let specs_toml = Path::new(&spec_paths.index_path).join("specs.toml");
    if !specs_toml.exists() {
        return cited;
    }

    // (The files with URLs; spec-analyze skips the others.)
    let (files, analysis) = (scratch.join("cite-files"), scratch.join("cite-analysis"));
    let mut citing = vec![];
    for path in paths {
        if path.starts_with("__GENERATED__/") {
            continue;
        }
        let Some(blob) = head
            .get_path(Path::new(path))
            .ok()
            .and_then(|entry| entry.to_object(repo).ok())
            .and_then(|object| object.peel_to_blob().ok())
        else {
            continue;
        };
        if !std::str::from_utf8(blob.content()).is_ok_and(|text| text.contains("://")) {
            continue;
        }
        let file = files.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, blob.content()).unwrap();
        if let Some(records) = read_records(&tip_root.join(path)) {
            write_lines(&analysis.join(path), &records);
        }
        citing.push(path.clone());
    }
    if citing.is_empty() {
        return cited;
    }
    let list = scratch.join("cite.lst");
    fs::write(
        &list,
        citing
            .iter()
            .map(|p| format!("{}\n", p))
            .collect::<String>(),
    )
    .unwrap();
    fs::create_dir_all(&analysis).unwrap();

    // (Like spec-analyze.sh's, for a tree whose `SPEC_` symbols are a linked
    // tree's.)
    let mut command = Command::new("spec-analyze");
    command
        .arg("--citations-only")
        .arg("--spec-files")
        .arg(&spec_paths.files_path)
        .arg("--spec-index")
        .arg(&spec_paths.index_path);
    if let (Some(git), Some(history)) = (&spec_paths.git_path, &spec_paths.history_path)
        && Path::new(history).join("timeline").is_dir()
    {
        command
            .arg("--stale-anchors")
            .arg(spec_tree)
            .arg(git)
            .arg(history)
            .arg(scratch.join("stale-anchors-cited.json"));
    }
    let status = command
        .arg(&files)
        .arg(&analysis)
        .arg(&specs_toml)
        .arg(&list)
        .status()
        .expect("Couldn't run spec-analyze");
    assert!(
        status.success(),
        "spec-analyze of the push's citations failed"
    );
    for path in citing {
        if let Some(records) = read_records(&analysis.join(&path)) {
            cited.insert(path, records);
        }
    }
    cited
}

fn main() {
    let cli = Cli::parse();
    let mut phases = Phases::new();
    let cfg = config::load(&cli.config_file, false, Some(&cli.tree_name), None, None);
    let tree_config = cfg.trees.get(&cli.tree_name).expect("No such tree");
    let paths = &tree_config.paths;
    let repo =
        Repository::open(paths.git_path.as_deref().expect("The tree has no git repo")).unwrap();

    // ## What the push changes, relative to the indexed revision.
    let base_commit = repo.head().unwrap().peel_to_commit().unwrap();
    let head_commit = repo
        .revparse_single(&cli.head)
        .and_then(|object| object.peel_to_commit())
        .expect("No such head");
    let diff = repo
        .diff_tree_to_tree(
            Some(&base_commit.tree().unwrap()),
            Some(&head_commit.tree().unwrap()),
            None,
        )
        .unwrap();
    let mut changed: BTreeMap<String, FileChange> = BTreeMap::new();
    for delta in diff.deltas() {
        let old = delta
            .old_file()
            .path()
            .map(|p| p.to_string_lossy().into_owned());
        let new = delta
            .new_file()
            .path()
            .map(|p| p.to_string_lossy().into_owned());
        match delta.status() {
            Delta::Added => {
                changed.insert(new.unwrap(), FileChange::Added);
            }
            Delta::Deleted => {
                changed.insert(old.unwrap(), FileChange::Removed);
            }
            _ => {
                if let Some(new) = new {
                    changed.insert(new, FileChange::Modified);
                }
            }
        }
    }
    let incomplete: BTreeSet<String> = match &cli.incomplete {
        Some(list) => fs::read_to_string(list)
            .unwrap()
            .lines()
            .map(String::from)
            .collect(),
        None => BTreeSet::new(),
    };

    phases.end("changes", &format!("{} files", changed.len()));

    let head = head_commit.id().to_string();
    let out = delta_dir(&paths.index_path, &head);
    let scratch = PathBuf::from(format!("{}.scratch", out.display()));
    let _ = fs::remove_dir_all(&out);
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&out).unwrap();

    // ## The files whose records the push changes.
    let tip_root = Path::new(&cli.tip_analysis);
    let mut written = vec![];
    walk(tip_root, tip_root, &mut written);

    // The generated files the push changes (which git doesn't know), with its
    // versions of them (see `TryDelta::generated_source`).
    if let (Some(list), Some(objdir)) = (&cli.changed_generated, &cli.objdir) {
        for path in fs::read_to_string(list).unwrap().lines() {
            let Some(rel) = path.strip_prefix("__GENERATED__/") else {
                continue;
            };
            let Ok(tip) = fs::read(Path::new(objdir).join(rel)) else {
                continue;
            };
            let dest = out.join("objdir").join(rel);
            fs::create_dir_all(dest.parent().unwrap()).unwrap();
            fs::write(&dest, &tip).unwrap();
            let in_base = Path::new(&paths.objdir_path).join(rel).exists();
            changed.insert(
                path.to_string(),
                if in_base {
                    FileChange::Modified
                } else {
                    FileChange::Added
                },
            );
        }
    }
    // Their citations of web specs, and those of the files it changes which
    // its compilation doesn't analyze (ex: JS), which have just those.
    let head_tree = head_commit.tree().unwrap();
    let mut candidates: BTreeSet<&String> = written.iter().collect();
    candidates.extend(
        changed
            .iter()
            .filter(|(_, change)| **change != FileChange::Removed)
            .map(|(path, _)| path),
    );
    let cited = cite_specs(
        &cli,
        tree_config,
        &repo,
        &head_tree,
        tip_root,
        candidates.into_iter(),
        &scratch,
    );
    for path in cited.keys() {
        if !written.contains(path) {
            written.push(path.clone());
        }
    }
    phases.end("citations", &format!("{} files", cited.len()));

    let base_analysis = Path::new(&paths.index_path).join("analysis");
    let mut analyzed: BTreeMap<String, AnalyzedFile> = BTreeMap::new();
    let (mut base_list, mut tip_list) = (vec![], vec![]);
    for path in &written {
        let tip = match cited.get(path) {
            Some(records) => records.clone(),
            None => read_records(&tip_root.join(path)).unwrap_or_default(),
        };
        let base = read_records(&base_analysis.join(path));
        let change = changed.get(path).copied();
        let complete = !incomplete.contains(path);
        let tip_set: BTreeSet<&String> = tip.iter().collect();
        let base_set: BTreeSet<&String> = base.iter().flatten().collect();
        if change.is_none() && tip_set == base_set {
            continue;
        }
        // (Incomplete files keep the base's records too; see `AnalyzedFile`.)
        let mut records: Vec<String> = tip.clone();
        if !complete {
            records.extend(
                base.iter()
                    .flatten()
                    .filter(|line| !tip_set.contains(line))
                    .cloned(),
            );
        }
        records.sort();
        records.dedup();
        let file = out.join("analysis").join(format!("{}.gz", path));
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let mut gz = GzEncoder::new(File::create(&file).unwrap(), Compression::default());
        for line in &records {
            writeln!(gz, "{}", line).unwrap();
        }
        gz.finish().unwrap();
        write_lines(&scratch.join("tip/analysis").join(path), &records);
        tip_list.push(path.clone());
        if let Some(base) = &base {
            write_lines(&scratch.join("base/analysis").join(path), base);
            base_list.push(path.clone());
        }
        analyzed.insert(path.clone(), AnalyzedFile { complete });
    }
    // (The files the push removes lose their results, and so do those it
    // changes which its analysis doesn't have, ex: Rust, or C++ the platform
    // doesn't build, rather than keeping the base's, at its lines.)
    let tip_set: BTreeSet<&String> = tip_list.iter().collect();
    for (path, change) in &changed {
        if *change != FileChange::Added
            && !tip_set.contains(path)
            && let Some(base) = read_records(&base_analysis.join(path))
        {
            write_lines(&scratch.join("base/analysis").join(path), &base);
            base_list.push(path.clone());
        }
    }

    phases.end(
        "records",
        &format!(
            "{} files' records read, {} changed",
            written.len(),
            analyzed.len()
        ),
    );

    // ## The push's versions of the files' sources (and generated files').
    let tip_files = scratch.join("tip-files");
    let tip_objdir = scratch.join("tip-objdir");
    for path in &tip_list {
        let (root, rel, source_root) = match path.strip_prefix("__GENERATED__/") {
            Some(rel) => (
                &tip_objdir,
                rel,
                cli.objdir
                    .as_deref()
                    .unwrap_or(&paths.objdir_path)
                    .to_string(),
            ),
            None => (&tip_files, path.as_str(), paths.files_path.clone()),
        };
        let dest = root.join(rel);
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        let blob = head_commit
            .tree()
            .unwrap()
            .get_path(Path::new(rel))
            .ok()
            .and_then(|entry| entry.to_object(&repo).ok())
            .and_then(|object| object.peel_to_blob().ok());
        match (path.starts_with("__GENERATED__/"), blob) {
            (false, Some(blob)) => fs::write(&dest, blob.content()).unwrap(),
            _ => {
                let _ = fs::copy(Path::new(&source_root).join(rel), &dest);
            }
        }
    }

    phases.end("sources", &format!("{} files", tip_list.len()));

    // ## Their crossref data, before and after.
    //
    // (Both at once, since neither uses all of the cores all of the time: on
    // a firefox push's 2,522 files, one after the other took 47 s and 36 s.
    // Each one's output, but its "File" line for each file it reads, which
    // were 4,800 of that import's 5,300 lines, is said after both are done,
    // rather than mixed together.)
    let crossref = |side: &str, list: &[String], files: &str, objdir: &str| {
        let dir = scratch.join(side);
        fs::create_dir_all(&dir).unwrap();
        let list_path = scratch.join(format!("{}.lst", side));
        fs::write(
            &list_path,
            list.iter().map(|p| format!("{}\n", p)).collect::<String>(),
        )
        .unwrap();
        let output = scratch.join(format!("{}-crossref.log", side));
        let child = Command::new("crossref")
            .args([&cli.config_file, &cli.tree_name])
            .arg(&list_path)
            .arg(cli.threads.to_string())
            .arg("--subset-of")
            .arg(&paths.index_path)
            .arg("--index-path")
            .arg(&dir)
            .args(["--files-path", files, "--objdir-path", objdir])
            .stdout(Stdio::from(File::create(&output).unwrap()))
            .spawn()
            .expect("Couldn't run crossref");
        (dir, output, child, Instant::now())
    };
    let finish =
        |side: &str, (dir, output, mut child, started): (PathBuf, PathBuf, Child, Instant)| {
            let status = child.wait().expect("Couldn't wait for crossref");
            let seconds = started.elapsed().as_secs_f64();
            println!(
                "build-try-delta: crossref of the {} records took {:.1} s:",
                side, seconds
            );
            for line in fs::read_to_string(&output).unwrap_or_default().lines() {
                if !line.starts_with("File ") {
                    println!("{}", line);
                }
            }
            assert!(status.success(), "crossref of the {} records failed", side);
            (dir, seconds)
        };
    let base = crossref("base", &base_list, &paths.files_path, &paths.objdir_path);
    let tip = crossref(
        "tip",
        &tip_list,
        tip_files.to_str().unwrap(),
        tip_objdir.to_str().unwrap(),
    );
    let (base_dir, base_seconds) = finish("base", base);
    let (tip_dir, tip_seconds) = finish("tip", tip);
    phases.end(
        "crossref",
        &format!(
            "the base's {} files in {:.1} s and the tip's {} in {:.1} s, at once",
            base_list.len(),
            base_seconds,
            tip_list.len(),
            tip_seconds
        ),
    );
    let mut base_data: BTreeMap<String, CrossrefData> = read_map(&base_dir, "crossref");
    let mut tip_data: BTreeMap<String, CrossrefData> = read_map(&tip_dir, "crossref");
    let tip_jumprefs: BTreeMap<String, JumprefData> = read_map(&tip_dir, "jumpref");

    // ## The symbols' patches, and their jumprefs.
    let tree_crossref: Option<BisectableMmap<CrossrefData>> = BisectableMmap::new(
        &format!("{}/crossref", paths.index_path),
        &format!("{}/crossref-extra", paths.index_path),
    );
    let tree_jumprefs: Option<BisectableMmap<JumprefData>> = BisectableMmap::new(
        &format!("{}/jumpref", paths.index_path),
        &format!("{}/jumpref-extra", paths.index_path),
    );
    let syms: BTreeSet<String> = base_data.keys().chain(tip_data.keys()).cloned().collect();
    let (mut patches, mut jumprefs) = (vec![], vec![]);
    let empty_data = serde_json::to_string(&CrossrefData::default()).unwrap();
    for sym in syms {
        let mut base = base_data.remove(&sym);
        let mut tip = tip_data.remove(&sym);
        // (The subsets have no history digests, or data roll-ups, which need
        // the whole tree's dataflow summaries; the tree's stay.  Their call
        // roll-ups are of the changed files' calls, before and after, which
        // are what the push changes about the tree's; see `RollupDelta`.)
        // (A class the changed files only call methods of has a roll-up and
        // nothing else.)
        let mut rollups = [None, None];
        for (side, data) in [&mut base, &mut tip].into_iter().enumerate() {
            if let Some(d) = data {
                d.recency = None;
                d.recency_from = None;
                d.data_rollup = None;
                rollups[side] = d.call_rollup.take();
                if serde_json::to_string(d).unwrap() == empty_data {
                    *data = None;
                }
            }
        }
        let [before, after] = rollups;
        let call_rollup_delta = (before != after).then_some(RollupDelta { before, after });
        let same = serde_json::to_string(&base).unwrap() == serde_json::to_string(&tip).unwrap();
        if same {
            // (Only its roll-up changes: the tree's data otherwise, and its
            // jumpref.)
            if call_rollup_delta.is_some() {
                patches.push((
                    sym,
                    CrossrefPatch {
                        paths: vec![],
                        data: CrossrefData::default(),
                        defined: false,
                        call_rollup_delta,
                    },
                ));
            }
            continue;
        }
        let mut paths: BTreeSet<Ustr> = BTreeSet::new();
        let mut defined = false;
        for data in [&base, &tip].into_iter().flatten() {
            paths.extend(result_paths(data));
            defined |= data.meta.is_some();
        }
        let patch = CrossrefPatch {
            paths: paths.into_iter().collect(),
            data: tip.unwrap_or_default(),
            defined,
            call_rollup_delta,
        };
        let tree_data = tree_crossref
            .as_ref()
            .and_then(|map| map.lookup(&sym).ok().flatten());
        let merged = patch.apply(tree_data);
        let fallback_pretty = tip_jumprefs
            .get(&sym)
            .map(|jumpref| jumpref.pretty)
            .or_else(|| {
                tree_jumprefs
                    .as_ref()
                    .and_then(|map| map.lookup(&sym).ok().flatten())
                    .map(|jumpref| jumpref.pretty)
            });
        jumprefs.push((
            sym.clone(),
            convert_crossref_value_to_sym_info_rep(merged, &ustr(&sym), fallback_pretty),
        ));
        patches.push((sym, patch));
    }
    println!(
        "{} files' analysis, {} symbols' crossref data",
        analyzed.len(),
        patches.len()
    );
    write_bisectable(
        out.join("crossref").to_str().unwrap(),
        out.join("crossref-extra").to_str().unwrap(),
        patches,
    )
    .unwrap();
    write_bisectable(
        out.join("jumpref").to_str().unwrap(),
        out.join("jumpref-extra").to_str().unwrap(),
        jumprefs,
    )
    .unwrap();
    phases.end("patches", "");

    // ## The identifiers it adds (in the order the tree's are).
    let read_identifiers = |dir: &Path| -> Vec<String> {
        fs::read_to_string(dir.join("identifiers"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    };
    let base_ids: BTreeSet<String> = read_identifiers(&base_dir).into_iter().collect();
    let added: String = read_identifiers(&tip_dir)
        .into_iter()
        .filter(|line| !base_ids.contains(line))
        .map(|line| line + "\n")
        .collect();
    fs::write(out.join("identifiers"), added).unwrap();

    let meta = TryDeltaMeta {
        push: cli.push.clone(),
        base: base_commit.id().to_string(),
        head,
        changed,
        analyzed,
    };
    fs::write(
        out.join("meta.json"),
        serde_json::to_string_pretty(&meta).unwrap(),
    )
    .unwrap();
    let _ = fs::remove_dir_all(&scratch);
    phases.end("identifiers", "");
    phases.summary();
}
