use std::collections::{HashMap, BTreeMap, BTreeSet};

use petgraph::{
    graph::{DefaultIx, NodeIndex},
    Directed, Graph,
};
use serde_json::{Value, json};

use crate::abstract_server::{Result, AbstractServer, ServerError, ErrorDetails, ErrorLayer};

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

/// A symbol and its cross-reference information plus caching helpers.
pub struct DerivedSymbolInfo {
    pub symbol: String,
    pub crossref_info: Value,
}

pub fn semantic_kind_is_callable(semantic_kind: &str) -> bool {
  match semantic_kind {
    "function" => true,
    "method" => true,
    _ => false,
  }
}

impl DerivedSymbolInfo {
  pub fn is_callable(&self) -> bool {
    let is_callable = match self.crossref_info.pointer("/meta/kind") {
        Some(Value::String(sem_kind)) => semantic_kind_is_callable(sem_kind),
        _ => false,
    };
    return is_callable;
  }
}

impl DerivedSymbolInfo {
    pub fn new(symbol: &str, crossref_info: Value) -> Self {
        DerivedSymbolInfo {
            symbol: symbol.to_string(),
            crossref_info,
        }
    }
}

/// A collection of one or more graphs that share a common underlying set of
/// per-symbol information across the graphs.
pub struct SymbolGraphCollection {
    pub node_set: SymbolGraphNodeSet,
    pub graphs: Vec<NamedSymbolGraph>,
}

impl SymbolGraphCollection {
    /// Return a sorted Object mapping from symbol identifiers to their meta, if
    /// available.  We sort the symbols for stability for testing purposes and
    /// for human readability reasons.
    pub fn symbols_meta_to_json(&self) -> Value {
        let mut metas = BTreeMap::new();
        for sym_info in self.node_set.symbol_crossref_infos.iter() {
            if let Some(meta) = sym_info.crossref_info.get("meta") {
                metas.insert(sym_info.symbol.clone(), meta.clone());
            }
        }

        json!(metas)
    }

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
        let mut nodes = BTreeSet::new();
        let mut edges = BTreeMap::new();
        for (source_id, target_id) in graph.list_edges() {
            let source_info = self.node_set.get(&source_id);
            nodes.insert(source_info.symbol.clone());
            let source_sym = source_info.symbol.clone();

            let target_info = self.node_set.get(&target_id);
            nodes.insert(target_info.symbol.clone());
            let target_sym = target_info.symbol.clone();

            edges.insert(
                format!("{}-{}", source_sym, target_sym),
                json!({ "from": source_sym, "to": target_sym }));
        }

        json!({
            "nodes": nodes.into_iter().collect::<Vec<String>>(),
            "edges": edges.into_values().collect::<Value>(),
        })
    }

    pub fn to_json(&self) -> Value {
        let mut graphs = vec![];
        for i in 0..self.graphs.len() {
            graphs.push(self.graph_to_json(i));
        }

        json!({
            "symbol_metas": self.symbols_meta_to_json(),
            "graphs": graphs,
        })
    }
}

