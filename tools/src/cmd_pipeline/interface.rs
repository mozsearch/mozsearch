use async_trait::async_trait;
use clap::arg_enum;
use serde::Serialize;
use serde_json::{to_string_pretty, Value};
use std::{cmp::Ordering, collections::HashSet, fmt::Debug};
use structopt::StructOpt;
use tracing::{trace, trace_span};

use crate::abstract_server::TextMatches;
pub use crate::abstract_server::{AbstractServer, Result};

use super::symbol_graph::SymbolGraphCollection;

arg_enum! {
  #[derive(Debug, PartialEq)]
  pub enum RecordType {
      Source,
      Target,
      Structured,
  }
}

#[derive(Debug, StructOpt)]
pub struct SymbolicQueryOpts {
    /// Exact symbol match
    #[structopt(short)]
    pub symbol: Option<String>,

    /// Exact identifier match
    #[structopt(short)]
    pub identifier: Option<String>,
}

/// The input and output of each pipeline segment
#[derive(Serialize)]
pub enum PipelineValues {
    IdentifierList(IdentifierList),
    SymbolList(SymbolList),
    SymbolCrossrefInfoList(SymbolCrossrefInfoList),
    SymbolGraphCollection(SymbolGraphCollection),
    JsonValue(JsonValue),
    JsonRecords(JsonRecords),
    FileMatches(FileMatches),
    TextMatches(TextMatches),
    HtmlExcerpts(HtmlExcerpts),
    FlattenedResultsBundle(FlattenedResultsBundle),
    TextFile(TextFile),
    Void,
}

/// A list of (searchfox) identifiers.
#[derive(Serialize)]
pub struct IdentifierList {
    pub identifiers: Vec<String>,
}

