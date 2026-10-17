//! A try push's semantic delta: what the push changes about the tree's
//! analysis, for its stack rebased onto the tree's indexed revision (as
//! `refs/try-rebased/{push}`, whose commit is the delta's "head"; see
//! `bin/build-try-delta.rs`, which makes it), in
//! `{index_path}/try-deltas/{head}/`:
//!
//! - `meta.json`: `TryDeltaMeta`.
//! - `analysis/{path}.gz`: the push's analysis of the files whose analysis it
//!   changes.
//! - `crossref` and `crossref-extra`: the `CrossrefPatch`es of the symbols
//!   whose crossref data (or call roll-ups; see `RollupDelta`) it changes, as
//!   a bisectable map (see `BisectableMmap`).
//! - `jumpref` and `jumpref-extra`: those symbols' jumprefs, with the patches
//!   (but for those whose roll-ups alone it changes).
//! - `identifiers`: the identifiers it adds, like the tree's `identifiers`.
//! - `objdir/{path}`: its versions of the generated files it changes
//!   (`__GENERATED__/{path}`), which git doesn't have.
//!
//! Queries with `rev={head}` (see pipeline-server) use it through
//! `abstract_server::TryOverlay`, and so do the head's `/rev/` pages.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::abstract_server::Result;
use crate::file_format::bisectable_mmap::BisectableMmap;
use crate::file_format::crossref::CrossrefPatch;
use crate::file_format::identifiers::IdentMap;
use crate::file_format::jumpref::JumprefData;

/// How a try push changes a file, relative to the indexed revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileChange {
    Added,
    Modified,
    Removed,
}

/// A file whose analysis a try push changes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnalyzedFile {
    /// Whether the push's analysis of the file is all of it: every
    /// translation unit including it was compiled.  Otherwise it's the base's
    /// with what the push adds, since others may still have what the push's
    /// lack.
    pub complete: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TryDeltaMeta {
    /// The push (its name in the history's `try-pushes/`, ex: an hg revision).
    pub push: String,
    /// The indexed revision the push is analyzed against.
    pub base: String,
    /// The commit of the push's stack rebased onto `base`, whose files the
    /// analysis is of.
    pub head: String,
    /// The files the push changes, relative to `base`.
    pub changed: BTreeMap<String, FileChange>,
    /// The files whose analysis the push changes (`analysis/{path}.gz`).
    pub analyzed: BTreeMap<String, AnalyzedFile>,
}

pub struct TryDelta {
    pub dir: PathBuf,
    pub meta: TryDeltaMeta,
    crossref: Option<BisectableMmap<CrossrefPatch>>,
    jumpref: Option<BisectableMmap<JumprefData>>,
    identifiers: Option<IdentMap>,
}

/// Whether `rev` is a full git revision (as deltas' heads are named).
pub fn is_full_rev(rev: &str) -> bool {
    rev.len() == 40
        && rev
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// The directory of the tree's delta for `head`.
pub fn delta_dir(index_path: &str, head: &str) -> PathBuf {
    Path::new(index_path).join("try-deltas").join(head)
}

/// A bisectable map in the delta's directory, if it has one with values.
fn open_map<T>(dir: &Path, name: &str) -> Option<BisectableMmap<T>>
where
    T: for<'de> Deserialize<'de>,
{
    let inline = dir.join(name);
    let extra = dir.join(format!("{}-extra", name));
    // (`BisectableMmap::new` can't map an empty file.)
    if std::fs::metadata(&inline).ok()?.len() == 0 || !extra.exists() {
        return None;
    }
    BisectableMmap::new(inline.to_str()?, extra.to_str()?)
}

impl TryDelta {
    /// The tree's delta for the head `head`, if it has one.
    pub fn open(index_path: &str, head: &str) -> Option<TryDelta> {
        if !is_full_rev(head) {
            return None;
        }
        let dir = delta_dir(index_path, head);
        let meta: TryDeltaMeta =
            serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json")).ok()?).ok()?;
        Some(TryDelta {
            crossref: open_map(&dir, "crossref"),
            jumpref: open_map(&dir, "jumpref"),
            identifiers: dir
                .join("identifiers")
                .to_str()
                .filter(|path| std::fs::metadata(path).is_ok_and(|m| m.len() > 0))
                .and_then(IdentMap::new),
            dir,
            meta,
        })
    }

    /// The push's changes to the symbol's crossref data, if any.
    pub fn crossref_patch(&self, sym: &str) -> Result<Option<CrossrefPatch>> {
        match &self.crossref {
            Some(crossref) => crossref.lookup(sym),
            None => Ok(None),
        }
    }

    /// The jumprefs of the symbols the push changes (see
    /// `TreeJumprefs::delta`).
    pub fn jumprefs(&self) -> Option<BisectableMmap<JumprefData>> {
        self.jumpref.clone()
    }

    /// The identifiers the push adds.
    pub fn identifiers(&self) -> Option<&IdentMap> {
        self.identifiers.as_ref()
    }

    /// The push's analysis of the file, if it changes the file's analysis.
    pub fn analysis_file(&self, path: &str) -> Option<PathBuf> {
        self.meta
            .analyzed
            .contains_key(path)
            .then(|| self.dir.join("analysis").join(format!("{}.gz", path)))
    }

    /// The push's version of a generated file it changes
    /// (`__GENERATED__/...`).
    pub fn generated_source(&self, path: &str) -> Option<PathBuf> {
        let rel = path.strip_prefix("__GENERATED__/")?;
        self.change(path)?;
        Some(self.dir.join("objdir").join(rel)).filter(|file| file.exists())
    }

    /// How the push changes the file, if it does.
    pub fn change(&self, path: &str) -> Option<FileChange> {
        self.meta.changed.get(path).copied()
    }
}