/// A graph whose nodes are symbols from a `SymbolGraphNodeSet`.
pub struct NamedSymbolGraph {
    pub name: String,
    graph: Graph<u32, (), Directed>,
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
            graph: Graph::new(),
            node_id_to_ix: HashMap::new(),
            node_ix_to_id: HashMap::new(),
        }
    }

    pub fn containts_node(&self, sym_id: SymbolGraphNodeId) -> bool {
      self.node_id_to_ix.contains_key(&sym_id.0)
    }

    fn ensure_node(&mut self, sym_id: SymbolGraphNodeId) -> NodeIndex {
        if let Some(idx) = self.node_id_to_ix.get(&sym_id.0) {
            return NodeIndex::new(*idx as usize);
        }

        let idx = self.graph.add_node(sym_id.0).index() as u32;
        self.node_id_to_ix.insert(sym_id.0, idx);
        self.node_ix_to_id.insert(idx, sym_id.0);

        NodeIndex::new(idx as usize)
    }

    pub fn add_edge(&mut self, source: SymbolGraphNodeId, target: SymbolGraphNodeId) {
        let source_ix = self.ensure_node(source);
        let target_ix = self.ensure_node(target);
        self.graph.add_edge(source_ix, target_ix, ());
    }

    pub fn list_edges(&self) -> Vec<(SymbolGraphNodeId, SymbolGraphNodeId)> {
        let mut id_edges = vec![];
        for edge in self.graph.raw_edges() {
            let source_id = self.node_ix_to_id.get(&(edge.source().index() as u32)).unwrap();
            let target_id = self.node_ix_to_id.get(&(edge.target().index() as u32)).unwrap();
            id_edges.push((SymbolGraphNodeId(*source_id), SymbolGraphNodeId(*target_id)));
        }
        id_edges
    }
}

/// Wrapped u32 identifier for DerivedSymbolInfo nodes in a SymbolGraphNodeSet
/// for type safety.  The values correspond to the index of the node in the
/// `symbol_crossref_infos` vec in `SymbolGraphNodeSet`.
#[derive(Clone)]
pub struct SymbolGraphNodeId(u32);

pub struct SymbolGraphNodeSet {
    pub symbol_crossref_infos: Vec<DerivedSymbolInfo>,
    pub symbol_to_index_map: BTreeMap<String, u32>,
}

fn make_data_invariant_err() -> ServerError {
    ServerError::StickyProblem(ErrorDetails {
        layer: ErrorLayer::RuntimeInvariantViolation,
        message: "SymbolGraphNodeSet desynchronized".to_string(),
    })
}

impl SymbolGraphNodeSet {
    pub fn new() -> Self {
        SymbolGraphNodeSet {
            symbol_crossref_infos: vec![],
            symbol_to_index_map: BTreeMap::new(),
        }
    }

    pub fn get(&self, node_id: &SymbolGraphNodeId) -> &DerivedSymbolInfo {
        // It's very much an invariant that only we mint SymbolGraphNodeId's, so
        // the entry should always exist.
        self.symbol_crossref_infos.get(node_id.0 as usize).unwrap()
    }

    /// Look-up a symbol returning its id (for graph purposes) and its
    /// DerivedSymbolInfo (for data inspection).
    pub fn lookup_symbol(&self, symbol: &str) -> Option<(SymbolGraphNodeId, &DerivedSymbolInfo)> {
        if let Some(index) = self.symbol_to_index_map.get(symbol) {
            let sym_info = self.symbol_crossref_infos.get(*index as usize);
            sym_info.map(|info| (SymbolGraphNodeId(*index), info))
        } else {
            None
        }
    }

    /// Add a symbol and return the unwrapped data that lookup_symbol would have provided.
    pub fn add_symbol(&mut self, sym_info: DerivedSymbolInfo) -> (SymbolGraphNodeId, &DerivedSymbolInfo) {
        let index = self.symbol_crossref_infos.len();
        let symbol = sym_info.symbol.clone();
        self.symbol_crossref_infos.push(sym_info);
        self.symbol_to_index_map
            .insert(symbol, index as u32);
        (SymbolGraphNodeId(index as u32), self.symbol_crossref_infos.get(index).unwrap())
    }

    pub async fn ensure_symbol(&mut self, sym: &str, server: &Box<dyn AbstractServer + Send + Sync>) -> Result<(SymbolGraphNodeId, &DerivedSymbolInfo)> {
        if let Some(index) = self.symbol_to_index_map.get(sym) {
            let sym_info = self.symbol_crossref_infos.get(*index as usize).ok_or_else(make_data_invariant_err)?;
            return Ok((SymbolGraphNodeId(*index), sym_info));
        }

        let info = server.crossref_lookup(&sym).await?;
        Ok(self.add_symbol(DerivedSymbolInfo::new(&sym, info)))
    }
}
