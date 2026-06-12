use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use ustr::Ustr;

use super::analysis::{AnalysisStructured, BindingSlotKind, BindingSlotLang, PathSearchResult};
use super::crossref::CrossrefData;

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
}

impl Jumps {
    fn is_empty(&self) -> bool {
        matches!(
            self,
            Jumps {
                idl: None,
                glean: None,
                definition: None,
                declaration: None
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
            },
            meta: None,
            idl_syms: None,
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
    if let Some(slots) = jumpref.meta.as_ref().map(|meta| &meta.binding_slots) {
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
    if let Some(overridden) = jumpref.meta.as_ref().map(|meta| &meta.overridden_by_syms)
        && overridden.len() <= 2
    {
        for over_info in overridden {
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
