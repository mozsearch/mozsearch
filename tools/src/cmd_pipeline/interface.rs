use async_trait::async_trait;
use bitflags::bitflags;
use clap::{Args, ValueEnum};
use serde::{
    ser::{SerializeSeq, SerializeStruct},
    Serialize, Serializer,
};
use serde_json::{json, to_string_pretty, Value};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap, HashSet},
    fmt::Debug,
};
use tracing::{trace, trace_span, Instrument};
use ustr::{ustr, Ustr, UstrMap};

pub use crate::abstract_server::{AbstractServer, Result};
use crate::{
    abstract_server::{FileMatches, TextMatches},
    file_format::crossref_converter::convert_crossref_value_to_sym_info_rep,
};

use super::symbol_graph::{SymbolGraphCollection, SymbolGraphNodeId, SymbolGraphNodeSet};

#[derive(Clone, Debug, PartialEq, ValueEnum)]
pub enum RecordType {
    Source,
    Target,
    Structured,
}

impl RecordType {
    pub fn name(&self) -> &'static str {
        match self {
            RecordType::Source => "source",
            RecordType::Target => "target",
            RecordType::Structured => "structured",
        }
    }
}

#[derive(Debug, Args)]
pub struct SymbolicQueryOpts {
    /// Exact symbol match
    #[clap(short, value_parser)]
    pub symbol: Option<String>,

    /// Exact identifier match
    #[clap(short, value_parser)]
    pub identifier: Option<String>,
}

#[derive(Serialize)]
pub struct BatchGroups {
    pub groups: Vec<BatchGroupItem>,
}

#[derive(Serialize)]
pub struct BatchGroupItem {
    pub name: String,
    pub value: PipelineValues,
}

/// Hierarchical table whose rows may be optionally associated with symbols.
pub struct SymbolTreeTable {
    pub node_set: SymbolGraphNodeSet,
    pub columns: Vec<SymbolTreeTableColumn>,
    pub rows: Vec<SymbolTreeTableNode>,
}

#[derive(Serialize)]
pub struct SymbolTreeTableList {
    pub tables: Vec<SymbolTreeTable>,
}

impl SymbolTreeTableList {
    pub fn unioned_node_sets_as_jumprefs(&self) -> Value {
        let mut jumprefs = BTreeMap::new();
        for table in &self.tables {
            for sym_info in table.node_set.symbol_crossref_infos.iter() {
                let info = sym_info.crossref_info.clone();
                jumprefs.insert(
                    sym_info.symbol.clone(),
                    convert_crossref_value_to_sym_info_rep(info, &sym_info.symbol, None),
                );
            }
        }

        json!(jumprefs)
    }
}

#[derive(Serialize)]
pub struct SymbolTreeTableColumn {
    pub label: Vec<BasicMarkup>,
}

#[derive(Serialize)]
pub struct BasicLink {
    // Text to go within the `a` tag; this will be escaped.
    pub text: String,
    // This is not intended to be a fully valid href yet; the containing enum
    // helps contextualize what the base of the URL should be.  This will be
    // URL encoded to prevent escaping from attributes but not subject to text
    // escaping.
    pub link: String,
}

/// Very basic markup initially being introduced for SymbolTreeTable cells with
/// the expectation this will serialize into a much more terse JSON rep than the
/// structure might imply.
///
/// It's an explicit goal here to avoid generating HTML in our pipeline because:
/// - This simplifies thinking/worrying about escaping.
/// - It's significantly more pleasant to review prettified JSON snapshots of
///   this rather than the resulting HTML, but this should still provide us with
///   sufficient fidelity for review purposes in most cases, minimizing the
///   number of HTML snapshots we need.
/// - The decoupling could potentially be useful for people writing editor
///   plugins or maybe trying an alternate presentation on searchfox results,
///   etc.  This is not remotely a primary goal, but it does feel like a
///   benefit.  (For example, an editor plugin would probably like that the
///   SourceLink is tree-relative rather than a full absolute URL that has to
///   transformed, etc.)
#[derive(Serialize)]
pub enum BasicMarkup {
    Heading(String),
    // This is just text, it doesn't need to go in a tag at all.  It will get
    // escaped.
    Text(String),
    // This is text that should link to a query endpoint.  For now we just point
    // it at the "default" config, but one might imagine that we might propagate
    // the current config in use through to any subsequent links.  We might also
    // mark these links up with extra metadata so that we can transform them on
    // the client side so that a user's preferred config can perform an
    // override.
    QueryLink(BasicLink),
    // This is a
    SourceLink(BasicLink),
}

