//! The SYM_INFO (see `format::format_code`) of symbols, over a server's
//! jumprefs, as `TreeJumprefs::add_sym_info` makes it from a local index's:
//! for pages built over an `AbstractServer` (ex: a try push's canvas; see
//! `patch_canvas`, and `/query/`'s results).

use std::collections::BTreeMap;

use ustr::{Ustr, UstrMap, ustr};

use super::AbstractServer;
use crate::file_format::jumpref::{
    JumprefData, JumprefTraversals, determine_desired_extra_syms_from_jumpref,
    extra_syms_next_step_lookups,
};

/// Add the jumpref of `sym` to `sym_info`, if the server has it, with those of
/// the extra symbols the context menu uses for it (ex: an XPIDL method's C++
/// binding; see `determine_desired_extra_syms_from_jumpref`), tracking the
/// traversals made for them in `traversed`.
pub async fn add_sym_info(
    server: &(dyn AbstractServer + Send + Sync),
    sym: Ustr,
    sym_info: &mut BTreeMap<Ustr, Option<JumprefData>>,
    traversed: &mut UstrMap<JumprefTraversals>,
) {
    let Ok(jumpref) = server.jumpref_lookup(&sym).await else {
        return;
    };
    let mut extra_syms = determine_desired_extra_syms_from_jumpref(jumpref.as_ref());
    traversed
        .entry(sym)
        .and_modify(|t| *t |= JumprefTraversals::NormalExtra)
        .or_insert(JumprefTraversals::NormalExtra);
    while let Some((extra_sym, next_step)) = extra_syms.pop() {
        // (A symbol already added only needs its next step's lookups, if it
        // hasn't had them.)
        if let Some(extra_traversed) = traversed.get_mut(&extra_sym) {
            if extra_traversed.contains(next_step) {
                continue;
            }
            *extra_traversed |= next_step;
            if let Some(extra_jumpref) = sym_info.get(&extra_sym) {
                extra_syms.extend(extra_syms_next_step_lookups(
                    extra_jumpref.as_ref(),
                    next_step,
                ));
            }
        } else if let Ok(extra_jumpref) = server.jumpref_lookup(&extra_sym).await {
            if !next_step.is_empty() {
                extra_syms.extend(extra_syms_next_step_lookups(
                    extra_jumpref.as_ref(),
                    next_step,
                ));
            }
            traversed.insert(extra_sym, next_step);
            sym_info.insert(extra_sym, extra_jumpref);
        }
    }
    sym_info.insert(sym, jumpref);
}

/// The symbols of rows' (or excerpts') `html` (their `data-symbols`), in
/// order.
pub fn html_symbols(html: &str) -> Vec<Ustr> {
    const DATA_SYMBOLS: &str = "data-symbols=\"";
    let mut syms = vec![];
    let mut rest = html;
    while let Some(start) = rest.find(DATA_SYMBOLS) {
        rest = &rest[start + DATA_SYMBOLS.len()..];
        let end = rest.find('"').unwrap_or(rest.len());
        syms.extend(
            rest[..end]
                .split(',')
                .filter(|sym| !sym.is_empty())
                .map(ustr),
        );
        rest = &rest[end..];
    }
    syms
}

/// `add_sym_info` for each of `syms` (in order) that `sym_info` doesn't
/// have, with their jumprefs prefetched (see
/// `AbstractServer::prefetch_jumpref`).
pub async fn add_syms_info(
    server: &(dyn AbstractServer + Send + Sync),
    syms: impl IntoIterator<Item = Ustr>,
    sym_info: &mut BTreeMap<Ustr, Option<JumprefData>>,
    traversed: &mut UstrMap<JumprefTraversals>,
) {
    let syms: Vec<Ustr> = syms
        .into_iter()
        .filter(|sym| !sym_info.contains_key(sym))
        .collect();
    server.prefetch_jumpref(&syms).await;
    for sym in syms {
        if !sym_info.contains_key(&sym) {
            add_sym_info(server, sym, sym_info, traversed).await;
        }
    }
}

/// Add the SYM_INFO of the symbols in `html` (see `html_symbols`) to
/// `sym_info` (see `add_sym_info`).
pub async fn add_html_sym_info(
    server: &(dyn AbstractServer + Send + Sync),
    html: &str,
    sym_info: &mut BTreeMap<Ustr, Option<JumprefData>>,
    traversed: &mut UstrMap<JumprefTraversals>,
) {
    add_syms_info(server, html_symbols(html), sym_info, traversed).await;
}
