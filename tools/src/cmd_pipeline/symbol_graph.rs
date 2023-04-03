use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use dot_generator::*;
use dot_structures::*;
use itertools::Itertools;
use petgraph::{
    algo::all_simple_paths,
    graph::{DefaultIx, NodeIndex},
    Directed, Graph as PetGraph,
};
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};
use serde_json::{json, Value, to_value};
use tracing::trace;
use ustr::{ustr, Ustr, UstrMap, UstrSet};

use crate::{
    abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError},
    file_format::crossref_converter::convert_crossref_value_to_sym_info_rep,
};

use super::interface::OverloadInfo;

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
#[derive(Clone)]
pub struct DerivedSymbolInfo {
    pub symbol: Ustr,
    pub crossref_info: Value,
}

pub fn semantic_kind_is_callable(semantic_kind: &str) -> bool {
    match semantic_kind {
        "function" => true,
        "method" => true,
        _ => false,
    }
}

// TODO: evaluate the type of kinds we now allow thanks to SCIP; we may need to
// expand this match branch or just normalize more in SCIP indexing.
pub fn semantic_kind_is_class(semantic_kind: &str) -> bool {
    match semantic_kind {
        "class" => true,
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
}

impl DerivedSymbolInfo {
    pub fn new(symbol: Ustr, crossref_info: Value) -> Self {
        DerivedSymbolInfo {
            symbol,
            crossref_info,
        }
    }
}

/// A collection of one or more graphs that share a common underlying set of
/// per-symbol information across the graphs.
pub struct SymbolGraphCollection {
    pub node_set: SymbolGraphNodeSet,
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
        sgc.serialize_field("jumprefs", &self.symbols_meta_to_jumpref_json_nomut())?;
        sgc.serialize_field("graphs", &graphs)?;
        sgc.serialize_field("hierarchicalGraphs", &hierarchical_graphs)?;
        sgc.end()
    }
}

fn escaped_node_id(id: &str) -> NodeId {
    NodeId(Id::Escaped(format!("\"{}\"", id)), None)
}

impl SymbolGraphCollection {
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
        for sym_info in self.node_set.symbol_crossref_infos.iter_mut() {
            let info = sym_info.crossref_info.take();
            jumprefs.insert(
                sym_info.symbol.clone(),
                convert_crossref_value_to_sym_info_rep(info, &sym_info.symbol, None),
            );
        }