#[derive(Serialize)]
pub struct SymbolTreeTableCell {
    pub header: bool,
    pub contents: Vec<BasicMarkup>,
}

impl SymbolTreeTableCell {
    pub fn empty() -> Self {
        Self {
            header: false,
            contents: vec![],
        }
    }

    pub fn header_text(s: String) -> Self {
        Self {
            header: true,
            contents: vec![BasicMarkup::Text(s)],
        }
    }

    pub fn text(s: String) -> Self {
        Self {
            header: false,
            contents: vec![BasicMarkup::Text(s)],
        }
    }
}

pub struct SymbolTreeTableNode {
    pub label: Vec<BasicMarkup>,
    pub sym_id: Option<SymbolGraphNodeId>,
    pub col_vals: Vec<SymbolTreeTableCell>,
    pub children: Vec<SymbolTreeTableNode>,
}

impl SymbolTreeTable {
    pub fn new() -> Self {
        Self {
            node_set: SymbolGraphNodeSet::new(),
            columns: vec![],
            rows: vec![],
        }
    }
}

// Ephemeral class to allow for us to implement a serialization helper for
// SymbolTreeTableNode so that it can have the root/owning SymbolTreeTable's
// SymbolGraphNodeSet available to convert the SymbolGraphNodeId to a string
// without us needing to create an full serde_json::Value tree.
struct SerializingSymbolTreeTableNode<'a> {
    pub node_set: &'a SymbolGraphNodeSet,
    pub node: &'a SymbolTreeTableNode,
}

/// Ephemeral helper for SerializingSymbolTreeTableNode to wrap the sequence
/// serialization.
struct SerializingSymbolTreeTableRows<'a> {
    pub node_set: &'a SymbolGraphNodeSet,
    pub rows: &'a Vec<SymbolTreeTableNode>,
}

/// Custom serializer so that the node_set information can be expressed on the
/// serialization of the nodes as a stringified symbol that can be looked up
/// rather than an integer identifier.  This makes the test snapshots more
/// useful and stable as well as letting any symbol lookup table be
/// transparently unioned with other similar maps.
impl Serialize for SymbolTreeTable {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut stt = serializer.serialize_struct("SymbolTreeTable", 2)?;
        stt.serialize_field(
            "jumprefs",
            &self.node_set.symbols_meta_to_jumpref_json_nomut(),
        )?;
        stt.serialize_field("columns", &self.columns)?;

        let wrapped_rows = SerializingSymbolTreeTableRows {
            node_set: &self.node_set,
            rows: &self.rows,
        };
        stt.serialize_field("rows", &wrapped_rows)?;
        stt.end()
    }
}

impl<'a> Serialize for SerializingSymbolTreeTableNode<'a> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut stt = serializer.serialize_struct("SymbolTreeTableNode", 2)?;
        stt.serialize_field("label", &self.node.label)?;
        if let Some(sym_id) = &self.node.sym_id {
            stt.serialize_field("sym", &self.node_set.get(sym_id).symbol)?;
        } else {
            stt.serialize_field("sym", &Value::Null)?;
        }
        stt.serialize_field("colVals", &self.node.col_vals)?;
        let wrapped_rows = SerializingSymbolTreeTableRows {
            node_set: &self.node_set,
            rows: &self.node.children,
        };
        stt.serialize_field("children", &wrapped_rows)?;
        stt.end()
    }
}

impl<'a> Serialize for SerializingSymbolTreeTableRows<'a> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(self.rows.len()))?;
        for e in self.rows {
            let node = SerializingSymbolTreeTableNode {
                node_set: self.node_set,
                node: &e,
            };
            seq.serialize_element(&node)?;
        }
        seq.end()
    }
}

/// The input and output of each pipeline segment
#[derive(Serialize)]
pub enum PipelineValues {
    IdentifierList(IdentifierList),
    SymbolList(SymbolList),
    SymbolCrossrefInfoList(SymbolCrossrefInfoList),
    SymbolGraphCollection(SymbolGraphCollection),
    JsonValue(JsonValue),
    JsonValueList(JsonValueList),
    JsonRecords(JsonRecords),
    FileMatches(FileMatches),
    TextMatches(TextMatches),
    HtmlExcerpts(HtmlExcerpts),
    FlattenedResultsBundle(FlattenedResultsBundle),
    GraphResultsBundle(GraphResultsBundle),
    TextFile(TextFile),
    BatchGroups(BatchGroups),
    SymbolTreeTableList(SymbolTreeTableList),
    Void,
}

