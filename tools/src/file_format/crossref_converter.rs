use bitflags::bitflags;
use serde_json::{from_value, json, Map, Value};
use ustr::Ustr;

use super::analysis::{BindingSlotKind, BindingSlotLang, StructuredBindingSlotInfo};

/// Transform a crossref Value that will be written into crossref into the
/// digested representation we emit into the SYM_INFO structure for source
/// listings and diagrams.  This method also takes some fallback information for
/// population of the representation if the value is Null or lacks a "meta"
/// key.  (Currently "meta" key contents only come from structured analysis
/// records, but this may change in the future.)
///
/// Note: We actually have quite strong invariants about the data we're consuming
/// here but some of the JSON processing logic is written in a more defensive
/// idiom (if let) than it needs to be, especially since for a lot of values we
/// just unwrap them.
pub fn convert_crossref_value_to_sym_info_rep(
    cross_val: Value,
    sym: &Ustr,
    fallback_pretty: Option<&Ustr>,
) -> Value {
    // Process a path hit-list if there's a single hit inside it, writing an
    // entry with the given jump_kind.
    let jumpify = |path_hits: Option<Value>, jump_kind: &str, jump_map: &mut Map<String, Value>| {
        if let Some(Value::Array(mut paths)) = path_hits {
            if paths.len() != 1 {
                return;
            }
            if let Value::Object(mut path_hit) = paths.remove(0) {
                let path_val = path_hit.remove("path").unwrap();
                let path = path_val.as_str().unwrap();
                if let Some(Value::Array(mut lines)) = path_hit.remove("lines") {
                    if lines.len() != 1 {
                        return;
                    }
                    if let Value::Object(line_hit) = lines.remove(0) {
                        let lno = line_hit.get("lno").unwrap().as_u64().unwrap();
                        jump_map.insert(jump_kind.to_string(), json!(format!("{}#{}", path, lno)));
                    }
                }
            }
        }
    };

    match cross_val {
        Value::Object(mut xref) => {
            let mut rep = Map::new();
            rep.insert("sym".to_string(), json!(sym.to_string()));

            if let Some((key, meta)) = xref.remove_entry("meta") {
                // Favor the "pretty" value from the meta since it may eventually
                // start doing more clever things.
                if let Some(Value::String(pretty)) = meta.get("pretty") {
                    rep.insert("pretty".to_string(), json!(pretty.clone()));
                } else {
                    rep.insert(
                        "pretty".to_string(),
                        json!(fallback_pretty.unwrap_or(sym).to_string()),
                    );
                }
                rep.insert(key, meta);
            } else {
                rep.insert(
                    "pretty".to_string(),
                    json!(fallback_pretty.unwrap_or(sym).to_string()),
                );
            }

            let mut jumps = Map::new();
            jumpify(xref.remove("idl"), "idl", &mut jumps);
            jumpify(xref.remove("defs"), "def", &mut jumps);
            jumpify(xref.remove("decls"), "decl", &mut jumps);

            if let Some((key, idl_syms)) = xref.remove_entry("idl_syms") {
                rep.insert(key, idl_syms);
            }

            // TODO: Need to handle the IDL search permutations issue that currently allows
            // the language indexer to define multiple symbol groupings.

            if !jumps.is_empty() {
                rep.insert("jumps".to_string(), json!(jumps));
            }

            json!(rep)
        }
        _ => {
            json!({
                "sym": sym.to_string(),
                "pretty": fallback_pretty.unwrap_or(sym).to_string(),
            })
        }
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
    jumpref: &Value,
) -> Vec<(String, JumprefTraversals)> {
    let mut extra_syms = vec![];
    if let Some(Value::Array(idl_syms)) = jumpref.pointer("/idl_syms") {
        for sym in idl_syms {
            if let Value::String(s) = sym {
                extra_syms.push((s.clone(), JumprefTraversals::NormalExtra));
            }
        }
    }
    if let Some(owner) = jumpref.pointer("/meta/slotOwner") {
        let next_step = match owner["slotKind"].as_str() {
            Some("send") => JumprefTraversals::Receive,
            _ => JumprefTraversals::empty(),
        };
        if let Some(Value::String(sym)) = owner.get("sym") {
            extra_syms.push((sym.clone(), next_step));
        }
    }
    if let Some(Value::Array(slots)) = jumpref.pointer("/meta/bindingSlots") {
        for slot in slots {
            if let Ok(slot_info) = from_value::<StructuredBindingSlotInfo>(slot.clone()) {
                let next_step = match (slot_info.props.slot_kind, slot_info.props.slot_lang) {
                    // For IDL symbols in an .idl file, this implies XPIDL (we could check more
                    // thoroughly) and the binding slot will reference the pure virtual decl that
                    // we upgrade to a def.  That's not useful, so we also want to traverse its
                    // overridenBy edges so we can provide go to the actual impl definition.
                    (BindingSlotKind::Method, BindingSlotLang::Cpp) => {
                        JumprefTraversals::OverriddenBy
                    }
                    _ => JumprefTraversals::empty(),
                };
                extra_syms.push((slot_info.sym.to_string(), next_step));
            }
        }
    }
    if let Some(Value::Array(overridden)) = jumpref.pointer("/meta/overriddenBy") {
        if overridden.len() <= 2 {
            for over_info in overridden {
                if let Value::String(over_sym) = over_info {
                    // The override is all we need.
                    extra_syms.push((over_sym.to_owned(), JumprefTraversals::empty()));
                }
            }
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
    jumpref: &Value,
    next_step: JumprefTraversals,
) -> Vec<(String, JumprefTraversals)> {
    let mut extra_syms = vec![];
    if next_step.contains(JumprefTraversals::Receive) {
        if let Some(Value::Array(slots)) = jumpref.pointer("/meta/bindingSlots") {
            for slot in slots {
                if let Ok(slot_info) = from_value::<StructuredBindingSlotInfo>(slot.clone()) {
                    if slot_info.props.slot_kind == BindingSlotKind::Recv {
                        extra_syms.push((slot_info.sym.to_string(), JumprefTraversals::empty()));
                    }
                }
            }
        }
    }
    if next_step.contains(JumprefTraversals::OverriddenBy) {
        // this is copied from the same logic in determine_desired_extra_syms_from_jumpref.
        if let Some(Value::Array(overridden)) = jumpref.pointer("/meta/overriddenBy") {
            if overridden.len() <= 2 {
                for over_info in overridden {
                    if let Value::String(over_sym) = over_info {
                        // The override is all we need.
                        extra_syms.push((over_sym.to_owned(), JumprefTraversals::empty()));
                    }
                }
            }
        }
    }

    extra_syms
}