        json!(jumprefs)
    }

    pub fn symbols_meta_to_jumpref_json_nomut(&self) -> Value {
        let mut jumprefs = BTreeMap::new();
        for sym_info in self.node_set.symbol_crossref_infos.iter() {
            // XXX This is inefficient!
            let info = sym_info.crossref_info.clone();
            jumprefs.insert(
                sym_info.symbol.clone(),
                convert_crossref_value_to_sym_info_rep(info, &sym_info.symbol, None),
            );
        }

        json!(jumprefs)
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
        for (source_id, target_id) in graph.list_edges() {
            let source_info = self.node_set.get(&source_id);
            let source_sym = source_info.symbol.clone();
            if nodes.insert(source_sym.clone()) {
                let mut node =
                    node!(esc source_sym.clone(); attr!("label", esc source_info.get_pretty()));
                node_decorate(&mut node, source_info);
                dot_graph.add_stmt(stmt!(node));
            }

            let target_info = self.node_set.get(&target_id);
            let target_sym = target_info.symbol.clone();
            if nodes.insert(target_sym.clone()) {
                let mut node =
                    node!(esc target_sym.clone(); attr!("label", esc target_info.get_pretty()));
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
            segment: HierarchySegment::PrettySegment("".to_string()),
            display_name: "".to_string(),
            symbols: vec![],
            action: None,
            children: BTreeMap::default(),
            edges: vec![],
            descendant_edge_count: 0,
            height: 0,
        };

        let mut checked_pretties = UstrSet::default();

        // ## Populate the hierarchy nodes.
        for sym_id in graph.list_nodes() {
            let sym_pretty = self.node_set.get(&sym_id).get_pretty();
            let mut pretty_so_far = "".to_string();
            let mut segments = vec![];
            trace!(sym = %sym_pretty, "processing symbol");
            for piece in sym_pretty.split("::") {
                trace!(piece = %piece, "processing piece");
                segments.push(HierarchySegment::PrettySegment(piece.to_string()));
                pretty_so_far = if pretty_so_far.is_empty() {
                    piece.to_string()
                } else {
                    format!("{}::{}", pretty_so_far, piece)
                };
                let ustr_so_far = ustr(&pretty_so_far);

                // If this is a partial pretty, then we only need to perform a lookup
                // for it once, but if it's a full pretty then we need to process it
                // because overloads exist (and have the same pretty)!
                if sym_pretty == pretty_so_far || checked_pretties.insert(ustr_so_far) {
                    // We haven't checked this before, so process it.

                    // See if we can find a symbol for this identifier.
                    let use_sym_id = if sym_pretty == pretty_so_far {
                        trace!(pretty = %pretty_so_far, "reusing known symbol");
                        sym_id.clone()
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
                            let (match_sym_id, _) =
                                self.node_set.ensure_symbol(&match_sym, server).await?;
                            match_sym_id
                        } else {
                            trace!(pretty = %pretty_so_far, "failed to locate symbol for identifier");
                            continue;
                        }
                    };

                    let mut reversed_segments = segments.clone();
                    reversed_segments.reverse();
                    trace!(pretty = %pretty_so_far, "placing found symbol");
                    root.place_sym(reversed_segments, use_sym_id);
                }
            }
        }

        // ## Populate the hierarchy edges
        for (from_id, to_id) in graph.list_edges() {
            let from_pretty = self.node_set.get(&from_id).get_pretty();
            let from_pieces = from_pretty.split("::");
            let to_pretty = self.node_set.get(&to_id).get_pretty();
            let to_pieces = to_pretty.split("::");

            let mut common_path: Vec<HierarchySegment> = from_pieces
                .zip(to_pieces)
                .take_while(|(a, b)| a == b)
                .map(|(a, _)| HierarchySegment::PrettySegment(a.to_string()))
                .collect();
            common_path.reverse();
            root.place_edge(common_path, from_id, to_id);
        }

        self.hierarchical_graphs.push(HierarchicalSymbolGraph {
            name: graph.name.clone(),
            root,
        });

        Ok(())
    }

    /// Convert the graph with the given index to a graphviz rep.
    pub fn hierarchical_graph_to_graphviz(&mut self, graph_idx: usize) -> Graph {
        trace!(graph_idx = %graph_idx, "hierarchical_graph_to_graphviz");
        let graph = match self.hierarchical_graphs.get_mut(graph_idx) {
            Some(g) => g,
            None => {
                trace!("no such graph");
                return graph!(
                    di id!("g");
                    node!("node"; attr!("shape","box"), attr!("fontname", esc "Courier New"), attr!("fontsize", "10"))
                );
            }
        };

        let mut state = HierarchicalRenderState {
            next_synthetic_id: 0,
            sym_to_edges: HashMap::default(),
        };
        graph.root.compile(0, 0, false, &self.node_set, &mut state);

        let dot_graph = Graph::DiGraph {
            id: id!("g"),
            strict: false,
            // Note that the root node renders the default style directives.
            stmts: graph.root.render(&self.node_set, &mut state),
        };

        dot_graph
    }
}

/// A graph whose nodes are symbols from a `SymbolGraphNodeSet`.
pub struct NamedSymbolGraph {
    pub name: String,
    graph: PetGraph<u32, (), Directed>,
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

    fn ensure_node(&mut self, sym_id: SymbolGraphNodeId) -> NodeIndex {
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

    pub fn add_edge(&mut self, source: SymbolGraphNodeId, target: SymbolGraphNodeId) {
        let source_ix = self.ensure_node(source);
        let target_ix = self.ensure_node(target);
        self.graph.add_edge(source_ix, target_ix, ());
    }

    pub fn list_edges(&self) -> Vec<(SymbolGraphNodeId, SymbolGraphNodeId)> {
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
            id_edges.push((SymbolGraphNodeId(*source_id), SymbolGraphNodeId(*target_id)));
        }
        id_edges
    }