/// A list of (searchfox) identifiers.
#[derive(Serialize)]
pub struct IdentifierList {
    pub identifiers: Vec<Ustr>,
}

#[derive(Serialize)]
pub struct SymbolWithContext {
    pub symbol: Ustr,
    pub quality: SymbolQuality,
    pub from_identifier: Option<Ustr>,
}

/// A list of (searchfox) symbols.
#[derive(Serialize)]
pub struct SymbolList {
    pub symbols: Vec<SymbolWithContext>,
}

/// Metadata about how we got to this symbol from the root query.  Intended to
/// help in clustering and/or results ordering.
#[derive(Clone, Serialize)]
pub enum SymbolRelation {
    /// The symbol was directly queried for.
    Queried,
    /// This symbol is an override of the payload symbol (and was added via that
    /// symbol by following the "overriddenBy" downward edges).  The u32 is the
    /// distance.
    OverrideOf(Ustr, u32),
    /// This symbol was overridden by the payload symbol (and was added via that
    /// symbol by following the "overrides" upward edges).  The u32 is the
    /// distance.
    OverriddenBy(Ustr, u32),
    /// This symbol is in the same root override set of the payload symbol (and
    /// was added by following that symbol's "overrides" upward edges and then
    /// "overriddenBy" downward edges), but is a cousin rather than an ancestor
    /// or descendant in the graph.  The u32 is the number of steps to get to
    /// the common ancestor.
    CousinOverrideOf(Ustr, u32),
    /// This symbol is a subclass of the payload symbol (and was added via that
    /// symbol by following the "subclasses" downward edges).  The u32 is the
    /// distance.
    SubclassOf(Ustr, u32),
    /// This symbol is a superclass of the payload symbol (and was added via
    /// that symbol by following the "supers" upward edges).  The u32 is the
    /// distance.
    SuperclassOf(Ustr, u32),
    /// This symbol is a cousin class of the payload symbol (and was added via
    /// that symbol by following the "supers" upward edges and then "subclasses"
    /// downward edges) with a distance indicating the number of steps to get to
    /// the common ancestor.
    CousinClassOf(Ustr, u32),
}

/// Metadata about how likely we think it is that the user was actually looking
/// for this symbol; primarily intended to capture whether or not we got to this
/// symbol by prefix search on an identifier and how much was guessed so that we
/// can scale any speculative effort appropriately, especially during
/// incremental search.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub enum SymbolQuality {
    /// The symbol was explicitly specified and not the result of identifier
    /// lookup.
    ExplicitSymbol,
    /// The identifier was explicitly specified without prefix search enabled.
    ExplicitIdentifier,
    /// We did identifier search and the identifier was an exact match, but this
    /// was done in a context where we prefix search is also performed and
    /// expected.  The difference from `ExplicitIdentifier` here is that it can
    /// make sense to be more limited in automatically expanding the scope of
    /// results.
    ExactIdentifier,
    /// We did identifier search and the prefix matched; the values are how many
    /// characters matched and how many additional characters are in the
    /// identifier beyond the match point.  The latter number should always be
    /// at least 1, as 0 would make this `ExactIdentifier`.
    IdentifierPrefix(u32, u32),
}

impl SymbolQuality {
    /// Compute a quality rank where lower values are higher quality / closer to
    /// what the user typed.
    pub fn numeric_rank(&self) -> u32 {
        match self {
            SymbolQuality::ExplicitSymbol => 0,
            SymbolQuality::ExplicitIdentifier => 1,
            SymbolQuality::ExactIdentifier => 2,
            SymbolQuality::IdentifierPrefix(_matched, extra) => 2 + extra,
        }
    }
}

impl PartialOrd for SymbolQuality {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        let self_rank = self.numeric_rank();
        let other_rank = other.numeric_rank();
        self_rank.partial_cmp(&other_rank)
    }
}

impl Ord for SymbolQuality {
    fn cmp(&self, other: &Self) -> Ordering {
        let self_rank = self.numeric_rank();
        let other_rank = other.numeric_rank();
        self_rank.cmp(&other_rank)
    }
}

///
#[derive(Clone, Serialize)]
pub enum OverloadKind {
    /// There's just too many overrides!  This would happen for
    /// nsISupports::AddRef for example.
    Overrides,
    /// There's just too many subclasses!  This would happen for nsISupports for
    /// example.
    Subclasses,
    /// There's just too many uses!  This happens for mozilla::Runnable::Run for
    /// example when following the "uses" edges in traverse (and no other
    /// heuristics prevent considering the symbol when travelling up the
    /// override edge from the subclass).
    ///
    /// The Paths variant here captures that we gave up based on the number of
    /// paths which is a temporary thing (and we should change to Uses when
    /// correcting).
    UsesPaths,
    /// Uses limit but based on lines.
    UsesLines,
    FieldMemberUses,
    NodeLimit,
}

