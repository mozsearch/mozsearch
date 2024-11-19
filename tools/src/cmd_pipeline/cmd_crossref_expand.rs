use std::collections::{HashSet, VecDeque};

use async_trait::async_trait;
use clap::Args;
use serde_json::Value;
use tracing::trace;
use ustr::ustr;

use super::interface::{
    OverloadInfo, OverloadKind, PipelineCommand, PipelineValues, SymbolCrossrefInfo,
    SymbolCrossrefInfoList, SymbolMetaFlags, SymbolRelation,
};

use crate::abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError};

/// Given a set of symbol crossref data, expand the set via relevant semantic
/// relationships like override set membership.  This is fundamentally entwined
/// with what information we want to present in search results and how we want
/// to present it.
#[derive(Debug, Args)]
pub struct CrossrefExpand {
    #[clap(long, value_parser, default_value = "100")]
    pub subclass_local_limit: u32,
    #[clap(long, value_parser, default_value = "400")]
    pub subclass_global_limit: u32,

    #[clap(long, value_parser, default_value = "100")]
    pub override_local_limit: u32,
    #[clap(long, value_parser, default_value = "400")]
    pub override_global_limit: u32,
}

/// Crosseref expansion exists to help us:
/// 1. Provide relevant context to symbol results, like class hierarchies.
/// 2. Let us unify results that could be said to be conceptually equivalent but
///    which logistically involve distinct symbols.
///
/// Before the introduction of "structured" records, we unified results by
/// leveraging the ability to associate multiple symbols with a single
/// identifier.  Clever langage-specific analyses could do things like tie the
/// JS symbol and C++ getter and setter symbols to the identifier for an IDL
/// file, or associate all C++ overrides with every layer of their C++ class
/// ancestry.  This was generally amazing with the one downside that if you
/// searched for a specific sub-class's override by identifier that in turn was
/// overridden by its own sub-classes, you would also get its cousins' overrides
/// with no way in the UI to ignore the cousins.  One would have to arrive at
/// a symbol search that listed the unified list of symbols as would be produced
/// by the "search" context menu option and remove the symbols that were not of
/// interest.
///
/// Post-structured refactoring, an intentional decision has been made to have
/// mozsearch understand the semantic relationships explicitly so that analyses
/// don't have to do hacky things which destroy information and potentially
/// require other layers to have to undo or work-around cleverness, especially
/// since it doesn't scale across multiple langauge analyzers trying to be
/// clever in their own ways.  (These changes also allow for other meaningful
/// optimizations like replacing the jumps/searches ANALYSIS_DATA table with
/// crossref data and massively eliminating data duplication.)
///
/// Currently we still exist in a hybrid scenario, where overrides must be
/// manually traversed but IPC/IDL edges currently still involve the identifiers
/// lookup mechanism unifying things.  Our initial focus in this implementation
/// will be on the override set because this is where we've regressed some use
/// cases and we have tentative plans for faceting and diagramming.
#[derive(Debug)]
pub struct CrossrefExpandCommand {
    pub args: CrossrefExpand,
}

struct LimitGroup {
    kind: OverloadKind,
    local_limit: u32,
    // Hitting the global limit is a multi-step process, so we have to keep a
    // running tally.
    global_count: u32,
    global_limit: u32,
}