    pub fn all_simple_paths(
        &mut self,
        source: SymbolGraphNodeId,
        target: SymbolGraphNodeId,
    ) -> Vec<Vec<SymbolGraphNodeId>> {
        let source_ix = self.ensure_node(source);
        let target_ix = self.ensure_node(target);
        let paths = all_simple_paths(&self.graph, source_ix, target_ix, 0, None);
        let node_paths = paths
            .map(|v: Vec<_>| {
                v.into_iter()
                    .map(|idx| {
                        SymbolGraphNodeId(*self.node_ix_to_id.get(&(idx.index() as u32)).unwrap())
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
    pub contents: String,
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
            row_pieces.push(format!(
                r#"<td href="{}" port="{}" align="left">{}{}</td>"#,
                cell.symbol, cell.port, indent_str, cell.contents
            ));
        }
        format!("<tr>{}</tr>", row_pieces.join(""))
    }
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
#[derive(Clone, Eq, PartialEq, PartialOrd, Ord, Debug)]
pub enum HierarchySegment {
    PrettySegment(String),
}

impl HierarchySegment {
    pub fn to_human_readable(&self) -> String {
        match self {
            Self::PrettySegment(p) => p.clone(),
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
    pub edges: Vec<(SymbolGraphNodeId, SymbolGraphNodeId)>,
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
            .map(|(from_id, to_id)| {
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
            self.symbols.push(sym_id);
        }
    }

    pub fn place_edge(
        &mut self,
        mut reversed_segments: Vec<HierarchySegment>,
        from_id: SymbolGraphNodeId,
        to_id: SymbolGraphNodeId,
    ) {
        if let Some(next_seg) = reversed_segments.pop() {
            if let Some(kid) = self.children.get_mut(&next_seg) {
                // The edge will go in our descendant, so we bump the count.
                self.descendant_edge_count += 1;
                kid.place_edge(reversed_segments, from_id, to_id);
            }
        } else {
            // We do not modify the descendant_edge_count because it's our own
            // edge.
            self.edges.push((from_id, to_id));
        }
    }

    pub fn compile(
        &mut self,
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
                    sole_kid.display_name =
                        format!("{}::{}", self.display_name, sole_kid.display_name);
                    self.display_name = "".to_string();
                }
                sole_kid.compile(
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
                kid.compile(depth + 1, collapsed_ancestors, be_class, node_set, state);
            }
        } else if self.edges.len() > 0 {
            // If there are edges at this level, it does not make sense to be a
            // table because the self-edges end up quite gratuitous.  (The edges
            // vec only contains edges among our immediate children.)
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
                let in_target = node_id!(esc parent_id_str, port!(id!(esc parent_id_str), "w"));
                let out_target = node_id!(esc parent_id_str, port!(id!(esc parent_id_str), "e"));
                state.register_symbol_edge_targets(&self.symbols, in_target, out_target);
            }

            for kid in self.children.values_mut() {
                let kid_id_str = kid.derive_id(node_set, state);
                let in_target = node_id!(esc parent_id_str, port!(id!(esc kid_id_str), "w"));
                let out_target = node_id!(esc parent_id_str, port!(id!(esc kid_id_str), "e"));
                state.register_symbol_edge_targets(&kid.symbols, in_target, out_target);
                kid.action = Some(HierarchicalLayoutAction::Record(kid_id_str));
            }
            self.action = Some(HierarchicalLayoutAction::Table(parent_id_str));
        } else if self.children.len() > 0 {
            // If there are kids, we want to be a cluster after all.
            be_cluster = true;
        } else {
            let node_id_str = self.derive_id(node_set, state);
            let id = node_id!(esc node_id_str);
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
            state.register_symbol_edge_targets(&self.symbols, placeholder_id.clone(), placeholder_id);

            for kid in self.children.values_mut() {
                kid.compile(depth + 1, 0, be_class, node_set, state);
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
            self.symbols.iter().map(|sym_id| node_set.get(sym_id).symbol.clone()).join(",")
        } else {
            state.issue_new_synthetic_id()
        }
    }

    pub fn render(
        &self,
        node_set: &SymbolGraphNodeSet,
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
                // provide the default settings here in the root too
                result.push(stmt!(attr!("concentrate", "true")));
                result.push(stmt!(
                    node!("graph"; attr!("fontname", esc "Courier New"), attr!("fontsize", "12"))
                ));
                result.push(stmt!(node!("node"; attr!("shape","box"), attr!("fontname", esc "Courier New"), attr!("fontsize", "10"))));
                for kid in self.children.values() {
                    result.extend(kid.render(node_set, state));
                }
            }
            HierarchicalLayoutAction::Collapse => {
                for kid in self.children.values() {
                    result.extend(kid.render(node_set, state));
                }
            }
            HierarchicalLayoutAction::Cluster(cluster_id, placeholder_id) => {
                let mut sg = subgraph!(esc cluster_id; attr!("cluster", "true"), attr!("label", esc self.display_name));
                sg.stmts.push(stmt!(
                    node!(esc placeholder_id; attr!("shape", "point"), attr!("style", "invis"))
                ));

                for kid in self.children.values() {
                    sg.stmts.extend(kid.render(node_set, state));
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
                        contents: format!("<b>{}</b>", self.display_name),
                        symbol: node_id.clone(),
                        port: node_id.clone(),
                        indent_level: 0,
                    }],
                });

                let mut kid_edges = vec![];
                for kid in self.children.values() {
                    if let Some(HierarchicalLayoutAction::Record(kid_port_name)) = &kid.action {
                        table.rows.push(LabelRow {
                            cells: vec![LabelCell {
                                contents: kid.display_name.clone(),
                                symbol: kid_port_name.clone(),
                                port: kid_port_name.clone(),
                                indent_level: 1,
                            }],
                        });

                        for (from_id, to_id) in &kid.edges {
                            if let Some((from_node, to_node)) = state.lookup_edge(from_id, to_id) {
                                kid_edges.push(stmt!(edge!(from_node => to_node)));
                            }
                        }
                    }
                }

                table.compile();
                let table_html = table.render();

                let node =
                    node!(esc node_id; attr!("shape", "none"), attr!("label", html table_html));
                result.push(stmt!(node));

                result.extend(kid_edges);
            }
            HierarchicalLayoutAction::Record(_) => {
                // Records will be handled by their parent table.
            }
            HierarchicalLayoutAction::Node(node_id) => {
                result.push(stmt!(
                    node!(esc node_id; attr!("label", esc self.display_name))
                ));
            }
        }