/// Information about overloads encountered when processing some aspect of a
/// symbol.  We've had a history of being unclear when limits are hit, so the
/// goal here is to be able to explicitly convey when we're hitting limits and
/// ideally to make it possible for the UI to generate links that can help the
/// user take an informed action to re-run with the limit bypassed.  (Our
/// concern is not so much abuse as much as it is about helping provide
/// consistently fast results as a user types a query and that the user opts in
/// to multi-second results rather than stumbling upon them.)
///
/// This is not currently intended to be used for `compile-results`, but could
/// perhaps be adapted for that.
#[derive(Clone, Serialize)]
pub struct OverloadInfo {
    pub kind: OverloadKind,
    /// The symbol, if any, this overload is associated with beyond the owner
    /// of this record.  That is, if this overload is hanging off a symbol's
    /// data structure (ex: SymbolCrossrefInfo), this field will be None unless
    /// we are characterizing and edge to another symbol.  But if this overload
    /// is hanging off a generic container, we may include a symbol name for
    /// additional context.  It's possible we might report the same overload in
    /// both places, even!  Overloads are intended as a diagnostic and for
    /// human comprehension to know the results have been truncated; we don't
    /// have to have a perfectly efficient data structure.
    pub sym: Option<String>,
    /// How many results do we think exist?
    pub exist: u32,
    /// How many results did we include before giving up?  This can be zero or
    /// otherwise less than the `limit`.
    pub included: u32,
    /// If this was a limit on this specific piece of data, what was the limit?
    /// 0 means there was no local limit hit (not that there was no limit).
    pub local_limit: u32,
    /// If this was a limit across multiple pieces of data, what was the limit?
    /// 0 means there was no global limit hit (not that there was no limit).
    pub global_limit: u32,
}

bitflags! {
    /// Experimental/hacky set of flags to enable a single pipeline to hold a
    /// heterogeneous mixture of symbols and where these flags are what makes
    /// the difference.  There is no unifying concept for the flags; it's fine
    /// to pile random semantics into this.
    ///
    /// Being introduced explicitly for calls-between-{source,target} with the
    /// fuse-crossrefs junction merging normal crossref-lookup outputs at the
    /// junction and setting these flags.  This works with the existing
    /// search-identifiers/crossref-lookup sequence flow, and especially if we
    /// added more filtering to the resulting pipeline, but arguably the search/
    /// lookup could be a single op which could allow "crossref-lookup" to
    /// be run repeatedly in sequence with each step adding new infos with new
    /// flags.
    #[derive(Clone, Copy, Default, Serialize)]
    pub struct SymbolMetaFlags: u32 {
        /// Mark a symbol as an interesting point for calls to start in a
        /// calls-between diagram.
        const Source = 0b00000001;
        /// Mark a symbol as an interesting point for calls to end up in a calls
        /// between diagram.
        const Target = 0b00000010;
    }
}
/// A symbol and its cross-reference information.
#[derive(Serialize)]
pub struct SymbolCrossrefInfo {
    pub symbol: Ustr,
    pub crossref_info: Value,
    pub relation: SymbolRelation,
    pub quality: SymbolQuality,
    /// Any overloads encountered when processing this symbol.
    pub overloads_hit: Vec<OverloadInfo>,
    #[serde(rename = "type", skip_serializing_if = "SymbolMetaFlags::is_empty")]
    pub flags: SymbolMetaFlags,
}

impl SymbolCrossrefInfo {
    /// Return the pretty identifier for this symbol from its "meta" "pretty"
    /// field, falling back to the symbol name if we don't have a pretty name.
    pub fn get_pretty(&self) -> Ustr {
        if let Some(Value::String(s)) = self.crossref_info.pointer("/meta/pretty") {
            ustr(s)
        } else {
            self.symbol.clone()
        }
    }

    pub fn get_method_symbols(&self) -> Option<Vec<Ustr>> {
        if let Some(Value::Array(arr)) = self.crossref_info.pointer("/meta/methods") {
            if arr.len() == 0 {
                return None;
            }
            Some(
                arr.iter()
                    .map(|v| ustr(v["sym"].as_str().unwrap_or("")))
                    .collect(),
            )
        } else {
            None
        }
    }
}

