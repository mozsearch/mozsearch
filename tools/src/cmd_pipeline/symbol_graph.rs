use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use clap::ValueEnum;
use dot_generator::*;
use dot_structures::*;
use graphviz_rust::printer::{DotPrinter, PrinterContext};
use itertools::Itertools;
use petgraph::{
    algo::all_simple_paths,
    graph::{DefaultIx, NodeIndex},
    Directed, Graph as PetGraph,
};
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};
use serde_json::{from_value, json, to_value, Value};
use tracing::trace;
use ustr::{ustr, Ustr, UstrMap};

use crate::{
    abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError},
    file_format::{
        analysis::AnalysisStructured, analysis_manglings::split_pretty,
        crossref_converter::convert_crossref_value_to_sym_info_rep,
        ontology_mapping::label_to_badge_info,
    },
};

pub use crate::symbol_graph_edge_kind::EdgeKind;

use super::{
    cmd_graph::{GraphHierarchy, GraphLayout},
    interface::OverloadInfo,
};

/**
Graph abstraction for symbols built on top of petgraph.

### Motivation / Implementation Rationale

Conceptually, we want our graphs to operate in terms of searchfox symbols
where the symbol names are the identifiers and we associate a bunch of
information with the symbol.  In the JS fancy branch we were able to easily
implement a (naive, unoptimized) graph with strings as keys.  However,
petgraph is not architected to be used directly in this way.  Graph supports
using arbitrary values but operates in terms of the `NodeIndex<Ix>` values
returned by `add_node`.  GraphMap does exist and allows adding edges
directly by using the nodes directly (or rather, their "weights"), but
requires the weights to implement `Copy`, which is not the case for String.
Additionally, https://timothy.hobbs.cz/rust-play/petgraph-internals.html
indicates GraphMap has worse performance characteristics.

To this end, we implement wrappers around Petgraph that let us operate in
a more ergonomic fashion.  We structure our wrappers to support the creation
of multiple graphs backed by a shared pool of symbol information,
recognizing that:
- petgraph's `Graph` doesn't really like having nodes/edges removed (which
  is why `StableGraph` exists), favoring a graph that is incrementally built
  in an append-only fashion and then used immediately thereafter.
- For debugging and to make it easier for people to understand how searchfox
  works here, it's desirable to be able to visualize the various graph
  states that are produced in the process of the algorithms.  Which means
  that an approach where we take graphs as immutable inputs and produce new
  immutable graphs as output works for us.
- This probably works out better with rust's ownership model?

For a more sophisticated and elegant approach to things like this, it's
worth considering the approach used by cargo-guppy at
https://github.com/facebookincubator/cargo-guppy/tree/main/guppy/src/graph
which is built using custom index classes and other sophisticated things
that I (:asuth) likely won't understand until after this implementation
is working.

### Structs and their relationships

- SymbolGraphNodeSet holds the collection of symbols, which consists of a
  vector of the per-symbol crossref information wrapped into a
  DerivedSymbolInfo which provides us a location to put optionally caching
  getters for facts about the symbol that can be internally derived from
  just the symbol's crossref information.
- SymbolGraphNodeId is a u32 identifier for the symbol which is what we use
  as the node weight in the graphs.  The identifier is just the index of the
  DerivedSymbolInfo in its containing vec.
- NamedSymbolGraph wraps the underlying Graph and provides manipulation
  methods that operate using SymbolGraphNodeId values as nodes that can be
  used to describe edges.  This should gain metadata fields
- SymbolGraphCollection bundles a SymbolGraphNodeSet with all of the
  NamedSymbolGraph instances that depend on the node set and are appropriate
  to surface through the pipeline as results or interesting intermediary
  states for debugging.
*/

pub fn make_safe_port_id(dubious_id: &str) -> String {
    return dubious_id.replace(|x| x == '<' || x == '>' || x == ':' || x == '"', "_");
}

/// A symbol and its cross-reference information plus caching helpers.
#[derive(Clone)]
pub struct DerivedSymbolInfo {
    pub symbol: Ustr,
    pub crossref_info: Value,
    pub badges: Vec<SymbolBadge>,
    /// For symbols that are fields with pointer_infos, we set the effective
    /// subsystem to be the subsystem of the first pointer_info payload.  We
    /// use this as a first attempt at grouping fields, but it might make sense
    /// instead to store the SymbolNodeId of the first target here instead.
    pub effective_subsystem: Option<Ustr>,
    pub depth: u32,
}

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct SymbolBadge {
    // Priority for us to mess with the ordering of badges.
    pub pri: i32,
    pub label: Ustr,
    // XXX this doesn't work yet; we'll need to tunnel the extra metadata about
    // these through the cell's id.
    pub source_jump: Option<String>,
}

pub fn semantic_kind_is_callable(semantic_kind: &str) -> bool {
    match semantic_kind {
        "function" => true,
        "method" => true,
        // XXX this is to enable visualizing include deps using "calls-to"; this
        // makes some sense, but really it does imply that this should be a
        // pairwise function if we're going to overload "calls-to" like this.
        // It potentially seems reasonable to do this because it would be a bit
        // pedantic to demand people manually pick the right term; our whole
        // reason for calls-to/calls-from is mainly because "uses" is just so
        // ambiguous.  Also, this is really about avoiding "calls-from" showing
        // fields being referenced when we're talking about control-flow.  That
        // might suggest an orthogonal diagram setting or something.
        "file" => true,
        _ => false,
    }
}

// TODO: evaluate the type of kinds we now allow thanks to SCIP; we may need to
// expand this match branch or just normalize more in SCIP indexing.
pub fn semantic_kind_is_class(semantic_kind: &str) -> bool {
    match semantic_kind {
        "class" => true,
        // Gecko has many structs that are basically classes; also, for our
        // purposes in general, anything with fields is a class.
        "struct" => true,
        _ => false,
    }
}

impl DerivedSymbolInfo {
    pub fn is_callable(&self) -> bool {
        match self.crossref_info.pointer("/meta/kind") {
            Some(Value::String(sem_kind)) => semantic_kind_is_callable(sem_kind),
            _ => false,
        }
    }

    pub fn is_class(&self) -> bool {
        match self.crossref_info.pointer("/meta/kind") {
            Some(Value::String(sem_kind)) => semantic_kind_is_class(sem_kind),
            _ => false,
        }
    }

    /// Provide the structured rep of this symbol if it has one.  If we use this
    /// a lot we should potentially consider using interior mutability to cache
    /// this or have performed the conversion eagerly upon creation.
    pub fn get_structured(&self) -> Option<AnalysisStructured> {
        match self.crossref_info.get("meta") {
            Some(v) => from_value(v.clone()).ok(),
            _ => None,
        }
    }

    /// For hierarchy purposes we want to skip over namespace or namespace-like
    /// symbols in certain modes of operation, this centralizes the heuristic
    /// for that.  Right now this is C++ specific.
    ///
    /// TODO: handle rust and other types from SCIP; as noted in scip-indexer
    /// we probably need to do work there too.  Definitely a case for some unit
    /// tests.
    pub fn is_namespace(&self) -> bool {
        self.symbol.starts_with("NS_")
    }

    pub fn get_pretty(&self) -> Ustr {
        match self.crossref_info.pointer("/meta/pretty") {
            Some(Value::String(pretty)) => ustr(pretty),
            _ => self.symbol.clone(),
        }
    }

    pub fn get_binding_slot_sym(&self, kind: &str) -> Option<Ustr> {
        if let Some(Value::Array(slots)) = self.crossref_info.pointer("/meta/bindingSlots") {
            for slot in slots {
                if let Some(Value::String(slot_kind)) = slot.get("slotKind") {
                    if slot_kind.as_str() != kind {
                        continue;
                    }
                    if let Some(Value::String(sym)) = slot.get("sym") {
                        return Some(ustr(sym));
                    }
                    break;
                }
            }
        }
        None
    }

    pub fn get_subsystem(&self) -> Option<Ustr> {
        match self.crossref_info.pointer("/meta/subsystem") {
            Some(Value::String(subsystem)) => Some(ustr(subsystem)),
            _ => None,
        }
    }

    /// Find the path that contains a "def" record for this symbol.  (There
    /// should ideally only be a single definition path, even if we might have
    /// multiple line hits at the path because of #ifdefs.)
    pub fn get_def_path(&self) -> Option<&String> {
        match self.crossref_info.pointer("/defs/0/path") {
            Some(Value::String(path)) => Some(path),
            _ => None,
        }
    }

    /// If this symbol has a definition, return the definition's line number.
    /// This is intended to assist with lexically ordering fields within a
    /// structure/class.
    pub fn get_def_lno(&self) -> u64 {
        match self.crossref_info.pointer("/defs/0/lines/0/lno") {
            Some(Value::Number(lno)) => lno.as_u64().unwrap_or(0),
            _ => 0,
        }
    }

    // Potentially reduce our memory usage by dropping our uses and calls fields
    // if they are present, as they won't be used for jumpref production.
    pub fn reduce_memory_usage_by_dropping_non_jumpref_info(&mut self) {
        if let Some(obj) = self.crossref_info.as_object_mut() {
            obj.remove("uses");
            obj.remove("callees");
        }
    }
}

impl DerivedSymbolInfo {
    pub fn new(symbol: Ustr, crossref_info: Value, depth: u32) -> Self {
        DerivedSymbolInfo {
            symbol,
            crossref_info,
            badges: vec![],
            effective_subsystem: None,
            depth,
        }
    }
}

/// A collection of one or more graphs that share a common underlying set of
/// per-symbol information across the graphs.
pub struct SymbolGraphCollection {
    pub node_set: SymbolGraphNodeSet,
    pub edge_set: SymbolGraphEdgeSet,
    pub graphs: Vec<NamedSymbolGraph>,
    pub overloads_hit: Vec<OverloadInfo>,
    pub hierarchical_graphs: Vec<HierarchicalSymbolGraph>,
}

impl Serialize for SymbolGraphCollection {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut graphs = vec![];
        for i in 0..self.graphs.len() {
            graphs.push(self.graph_to_json(i));
        }

        let mut hierarchical_graphs = vec![];
        for i in 0..self.hierarchical_graphs.len() {
            hierarchical_graphs.push(self.hier_graph_to_json(i));
        }

        let mut sgc = serializer.serialize_struct("SymbolGraphCollection", 2)?;
        sgc.serialize_field(
            "jumprefs",
            &self.node_set.symbols_meta_to_jumpref_json_nomut(),
        )?;
        sgc.serialize_field("graphs", &graphs)?;
        sgc.serialize_field("hierarchicalGraphs", &hierarchical_graphs)?;
        sgc.end()
    }
}