        for (from_id, to_id) in &self.edges {
            if let Some((from_node, to_node)) = state.lookup_edge(from_id, to_id) {
                result.push(stmt!(edge!(from_node => to_node)));
            }
        }

        result
    }
}

pub struct HierarchicalRenderState {
    next_synthetic_id: u32,
    /// Maps the SymbolGraphNodeId to (in-edge id, out-edge-id)
    sym_to_edges: HashMap<u32, (NodeId, NodeId)>,
}

impl HierarchicalRenderState {
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
            self.sym_to_edges.insert(sym_id.0, (in_target.clone(), out_target.clone()));
        }
    }

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
}

/// Wrapped u32 identifier for DerivedSymbolInfo nodes in a SymbolGraphNodeSet
/// for type safety.  The values correspond to the index of the node in the
/// `symbol_crossref_infos` vec in `SymbolGraphNodeSet`.
#[derive(Clone)]
pub struct SymbolGraphNodeId(u32);

pub struct SymbolGraphNodeSet {
    pub symbol_crossref_infos: Vec<DerivedSymbolInfo>,
    pub symbol_to_index_map: UstrMap<u32>,
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
            symbol_to_index_map: UstrMap::default(),
        }
    }

    pub fn get(&self, node_id: &SymbolGraphNodeId) -> &DerivedSymbolInfo {
        // It's very much an invariant that only we mint SymbolGraphNodeId's, so
        // the entry should always exist.
        self.symbol_crossref_infos.get(node_id.0 as usize).unwrap()
    }

    pub fn propagate_paths(
        &self,
        paths: Vec<Vec<SymbolGraphNodeId>>,
        new_graph: &mut NamedSymbolGraph,
        new_symbol_set: &mut SymbolGraphNodeSet,
        suppression: &mut HashSet<(u32, u32)>,
    ) {
        for path in paths {
            for (path_source, path_target) in path.into_iter().tuple_windows() {
                if suppression.insert((path_source.0, path_target.0)) {
                    self.propagate_edge(&path_source, &path_target, new_graph, new_symbol_set);
                }
            }
        }
    }

    /// Given a pair of symbols in the current set, ensure that they exist in
    /// the new node set and create an edge in the new graph as well.
    pub fn propagate_edge(
        &self,
        source: &SymbolGraphNodeId,
        target: &SymbolGraphNodeId,
        new_graph: &mut NamedSymbolGraph,
        new_symbol_set: &mut SymbolGraphNodeSet,
    ) {
        let new_source_node = self.propagate_sym(source, new_symbol_set);
        let new_target_node = self.propagate_sym(target, new_symbol_set);
        new_graph.add_edge(new_source_node, new_target_node);
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
        sym_info: DerivedSymbolInfo,
    ) -> (SymbolGraphNodeId, &DerivedSymbolInfo) {
        let index = self.symbol_crossref_infos.len();
        let symbol = sym_info.symbol.clone();
        self.symbol_crossref_infos.push(sym_info);
        self.symbol_to_index_map.insert(symbol, index as u32);
        (
            SymbolGraphNodeId(index as u32),
            self.symbol_crossref_infos.get(index).unwrap(),
        )
    }

    pub async fn ensure_symbol<'a>(
        &'a mut self,
        sym: &'a Ustr,
        server: &'a Box<dyn AbstractServer + Send + Sync>,
    ) -> Result<(SymbolGraphNodeId, &DerivedSymbolInfo)> {
        if let Some(index) = self.symbol_to_index_map.get(sym) {
            let sym_info = self
                .symbol_crossref_infos
                .get(*index as usize)
                .ok_or_else(make_data_invariant_err)?;
            return Ok((SymbolGraphNodeId(*index), sym_info));
        }

        let info = server.crossref_lookup(&sym).await?;
        Ok(self.add_symbol(DerivedSymbolInfo::new(sym.clone(), info)))
    }
}