/// A list of `SymbolCrossrefInfo`s plus a list of any unknown symbols provided
/// to the input.
#[derive(Serialize)]
pub struct SymbolCrossrefInfoList {
    pub symbol_crossref_infos: Vec<SymbolCrossrefInfo>,
    pub unknown_symbols: Vec<String>,
}

/// router.py-style mozsearch compiled results that has top-level path-kind
/// (normal/test/generated) result clusters, where each cluster has file names /
/// paths and line hits grouped by symbol-with-kind and by file name/path
/// beneath that.
///
/// Line results can contain raw source text or HTML-rendered excerpts if
/// augmented by the `show-html` command.
#[derive(Serialize)]
pub struct FlattenedResultsBundle {
    pub path_kind_results: Vec<FlattenedPathKindGroupResults>,
    pub content_type: String,
}

impl FlattenedResultsBundle {
    pub fn compute_path_line_sets(&self, before: u32, after: u32) -> UstrMap<HashSet<u32>> {
        let mut path_line_sets = UstrMap::default();
        for path_kind_group in &self.path_kind_results {
            path_kind_group.accumulate_path_line_sets(&mut path_line_sets, before, after);
        }
        path_line_sets
    }

    pub fn ingest_html_lines(
        &mut self,
        path_line_contents: &UstrMap<HashMap<u32, String>>,
        before: u32,
        after: u32,
    ) {
        self.content_type = "text/html".to_string();
        for path_kind_group in &mut self.path_kind_results {
            path_kind_group.ingest_html_lines(&path_line_contents, before, after);
        }
    }
}

#[derive(Serialize)]
pub struct FlattenedPathKindGroupResults {
    pub path_kind: Ustr,
    pub file_names: Vec<Ustr>,
    pub kind_groups: Vec<FlattenedKindGroupResults>,
}

impl FlattenedPathKindGroupResults {
    pub fn accumulate_path_line_sets(
        &self,
        mut path_line_sets: &mut UstrMap<HashSet<u32>>,
        before: u32,
        after: u32,
    ) {
        for kind_group in &self.kind_groups {
            kind_group.accumulate_path_line_sets(&mut path_line_sets, before, after);
        }
    }

    pub fn ingest_html_lines(
        &mut self,
        path_line_contents: &UstrMap<HashMap<u32, String>>,
        before: u32,
        after: u32,
    ) {
        for kind_group in &mut self.kind_groups {
            kind_group.ingest_html_lines(&path_line_contents, before, after);
        }
    }
}

#[derive(Serialize)]
pub enum ResultFacetKind {
    /// We're faceting based on the relationship of symbols to the root symbol.
    SymbolByRelation,
    /// We're faceting based on the path of the definition for the symbol.
    PathByPath,
}

/// A context-sensitive facet for results.  Facets are only created when
/// multiple usefully sized groups would exist for the facet.  If there would
/// only be a single group, or there would be N groups for N results, then the
/// facet would not be useful and will not be emitted.
#[derive(Serialize)]
pub struct ResultFacetRoot {
    /// Terse human-readable explanation of the facet for UI display.
    pub label: String,
    pub kind: ResultFacetKind,
    pub groups: Vec<ResultFacetGroup>,
}

/// Hierarchical faceting group that gets nested inside a `ResultFacetRoot`.
#[derive(Serialize)]
pub struct ResultFacetGroup {
    /// Terse human-readable explanation of the facet for UI display.
    pub label: String,
    pub values: Vec<Ustr>,
    pub nested_groups: Vec<ResultFacetGroup>,
    /// The number of hits for this group, inclusive of nested groups.  This
    /// value should be equal to the sum of all of the nested_groups' counts if
    /// there are any nested groups.
    pub count: u32,
}

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Serialize)]
pub enum PresentationKind {
    // We don't give "Files" a kind because they don't look like path hit-lists.
    IDL,
    Definitions,
    Declarations,
    Assignments,
    Uses,
    // We do give textual occurrences a kind because they are path hit-lists.
    TextualOccurrences,
}

#[derive(Serialize)]
pub struct FlattenedKindGroupResults {
    pub kind: PresentationKind,
    pub pretty: Ustr,
    pub facets: Vec<ResultFacetRoot>,
    pub by_file: Vec<FlattenedResultsByFile>,
}