/// Escape double-quotes to safely use a string as an `esc` tagged value.
///
/// Although graphviz-rust's dot-generator has a concept of `esc`, this does not
/// actually perform any escaping of quotes at the current time. Instead, it
/// just wraps the string in double quotes.  But if we fail to escape any double
/// quotes in the value, they will not be escaped and the graphviz parse will
/// fail.
fn escape_quotes(s: &str) -> String {
    // We're using a raw string so this backslash is propagated as a backslash
    // and is not escaping the double-quote.
    s.replace('"', r#"\""#)
}

/// Perform the necessary escaping for `html` tagged value contents that aren't
/// supposed to be HTML.
fn escape_html(s: &str) -> String {
    s.replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
}

/// Helper for cases where we want a NodeId that's escaped because there currently
/// is no macro support for this.  We automatically call `escape_quotes` to
/// escape any double-quotes that might be in the identifier.
fn escaped_node_id(id: &str) -> NodeId {
    NodeId(Id::Escaped(format!("\"{}\"", escape_quotes(id))), None)
}

impl SymbolGraphCollection {
    /// Convert the graph with the given index to a { nodes, edges } rep where:
    ///
    /// - nodes is a sorted array of symbol strings.
    /// - edges is a sorted array of { from, to } where from/to are symbol
    ///   strings and the sort is over [from, to]
    pub fn graph_to_json(&self, graph_idx: usize) -> Value {
        let graph = match self.graphs.get(graph_idx) {
            Some(g) => g,
            None => return json!({}),
        };

        // I am biasing for code readability over performance.  In particular,
        // note that we need not infer the nodes from the edges, but it's less
        // code this way.
        //
        // XXX currently we're not serializing the edge information here either,
        // but probably should.
        let mut nodes = BTreeSet::new();
        let mut edges = BTreeMap::new();
        for (source_id, target_id, _edge_id) in graph.list_edges() {
            let source_info = self.node_set.get(&source_id);
            nodes.insert(source_info.symbol.clone());
            let source_sym = source_info.symbol.clone();

            let target_info = self.node_set.get(&target_id);
            nodes.insert(target_info.symbol.clone());
            let target_sym = target_info.symbol.clone();

            edges.insert(
                format!("{}-{}", source_sym, target_sym),
                json!({ "from": source_sym, "to": target_sym }),
            );
        }

        json!({
            "nodes": nodes.into_iter().collect::<Vec<Ustr>>(),
            "edges": edges.into_values().collect::<Value>(),
        })
    }

    /// Convert the graph with the given index to a { nodes, edges } rep where:
    ///
    /// - nodes is a sorted array of symbol strings.
    /// - edges is a sorted array of { from, to } where from/to are symbol
    ///   strings and the sort is over [from, to]
    pub fn hier_graph_to_json(&self, graph_idx: usize) -> Value {
        let graph = match self.hierarchical_graphs.get(graph_idx) {
            Some(g) => g,
            None => return json!({}),
        };

        graph.root.to_json(&self.node_set)
    }

    /// Convert the graph with the given index to a graphviz rep.
    pub fn graph_to_graphviz<F>(&self, graph_idx: usize, node_decorate: F) -> Graph
    where
        F: Fn(&mut Node, &DerivedSymbolInfo),
    {
        let mut dot_graph = graph!(
            di id!("g");
            node!("node"; attr!("shape","box"), attr!("fontname", esc "Courier New"), attr!("fontsize", "10"))
        );

        let graph = match self.graphs.get(graph_idx) {
            Some(g) => g,
            None => return dot_graph,
        };

        let mut nodes = BTreeSet::new();
        for (source_id, target_id, _edge_id) in graph.list_edges() {
            let source_info = self.node_set.get(&source_id);
            let source_sym = source_info.symbol.clone();
            if nodes.insert(source_sym.clone()) {
                let mut node = node!(esc source_sym.clone(); attr!("label", esc escape_quotes(&source_info.get_pretty())));
                node_decorate(&mut node, source_info);
                dot_graph.add_stmt(stmt!(node));
            }

            let target_info = self.node_set.get(&target_id);
            let target_sym = target_info.symbol.clone();
            if nodes.insert(target_sym.clone()) {
                let mut node = node!(esc target_sym.clone(); attr!("label", esc escape_quotes(&target_info.get_pretty())));
                node_decorate(&mut node, target_info);
                dot_graph.add_stmt(stmt!(node));
            }

            // node_id!'s macro_rules currently can't handle an `esc` prefix, so
            // we create the structs via a hand-rolled `escaped_node_id` that
            // replicates what the equivalent macros would do.
            dot_graph.add_stmt(stmt!(
                edge!(escaped_node_id(&source_sym) => escaped_node_id(&target_sym))
            ));
        }

        dot_graph
    }

    pub fn to_json(&self) -> Value {
        to_value(self).unwrap()
    }

    pub async fn derive_hierarchical_graph(
        &mut self,
        policies: &HierarchyPolicies,
        graph_idx: usize,
        server: &Box<dyn AbstractServer + Send + Sync>,
    ) -> Result<()> {
        trace!("derive_hierarchical_graph");
        let graph = match self.graphs.get(graph_idx) {
            Some(g) => g,
            None => {
                return Ok(());
            }
        };

        let mut root = HierarchicalNode {
            segment: HierarchySegment::PrettySegment("".to_string(), ""),
            display_name: "".to_string(),
            symbols: vec![],
            action: None,
            children: BTreeMap::default(),
            edges: vec![],
            descendant_edge_count: 0,
            height: 0,
        };

        let mut checked_pretties = UstrMap::default();
        let mut sym_segments: HashMap<SymbolGraphNodeId, Vec<HierarchySegment>> =
            HashMap::default();

        // ## Populate the hierarchy nodes.
        //
        // We have a few major modes of operation here:
        // - (Flat doesn't count; this method won't be called.)
        // - For GraphHierarchy::Pretty we just split everything by the pretty
        //   and try to lookup the pretty in case there is a symbol associated
        //   with it.  In some cases, we may not have symbol information.
        // - For everything else we want to figure out the topmost non-namespace
        //   symbol.  That is, if we have "foo::bar::OuterClass::InnerClass::method"
        //   then we want to lose "foo::bar::" but retain
        //   "OuterClass::InnerClass::method".  We would then prepend the
        //   subsystem/file/dir.  Our structured information does not currently
        //   provide an explicit relationship between nested classes and their
        //   containing classes, so the pretty splicing is our best option at
        //   this time.
        //
        // Because of the commonality of needing to potentially consider every
        // level of pretty symbol, we run this as a 3-pass approach:
        // 1. We parse the pretty symbols and attempt to perform a symbol lookup
        //    for every piece
        // 2. We branch based on the GraphHierarchy requested to populate the
        //    segments.
        // 3. We process the segments to populate the nodes.
        //
        // Extra complications:
        // - Inner classes (a class defined within another class) are a major
        //   practical problem because visually we would like to represent a
        //   class and its fields as a "record"-type HTML label display, but
        //   if we nest the inner class under the root class, this causes our
        //   heuristics to not fire.  We address this problem by aggregating
        //   the outer class name onto the inner class name.  A class "Foo" in
        //   the namespace "ns" which has an inner class "InnerFoo" would end
        //   up with Foo having pretty segments of ["ns", "Foo"] and InnerFoo
        //   having pretty segments of ["ns", "Foo::Innerfoo"] with no potential
        //   for "Foo::InnerFoo" to be split an additional time.

        // For synthetic nodes / clusters, give them a depth of 13 right now
        // which is current our last depth enum, although we saturate depth
        // visually at 10 right now.
        const SYNTHETIC_DEPTH: u32 = 13;

        for sym_id in graph.list_nodes() {
            let (sym, sym_pretty) = {
                let node_info = self.node_set.get(&sym_id);
                (node_info.symbol.as_str(), node_info.get_pretty().as_str())
            };
            let mut pretty_so_far = "".to_string();
            trace!(sym = %sym_pretty, "processing symbol");
            let (pieces, delim) = split_pretty(sym_pretty, sym);
            let mut pieces_and_syms = vec![];
            for mut piece in pieces {
                trace!(piece = %piece, "processing piece");
                pretty_so_far = if pretty_so_far.is_empty() {
                    piece.clone()
                } else {
                    format!("{}{}{}", pretty_so_far, delim, piece)
                };
                let ustr_so_far = ustr(&pretty_so_far);

                // If this is a partial pretty, then we only need to perform a lookup
                // for it once, but if it's a full pretty then we need to process it
                // because overloads exist (and have the same pretty)!
                if sym_pretty == pretty_so_far || !checked_pretties.contains_key(&ustr_so_far) {
                    // We haven't checked this before, so process it.

                    // inner class handling help
                    // (needs to borrow from node_set, so invoke before ensure_symbol below)
                    let container_is_class = match pieces_and_syms.last() {
                        Some((_, Some(container_sym_id))) => {
                            let container_info = self.node_set.get(container_sym_id);
                            container_info.is_class()
                        }
                        _ => false,
                    };

                    // See if we can find a symbol for this identifier.
                    if sym_pretty == pretty_so_far {
                        trace!(pretty = %pretty_so_far, "reusing known symbol");

                        if container_is_class && self.node_set.get(&sym_id).is_class() {
                            if let Some((container_piece, _)) = pieces_and_syms.pop() {
                                trace!(pretty = %pretty_so_far, "inner class heuristic merging class piece '{}' with container '{}'", container_piece, piece);
                                piece = format!("{}{}{}", container_piece, delim, piece);
                            }
                        }
                        pieces_and_syms.push((piece, Some(sym_id.clone())));
                    } else {
                        // TODO: Either don't set the limit to 1 or provide a better
                        // explanation or some assert on why this is okay.  In
                        // general, since we are taking a fast path on the full pretty
                        // match, we shouldn't be dealing with overloads here, so there
                        // really should only be a single ancestor symbol per pretty.
                        if let Some((match_sym, _)) = server
                            .search_identifiers(&pretty_so_far, true, false, 1)
                            .await?
                            .iter()
                            .next()
                        {
                            let (match_sym_id, match_sym_info) = self
                                .node_set
                                .ensure_symbol(&match_sym, server, SYNTHETIC_DEPTH)
                                .await?;

                            let needs_pop = if container_is_class && match_sym_info.is_class() {
                                if let Some((container_piece, _)) = pieces_and_syms.pop() {
                                    trace!(pretty = %pretty_so_far, "inner class heuristic merging class piece '{}' with container '{}'", container_piece, piece);
                                    piece = format!("{}{}{}", container_piece, delim, piece);
                                }
                                true
                            } else {
                                false
                            };

                            checked_pretties
                                .insert(ustr_so_far, Some((match_sym_id.clone(), needs_pop)));
                            pieces_and_syms.push((piece, Some(match_sym_id)));
                        } else {
                            trace!(pretty = %pretty_so_far, "failed to locate symbol for identifier");
                            checked_pretties.insert(ustr_so_far, None);
                            pieces_and_syms.push((piece, None));
                        }
                    };
                } else {
                    match checked_pretties.get(&ustr_so_far) {
                        Some(Some((use_sym_id, needs_pop))) => {
                            if *needs_pop {
                                if let Some((container_piece, _)) = pieces_and_syms.pop() {
                                    trace!(pretty = %pretty_so_far, "inner class heuristic merging class piece '{}' with container '{}'", container_piece, piece);
                                    piece = format!("{}{}{}", container_piece, delim, piece);
                                }
                            }
                            pieces_and_syms.push((piece, Some(use_sym_id.clone())));
                        }
                        _ => {
                            pieces_and_syms.push((piece, None));
                        }
                    }
                }
            }

            let first_real_sym = pieces_and_syms
                .iter()
                .position(|(_piece, maybe_sym)| {
                    if let Some(sym) = maybe_sym {
                        let info = self.node_set.get(sym);
                        !info.is_namespace()
                    } else {
                        false
                    }
                })
                .unwrap_or_else(|| pieces_and_syms.len() - 1);
            let segments_and_syms =
                match &policies.grouping {
                    GraphHierarchy::Flat | GraphHierarchy::Pretty => pieces_and_syms
                        .into_iter()
                        .map(|(piece, sym)| (HierarchySegment::PrettySegment(piece, delim), sym))
                        .collect_vec(),
                    GraphHierarchy::Subsystem => {
                        // For subsystem we always use the subsystem of the first
                        // symbol we find now in order to avoid weird cases where
                        // the methods in a symbol are defined in a different cpp
                        // file that is technically part of a different subsystem.
                        //
                        // This is different than the decision we've made for dir/file
                        // where we do allow fragmentation (but where the class pretty
                        // containment will mean that the class shows up in both the
                        // right place and also the wrong place(s)).
                        let subsystem = match &pieces_and_syms[first_real_sym].1 {
                            Some(sym) => self
                                .node_set
                                .get(&sym)
                                .get_subsystem()
                                .map(|x| x.as_str())
                                .unwrap_or(""),
                            None => "",
                        };
                        let segs = subsystem.split("/").map(|piece| {
                            (
                                HierarchySegment::PrettySegment(piece.to_string(), "/"),
                                None,
                            )
                        });
                        segs.chain(pieces_and_syms.into_iter().skip(first_real_sym).map(
                            |(piece, sym)| (HierarchySegment::PrettySegment(piece, delim), sym),
                        ))
                        .collect_vec()
                    }
                    grouping @ GraphHierarchy::File | grouping @ GraphHierarchy::Dir => {
                        // We use the most-specific symbol here.
                        let path = match &pieces_and_syms[pieces_and_syms.len() - 1].1 {
                            Some(sym) => self
                                .node_set
                                .get(&sym)
                                .get_def_path()
                                .map(|x| x.as_str())
                                .unwrap_or(""),
                            None => "",
                        };
                        let mut segs = path.split("/").collect_vec();
                        if let GraphHierarchy::Dir = grouping {
                            segs.pop();
                        };
                        segs.into_iter()
                            .map(|piece| {
                                (
                                    HierarchySegment::PrettySegment(piece.to_string(), "/"),
                                    None,
                                )
                            })
                            .chain(pieces_and_syms.into_iter().skip(first_real_sym).map(
                                |(piece, sym)| (HierarchySegment::PrettySegment(piece, delim), sym),
                            ))
                            .collect_vec()
                    }
                };

            let mut segments_so_far = vec![];
            for (segment, maybe_sym) in segments_and_syms {
                segments_so_far.push(segment);
                let mut reversed_segments = segments_so_far.clone();
                reversed_segments.reverse();
                if let Some(sym_id) = maybe_sym {
                    //trace!(pretty = %segments_so_far, "placing found symbol");
                    root.place_sym(reversed_segments, sym_id);
                }
            }

            sym_segments.insert(sym_id, segments_so_far);
        }

        // ## Populate the hierarchy edges
        for (from_id, to_id, edge_id) in graph.list_edges() {
            let from_segments = sym_segments.get(&from_id).unwrap();
            let to_segments = sym_segments.get(&to_id).unwrap();

            let mut common_path: Vec<HierarchySegment> = from_segments
                .iter()
                .zip(to_segments.iter())
                .take_while(|(a, b)| a == b)
                .map(|(a, _)| a.clone())
                .collect();

            // If one is an ancestor of the other, then put the edge above the
            // outer ancestor by popping off a segment.  This allows us to use a
            // table where we might otherwise fall back to a cluster.
            if common_path.len() == from_segments.len() || common_path.len() == to_segments.len() {
                common_path.pop();
            }
            common_path.reverse();
            root.place_edge(common_path, from_id, to_id, edge_id);
        }

        self.hierarchical_graphs.push(HierarchicalSymbolGraph {
            name: graph.name.clone(),
            root,
        });

        Ok(())
    }

    /// Convert the graph with the given index to a graphviz rep.
    pub fn hierarchical_graph_to_graphviz(
        &mut self,
        policies: &HierarchyPolicies,
        graph_idx: usize,
        graph_layout: &GraphLayout,
    ) -> (Graph, HierarchicalRenderState) {
        trace!(graph_idx = %graph_idx, "hierarchical_graph_to_graphviz");
        let mut state = HierarchicalRenderState::new();
        let graph = match self.hierarchical_graphs.get_mut(graph_idx) {
            Some(g) => g,
            None => {
                trace!("no such graph");
                return (
                    graph!(
                        di id!("g");
                        node!("node"; attr!("shape","box"), attr!("fontname", esc "Courier New"), attr!("fontsize", "10"))
                    ),
                    state,
                );
            }
        };

        graph
            .root
            .compile(&policies, 0, 0, false, &self.node_set, &mut state);

        let mut dot_graph = Graph::DiGraph {
            id: id!("g"),
            strict: false,
            stmts: vec![],
        };

        // XXX I'm trying something here where we add our own styling statements
        // here before rendering the graph and then propagating its statements
        // across.  As noted below, we previously were just having the styling
        // happening in the graph rendering process below; we really need to
        // figure out where this should happen, but this hybrid approach is
        // somewhat reasonable for now.
        match graph_layout {
            GraphLayout::Neato => {
                dot_graph.add_stmt(stmt!(node!("graph"; attr!("overlap","prism"), attr!("mode","hier"), attr!("sep",esc "+10"))));
            }
            _ => {}
        }

        // Note that the root node renders the default style directives.
        let stmts = graph
            .root
            .render(policies, &self.node_set, &self.edge_set, &mut state);
        for stmt in stmts {
            dot_graph.add_stmt(stmt);
        }

        (dot_graph, state)
    }
}

/// A graph whose nodes are symbols from a `SymbolGraphNodeSet`.
pub struct NamedSymbolGraph {
    pub name: String,
    graph: PetGraph<u32, SymbolGraphEdgeId, Directed>,
    /// Maps SymbolGraphNodeId values to NodeIndex values when the node is
    /// present in the graph.  Exclusively used by ensure_node and it's likely
    /// this could be improved to more directly use NodeIndex.
    node_id_to_ix: HashMap<u32, DefaultIx>,
    /// Inverted/reverse map of the above.
    node_ix_to_id: HashMap<DefaultIx, u32>,
}

impl NamedSymbolGraph {
    pub fn new(name: String) -> Self {
        NamedSymbolGraph {
            name,
            graph: PetGraph::new(),
            node_id_to_ix: HashMap::new(),
            node_ix_to_id: HashMap::new(),
        }
    }

    pub fn containts_node(&self, sym_id: SymbolGraphNodeId) -> bool {
        self.node_id_to_ix.contains_key(&sym_id.0)
    }

    pub fn ensure_node(&mut self, sym_id: SymbolGraphNodeId) -> NodeIndex {
        if let Some(idx) = self.node_id_to_ix.get(&sym_id.0) {
            return NodeIndex::new(*idx as usize);
        }

        let idx = self.graph.add_node(sym_id.0).index() as u32;
        self.node_id_to_ix.insert(sym_id.0, idx);
        self.node_ix_to_id.insert(idx, sym_id.0);

        NodeIndex::new(idx as usize)
    }

    pub fn list_nodes(&self) -> Vec<SymbolGraphNodeId> {
        self.graph
            .node_indices()
            .map(|ix| SymbolGraphNodeId(*self.node_ix_to_id.get(&(ix.index() as u32)).unwrap()))
            .collect()
    }

    pub fn ensure_edge(
        &mut self,
        source: SymbolGraphNodeId,
        target: SymbolGraphNodeId,
        edge: SymbolGraphEdgeId,
    ) {
        let source_ix = self.ensure_node(source);
        let target_ix = self.ensure_node(target);
        self.graph.update_edge(source_ix, target_ix, edge);
    }

    pub fn list_edges(&self) -> Vec<(SymbolGraphNodeId, SymbolGraphNodeId, SymbolGraphEdgeId)> {
        let mut id_edges = vec![];
        for edge in self.graph.raw_edges() {
            let source_id = self
                .node_ix_to_id
                .get(&(edge.source().index() as u32))
                .unwrap();
            let target_id = self
                .node_ix_to_id
                .get(&(edge.target().index() as u32))
                .unwrap();
            id_edges.push((
                SymbolGraphNodeId(*source_id),
                SymbolGraphNodeId(*target_id),
                edge.weight.clone(),
            ));
        }
        id_edges
    }

    /// Find all the paths between two nodes; if you have more than one pair of
    /// nodes you probably want to use `all_simple_paths_using_supernodes` which
    /// will induce source and sink supernodes.
    pub fn all_simple_paths(
        &mut self,
        source: SymbolGraphNodeId,
        target: SymbolGraphNodeId,
    ) -> Vec<Vec<(SymbolGraphNodeId, SymbolGraphNodeId, SymbolGraphEdgeId)>> {
        let source_ix = self.ensure_node(source);
        let target_ix = self.ensure_node(target);
        let paths = all_simple_paths(&self.graph, source_ix, target_ix, 0, None);
        let node_paths = paths
            .map(|v: Vec<_>| {
                v.into_iter()
                    .tuple_windows()
                    .map(|(src, tgt)| {
                        let edge_ix = self.graph.find_edge(src, tgt).unwrap();
                        (
                            SymbolGraphNodeId(
                                *self.node_ix_to_id.get(&(src.index() as u32)).unwrap(),
                            ),
                            SymbolGraphNodeId(
                                *self.node_ix_to_id.get(&(tgt.index() as u32)).unwrap(),
                            ),
                            self.graph[edge_ix].clone(),
                        )
                    })
                    .collect()
            })
            .collect();
        node_paths
    }

    /// XXX don't use this, use
    ///
    /// Variant of all_simple_paths that takes source and target sets and
    /// creates supernodes behind the source set and from the target set in
    /// order to potentially improve the net algorithmic complexity.
    ///
    /// Right now this will mutate our current graph and we don't bother
    /// cleaning that up because the expectation is a successor graph will be
    /// created.
    pub fn all_simple_paths_using_supernodes(
        &mut self,
        next_node_id: u32,
        next_edge_id: u32,
        source_nodes: &Vec<SymbolGraphNodeId>,
        target_nodes: &Vec<SymbolGraphNodeId>,
    ) -> Vec<Vec<(SymbolGraphNodeId, SymbolGraphNodeId, SymbolGraphEdgeId)>> {
        let super_source_id = next_node_id;
        let super_target_id = super_source_id + 1;

        let super_source_ix = self.graph.add_node(super_source_id);
        let super_target_ix = self.graph.add_node(super_target_id);

        let synth_source_edge_id = next_edge_id;
        let synth_target_edge_id = synth_source_edge_id + 1;

        // Add edges from the synthetic source supernode to all source nodes
        for source_id in source_nodes {
            let source_ix = self.ensure_node(source_id.clone());
            self.graph.add_edge(
                super_source_ix,
                source_ix,
                SymbolGraphEdgeId(synth_source_edge_id),
            );
        }

        // Add edges from all target nodes to the synthetic target supernode.
        for target_id in target_nodes {
            let target_ix = self.ensure_node(target_id.clone());
            self.graph.add_edge(
                target_ix,
                super_target_ix,
                SymbolGraphEdgeId(synth_target_edge_id),
            );
        }

        trace!(num_nodes=%super_target_id, num_edges=%synth_target_edge_id, "created supernodes, running petgraph all_simple_paths algorithm");

        // Now we get the paths...
        let paths = all_simple_paths(&self.graph, super_source_ix, super_target_ix, 0, None);

        trace!("have iterator");
        let node_paths = paths
            .map(|v: Vec<_>| {
                v.into_iter()
                    // skip the source supernode
                    .dropping(1)
                    // skip the target supernode
                    .dropping_back(1)
                    .tuple_windows()
                    .map(|(src, tgt)| {
                        let edge_ix = self.graph.find_edge(src, tgt).unwrap();
                        (
                            SymbolGraphNodeId(
                                *self.node_ix_to_id.get(&(src.index() as u32)).unwrap(),
                            ),
                            SymbolGraphNodeId(
                                *self.node_ix_to_id.get(&(tgt.index() as u32)).unwrap(),
                            ),
                            self.graph[edge_ix].clone(),
                        )
                    })
                    .collect()
            })
            .collect();
        node_paths
    }
}

/// Helper hierarchy for building graphviz html table labels as used for our
/// graph display.  This is not meant to be generic.
pub struct LabelTable {
    pub rows: Vec<LabelRow>,

    pub columns_needed: u32,
}

pub struct LabelRow {
    // note that currently our "compile" step assumes there's only ever one cell
    pub cells: Vec<LabelCell>,
}

pub struct LabelCell {
    pub id: Option<String>,
    pub bg_color: Option<&'static str>,
    pub contents: String,
    pub badges: Vec<SymbolBadge>,
    pub symbol: String,
    pub port: String,
    pub indent_level: u32,
}

impl LabelTable {
    pub fn compile(&mut self) {
        for row in &self.rows {
            self.columns_needed = std::cmp::max(self.columns_needed, row.compile());
        }
    }

    pub fn render(&self) -> String {
        let mut rows = vec![];
        for row in &self.rows {
            rows.push(row.render(self.columns_needed));
        }
        format!(
            r#"<<table border="0" cellborder="1" cellspacing="0" cellpadding="4">{}</table>>"#,
            rows.join("")
        )
    }
}

impl LabelRow {
    pub fn compile(&self) -> u32 {
        let mut columns = 0;
        for cell in &self.cells {
            columns += cell.indent_level + 1;
        }
        columns
    }

    pub fn render(&self, _column_count: u32) -> String {
        let mut row_pieces = vec![];
        for cell in &self.cells {
            let indent_str = "&nbsp;".repeat(cell.indent_level as usize);
            let maybe_id = match &cell.id {
                Some(idval) => format!("id=\"{}\" ", escape_quotes(idval)),
                None => "".to_string(),
            };
            let maybe_styling = match &cell.bg_color {
                Some(bgcolor) => format!("bgcolor=\"{}\" ", bgcolor),
                None => "".to_string(),
            };
            let badge_reps = cell
                .badges
                .iter()
                .map(|b| format!("<U>{}</U>", escape_html(&b.label)))
                .collect_vec();
            row_pieces.push(format!(
                r#"<td {}{}href="{}" port="{}" align="left">{}{}{}{}</td>"#,
                maybe_id,
                maybe_styling,
                urlencoding::encode(&cell.symbol),
                cell.port,
                indent_str,
                // The contents can explicitly contain HTML and so the populator is responsible to
                // escape as appropriate.
                cell.contents,
                if badge_reps.is_empty() { "" } else { "  " },
                badge_reps.join(""),
            ));
        }
        format!("<tr>{}</tr>", row_pieces.join(""))
    }
}

/// Default policy for when to summarize clusters in the hierarchical diagram;
/// specific overrides can be set in both directions.
#[derive(Clone, Debug, PartialEq, ValueEnum)]
pub enum HierarchyDefaultSummarizePolicy {
    /// Summarize everything so we can just have an overview.
    All,
    /// Summarize nothing; everything is expanded.
    None,
    /// Summarize clusters that don't have a root node in them.
    Other,
}

/// Policies to guide the hierarchy creation and rendering.
pub struct HierarchyPolicies {
    pub grouping: GraphHierarchy,
    pub summarize: HierarchyDefaultSummarizePolicy,
    pub force_summarize_pretties: HashSet<String>,
    pub force_expand_pretties: HashSet<String>,
    pub group_fields_at: u32,
    pub use_port_dirs: bool,
}

pub enum HierarchicalLayoutAction {
    /// Used for the root node; its contents are rendered at the same level as
    /// the parent and the parent is not rendered.
    Flatten,
    /// Collapse the node into its child.  This is used for situations like
    /// namespaces with one one child which is a sub-namespace and there is no
    /// benefit to creating a separate cluster for the parent.
    Collapse,
    /// Be a graphviz cluster.  Used for situations where there are multiple
    /// children that are eitehr conceptually distinct (ex: separate classes) or
    /// where there are simply too many edges directly between its children and
    /// so the use of a table would be very visually busy.
    ///
    /// Currently there is a NodeId for the cluster and one for the placeholder.
    Cluster(String, String),
    /// Be a table and therefore all children are records.  Used for situations
    /// like a class where the children are methods/fields and the containment
    /// relationship makes sense to express as a table and there is no issue
    /// with edges between the children making the diagram too busy.
    ///
    /// Payload is the node id for the node that is/holds the HTML label.
    Table(String),
    /// For children of a Table parent.
    ///
    /// Payload is the port name.
    Record(String),
    /// Just a normal graphviz node, either contained in a cluster or by the
    /// root.
    Node(String),
}

/// A typed hierarchy segment.  While initially we expect all segments to be
/// part of a pretty identifier, in the future this may include:
/// - Process Type (Parent, Content, Network, etc.)
/// - Subsystem / subcomponent / submodule
///
/// XXX for now we're just going to use `PrettySegment` for everything because
/// it containing the delimiter will let us get away with a lot, and we
/// explicitly associate symbols with the `HierarchicalNode` instances, which
/// means we don't need them on the segment here, but it could make sense to
/// revisit.
#[derive(Clone, Eq, PartialEq, PartialOrd, Ord, Debug)]
pub enum HierarchySegment {
    /// The pretty identifier segment and the delimiter that should be used to
    /// join it to its parent.  Note that in some cases like paths we choose to
    /// include the delimeter as part of the segment string and the delimiter
    /// may accordingly be an empty string.
    PrettySegment(String, &'static str),
}

impl HierarchySegment {
    pub fn to_human_readable(&self) -> String {
        match self {
            Self::PrettySegment(p, _) => p.clone(),
        }
    }

    pub fn join_with_str(&self, existing_str: &str) -> String {
        if existing_str.is_empty() {
            return self.to_human_readable();
        }
        match self {
            Self::PrettySegment(piece, delim) => {
                format!("{}{}{}", existing_str, delim, piece)
            }
        }
    }
}

/// A hierarchial graph derived from a NamedSymbolGraph.
pub struct HierarchicalSymbolGraph {
    pub name: String,
    pub root: HierarchicalNode,
}

pub struct HierarchicalNode {
    /// The segment this is named by in the parent node's `children`.
    pub segment: HierarchySegment,
    /// The display name to use for this segment.  This starts out as the
    /// segment or a more human-readable variation of the segment.  This may be
    /// updated in situations like if its parent is given a `Collapse` action.
    ///
    /// This may be empty in the case of the root node.
    pub display_name: String,
    /// The list of symbols associated with with location.  This can happen due
    /// to differences in signatures across platforms resulting in multiple
    /// symbols corresponding to a single pretty identifier, which is what we
    /// want to support here.  This can also happen due to explicitly overloaded
    /// methods, and ideally we would not fully coalesce these cases, but for
    /// now we do.
    ///
    /// TODO: Improve the explicit overloaded method situation.  (There are
    /// thoughts on how to address this elsewhere; maybe the graphing bug?)
    pub symbols: Vec<SymbolGraphNodeId>,
    /// The action to take when rendering this node to a graph.  Initially None,
    /// but set during `compile`, and then later used for rendering (and debug
    /// and test output so we can understand what decisions were taken).
    pub action: Option<HierarchicalLayoutAction>,
    pub children: BTreeMap<HierarchySegment, HierarchicalNode>,
    /// List of the edges for which this node was the common ancestor.  This is
    /// done so that the `compile` step can understand the number of edges
    /// amongst its descendants and avoid creating (too many) edges between
    /// table rows/cells.  It also results in a better organized dot file which
    /// is nice for readability.
    pub edges: Vec<(SymbolGraphNodeId, SymbolGraphNodeId, SymbolGraphEdgeId)>,
    /// How many edges are contained in the descendants of this node (but not
    /// including this node's own edges).
    pub descendant_edge_count: u32,
    /// The maximum descendant depth below this node.  A node with no children
    /// has a height of 0.  A node with a child with no children has a height of
    /// 1.
    pub height: u32,
}

impl HierarchicalNode {
    pub fn to_json(&self, node_set: &SymbolGraphNodeSet) -> Value {
        let symbols: Vec<Ustr> = self
            .symbols
            .iter()
            .map(|id| node_set.get(id).symbol.clone())
            .collect();
        let action = match &self.action {
            None => Value::Null,
            Some(HierarchicalLayoutAction::Flatten) => json!({
                "layoutAction": "flatten",
            }),
            Some(HierarchicalLayoutAction::Collapse) => json!({
                "layoutAction": "collapse",
            }),
            Some(HierarchicalLayoutAction::Cluster(cluster_id, placeholder_id)) => {
                json!({
                    "layoutAction": "cluster",
                    "clusterId": cluster_id,
                    "placeholderId": placeholder_id,
                })
            }
            Some(HierarchicalLayoutAction::Table(node_id)) => {
                json!({
                    "layoutAction": "table",
                    "nodeId": node_id,
                })
            }
            Some(HierarchicalLayoutAction::Record(port_name)) => {
                json!({
                    "layoutAction": "record",
                    "portName": port_name,
                })
            }
            Some(HierarchicalLayoutAction::Node(node_id)) => {
                json!({
                    "layoutAction": "node",
                    "nodeId": node_id,
                })
            }
        };
        let children: Vec<Value> = self
            .children
            .values()
            .map(|kid| kid.to_json(&node_set))
            .collect();
        let edges: Vec<Value> = self
            .edges
            .iter()
            .map(|(from_id, to_id, _edge_id)| {
                // TODO: consider propagating the edge info; it's not essential
                // to validating graph correctness right now, but probably
                // should be something the tests should ensure is stable.
                json!({
                    "from": node_set.get(from_id).symbol.clone(),
                    "to": node_set.get(to_id).symbol.clone(),
                })
            })
            .collect();
        json!({
            "segment": self.segment.to_human_readable(),
            "displayName": self.display_name,
            "height": self.height,
            "symbols": symbols,
            "action": action,
            "children": children,
            "edges": edges,
            "descendantEdgeCount": self.descendant_edge_count,
        })
    }

    /// Recursively traverse child nodes, creating them as needed, in order to
    /// place symbols and create hierarchy as a byproduct.
    pub fn place_sym(
        &mut self,
        mut reversed_segments: Vec<HierarchySegment>,
        sym_id: SymbolGraphNodeId,
    ) {
        if let Some(next_seg) = reversed_segments.pop() {
            let kid = self.children.entry(next_seg.clone()).or_insert_with(|| {
                let display_name = next_seg.to_human_readable();
                HierarchicalNode {
                    segment: next_seg,
                    display_name,
                    symbols: vec![],
                    action: None,
                    children: BTreeMap::default(),
                    edges: vec![],
                    descendant_edge_count: 0,
                    height: 0,
                }
            });
            kid.place_sym(reversed_segments, sym_id);
            // The height of the kid may have updated, so potentially update our
            // height.  This will bubble upwards appropriately.
            self.height = core::cmp::max(self.height, kid.height + 1);
        } else {
            if !self.symbols.contains(&sym_id) {
                self.symbols.push(sym_id);
            }
        }
    }

    pub fn place_edge(
        &mut self,
        mut reversed_segments: Vec<HierarchySegment>,
        from_id: SymbolGraphNodeId,
        to_id: SymbolGraphNodeId,
        edge_id: SymbolGraphEdgeId,
    ) {
        if let Some(next_seg) = reversed_segments.pop() {
            if let Some(kid) = self.children.get_mut(&next_seg) {
                // The edge will go in our descendant, so we bump the count.
                self.descendant_edge_count += 1;
                kid.place_edge(reversed_segments, from_id, to_id, edge_id);
            }
        } else {
            // We do not modify the descendant_edge_count because it's our own
            // edge.
            self.edges.push((from_id, to_id, edge_id));
        }
    }

    pub fn compile(
        &mut self,
        policies: &HierarchyPolicies,
        depth: usize,
        collapsed_ancestors: u32,
        has_class_ancestor: bool,
        node_set: &SymbolGraphNodeSet,
        state: &mut HierarchicalRenderState,
    ) {
        let is_root = depth == 0;

        let is_class = if self.symbols.len() >= 1 {
            let sym_info = node_set.get(&self.symbols[0]);
            sym_info.is_class()
        } else {
            false
        };
        let be_class = has_class_ancestor || is_class;

        // If the node has only one child and no edges, we can collapse it UNLESS
        // the child is a class, in which case we really don't want to.
        if !is_root && !be_class && self.children.len() == 1 && self.edges.len() == 0 {
            let sole_kid = self.children.values_mut().next().unwrap();
            // (not all nodes will have associated symbols)
            let kid_is_class = if let Some(kid_id) = sole_kid.symbols.get(0) {
                node_set.get(kid_id).is_class()
            } else {
                false
            };

            // The child's needs impact our ability to collapse:
            // - If the kid is a class, don't collapse into it.  (Classes can still
            //   be clusters, but the idea is they should/need to be distinguished
            //   from classes.)
            if !kid_is_class {
                self.action = Some(HierarchicalLayoutAction::Collapse);
                if !self.display_name.is_empty() {
                    // There are 2 potential delimiters in play here, although it's really only
                    // the synthetic root where we don't have a useful delimiter and we need to
                    // favor the kid, but this seems like a reasonable policy that the kid knows
                    // its best delimiter.
                    let delim = match &sole_kid.segment {
                        HierarchySegment::PrettySegment(_, delim) => delim,
                    };
                    sole_kid.display_name =
                        format!("{}{}{}", self.display_name, delim, sole_kid.display_name);
                    self.display_name = "".to_string();
                }
                sole_kid.compile(
                    policies,
                    depth + 1,
                    collapsed_ancestors + 1,
                    be_class,
                    node_set,
                    state,
                );
                return;
            }
        }

        let mut be_cluster = false;

        if is_root {
            self.action = Some(HierarchicalLayoutAction::Flatten);
            for kid in self.children.values_mut() {
                kid.compile(
                    policies,
                    depth + 1,
                    collapsed_ancestors,
                    be_class,
                    node_set,
                    state,
                );
            }
        } else if self.edges.len() > 0 && self.children.len() > 0 {
            // If there are edges at this level, it does not make sense to be a
            // table because the self-edges end up quite gratuitous.  (The edges
            // vec only contains edges among our immediate children.)
            //
            // The exception is that if the edge is just a self-edge, then
            // there's no need to make this a cluster.  This can happen for
            // overloaded methods where we collapse them to a single graphviz
            // node but one variant of the function calls the other variant.
            be_cluster = true;
        } else if be_class && self.descendant_edge_count < 5 && self.height == 1 {
            // If the number of internal edges are low and we've reached a class AND
            // we have a height of 1 (which implies having children), then we can
            // be a table.
            //
            // In the prototype, this choice was not aware of height and so could
            // result in trying to create a table that could be complicated by the
            // existence of inner classes.  Our introduction of height should
            // eliminate that concern while not precluding use of a table when
            // we are only dealing with classes.  Like it could be nice to have
            // a class and its immediate subclasses shown as a table as long as
            // there aren't methods nested under the sub-class.
            //
            // Note that the prototype never dealt with that more complex table
            // case, it just had a comment noting the weirdness possible.
            let parent_id_str = self.derive_id(node_set, state);
            {
                let port_id_str = make_safe_port_id(&parent_id_str);
                let in_target = if policies.use_port_dirs {
                    node_id!(esc parent_id_str, port!(id!(esc port_id_str), "w"))
                } else {
                    node_id!(esc parent_id_str, port!(id!(esc port_id_str)))
                };
                let out_target = if policies.use_port_dirs {
                    node_id!(esc parent_id_str, port!(id!(esc port_id_str), "e"))
                } else {
                    node_id!(esc parent_id_str, port!(id!(esc port_id_str)))
                };
                state.register_symbol_edge_targets(&self.symbols, in_target, out_target);
            }

            for kid in self.children.values_mut() {
                let kid_id_str = kid.derive_id(node_set, state);
                let port_id_str = make_safe_port_id(&kid_id_str);
                let in_target = if policies.use_port_dirs {
                    node_id!(esc parent_id_str, port!(id!(esc port_id_str), "w"))
                } else {
                    node_id!(esc parent_id_str, port!(id!(esc port_id_str)))
                };
                let out_target = if policies.use_port_dirs {
                    node_id!(esc parent_id_str, port!(id!(esc port_id_str), "e"))
                } else {
                    node_id!(esc parent_id_str, port!(id!(esc port_id_str)))
                };
                state.register_symbol_edge_targets(&kid.symbols, in_target, out_target);
                kid.action = Some(HierarchicalLayoutAction::Record(kid_id_str));
            }
            self.action = Some(HierarchicalLayoutAction::Table(parent_id_str));
        } else if self.children.len() > 0 {
            // If there are kids, we want to be a cluster after all.
            be_cluster = true;
        } else {
            let node_id_str = self.derive_id(node_set, state);
            let id = node_id!(esc escape_quotes(&node_id_str));
            state.register_symbol_edge_targets(&self.symbols, id.clone(), id);
            self.action = Some(HierarchicalLayoutAction::Node(node_id_str));
        }

        if be_cluster {
            let placeholder_id_str = state.issue_new_synthetic_id();
            let cluster_id = self.derive_id(node_set, state);
            let placeholder_id = node_id!(esc placeholder_id_str);

            self.action = Some(HierarchicalLayoutAction::Cluster(
                cluster_id,
                placeholder_id_str,
            ));

            // XXX The use of a placeholder is from the prototype; need to
            // understand and document the approach more.
            state.register_symbol_edge_targets(
                &self.symbols,
                placeholder_id.clone(),
                placeholder_id,
            );

            for kid in self.children.values_mut() {
                kid.compile(policies, depth + 1, 0, be_class, node_set, state);
            }
        }
    }

    /// Normalize situations for nodes which lack a symbol id so that we create
    /// a synthetic id which can be used as a node id and return the string
    /// representation of the symbol, if any.
    ///
    /// Returns the String to use as the node id in the graphviz graph.  For
    /// nodes that have backing symbols, this will be all of the symbols joined
    /// with commas because data-symbols will do the right thing
    /// post-transformation.
    pub fn derive_id(
        &self,
        node_set: &SymbolGraphNodeSet,
        state: &mut HierarchicalRenderState,
    ) -> String {
        if self.symbols.len() >= 1 {
            self.symbols
                .iter()
                .map(|sym_id| node_set.get(sym_id).symbol.clone())
                .join(",")
        } else {
            state.issue_new_synthetic_id()
        }
    }

    pub fn render(
        &self,
        policies: &HierarchyPolicies,
        node_set: &SymbolGraphNodeSet,
        edge_set: &SymbolGraphEdgeSet,
        state: &mut HierarchicalRenderState,
    ) -> Vec<Stmt> {
        let action = match &self.action {
            Some(a) => a,
            None => {
                return vec![];
            }
        };

        let mut result = vec![];
        match action {
            // Collapse ends up looking the same as flatten.
            HierarchicalLayoutAction::Flatten => {
                // Provide the default settings here in the root too.

                // concentrate has the advantage of bundling edges together that are going to the
                // same location (although not exhaustively), but has the downside that it can
                // result in wackier looking edges as we end up with more spline segments and each
                // segment can do its own wiggly thing, so concentrated edges can seem to wiggle for
                // no good reason.
                //
                // XXX turned this off because it messes up edge hover highlighting.
                //result.push(stmt!(attr!("concentrate", "true")));

                // Decidedly not better, but interesting!
                // (As discussed extensively on discourse, the LR is a rotation of the TD that
                // does not do well with records.)
                //result.push(stmt!(attr!("rankdir", "lr")));
                //result.push(stmt!(attr!("layout", "osage")));

                // The graph node affects both the root graph and subgraphs/clusters, which is why
                // we don't just set the attributes here at the root level on the root digraph.
                result.push(stmt!(node!("graph";
                    attr!("fontname", esc "Courier New"),
                    attr!("fontsize", "12"),
                    // Didn't notice a difference yet, but may be useful.
                    //attr!("ratio", "compress"),
                    // Currently newrank just ends up making clusters taller with weird whitespace
                    // because it's leaving space for nodes contained in sibling clusters (because
                    // the point of newrank is to allow nodes outside the cluster to impact rank
                    // calculations).  It only makes sense to turn this on when we have a benefit
                    // like being able to use rank=same to line nodes in the cluster up with nodes
                    // outside the cluster or in another cluster.
                    //attr!("newrank", "true"),
                    attr!("compound", "true")
                )));
                result.push(stmt!(node!("node"; attr!("shape","box"), attr!("fontname", esc "Courier New"), attr!("fontsize", "10"))));
                for kid in self.children.values() {
                    result.extend(kid.render(policies, node_set, edge_set, state));
                }
            }
            HierarchicalLayoutAction::Collapse => {
                for kid in self.children.values() {
                    result.extend(kid.render(policies, node_set, edge_set, state));
                }
            }
            HierarchicalLayoutAction::Cluster(cluster_id, placeholder_id) => {
                let mut sg = subgraph!(esc cluster_id; attr!("cluster", "true"), attr!("label", esc escape_quotes(&self.display_name)));
                sg.stmts.push(stmt!(
                    node!(esc placeholder_id; attr!("shape", "point"), attr!("style", "invis"))
                ));

                // Build a rank=same group for all nodes in the cluster that only have edges into
                // them external to the cluster.  We do this in order to make the graph more
                // vertical because this will help nodes with edges into our cluster have a rank
                // that doesn't overlap with our cluster.  At least for nodes that are only
                // public-facing; if the node has internal edges into it, this heuristic won't
                // apply.
                //
                // We achieve this by walking the list of edges inside this cluster and making note
                // of all of the "to" identifiers in a set.  As we walk our list of children, any
                // child without any of its symbols being in that to set gets added to this top
                // group.
                //
                // A limitation of this approach is that we don't distinguish between a node that
                // has external edges to it and a node that has no edges to it at all.  But we only
                // expect that to happen in overview situations, and those probably want additional
                // specializations that we can deal with then.
                //
                // XXX In some cases this may be leading to weird artifacts where nodes that have
                // an edge to nodes that are being floated upward end up with the same rank.  This
                // may be a result of not having an up-to-date graphviz on the live servers,
                // however.  (The original motivation for updating my docker copy was crashes, and
                // I believe I have seen that on the live server, but they have been less common
                // than I was worried about.)
                let mut internal_targets: HashSet<SymbolGraphNodeId> = HashSet::new();
                for (_, to_id, _) in &self.edges {
                    internal_targets.insert(to_id.clone());
                }
                let mut top_nodes = subgraph!(; attr!("rank", "same"), attr!("cluster", "false"), attr!("label", esc ""));

                for kid in self.children.values() {
                    sg.stmts
                        .extend(kid.render(policies, node_set, edge_set, state));

                    if !kid.symbols.iter().any(|id| internal_targets.contains(&id)) {
                        match &kid.action {
                            Some(HierarchicalLayoutAction::Node(kid_id)) => {
                                top_nodes.stmts.push(stmt!(node!(esc kid_id)));
                            }
                            _ => {}
                        }
                    }
                }

                // Only push the rank=same group if it will cover at least 2 nodes; we need to
                // account for the attributes we added above too.
                if top_nodes.stmts.len() >= (3 + 2) {
                    sg.stmts.push(stmt!(top_nodes));
                }

                result.push(stmt!(sg));
            }
            HierarchicalLayoutAction::Table(node_id) => {
                let mut table = LabelTable {
                    rows: vec![],
                    columns_needed: 0,
                };

                table.rows.push(LabelRow {
                    cells: vec![LabelCell {
                        id: Some(state.id_for_nodes(&self.symbols)),
                        bg_color: None,
                        contents: format!("<b>{}</b>", escape_html(&self.display_name)),
                        badges: node_set.get_merged_badges_for_symbols(&self.symbols),
                        symbol: node_id.clone(),
                        port: make_safe_port_id(node_id),
                        indent_level: 0,
                    }],
                });

                let grouped_kids = if self.children.len() >= policies.group_fields_at as usize {
                    let mut grouped = self
                        .children
                        .values()
                        .into_group_map_by(|kid| {
                            if let Some(kid_sym_id) = kid.symbols.first() {
                                let kid_info = node_set.get(kid_sym_id);
                                kid_info
                                    .effective_subsystem
                                    .or_else(|| kid_info.get_subsystem())
                            } else {
                                None
                            }
                        })
                        .into_iter()
                        .collect_vec();
                    grouped.sort_by_cached_key(|(g, _)| g.clone());
                    grouped
                } else {
                    self.children
                        .values()
                        .into_group_map_by(|_| -> Option<Ustr> { None })
                        .into_iter()
                        .collect_vec()
                };

                // Start from our own depth, but it's quite likely that the kids may actually have
                // lower depths.
                let mut min_depth = node_set.get_min_depth_for_symbols(&self.symbols);
                let mut kid_edges = vec![];
                // Only show group headers if there's more than 1 group!
                let use_groups = grouped_kids.len() > 1;
                for (group, ordered_kids) in grouped_kids {
                    if let Some(group_name) = group {
                        if use_groups {
                            table.rows.push(LabelRow {
                                cells: vec![LabelCell {
                                    id: None,
                                    bg_color: Some("#eee"),
                                    contents: format!("<I>{}</I>", escape_html(&group_name)),
                                    badges: vec![],
                                    port: "".to_string(),
                                    symbol: "".to_string(),
                                    indent_level: 0,
                                }],
                            });
                        }
                    }
                    for kid in ordered_kids {
                        if let Some(HierarchicalLayoutAction::Record(kid_id)) = &kid.action {
                            let kid_depth = node_set.get_min_depth_for_symbols(&kid.symbols);
                            if kid_depth < min_depth {
                                min_depth = kid_depth;
                            }
                            let kid_port_name = make_safe_port_id(kid_id);
                            table.rows.push(LabelRow {
                                cells: vec![LabelCell {
                                    id: Some(state.id_for_nodes(&kid.symbols)),
                                    bg_color: None,
                                    contents: escape_html(&kid.display_name),
                                    badges: node_set.get_merged_badges_for_symbols(&kid.symbols),
                                    symbol: kid_id.clone(),
                                    port: kid_port_name,
                                    indent_level: 1,
                                }],
                            });

                            for (from_id, to_id, _edge_id) in &kid.edges {
                                if let Some((from_node, to_node)) =
                                    state.lookup_edge(from_id, to_id)
                                {
                                    kid_edges.push(stmt!(edge!(from_node => to_node)));
                                }
                            }
                        }
                    }
                }

                table.compile();
                let table_html = table.render();

                // We don't put a custom "id" on this because we only want the rows to have our
                // identifiers.
                let node = node!(esc node_id;
                          attr!("shape", "none"),
                          attr!("label", html table_html),
                          attr!("class", esc format!("diagram-depth-{}", min_depth)));
                result.push(stmt!(node));

                result.extend(kid_edges);
            }
            HierarchicalLayoutAction::Record(_) => {
                // Records will be handled by their parent table.
            }
            HierarchicalLayoutAction::Node(node_id) => {
                let badges = node_set.get_merged_badges_for_symbols(&self.symbols);
                let maybe_labels = if !badges.is_empty() {
                    format!(
                        " {}",
                        badges
                            .into_iter()
                            .map(|b| format!("<U>{}</U>", escape_html(&b.label)))
                            .collect_vec()
                            .join("")
                    )
                } else {
                    "".to_string()
                };
                result.push(stmt!(
                    node!(esc node_id;
                          attr!("id", state.id_for_nodes(&self.symbols)),
                          attr!("label", html format!("<{}{}>", escape_html(&self.display_name), maybe_labels)),
                          attr!("class", esc format!("diagram-depth-{}", node_set.get_min_depth_for_symbols(&self.symbols))))
                ));
            }
        }

        let mut emitted_edges = HashSet::new();
        let mut ctx = PrinterContext::default();
        for (from_id, to_id, edge_id) in &self.edges {
            if let Some((from_node, to_node)) = state.lookup_edge(from_id, to_id) {
                // We de-duplicate on what the string rep ends up looking like because the NodeId type
                // and its sub-types don't really want to get put in a set.
                //
                // The need to de-duplicate currently arises from multiple symbols being associated
                // with a single node/pretty identifier due to multiple platforms.  Which is to say
                // that we have already de-duplicated edges on a symbol basis upstream, but only
                // now are we de-duplicating in (pretty) node space.
                //
                // TODO: Do a better job of handling unification of EdgeDetails for these de-duped
                // edges.  Right now we just use the details of the first edge we end up emitting.
                // As long as the de-duplication is just dealing with platform variations, the net
                // result should be the same, but it would be preferable to have explicitly had an
                // edge unification step when doing the hierarchical processing.
                let edge_info = edge_set.get(edge_id);

                let (style, loc, arrow) = match edge_info.kind {
                    EdgeKind::Default => ("solid", "arrowhead", "normal"),
                    EdgeKind::Inheritance => ("solid", "arrowtail", "onormal"),
                    EdgeKind::Implementation => ("dashed", "arrowhead", "onormal"),
                    EdgeKind::Composition => ("solid", "arrowtail", "diamond"),
                    EdgeKind::Aggregation => ("solid", "arrowtail", "odiamond"),
                    EdgeKind::IPC => ("dotted", "arrowhead", "vee"),
                    EdgeKind::CrossLanguage => ("solid", "arrowhead", "lnormal"),
                };

                let mut maybe_edge =
                    edge!(from_node => to_node; attr!("style", style), attr!(loc, arrow));
                // arrowtail only works if dir=back or dir=both
                // XXX eh, make this more efficient maybe.
                if loc == "arrowtail" {
                    maybe_edge.attributes.push(attr!("dir", "back"));
                }
                if emitted_edges.insert(maybe_edge.print(&mut ctx)) {
                    // As per the TODO above, here is us now translating the edge details.  In
                    // theory we could do this above if we expect the data to always be the same,
                    // but it's better to err on the side of not creating multiple edges if we're
                    // wrong about that.
                    maybe_edge
                        .attributes
                        .push(attr!("id", state.id_for_edge(edge_id)));

                    state.set_edge_metadata(from_id, to_id, edge_id, &edge_info.data);

                    result.push(stmt!(maybe_edge));
                }
            }
        }

        result
    }
}

/// Rendering pass shared state.  This object is passed through the recursive
/// call trees for the hierarchical objects which can't have a durable reference
/// to global state because then the object hierarchy wouldn't be a clean tree.
///
/// Of particular importance is the `sym_to_edges` mapping which is used to
/// store a mapping from `SymbolGraphNodeId` (one per underlying symbol) to
/// the graphviz identifiers that should use when drawing an edge into the node
/// and out of the node.  This is necessary for several reasons:
/// - We map multiple symbols onto a single visual node (usually based on
///   pretty identifier), which means a mapping is necessary somewhere.
/// - We use record-style labels and currently have edges enter on the left and
///   exit on the right.
///
/// ### SVG Metadata
///
/// The object also accumulates state that is subsequently used to fix-up the
/// graphviz SVG output to include extra metadata.  Long term, it would likely
/// be preferable to contribute fixes to graphviz upstream to let specific
/// attributes be propagated through SVG layout; from the discourse server
/// discussion I believe there has been interest in generally supporting the
/// propagation of user-defined attributes more generally.
///
/// Note that this is a new second way that we are propagating data to the UI.
/// We currently perform regexp transforms on title and xlink nodes to turn
/// those into "data-symbols" attributes.  This is desirable for legibility of
/// the resulting SVG doc, as these are data payloads.  It would be nice not to
/// need to use regexps for this, but it doesn't look like lol_html
/// intentionally supports SVG (there's very trivial test coverage where it's
/// not really clear what's intended to be supported), and it doesn't seem worth
/// the work to more properly support XML streaming.
///
/// This info is generally more like metadata, and so a JSON sidecar keyed by
/// newly generated node/edge identifiers is workable.  (The dot SVG output
/// layer right now is making identifiers we can't predict, but if we manually
/// assign identifiers, it will use them, and so we use that.)
pub struct HierarchicalRenderState {
    next_synthetic_id: u32,
    /// Maps the SymbolGraphNodeId to (in-edge id, out-edge-id)
    sym_to_edges: HashMap<u32, (NodeId, NodeId)>,
    /// Maps node identifiers to data for hover purposes; value tuple is:
    pub svg_node_extra: BTreeMap<String, SvgNodeExtra>,
    pub svg_edge_extra: BTreeMap<String, SvgEdgeExtra>,
}

#[derive(Default, Serialize)]
pub struct SvgNodeExtra {
    /// List of edge identifiers for edges that should be highlighted with the
    /// in-edge color on hover.
    pub in_edges: Vec<String>,
    /// List of pairs of:
    /// - node identifier for input/source nodes with higlighting
    /// - List of CSS classes to apply to the nodes on hover if specific colors
    ///   should be used.  If the list is empty, a default hover style will be
    ///   used.
    pub in_nodes: Vec<(String, Vec<String>)>,
    /// List of edge identifiers for edges that should be highlighted with the
    /// out-edge color on hover.
    pub out_edges: Vec<String>,
    /// See `in_nodes`.
    pub out_nodes: Vec<(String, Vec<String>)>,
}

#[derive(Serialize)]
pub struct SvgEdgeExtra {
    pub jump: String,
}

impl HierarchicalRenderState {
    pub fn new() -> Self {
        Self {
            next_synthetic_id: 0,
            sym_to_edges: HashMap::default(),
            svg_node_extra: BTreeMap::default(),
            svg_edge_extra: BTreeMap::default(),
        }
    }

    pub fn issue_new_synthetic_id(&mut self) -> String {
        let use_id = self.next_synthetic_id;
        self.next_synthetic_id += 1;
        format!("SYN_{}", use_id)
    }

    pub fn register_symbol_edge_targets(
        &mut self,
        sym_ids: &Vec<SymbolGraphNodeId>,
        in_target: NodeId,
        out_target: NodeId,
    ) {
        for sym_id in sym_ids {
            self.sym_to_edges
                .insert(sym_id.0, (in_target.clone(), out_target.clone()));
        }
    }

    /// Given an edge defined by symbol node id's, look-up the correct graphviz
    /// out-edge id and graphviz in-edge id, respectively.
    pub fn lookup_edge(
        &self,
        from_id: &SymbolGraphNodeId,
        to_id: &SymbolGraphNodeId,
    ) -> Option<(NodeId, NodeId)> {
        let from_id = match self.sym_to_edges.get(&from_id.0) {
            Some((_, out_target)) => out_target.clone(),
            _ => {
                return None;
            }
        };

        match self.sym_to_edges.get(&to_id.0) {
            Some((in_target, _)) => Some((from_id, in_target.clone())),
            _ => None,
        }
    }

    pub fn id_for_node(&self, node_id: &SymbolGraphNodeId) -> String {
        format!("Gidn{}", node_id.0)
    }

    /// Return a node identifier for the set of nodes.  We just pick the first
    /// one.
    pub fn id_for_nodes(&self, nodes: &Vec<SymbolGraphNodeId>) -> String {
        // XXX there are cases where nodes is empty and the code assumed it
        // would not be.
        if nodes.is_empty() {
            "".to_string()
        } else {
            format!("Gidn{}", nodes[0].0)
        }
    }

    pub fn id_for_edge(&self, edge_id: &SymbolGraphEdgeId) -> String {
        format!("Gide{}", edge_id.0)
    }

    pub fn set_edge_metadata(
        &mut self,
        from_id: &SymbolGraphNodeId,
        to_id: &SymbolGraphNodeId,
        edge_id: &SymbolGraphEdgeId,
        edge_data: &Vec<EdgeDetail>,
    ) {
        let from_eid = self.id_for_node(from_id);
        let to_eid = self.id_for_node(to_id);
        let edge_eid = self.id_for_edge(edge_id);

        let mut hover_classes = vec![];
        let mut jump = "".to_string();
        for detail in edge_data {
            match detail {
                EdgeDetail::Jump(jump_detail) => {
                    jump = jump_detail.clone();
                }
                EdgeDetail::HoverClass(class_detail) => {
                    hover_classes.push(class_detail.clone());
                }
            }
        }

        let from_extra = self
            .svg_node_extra
            .entry(from_eid.clone())
            .or_insert_with(|| SvgNodeExtra::default());
        from_extra.out_edges.push(edge_eid.clone());
        from_extra.out_nodes.push((to_eid.clone(), hover_classes));

        if let Some(extra) = self.svg_edge_extra.get_mut(&edge_eid) {
            extra.jump = jump;
        } else {
            self.svg_edge_extra
                .insert(edge_eid.clone(), SvgEdgeExtra { jump });
        }

        let to_extra = self
            .svg_node_extra
            .entry(to_eid)
            .or_insert_with(|| SvgNodeExtra::default());
        to_extra.in_edges.push(edge_eid);
        to_extra.in_nodes.push((from_eid, vec![]));
    }
}

/// Wrapped u32 identifier for DerivedSymbolInfo nodes in a SymbolGraphNodeSet
/// for type safety.  The values correspond to the index of the node in the
/// `symbol_crossref_infos` vec in `SymbolGraphNodeSet`.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct SymbolGraphNodeId(u32);

/// Wrapped u32 identifier for EdgeInfo objects in a SymbolGraphNodeSet for type
/// safety.  The values correspond to the index of the edge in the
/// `edge_infos` vec in `SymbolGraphNodeSet`.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct SymbolGraphEdgeId(u32);

pub struct SymbolGraphNodeSet {
    pub symbol_crossref_infos: Vec<DerivedSymbolInfo>,
    pub symbol_to_index_map: UstrMap<u32>,
}

pub struct SymbolGraphEdgeSet {
    pub edge_infos: Vec<EdgeInfo>,
    edge_lookup: HashMap<(u32, u32), u32>,
}

#[derive(Clone, PartialEq)]
pub enum EdgeDetail {
    /// Provide a source code jump.
    Jump(String),
    /// Hover class for the target node when the source node is hovered.  This
    /// could also be applied to the edge if desired.
    HoverClass(String),
}

/// Information about the edge between two nodes that is something we either
/// want for debugging, layout, or to explicitly present to the user.
pub struct EdgeInfo {
    pub from_id: SymbolGraphNodeId,
    pub to_id: SymbolGraphNodeId,
    pub kind: EdgeKind,
    pub data: Vec<EdgeDetail>,
}

fn make_data_invariant_err() -> ServerError {
    ServerError::StickyProblem(ErrorDetails {
        layer: ErrorLayer::RuntimeInvariantViolation,
        message: "SymbolGraphNodeSet desynchronized".to_string(),
    })
}

impl SymbolGraphNodeSet {
    pub fn new() -> Self {
        Self {
            symbol_crossref_infos: vec![],
            symbol_to_index_map: UstrMap::default(),
        }
    }

    pub fn get(&self, node_id: &SymbolGraphNodeId) -> &DerivedSymbolInfo {
        // It's very much an invariant that only we mint SymbolGraphNodeId's, so
        // the entry should always exist.
        self.symbol_crossref_infos.get(node_id.0 as usize).unwrap()
    }

    pub fn get_mut(&mut self, node_id: &SymbolGraphNodeId) -> &mut DerivedSymbolInfo {
        // It's very much an invariant that only we mint SymbolGraphNodeId's, so
        // the entry should always exist.
        self.symbol_crossref_infos
            .get_mut(node_id.0 as usize)
            .unwrap()
    }

    pub fn get_merged_badges_for_symbols(
        &self,
        nodes: &Vec<SymbolGraphNodeId>,
    ) -> Vec<SymbolBadge> {
        let mut badges: BTreeSet<SymbolBadge> = BTreeSet::default();
        for sym_id in nodes {
            let sym_info = self.get(sym_id);
            for badge in &sym_info.badges {
                badges.insert(badge.clone());
            }
        }
        badges.into_iter().collect()
    }

    pub fn get_min_depth_for_symbols(&self, nodes: &Vec<SymbolGraphNodeId>) -> u32 {
        // Currently 13 is the highest depth we can report.
        let mut min_depth: u32 = 13;
        for sym_id in nodes {
            let sym_info = self.get(sym_id);
            if sym_info.depth < min_depth {
                min_depth = sym_info.depth;
            }
        }
        min_depth
    }

    pub fn propagate_paths(
        &self,
        nsgraph: &mut NamedSymbolGraph,
        source_nodes: &Vec<SymbolGraphNodeId>,
        target_nodes: &Vec<SymbolGraphNodeId>,
        edge_set: &SymbolGraphEdgeSet,
        node_soft_limit: u32,
        path_length_limit: u32,
        new_graph: &mut NamedSymbolGraph,
        new_symbol_set: &mut SymbolGraphNodeSet,
        new_edge_set: &mut SymbolGraphEdgeSet,
    ) {
        let super_source_id = self.symbol_crossref_infos.len() as u32;
        let super_target_id = super_source_id + 1;

        let super_source_ix = nsgraph.graph.add_node(super_source_id);
        let super_target_ix = nsgraph.graph.add_node(super_target_id);

        let synth_source_edge_id = edge_set.edge_infos.len() as u32;
        let synth_target_edge_id = synth_source_edge_id + 1;

        let mut suppression = HashSet::new();

        // Add edges from the synthetic source supernode to all source nodes
        for source_id in source_nodes {
            let source_ix = nsgraph.ensure_node(source_id.clone());
            nsgraph.graph.add_edge(
                super_source_ix,
                source_ix,
                SymbolGraphEdgeId(synth_source_edge_id),
            );
        }

        // Add edges from all target nodes to the synthetic target supernode.
        for target_id in target_nodes {
            let target_ix = nsgraph.ensure_node(target_id.clone());
            nsgraph.graph.add_edge(
                target_ix,
                super_target_ix,
                SymbolGraphEdgeId(synth_target_edge_id),
            );
        }

        trace!(num_nodes=%super_target_id, num_edges=%synth_target_edge_id, "created supernodes, running petgraph all_simple_paths algorithm");

        // Now we get the paths...
        let paths = all_simple_paths::<Vec<_>, _>(
            &nsgraph.graph,
            super_source_ix,
            super_target_ix,
            0,
            Some(path_length_limit as usize),
        );

        for path in paths {
            for (src, tgt) in path
                .into_iter() // skip the source supernode
                .dropping(1)
                // skip the target supernode
                .dropping_back(1)
                .tuple_windows()
            {
                let source_ix = src.index() as u32;
                let target_ix = tgt.index() as u32;

                if suppression.insert((source_ix, target_ix)) {
                    let source_id =
                        SymbolGraphNodeId(*nsgraph.node_ix_to_id.get(&source_ix).unwrap());
                    let target_id =
                        SymbolGraphNodeId(*nsgraph.node_ix_to_id.get(&target_ix).unwrap());
                    let edge_ix = nsgraph.graph.find_edge(src, tgt).unwrap();
                    let edge_id = nsgraph.graph[edge_ix].clone();
                    self.propagate_edge(
                        edge_set,
                        &source_id,
                        &target_id,
                        &edge_id,
                        new_graph,
                        new_symbol_set,
                        new_edge_set,
                    );
                }
            }

            if new_symbol_set.symbol_crossref_infos.len() as u32 >= node_soft_limit {
                return;
            }
        }
    }

    /// Given a pair of symbols in the current set, ensure that they exist in
    /// the new node set and propagate the edge and its info in the new graph as well.
    pub fn propagate_edge(
        &self,
        edge_set: &SymbolGraphEdgeSet,
        source: &SymbolGraphNodeId,
        target: &SymbolGraphNodeId,
        edge_id: &SymbolGraphEdgeId,
        new_graph: &mut NamedSymbolGraph,
        new_symbol_set: &mut SymbolGraphNodeSet,
        new_edge_set: &mut SymbolGraphEdgeSet,
    ) {
        let new_source_node = self.propagate_sym(source, new_symbol_set);
        let new_target_node = self.propagate_sym(target, new_symbol_set);
        let old_edge_info = &edge_set.edge_infos[edge_id.0 as usize];
        let edge_index = new_edge_set.edge_infos.len();
        new_edge_set.edge_infos.push(EdgeInfo {
            from_id: new_source_node.clone(),
            to_id: new_target_node.clone(),
            kind: old_edge_info.kind.clone(),
            data: old_edge_info.data.clone(),
        });
        let new_edge_id = SymbolGraphEdgeId(edge_index as u32);
        new_graph.ensure_edge(new_source_node, new_target_node, new_edge_id);
    }

    fn propagate_sym(
        &self,
        node_id: &SymbolGraphNodeId,
        new_symbol_set: &mut SymbolGraphNodeSet,
    ) -> SymbolGraphNodeId {
        let info = self.get(node_id);
        match new_symbol_set.symbol_to_index_map.get(&info.symbol) {
            Some(index) => SymbolGraphNodeId(*index as u32),
            None => new_symbol_set.add_symbol(info.clone()).0,
        }
    }

    /// Look-up a symbol returning its id (for graph purposes) and its
    /// DerivedSymbolInfo (for data inspection).
    pub fn lookup_symbol(&self, symbol: &Ustr) -> Option<(SymbolGraphNodeId, &DerivedSymbolInfo)> {
        if let Some(index) = self.symbol_to_index_map.get(symbol) {
            let sym_info = self.symbol_crossref_infos.get(*index as usize);
            sym_info.map(|info| (SymbolGraphNodeId(*index), info))
        } else {
            None
        }
    }

    /// Add a symbol and return the unwrapped data that lookup_symbol would have provided.
    pub fn add_symbol(
        &mut self,
        mut sym_info: DerivedSymbolInfo,
    ) -> (SymbolGraphNodeId, &mut DerivedSymbolInfo) {
        // Propagate any labels from the symbol's structured information into
        // badges set.
        // XXX this is a somewhat weird conflation of presentation logic with
        // more abstract logic.  Also, this on its own isn't entirely sufficient
        // because of how "fields" on a class are a special case.  (In
        // particular, as of writing this, we store information on the class's
        // structured "fields" that we don't store on the structured info for
        // each individual field, and that's weird.  This may or may not change,
        // since there is also some sense in field (meta)data being most
        // useful in the context of the class.)
        if let Some(Value::Array(labels_json)) = sym_info.crossref_info.pointer("/meta/labels") {
            for label in labels_json {
                if let Value::String(label) = label {
                    if let Some((pri, shorter_label)) = label_to_badge_info(&label) {
                        sym_info.badges.push(SymbolBadge {
                            pri,
                            label: ustr(shorter_label),
                            source_jump: None,
                        });
                    }
                }
            }
        }
        // Insert the symbol and issue a node id.
        let index = self.symbol_crossref_infos.len();
        let symbol = sym_info.symbol.clone();
        self.symbol_crossref_infos.push(sym_info);
        self.symbol_to_index_map.insert(symbol, index as u32);
        (
            SymbolGraphNodeId(index as u32),
            self.symbol_crossref_infos.get_mut(index).unwrap(),
        )
    }

    /// Check if a symbol is already known and return it if so, otherwise
    /// perform a crossref_lookup and add the symbol.  The caller should provide
    /// the depth that should be associated with the symbol if we need to
    /// perform the lookup; no change will be made to the existing depth if the
    /// symbol is already known.
    pub async fn ensure_symbol<'a>(
        &'a mut self,
        sym: &'a Ustr,
        server: &'a Box<dyn AbstractServer + Send + Sync>,
        depth: u32,
    ) -> Result<(SymbolGraphNodeId, &mut DerivedSymbolInfo)> {
        if let Some(index) = self.symbol_to_index_map.get(sym) {
            let sym_info = self
                .symbol_crossref_infos
                .get_mut(*index as usize)
                .ok_or_else(make_data_invariant_err)?;
            return Ok((SymbolGraphNodeId(*index), sym_info));
        }

        let info = server.crossref_lookup(&sym, false).await?;
        Ok(self.add_symbol(DerivedSymbolInfo::new(sym.clone(), info, depth)))
    }

    /// Destructively return a sorted Object mapping from symbol identifiers to
    /// their jumpref info.  We sort the symbols for stability for testing
    /// purposes and for human readability reasons.  The destruction is that
    /// the DerivedSymbolInfo's have their `crossref_info` serde_json::Value
    /// instances take()n.
    ///
    /// This method is currently destructive because the
    /// convert_crossref_value_to_sym_info_rep currently is destructive and
    /// because it seems like nothing else currently needs that info.  But it
    /// should be fine to make this optionally non-destructive.
    ///
    /// Okay, now there's a nondestructive version below this that's less
    /// efficient.
    pub fn symbols_meta_to_jumpref_json_destructive(&mut self) -> Value {
        let mut jumprefs = BTreeMap::new();
        for sym_info in self.symbol_crossref_infos.iter_mut() {
            let info = sym_info.crossref_info.take();
            jumprefs.insert(
                sym_info.symbol.clone(),
                convert_crossref_value_to_sym_info_rep(info, &sym_info.symbol, None),
            );
        }

        json!(jumprefs)
    }

    /// Nondestructive, less efficient version of `symbols_meta_to_jumpref_json_destructive`.
    pub fn symbols_meta_to_jumpref_json_nomut(&self) -> Value {
        let mut jumprefs = BTreeMap::new();
        for sym_info in self.symbol_crossref_infos.iter() {
            // XXX This is inefficient!
            let info = sym_info.crossref_info.clone();
            jumprefs.insert(
                sym_info.symbol.clone(),
                convert_crossref_value_to_sym_info_rep(info, &sym_info.symbol, None),
            );
        }

        json!(jumprefs)
    }
}

impl SymbolGraphEdgeSet {
    pub fn new() -> Self {
        Self {
            edge_infos: vec![],
            edge_lookup: HashMap::default(),
        }
    }

    pub fn get(&self, edge_id: &SymbolGraphEdgeId) -> &EdgeInfo {
        &self.edge_infos[edge_id.0 as usize]
    }

    pub fn get_mut(&mut self, edge_id: &SymbolGraphEdgeId) -> &mut EdgeInfo {
        &mut self.edge_infos[edge_id.0 as usize]
    }

    /// Merge the provided edge metadata for any existing edge between the
    /// provided symbol, creating the edge metadata if it does not already
    /// exist.  Then calls ensure_edge on the underlying graph using the
    /// underlying SymbolGraphEdgeId.
    ///
    /// Currently we don't return the SymbolGraphEdgeId but we could if callers
    /// needed it.
    pub fn ensure_edge_in_graph(
        &mut self,
        source: SymbolGraphNodeId,
        target: SymbolGraphNodeId,
        kind: EdgeKind,
        data: Vec<EdgeDetail>,
        graph: &mut NamedSymbolGraph,
    ) {
        let edge_id = if let Some(idx) = self.edge_lookup.get(&(source.0, target.0)) {
            let info = self.edge_infos.get_mut(*idx as usize).unwrap();
            for detail in data {
                if !info.data.iter().contains(&detail) {
                    info.data.push(detail);
                }
            }
            SymbolGraphEdgeId(*idx)
        } else {
            let index = self.edge_infos.len();
            self.edge_infos.push(EdgeInfo {
                from_id: source.clone(),
                to_id: target.clone(),
                kind,
                data,
            });
            SymbolGraphEdgeId(index as u32)
        };
        graph.ensure_edge(source, target, edge_id);
    }
}
