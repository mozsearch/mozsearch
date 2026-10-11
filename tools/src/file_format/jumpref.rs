use std::collections::BTreeMap;
use std::path::Path;

use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use ustr::{Ustr, ustr};

use crate::abstract_server::Result;
use crate::utils::OverflowingVec;

use super::analysis::{AnalysisStructured, BindingSlotKind, BindingSlotLang, PathSearchResult};
use super::bisectable_mmap::BisectableMmap;
use super::config::TreeConfigPaths;
use super::crossref::CrossrefData;
use super::recency::{Recency, RecencyFrom};

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct Jumps {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idl: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glean: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "def")]
    pub definition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "decl")]
    pub declaration: Option<String>,
    /// The symbol's documentation, if it has one doc-def.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

impl Jumps {
    fn is_empty(&self) -> bool {
        matches!(
            self,
            Jumps {
                idl: None,
                glean: None,
                definition: None,
                declaration: None,
                doc: None,
            }
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JumprefData {
    pub sym: Ustr,
    pub pretty: Ustr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<AnalysisStructured>,
    #[serde(default, skip_serializing_if = "Jumps::is_empty")]
    pub jumps: Jumps,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idl_syms: Option<Vec<Ustr>>,
    /// Candidate scip-typescript symbols if this is a js-analyze symbol
    #[serde(default, skip_serializing_if = "OverflowingVec::is_empty")]
    pub ts_syms: OverflowingVec<10, Ustr>,
    /// See `CrossrefData::recency`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recency: Option<Recency>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recency_from: Option<RecencyFrom>,
    /// The tree whose data this is, if it's another tree's, whose files the
    /// jumps are to (see `TreeJumprefs`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<Ustr>,
}

/// A tree's jumpref, with its linked trees' (see
/// `TreeConfigPaths::linked_trees`), whose data a symbol with one of their
/// prefixes gets instead, if they have it and the tree's own has no jumps (ex:
/// a web spec's definition, for a Gecko comment's citation of it, but not
/// for the spec's own tree, whose linked tree only has citations).
#[derive(Clone, Debug, Default)]
pub struct TreeJumprefs {
    pub own: Option<BisectableMmap<JumprefData>>,
    /// (prefix, tree, its jumpref)
    pub linked: Vec<(String, Ustr, BisectableMmap<JumprefData>)>,
}

impl TreeJumprefs {
    /// The jumpref of the tree with `paths`, and of its linked trees, by
    /// their index paths, if they have them.
    pub fn open(paths: &TreeConfigPaths, tree_index_paths: &BTreeMap<String, String>) -> Self {
        let open = |index_path: &str| {
            BisectableMmap::new(
                &format!("{}/jumpref", index_path),
                &format!("{}/jumpref-extra", index_path),
            )
        };
        let linked = paths
            .linked_trees
            .iter()
            .filter_map(|(prefix, tree)| {
                let index_path = tree_index_paths.get(tree)?;
                // (A linked tree which isn't indexed (yet) has no data.)
                if !Path::new(&format!("{}/jumpref", index_path)).exists() {
                    return None;
                }
                Some((prefix.clone(), ustr(tree), open(index_path)?))
            })
            .collect();
        TreeJumprefs {
            own: open(&paths.index_path),
            linked,
        }
    }

    pub fn lookup(&self, sym: &str) -> Result<Option<JumprefData>> {
        let own = match &self.own {
            Some(own) => own.lookup(sym)?,
            None => None,
        };
        if own.as_ref().is_some_and(|data| !data.jumps.is_empty()) {
            return Ok(own);
        }
        for (prefix, tree, jumpref) in &self.linked {
            if sym.starts_with(prefix.as_str())
                && let Ok(Some(mut data)) = jumpref.lookup(sym)
                && !data.jumps.is_empty()
            {
                data.tree = Some(*tree);
                return Ok(Some(data));
            }
        }
        Ok(own)
    }
}

/// Transform a crossref data that will be written into crossref into the
/// digested representation we emit into the SYM_INFO structure for source
/// listings and diagrams.  This method also takes some fallback information for
/// population of the representation if the value lacks a "meta" key.
/// (Currently "meta" key contents only come from structured analysis records,
/// but this may change in the future.)
pub fn convert_crossref_value_to_sym_info_rep(
    cross_val: Option<CrossrefData>,
    sym: &Ustr,
    fallback_pretty: Option<Ustr>,
) -> JumprefData {
    let Some(cross_val) = cross_val else {
        return JumprefData {
            sym: *sym,
            pretty: fallback_pretty.unwrap_or(*sym),
            jumps: Jumps {
                idl: None,
                glean: None,
                definition: None,
                declaration: None,
                doc: None,
            },
            meta: None,
            idl_syms: None,
            ts_syms: Default::default(),
            recency: None,
            recency_from: None,
            tree: None,
        };
    };

    // Turns a list of path results into a single result
    let jumpify = |path_hits: &Option<Vec<PathSearchResult>>| match path_hits.as_deref() {
        Some([path_hit]) => match &path_hit.lines[..] {
            [line_hit] => Some(format!("{}#{}", path_hit.path, line_hit.lineno)),
            _ => None,
        },
        _ => None,
    };

    let jumps = Jumps {
        idl: jumpify(&cross_val.idl),
        glean: jumpify(&cross_val.glean),
        definition: jumpify(&cross_val.definitions),
        declaration: jumpify(&cross_val.declarations),
        doc: jumpify(&cross_val.doc_defs),
    };

    let pretty = cross_val
        .meta
        .as_ref()
        .map(|meta| meta.pretty)
        .or(fallback_pretty)
        .unwrap_or(*sym);

    JumprefData {
        sym: *sym,
        pretty,
        jumps,
        meta: cross_val.meta,
        idl_syms: cross_val.idl_syms,
        ts_syms: cross_val.ts_syms.into(),
        recency: cross_val.recency,
        recency_from: cross_val.recency_from,
        tree: None,
    }
}

bitflags! {
    #[derive(Clone, Copy)]
    /// Bitflag that allows us to express what additional traversals we want of
    /// a jumpref symbol as returned by
    /// `determine_desired_extra_syms_from_jumpref`.  And when populating such
    /// a map, the bitflags let us track what traversals we have already
    /// performed for a symbol.
    pub struct JumprefTraversals: u32 {
        /// We called `determine_desired_extra_syms_from_jumpref` and processed
        /// the results.
        const NormalExtra  = 0b00000001;
        /// We want to traverse any "recv" binding slots.
        const Receive      = 0b00000010;
        /// We want to traverse any overriddenBy values.
        const OverriddenBy = 0b00000100;
    }
}

/// Helper that processes a jumpref-formatted Value in order to extract any
/// binding slot symbols that should also be looked up.
pub fn determine_desired_extra_syms_from_jumpref(
    jumpref: Option<&JumprefData>,
) -> Vec<(Ustr, JumprefTraversals)> {
    let Some(jumpref) = jumpref else {
        return Default::default();
    };

    let mut extra_syms = vec![];
    if let Some(idl_syms) = &jumpref.idl_syms {
        for sym in idl_syms {
            extra_syms.push((*sym, JumprefTraversals::NormalExtra));
        }
    }
    extra_syms.extend(
        jumpref
            .ts_syms
            .into_iter()
            .copied()
            .map(|sym| (sym, JumprefTraversals::NormalExtra)),
    );
    if let Some(owner) = jumpref
        .meta
        .as_ref()
        .and_then(|meta| meta.slot_owner.as_ref())
    {
        let next_step = match owner.props.slot_kind {
            BindingSlotKind::Send => JumprefTraversals::Receive,
            _ => JumprefTraversals::empty(),
        };
        extra_syms.push((owner.sym, next_step));
    }
    // (Events can have many senders and receivers, which the context menu
    // only links to if they're few; see `MAX_EVENT_SLOT_LINKS` in
    // context-menu.js.)
    const MAX_EVENT_SLOT_LINKS: usize = 10;
    if let Some(slots) = jumpref
        .meta
        .as_ref()
        .filter(|meta| meta.kind != "event" || meta.binding_slots.len() <= MAX_EVENT_SLOT_LINKS)
        .map(|meta| &meta.binding_slots)
    {
        for slot in slots {
            let next_step = match (slot.props.slot_kind, slot.props.slot_lang) {
                // For IDL symbols in an .idl file, this implies XPIDL (we could check more
                // thoroughly) and the binding slot will reference the pure virtual decl that
                // we upgrade to a def.  That's not useful, so we also want to traverse its
                // overridenBy edges so we can provide go to the actual impl definition.
                (BindingSlotKind::Method, BindingSlotLang::Cpp) => JumprefTraversals::OverriddenBy,
                _ => JumprefTraversals::empty(),
            };
            extra_syms.push((slot.sym, next_step));
        }
    }
    if let Some(overridden) = jumpref.meta.as_ref().map(|meta| &meta.overridden_by_syms) {
        // Keep in sync with overrideJumpifyHelper in context-menu.js
        const MAX_OVERRIDEN_BY_LINKS: usize = 10;

        for over_info in overridden.into_iter().take(MAX_OVERRIDEN_BY_LINKS) {
            // The override is all we need.
            extra_syms.push((*over_info, JumprefTraversals::empty()));
        }
    }
    extra_syms
}

/// Given a jumpref with a next step, return any additional symbols that should
/// be looked up.  Currently, we do not allow additional next steps to be
/// requested because normally we only need the level of indirection of traversing
/// to the binding slot owner and then via one of its binding slots, for example
/// to get from the IPC send method to the IPC receive method.
pub fn extra_syms_next_step_lookups(
    jumpref: Option<&JumprefData>,
    next_step: JumprefTraversals,
) -> Vec<(Ustr, JumprefTraversals)> {
    let Some(jumpref) = jumpref else {
        return Default::default();
    };

    let mut extra_syms = vec![];
    if next_step.contains(JumprefTraversals::Receive)
        && let Some(slots) = jumpref.meta.as_ref().map(|meta| &meta.binding_slots)
    {
        for slot in slots {
            if slot.props.slot_kind == BindingSlotKind::Recv {
                extra_syms.push((slot.sym, JumprefTraversals::empty()));
            }
        }
    }
    if next_step.contains(JumprefTraversals::OverriddenBy) {
        // this is copied from the same logic in determine_desired_extra_syms_from_jumpref.
        if let Some(overridden) = jumpref.meta.as_ref().map(|meta| &meta.overridden_by_syms)
            && overridden.len() <= 2
        {
            for over_info in overridden {
                // The override is all we need.
                extra_syms.push((*over_info, JumprefTraversals::empty()));
            }
        }
    }

    extra_syms
}