impl FlattenedKindGroupResults {
    pub fn accumulate_path_line_sets(
        &self,
        mut path_line_sets: &mut UstrMap<HashSet<u32>>,
        before: u32,
        after: u32,
    ) {
        for by_file in &self.by_file {
            by_file.accumulate_path_line_sets(&mut path_line_sets, before, after);
        }
    }

    pub fn ingest_html_lines(
        &mut self,
        path_line_contents: &UstrMap<HashMap<u32, String>>,
        before: u32,
        after: u32,
    ) {
        for by_file in &mut self.by_file {
            by_file.ingest_html_lines(&path_line_contents, before, after);
        }
    }
}

#[derive(Serialize)]
pub struct FlattenedResultsByFile {
    pub file: Ustr,
    pub line_spans: Vec<FlattenedLineSpan>,
}

impl FlattenedResultsByFile {
    pub fn accumulate_path_line_sets(
        &self,
        path_line_sets: &mut UstrMap<HashSet<u32>>,
        before: u32,
        after: u32,
    ) {
        let line_set = path_line_sets
            .entry(self.file.clone())
            .or_insert_with(|| HashSet::new());
        for span in &self.line_spans {
            let range = span.expand_range_in_isolation(before, after);
            for line in range.0..=range.1 {
                line_set.insert(line);
            }
        }
    }

    pub fn ingest_html_lines(
        &mut self,
        path_line_contents: &UstrMap<HashMap<u32, String>>,
        before: u32,
        after: u32,
    ) {
        if let Some(file_contents) = path_line_contents.get(&self.file) {
            let mut highest_line: u32 = 0;
            for i_span in 0..self.line_spans.len() {
                let (mut this_start, mut this_end) =
                    self.line_spans[i_span].expand_range_in_isolation(before, after);
                // adjust to avoid overlapping the prior span.
                if this_start <= highest_line {
                    this_start = highest_line + 1;
                }
                // avoid bumping into the next span if there is one
                if i_span < self.line_spans.len() - 1 {
                    let next_start = self.line_spans[i_span + 1].line_range.0;
                    if this_end >= next_start {
                        this_end = next_start - 1;
                    }
                }

                let mut lines = vec![];
                for line in this_start..=this_end {
                    if let Some(content) = file_contents.get(&line) {
                        lines.push(content.as_str());
                    }
                }
                // this_end was aspirational; we may have run out of lines,
                // so use the length.
                self.line_spans[i_span].line_range =
                    (this_start, this_start + (lines.len() - 1) as u32);
                self.line_spans[i_span].contents = lines.join("\n");

                highest_line = this_end;
            }
        }
    }
}

/// Represents a range of lines in a file.
#[derive(Serialize)]
pub struct FlattenedLineSpan {
    /// Canonical line number for this span of lines; the one that should be
    /// highlighted and the key term should be found in. 1-based line numbers.
    pub key_line: u32,
    /// The range of lines the core content results should include; when there's
    /// a block comment preceding something or if the statement/expression spans
    /// multiple lines, this could potentially be larger than just the key_line.
    pub line_range: (u32, u32),
    /// When the FlattenedResultsBundle has a `content_type` of "text/plain"
    /// this is expected to just be the single line of plaintext included in the
    /// `crossref` database.  When the type is "text/html" this is expected to
    /// be the formatted HTML output mutated into place by `ingest_html_lines`
    /// as provided by `cmd_augment_results.rs` and in that case before/after
    /// lines of context may be provided here but not incorporated into the
    /// `line_range` above.
    pub contents: String,
    // context and contextsym are normalized to empty upstream of here instead
    // of being `Option<String>` so we just maintain that for now.
    pub context: Ustr,
    pub contextsym: Ustr,
}

impl FlattenedLineSpan {
    /// Expand the range by before/after, ensuring we don't go below line 1 for
    /// the start, and ignoring the fact that we potentially will expand into
    /// adjacent spans.
    pub fn expand_range_in_isolation(&self, before: u32, after: u32) -> (u32, u32) {
        let start = std::cmp::max(1, self.line_range.0 as i64 - before as i64) as u32;
        let end = self.line_range.1 + after;
        (start, end)
    }
}

/// Rendered graphs and associated metadata.
#[derive(Serialize)]
pub struct GraphResultsBundle {
    pub graphs: Vec<RenderedGraph>,
    pub overloads_hit: Vec<OverloadInfo>,
    pub symbols: Value,
}

#[derive(Serialize)]
pub struct RenderedGraph {
    pub graph: String,
    pub extra: Value,
}

/// JSON records are raw analysis records from a single file (for now)
#[derive(Serialize)]
pub struct JsonRecordsByFile {
    pub file: String,
    pub records: Vec<Value>,
}

