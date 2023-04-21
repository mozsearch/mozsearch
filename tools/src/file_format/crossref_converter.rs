use serde_json::{Map, Value, json};
use ustr::Ustr;

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
pub fn convert_crossref_value_to_sym_info_rep(cross_val: Value, sym: &Ustr, fallback_pretty: Option<&Ustr>) -> Value {
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
                    rep.insert("pretty".to_string(), json!(fallback_pretty.unwrap_or(sym).to_string()));
                }
                rep.insert(key, meta);
            } else {
                rep.insert("pretty".to_string(), json!(fallback_pretty.unwrap_or(sym).to_string()));
            }

            let mut jumps = Map::new();
            jumpify(xref.remove("idl"), "idl", &mut jumps);
            jumpify(xref.remove("defs"), "def", &mut jumps);
            jumpify(xref.remove("decls"), "decl", &mut jumps);

            // TODO: Need to handle the IDL search permutations issue that currently allows
            // the language indexer to define multiple symbol groupings.

            if jumps.len() > 0 {
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

/// Helper that processes a jumpref-formatted Value in order to extract any
/// binding slot symbols that should also be looked up.  In general there should
/// be no need to transitively traverse the resulting symbols at this time, but
/// if you did it should not end up pulling in a lot of symbols.  (Compare with
/// walking subclasses/superclasses, which are explicitly not binding slots;
/// that could pull in an immense amount of information!)
pub fn extra_binding_slot_syms_from_jumpref(jumpref: &Value) -> Vec<String> {
    let mut extra_syms = vec![];
    if let Some(owner) = jumpref.pointer("/meta/slotOwner") {
        if let Some(Value::String(sym)) = owner.get("sym") {
            extra_syms.push(sym.clone());
        }
    }
    if let Some(Value::Array(slots)) = jumpref.pointer("/meta/bindingSlots") {
        for slot in slots {
            if let Some(Value::String(sym)) = slot.get("sym") {
                extra_syms.push(sym.clone());
            }
        }
    }
    extra_syms
}