#[derive(Serialize)]
pub struct SymbolWithContext {
    pub symbol: String,
    pub quality: SymbolQuality,
    pub from_identifier: Option<String>,
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
    OverrideOf(String, u32),
    /// This symbol was overridden by the payload symbol (and was added via that
    /// symbol by following the "overrides" upward edges).  The u32 is the
    /// distance.
    OverriddenBy(String, u32),
    /// This symbol is in the same root override set of the payload symbol (and
    /// was added by following that symbol's "overrides" upward edges and then
    /// "overriddenBy" downward edges), but is a cousin rather than an ancestor
    /// or descendant in the graph.  The u32 is the number of steps to get to
    /// the common ancestor.
    CousinOverrideOf(String, u32),
    /// This symbol is a subclass of the payload symbol (and was added via that
    /// symbol by following the "subclasses" downward edges).  The u32 is the
    /// distance.
    SubclassOf(String, u32),
    /// This symbol is a superclass of the payload symbol (and was added via
    /// that symbol by following the "supers" upward edges).  The u32 is the
    /// distance.
    SuperclassOf(String, u32),
    /// This symbol is a cousin class of the payload symbol (and was added via
    /// that symbol by following the "supers" upward edges and then "subclasses"
    /// downward edges) with a distance indicating the number of steps to get to
    /// the common ancestor.
    CousinClassOf(String, u32),
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

/// A symbol and its cross-reference information.
#[derive(Serialize)]
pub struct SymbolCrossrefInfo {
    pub symbol: String,
    pub crossref_info: Value,
    pub relation: SymbolRelation,
    pub quality: SymbolQuality,
    /// Any overloads encountered when processing this symbol.
    pub overloads_hit: Vec<OverloadInfo>,
}

impl SymbolCrossrefInfo {
    /// Return the pretty identifier for this symbol from its "meta" "pretty"
    /// field, falling back to the symbol name if we don't have a pretty name.
    pub fn get_pretty(&self) -> String {
        if let Some(Value::String(s)) = self.crossref_info.pointer("/meta/pretty") {
            s.clone()
        } else {
            self.symbol.clone()
        }
    }
}

/// A list of `SymbolCrossrefInfo`s.
#[derive(Serialize)]
pub struct SymbolCrossrefInfoList {
    pub symbol_crossref_infos: Vec<SymbolCrossrefInfo>,
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

#[derive(PartialEq, PartialOrd, Eq, Ord, Serialize)]
pub enum PathKind {
    Normal,
    ThirdParty,
    Test,
    Generated,
}

#[derive(Serialize)]
pub struct FlattenedPathKindGroupResults {
    pub path_kind: PathKind,
    pub file_names: Vec<String>,
    pub kind_groups: Vec<FlattenedKindGroupResults>,
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
    pub values: Vec<String>,
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
    pub pretty: String,
    pub facets: Vec<ResultFacetRoot>,
    pub by_file: Vec<FlattenedResultsByFile>,
}

#[derive(Serialize)]
pub struct FlattenedResultsByFile {
    pub file: String,
    pub line_spans: Vec<FlattenedLineSpan>,
}

/// Represents a range of lines in a file.
#[derive(Serialize)]
pub struct FlattenedLineSpan {
    pub line_range: (u32, u32),
    pub contents: String,
    // context and contextsym are normalized to empty upstream of here instead
    // of being `Option<String>` so we just maintain that for now.
    pub context: String,
    pub contextsym: String,
}

/// This currently boring struct exists so that we have a place to put metadata
/// about files that can ride-along with the name.  However, it could end up
/// that we want to just treat files as a special type of symbol, in which case
/// maybe we don't put that info here and let later stages look it up
/// themselves?  Optionally, maybe this ends up being an optional serde_json
/// Value (where Some(null) means it had no data and None means we haven't
/// looked).
#[derive(Serialize)]
pub struct FileMatch {
    pub path: String,
}

#[derive(Serialize)]
pub struct FileMatches {
    pub file_matches: Vec<FileMatch>,
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
        input: Vec<PipelineValues>,
    ) -> Result<PipelineValues>;
}

/// A linear pipeline sequence.
pub struct ServerPipeline {
    pub server_kind: String,
    pub server: Box<dyn AbstractServer + Send + Sync>,
    pub commands: Vec<Box<dyn PipelineCommand>>,
}

/// A linear pipeline sequence that potentially runs in parallel with other
/// named pipelines in a `ParallelPipelines` node which can be one in a sequence
/// of `ParallelPipelines` in a `ServerpipelineGraph`.  Inputs and outputs are
/// consumed from and added to a global dictionary.
pub struct NamedPipeline {
    /// Previous pipeline's output to consume.
    pub input_name: Option<String>,
    pub output_name: String,
    pub commands: Vec<Box<dyn PipelineCommand>>,
}

/// Consumes one or more inputs from the `NamedPipeline`s that ran prior to it
/// in the same `ParallelPipelines` node or possibly an earlier
/// `ParallelPipelines` node, producting a new output.  Inputs and outputs are
/// consumed from and added to a global dictionary.
pub struct JunctionInvocation {
    pub input_names: Vec<String>,
    pub output_name: String,
    pub command: Box<dyn PipelineJunctionCommand>,
}

pub struct ParallelPipelines {
    pub pipelines: Vec<NamedPipeline>,
    pub junctions: Vec<JunctionInvocation>,
}

///
pub struct ServerPipelineGraph {
    pub server_kind: String,
    pub server: Box<dyn AbstractServer + Send + Sync>,
    pub pipelines: Vec<ParallelPipelines>,
}

impl ServerPipeline {
    pub async fn run(&self, traced: bool) -> Result<PipelineValues> {
        let mut cur_values = PipelineValues::Void;

        for cmd in &self.commands {
            let span = trace_span!("run_pipeline_step", cmd = ?cmd);
            let _span_guard = span.enter();

            match cmd.execute(&self.server, cur_values).await {
                Ok(next_values) => {
                    cur_values = next_values;
                }
                Err(err) => {
                    trace!(err = ?err);
                    return Err(err);
                }
            }

            if traced {
                let value_str = to_string_pretty(&cur_values).unwrap();
                trace!(output_json = %value_str);
            }
        }

        Ok(cur_values)
    }
}

impl ServerPipelineGraph {
    pub async fn run(&self, _traced: bool) -> Result<PipelineValues> {
        let cur_values = PipelineValues::Void;

        // XXX impl

        Ok(cur_values)
    }
}