impl JsonRecordsByFile {
    /// Return the set of lines covered by the records in this structure.
    ///
    /// A HashSet is returned for ease of consumption even though it would
    /// almost certainly be more efficient to return a vec that the caller
    /// caller can consume in concert with a parallel traversal of (ex) the
    /// generated HTML for the given file.
    pub fn line_set(&self) -> HashSet<u32> {
        let mut line_set = HashSet::new();
        for value in &self.records {
            if let Some(loc) = value["loc"].as_str() {
                let lno = loc.split(":").next().unwrap_or("0").parse().unwrap_or(0);
                line_set.insert(lno);
            }
        }

        line_set
    }
}

/// A single JSON value, usually expected to be from a search query.
///
/// It might make sense to add a type-indicating value or origin of the JSON,
/// but for now this will only be from the query.
#[derive(Serialize)]
pub struct JsonValue {
    pub value: Value,
}

/// Multiple JSON values, as wrapped into a JsonValue.  While any values stored
/// here can obviously just be placed into a JSON array instead, the intent is
/// to avoid confusing aggregation for transport like we're doing here versus
/// what an endpoint that's not aggregating would return.
///
/// This does mean that the JSON serialization of this struct will look a little
/// awkward, but this will make it easier if we start labeling the JsonValue
/// values with their source/etc.
#[derive(Serialize)]
pub struct JsonValueList {
    pub values: Vec<JsonValue>,
}

/// JSON Analysis Records grouped by (source) file.
#[derive(Serialize)]
pub struct JsonRecords {
    pub by_file: Vec<JsonRecordsByFile>,
}

#[derive(Serialize)]
pub struct HtmlExcerptsByFile {
    pub file: String,
    pub excerpts: Vec<String>,
}

#[derive(Serialize)]
pub struct HtmlExcerpts {
    pub by_file: Vec<HtmlExcerptsByFile>,
}

#[derive(Serialize)]
pub struct TextFile {
    pub mime_type: String,
    pub contents: String,
}

/// A command that takes a single input and produces a single output.  At the
/// start of the pipeline, the input may be ignored / expected to be void.
#[async_trait]
pub trait PipelineCommand: Debug {
    async fn execute(
        &self,
        server: &Box<dyn AbstractServer + Send + Sync>,
        input: PipelineValues,
    ) -> Result<PipelineValues>;
}

/// A command that takes multiple inputs and produces a single output.
/// XXX speculative while implementing parallel processing.
#[async_trait]
pub trait PipelineJunctionCommand: Debug {
    async fn execute(
        &self,
        server: &Box<dyn AbstractServer + Send + Sync>,
        input: Vec<(String, PipelineValues)>,
    ) -> Result<PipelineValues>;
}

/// Multiple-use linear pipeline sequence.
pub struct ServerPipeline {
    pub server_kind: String,
    pub server: Box<dyn AbstractServer + Send + Sync>,
    pub commands: Vec<Box<dyn PipelineCommand + Send + Sync>>,
}

/// A linear pipeline sequence that potentially runs in parallel with other
/// named pipelines in a `ParallelPipelines` node which can be one in a sequence
/// of `ParallelPipelines` in a `ServerpipelineGraph`.  Inputs and outputs are
/// consumed from and added to a global dictionary.
pub struct NamedPipeline {
    /// Previous pipeline's output to consume.
    pub input_name: Option<String>,
    pub output_name: String,
    pub commands: Vec<Box<dyn PipelineCommand + Send + Sync>>,
}

impl NamedPipeline {
    pub async fn run(
        self,
        server: Box<dyn AbstractServer + Send + Sync>,
        mut cur_values: PipelineValues,
        traced: bool,
    ) -> Result<PipelineValues> {
        for cmd in &self.commands {
            let span = trace_span!("run_named_pipeline_step", cmd = ?cmd);

            match cmd
                .execute(&server, cur_values)
                .instrument(span.clone())
                .await
            {
                Ok(next_values) => {
                    cur_values = next_values;
                }
                Err(err) => {
                    trace!(err = ?err);
                    return Err(err);
                }
            }

            let _span_guard = span.entered();
            if traced {
                let value_str = to_string_pretty(&cur_values).unwrap();
                trace!(output_json = %value_str);
            }
        }

        Ok(cur_values)
    }
}