#[async_trait]
impl PipelineCommand for CrossrefExpandCommand {
    async fn execute(
        &self,
        server: &(dyn AbstractServer + Send + Sync),
        input: PipelineValues,
    ) -> Result<PipelineValues> {
        let source_crossrefs = match input {
            PipelineValues::SymbolCrossrefInfoList(scil) => scil,
            _ => {
                return Err(ServerError::StickyProblem(ErrorDetails {
                    layer: ErrorLayer::ConfigLayer,
                    message: "crossref-expand needs a CrossrefInfoList".to_string(),
                }));
            }
        };

        // Our approach here is derived from that of `traverse` because of the
        // inherent conceptual overlap and the implementation necessity to
        // ensure we only process a given symbol at most once.  Things are
        // simpler here, though, because we're just dealing with set membership
        // here, not edges (which can be erroneously elided if naively
        // supressing vertices without being aware of the tupling).
        //
        // That said, when our input set involves multiple symbols that exist
        // within the same connected groups but have different identifiers, we
        // will likely run into problems with the single `SymbolRelation` we use
        // to label each symbol.  We do take care to list distances in the enum,
        // but even a min() heuristic is potentially going to look weird.

        let mut to_traverse = VecDeque::new();
        let mut considered = HashSet::new();

        for info in source_crossrefs.symbol_crossref_infos {
            considered.insert(info.symbol);
            to_traverse.push_back((
                info.symbol,
                info.relation.clone(),
                info.quality.clone(),
                Some(info),
            ));
        }

        let mut expanded = vec![];

        // Running tallies for our limits.
        let mut override_limits = LimitGroup {
            kind: OverloadKind::Overrides,
            local_limit: self.args.override_local_limit,
            global_count: 0,
            global_limit: self.args.override_global_limit,
        };
        let mut subclass_limits = LimitGroup {
            kind: OverloadKind::Subclasses,
            local_limit: self.args.subclass_local_limit,
            global_count: 0,
            global_limit: self.args.subclass_global_limit,
        };

        while let Some((symbol, relation, quality, maybe_info)) = to_traverse.pop_front() {
            let mut info = match maybe_info {
                Some(existing) => existing,
                None => {
                    let fresh_info = server.crossref_lookup(&symbol, false).await?;
                    SymbolCrossrefInfo {
                        symbol,
                        crossref_info: fresh_info,
                        relation: relation.clone(),
                        quality,
                        overloads_hit: vec![],
                        flags: SymbolMetaFlags::default(),
                    }
                }
            };

            // Given a JSON pointer to a an array, if present, process it, transforming
            // each array value using `xfunc` to extract the symbol.  `use_relation`
            // specifies the resulting relationship that should be associated
            // with the extracted symbol.  `use_limits` is the `LimitGroup` to
            // adjust and apply.
            let mut proc_ptr =
                |ptr: &str,
                 xfunc: &dyn Fn(&Value) -> &Value,
                 use_relation: SymbolRelation,
                 use_limits: Option<&mut LimitGroup>| {
                    if let Some(Value::Array(arr)) = info.crossref_info.pointer(ptr) {
                        if let Some(limits) = use_limits {
                            if limits.local_limit > 0 && arr.len() as u32 > limits.local_limit {
                                info.overloads_hit.push(OverloadInfo {
                                    kind: limits.kind.clone(),
                                    // We're explicitly hanging off a symbol, so we don't need to
                                    // encode any other symbol here.
                                    sym: None,
                                    exist: arr.len() as u32,
                                    included: 0,
                                    local_limit: limits.local_limit,
                                    global_limit: 0,
                                });
                                return;
                            }
                            if limits.global_limit > 0
                                && limits.global_count + arr.len() as u32 > limits.global_limit
                            {
                                info.overloads_hit.push(OverloadInfo {
                                    kind: limits.kind.clone(),
                                    // We're explicitly hanging off a symbol, so we don't need to
                                    // encode any other symbol here.
                                    sym: None,
                                    exist: arr.len() as u32,
                                    included: 0,
                                    local_limit: 0,
                                    global_limit: limits.global_limit,
                                });
                                return;
                            }
                            limits.global_count += arr.len() as u32;
                        }
                        trace!(edge = ptr, count = arr.len(), "considering");
                        for wrapped in arr.iter() {
                            if let Value::String(sym) = xfunc(wrapped) {
                                let usym = ustr(sym);
                                if considered.insert(usym) {
                                    to_traverse.push_back((
                                        usym,
                                        use_relation.clone(),
                                        info.quality.clone(),
                                        None,
                                    ));
                                }
                            }
                        }
                    }
                };

            // Build up our traversal lists based on the relation for the node
            // we're processing.  Our general approach is:
            // - `Queried` nodes are the roots of our search and all edges are
            //   possible.
            // - As we move down into descendant nodes we only continue to move
            //   in this direction because upward movement would be
            //   backtracking.
            // - As we move upward into ancestor nodes, there are new downward
            //   edges to consider, and we do consider these and label them
            //   cousins.  We rely on `considered` to avoid creating loops.
            // - We apply local and global (within this command) limits to avoid
            //   performing traversals of edge sets that are too large.  This is
            //   primarily about not trying to show all the subclasses of
            //   nsISupports or all the overrides of nsISupports::AddRef
            //   automatically without the user explicitly indicating that's
            //   what they want.
            match &relation {
                SymbolRelation::Queried => {
                    proc_ptr(
                        "/meta/overridenBy",
                        &|x| x,
                        SymbolRelation::OverrideOf(symbol, 1),
                        Some(&mut override_limits),
                    );
                    proc_ptr(
                        "/meta/overrides",
                        &|x| &x["sym"],
                        SymbolRelation::OverriddenBy(symbol, 1),
                        None,
                    );
                    proc_ptr(
                        "/meta/subclasses",
                        &|x| x,
                        SymbolRelation::SubclassOf(symbol, 1),
                        Some(&mut subclass_limits),
                    );
                    proc_ptr(
                        "/meta/supers",
                        &|x| &x["sym"],
                        SymbolRelation::SuperclassOf(symbol, 1),
                        None,
                    );
                }
                SymbolRelation::OverriddenBy(root_sym, dist) => {
                    proc_ptr(
                        "/meta/overrides",
                        &|x| &x["sym"],
                        SymbolRelation::OverriddenBy(*root_sym, dist + 1),
                        None,
                    );
                    proc_ptr(
                        "/meta/overridenBy",
                        &|x| x,
                        SymbolRelation::CousinOverrideOf(*root_sym, dist + 1),
                        Some(&mut override_limits),
                    );
                }
                SymbolRelation::OverrideOf(root_sym, dist) => {
                    proc_ptr(
                        "/meta/overridenBy",
                        &|x| x,
                        SymbolRelation::OverrideOf(*root_sym, dist + 1),
                        Some(&mut override_limits),
                    );
                }
                SymbolRelation::CousinOverrideOf(root_sym, dist) => {
                    proc_ptr(
                        "/meta/overridenBy",
                        &|x| x,
                        SymbolRelation::CousinOverrideOf(*root_sym, dist + 1),
                        Some(&mut override_limits),
                    );
                }
                SymbolRelation::SubclassOf(root_sym, dist) => {
                    proc_ptr(
                        "/meta/subclasses",
                        &|x| x,
                        SymbolRelation::SubclassOf(*root_sym, dist + 1),
                        Some(&mut subclass_limits),
                    );
                }
                SymbolRelation::SuperclassOf(root_sym, dist) => {
                    proc_ptr(
                        "/meta/supers",
                        &|x| &x["sym"],
                        SymbolRelation::SuperclassOf(*root_sym, dist + 1),
                        None,
                    );
                    proc_ptr(
                        "/meta/subclasses",
                        &|x| x,
                        SymbolRelation::CousinClassOf(*root_sym, dist + 1),
                        Some(&mut subclass_limits),
                    );
                }
                SymbolRelation::CousinClassOf(root_sym, dist) => {
                    proc_ptr(
                        "/meta/subclasses",
                        &|x| x,
                        SymbolRelation::CousinClassOf(*root_sym, dist + 1),
                        Some(&mut subclass_limits),
                    );
                }
            }

            expanded.push(info);
        }

        Ok(PipelineValues::SymbolCrossrefInfoList(
            SymbolCrossrefInfoList {
                symbol_crossref_infos: expanded,
                unknown_symbols: vec![],
            },
        ))
    }
}