/// Consumes one or more inputs from the `NamedPipeline`s that ran prior to it
/// in the same `ParallelPipelines` node or possibly an earlier
/// `ParallelPipelines` node, producting a new output.  Inputs and outputs are
/// consumed from and added to a global dictionary.
pub struct JunctionInvocation {
    pub input_names: Vec<String>,
    pub output_name: String,
    pub command: Box<dyn PipelineJunctionCommand + Send + Sync>,
}

impl JunctionInvocation {
    pub async fn run(
        self,
        server: Box<dyn AbstractServer + Send + Sync>,
        input_values: Vec<(String, PipelineValues)>,
        traced: bool,
    ) -> Result<PipelineValues> {
        let span = trace_span!("run junction step", junction = ?self.command);

        let result = match self
            .command
            .execute(&server, input_values)
            .instrument(span.clone())
            .await
        {
            Ok(res) => res,
            Err(err) => {
                trace!(err = ?err);
                return Err(err);
            }
        };

        let _span_guard = span.entered();
        if traced {
            let value_str = to_string_pretty(&result).unwrap();
            trace!(output_json = %value_str);
        }

        Ok(result)
    }
}

pub struct ParallelPipelines {
    pub pipelines: Vec<NamedPipeline>,
    pub junctions: Vec<JunctionInvocation>,
}

/// Single-use pipeline graph.  Calling `run` consumes the graph for lifetime
/// simplicity because multiple parallel tasks are run and the borrows end up
/// awkward.  Also, we always expect the graphs to be built dynamically for each
/// use so we don't actually want to be able to reuse graphs at this time.
pub struct ServerPipelineGraph {
    pub server: Box<dyn AbstractServer + Send + Sync>,
    pub pipelines: Vec<ParallelPipelines>,
}

impl ServerPipeline {
    pub async fn run(&self, traced: bool) -> Result<PipelineValues> {
        let mut cur_values = PipelineValues::Void;

        for cmd in &self.commands {
            let span = trace_span!("run_pipeline_step", cmd = ?cmd);

            match cmd
                .execute(&self.server, cur_values)
                .instrument(span.clone())
                .await
            {
                Ok(next_values) => {
                    cur_values = next_values;
                }
                Err(err) => {
                    trace!(err = ?err);
                    return Err(err);
                }
            }

            let _span_guard = span.entered();
            if traced {
                let value_str = to_string_pretty(&cur_values).unwrap();
                trace!(output_json = %value_str);
            }
        }

        Ok(cur_values)
    }
}

impl ServerPipelineGraph {
    pub async fn run(self, traced: bool) -> Result<PipelineValues> {
        let mut named_values: BTreeMap<String, PipelineValues> = BTreeMap::new();

        for pipeline in self.pipelines {
            // ## kick off all the named pipelines in parallel
            let mut pipeline_tasks = vec![];
            for named_pipeline in pipeline.pipelines {
                let output = named_pipeline.output_name.clone();
                let input = match &named_pipeline.input_name {
                    Some(name) => {
                        // TODO: There could be cases like for compile-results
                        // where we would want a second pipeline to be able to
                        // consume the same input.
                        match named_values.remove(name) {
                            Some(val) => val,
                            None => PipelineValues::Void,
                        }
                    }
                    None => PipelineValues::Void,
                };
                let span = trace_span!("pipeline_task", input_name=?named_pipeline.input_name, output_name=?named_pipeline.output_name).or_current();
                pipeline_tasks.push((
                    output,
                    tokio::spawn(
                        named_pipeline
                            .run(self.server.clonify(), input, traced)
                            .instrument(span),
                    ),
                ));
            }

            // ## join the pipelines in sequence
            for (output, handle) in pipeline_tasks {
                let value = handle.await??;
                named_values.insert(output, value);
            }

            // ## kick off junctions in parallel
            let mut junction_tasks = vec![];
            for junction in pipeline.junctions {
                let output = junction.output_name.clone();
                let mut input_values = vec![];
                for name in &junction.input_names {
                    input_values.push((
                        name.clone(),
                        match named_values.remove(name) {
                            Some(val) => val,
                            None => PipelineValues::Void,
                        },
                    ));
                }

                let span = trace_span!("junction_task", input_names=?junction.input_names, output_name=?junction.output_name).or_current();
                junction_tasks.push((
                    output,
                    tokio::spawn(
                        junction
                            .run(self.server.clonify(), input_values, traced)
                            .instrument(span),
                    ),
                ));
            }

            for (output, handle) in junction_tasks {
                let value = handle.await??;
                named_values.insert(output, value);
            }
        }

        Ok(match named_values.remove("result") {
            Some(val) => val,
            None => PipelineValues::Void,
        })
    }
}
