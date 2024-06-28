use std::collections::{BTreeMap, HashSet};

use async_trait::async_trait;
use serde_json::{from_value, Value};
use clap::Args;
use ustr::{UstrMap, Ustr, ustr};

use super::interface::{
    FlattenedKindGroupResults, FlattenedLineSpan, FlattenedPathKindGroupResults,
    FlattenedResultsBundle, FlattenedResultsByFile, PipelineJunctionCommand,
    PipelineValues, PresentationKind, ResultFacetGroup, ResultFacetKind, ResultFacetRoot,
    SymbolCrossrefInfo, SymbolQuality, SymbolRelation,
};

use crate::{
    abstract_server::{
        AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError, TextMatchesByFile, FileMatch,
    },
    file_format::analysis::PathSearchResult,
};

/// Process file, crossref, and fulltext search results into a classic
/// mozsearch mixed results representation consisting of groups of results
/// clustered by "path kind" (normal, test, generated, third party), and then
/// by key/kind precedence (files, IDL, defs, override stuff, super/subclass
/// stuff, assignments, uses, declarations, text matches), noting that
/// precedences will likely change.
#[derive(Debug, Args)]
pub struct CompileResults {
    /// Maximum number of file results to list, truncating at the limit.
    #[clap(short, long, value_parser, default_value = "2000")]
    file_limit: usize,

    /// Maximum number of result lines to limit, truncating at the limit.
    /// Context lines don't impact this limit.
    #[clap(short, long, value_parser, default_value = "2000")]
    line_limit: usize,
}

/// Core result processing logic / helper data-structures most analogous to the
/// router.py `SearchResult` class but with the inputs and outputs retaining a
/// bit more semantic linkage the whole way.
///
/// ### Python SearchResult Relation
///
/// We retain this extra information in order to enable:
/// - Interactive faceting of results related to override sets.  In particular,
///   the ability to rapidly toggle on/off cousin overrides that may not be of
///   interest.
/// - Automatic collapsing of sections that may be useful to have present for
///   completeness but which we believe are likely to not be something the user
///   wants to see.
/// - Better indication of when and where overload situations were hit and so
///   we can generate links that will show the user what was elided by either
///   expanding limits and/or showing the specific elided subset.
///
/// In particular, the python SearchResult has a concept of "qualified" results
/// which is a means of retaining the binding between symbols and the (pretty)
/// identifier that mapped to the symbol in identifier lookup.  SearchResult
/// then uses that information to tuple the "kind" over the pretty identifier.
/// (It's also the case that, historically, pre-structured-analysis landing,
/// things like overrides would destructively be aliased to the same pretty
/// identifier.  We no longer to this; see `CrossrefExpandCommand` for more.)
///
/// For our rust `SearchResults`, we inherently know the "pretty" identifier
/// associated with a symbol from its "meta" `crossref_info` contents.  We also
/// track how we learned about the symbol from a root symbol via its
/// `SymbolRelation`.  Note that for presentation purposes we will continue to
/// do all grouping based on the "pretty" identifier for now, although some day
/// we probably will need to address the existence of overloads better, but
/// right now overload coalescing is an important feature for cross-platform
/// merging where we expect, for example, 32-bit ARM to have different
/// signatures for things, etc.
///
/// ### Overview
///
/// Conceptually we build a [path kind, (kind, identifier), path] ordered
/// hierarchy where the values are line hits and where we flatten the hierarchy
/// by walking it in order.  In router.py, the "qkind" which tupled the kind and
/// identifier relied on an OrderedDict and the sequence in which symbols were
/// added.  The path kind had an explicit order and extraction was done in that
/// order.  paths were sorted and extracted in that order.
#[derive(Default)]
pub struct SearchResults {
    /// Cache mapping observed symbols to their pretty identifiers.  This
    /// depends on us seeing root symbols of relations before their related
    /// symbols, but that's explicitly how things are ordered.
    pub sym_to_pretty: UstrMap<Ustr>,
    /// We retain the meta information for any symbols we include our results
    /// for the benefit of the UI for future use.  We may also end up expanding
    /// the set of symbols-with-meta here as we address the class hierarchy, if
    /// that doesn't end up in a separate output structure.
    pub sym_to_meta: UstrMap<Value>,
    pub path_kind_groups: UstrMap<PathKindGroup>,
    /// Every key_line gets added to this set like `{path}:{key_line}` to
    /// suppress redundant hits on the line (from fulltext matches).
    pub path_line_suppressions: HashSet<String>,
}

#[derive(Default)]
pub struct PathKindGroup {
    pub file_names: Vec<Ustr>,
    pub qual_kind_groups: BTreeMap<QualKindDescriptor, QualKindGroup>,
}

/// Results for a specific kind (definition/use/etc.) for a specific pretty
/// identifier and potentially a set of
#[derive(Clone, PartialEq, PartialOrd, Eq, Ord)]
pub struct QualKindDescriptor {
    pub kind: PresentationKind,
    pub quality: SymbolQuality,
    pub pretty: Ustr,
}

pub struct QualKindGroup {
    pub path_facet: MaybeFacetRoot,
    pub relation_facet: MaybeFacetRoot,
    pub path_hits: BTreeMap<Ustr, FlattenedResultsByFile>,
}

impl QualKindGroup {
    pub fn new() -> Self {
        QualKindGroup {
            path_facet: MaybeFacetRoot::new(ResultFacetKind::PathByPath),
            relation_facet: MaybeFacetRoot::new(ResultFacetKind::SymbolByRelation),
            path_hits: BTreeMap::new(),
        }
    }
}

impl SearchResults {
    /// For each symbol we:
    /// - Figure out what identifier this symbol should be filed under based on
    ///   the `SymbolRelation`, and what "kinds" are applicable for line
    ///   result inclusion.  This helps us determine the `QualKind` to use.
    ///   - Some kinds of relations, like subclass/superclass relationships are
    ///     not intended to have any of their crossref "kinds" used for line
    ///     results, but instead for context which is still a TODO and maybe
    ///     be handled by a different command or a sidecar data structure as
    ///     part of this command.
    /// - Figure out the quality of this identifier based on the `SymbolQuality`
    ///   (which should be the same for all members of the same QualKind).
    /// - Proces the relevant kinds for each symbol, processing each path and
    ///   its associated hits.  Different paths can/will map to different
    ///   pathkinds and this will result in different faceting sets, etc. so the
    ///   processing here
    ///
    pub fn ingest_symbol(&mut self, info: SymbolCrossrefInfo) -> Result<()> {
        lazy_static! {
            static ref SELF: Ustr = ustr("Self");
            static ref OVERRIDDEN_BY: Ustr = ustr("Overriden By");
            static ref OVERRIDES: Ustr = ustr("Overrides");
            static ref COUSIN_OVERRIDES: Ustr = ustr("Cousin Overrides");
        }

        // There are other ways we could get this mapping like always baking the
        // "pretty" into the SymbolRelation or having our crossref infos be in a
        // map, but that complicates ownership issues massively.
        self.sym_to_pretty
            .insert(info.symbol.clone(), info.get_pretty());

        // Skip symbols that are only here for class relationship purposes.
        let (root_sym, relation_facet): (Ustr, &'static Ustr) = match &info.relation {
            SymbolRelation::SubclassOf(_, _)
            | SymbolRelation::SuperclassOf(_, _)
            | SymbolRelation::CousinClassOf(_, _) => {
                return Ok(());
            }
            SymbolRelation::Queried => (info.symbol.clone(), &SELF),
            SymbolRelation::OverrideOf(sym, _) => (sym.clone(), &OVERRIDDEN_BY),
            SymbolRelation::OverriddenBy(sym, _) => (sym.clone(), &OVERRIDES),
            SymbolRelation::CousinOverrideOf(sym, _) => (sym.clone(), &COUSIN_OVERRIDES),
        };

        let root_pretty = self
            .sym_to_pretty
            .get(&root_sym)
            .ok_or_else(|| {
                ServerError::StickyProblem(ErrorDetails {
                    layer: ErrorLayer::RuntimeInvariantViolation,
                    message: format!("no pretty available for root_sym {}", root_sym),
                })
            })?
            .clone();

        if let Value::Object(obj) = info.crossref_info {
            // This generic traversal is currently somewhat required because we
            // have different shapes for "meta", "callees", and everything else
            // (the kinds).  It could make sense to normalize the schema by not
            // having everything at the top-level.  (If doing that, it might
            // also be worth re-thinking other aspects of storage if it lets us
            // be lazier about parsing, etc.)
            for (kind, val) in obj.into_iter() {
                let pkind = match kind.as_str() {
                    "idl" => PresentationKind::IDL,
                    "defs" => PresentationKind::Definitions,
                    "decls" => PresentationKind::Declarations,
                    "assignments" => PresentationKind::Assignments,
                    "uses" => PresentationKind::Uses,
                    "meta" => {
                        // We save off the meta for this symbol for the UI.
                        self.sym_to_meta.insert(info.symbol.clone(), val);
                        continue;
                    }
                    // We expect this to match:
                    // - "callees": This is used only for call-graph stuff and is
                    //   something a human can learn from just looking at the
                    //   contents of a given method/symbol, etc.
                    _ => {
                        continue;
                    }
                };

                let descriptor = QualKindDescriptor {
                    kind: pkind,
                    quality: info.quality.clone(),
                    pretty: root_pretty.clone(),
                };

                let path_containers: Vec<PathSearchResult> = from_value(val)?;
                for path_container in path_containers {
                    self.ingest_path_hits(
                        &info.symbol,
                        descriptor.clone(),
                        relation_facet,
                        path_container,
                    );
                }
            }
        }

        Ok(())
    }

    fn ingest_path_hits(
        &mut self,
        sym: &Ustr,
        descriptor: QualKindDescriptor,
        relation_facet: &Ustr,
        path_container: PathSearchResult,
    ) {
        let path_kind_group = self
            .path_kind_groups
            .entry(path_container.path_kind)
            .or_insert_with(|| PathKindGroup::default());
        let qual_kind_group = path_kind_group
            .qual_kind_groups
            .entry(descriptor)
            .or_insert_with(|| QualKindGroup::new());

        // ### path faceting
        let path_sans_filename = match path_container.path.rfind('/') {
            Some(offset) => ustr(&path_container.path[0..offset + 1]),
            None => ustr(""),
        };
        let mut path_pieces: Vec<Ustr> = path_sans_filename
            .split_inclusive('/')
            .map(|s| ustr(s))
            .collect();
        // drop the filename portion.
        path_pieces.truncate(path_pieces.len() - 1);
        qual_kind_group
            .path_facet
            .place_item(path_pieces, path_sans_filename);

        // ### symbol relation faceting
        qual_kind_group
            .relation_facet
            .place_item(vec![relation_facet.clone()], sym.clone());

        // ### line results
        let file_results = qual_kind_group
            .path_hits
            .entry(path_container.path.clone())
            .or_insert_with(|| FlattenedResultsByFile {
                file: path_container.path.clone(),
                line_spans: vec![],
            });
        for search_result in path_container.lines {
            // Path-line suppressions exist to avoid redundant fulltext matches
            // showing up, so we don't actually care if there already was
            // another instance of this line already.
            //
            // At least, probably; obviously if it turns out we are ending up
            // with a ton of semantic results on the same line, maybe we need to
            // change the heuristic here to be a Map that suppresses redundant
            // lines for the same symbol on the same line, having the map store
            // the most recently used symbol.  (So we would have a limited
            // memory instead of a growing set.)  Practically speaking, we'd
            // expect this to really only happen in cases like implicit
            // constructors or macros where a ton of actual under-the-hood code
            // gets mapped down to a single token, but in that case we already
            // should have merged all of those redundant same-symbols.
            self.path_line_suppressions
                .insert(format!("{}:{}", path_container.path, search_result.lineno));
            file_results.line_spans.push(FlattenedLineSpan {
                key_line: search_result.lineno,
                line_range: if search_result.peek_range.is_empty() {
                    (search_result.lineno, search_result.lineno)
                } else {
                    (
                        search_result.peek_range.start_lineno,
                        search_result.peek_range.end_lineno,
                    )
                },
                contents: search_result.line,
                context: search_result.context,
                contextsym: search_result.contextsym,
            });
        }
    }

    pub fn ingest_file_match_hits(&mut self, file_matches: Vec<FileMatch>) {
        for file_match in file_matches {
            let path_kind_group = self
                .path_kind_groups
                .entry(file_match.concise.path_kind.clone())
                .or_insert_with(|| PathKindGroup::default());
            path_kind_group.file_names.push(file_match.path);
        }
    }

    pub fn ingest_fulltext_hits(&mut self, matches_by_file: Vec<TextMatchesByFile>) {
        let descriptor = QualKindDescriptor {
            kind: PresentationKind::TextualOccurrences,
            // The quality doesn't matter; there's only one class of text matches.
            quality: SymbolQuality::ExplicitSymbol,
            pretty: ustr(""),
        };

        for file_match in matches_by_file {
            let path = file_match.file;
            let path_kind_group = self
                .path_kind_groups
                .entry(file_match.path_kind)
                .or_insert_with(|| PathKindGroup::default());
            let qual_kind_group = path_kind_group
                .qual_kind_groups
                .entry(descriptor.clone())
                .or_insert_with(|| QualKindGroup::new());

            // ### line results
            let file_results = qual_kind_group
                .path_hits
                .entry(path.clone())
                .or_insert_with(|| FlattenedResultsByFile {
                    file: path.clone(),
                    line_spans: vec![],
                });
            for text_match in file_match.matches {
                if self.path_line_suppressions
                    .insert(format!("{}:{}", path, text_match.line_num)) {
                        file_results.line_spans.push(FlattenedLineSpan {
                            key_line: text_match.line_num,
                            line_range: (text_match.line_num, text_match.line_num),
                            contents: text_match.line_str,
                            context: ustr(""),
                            contextsym: ustr(""),
                        });
                }
            }
            // The suppressions could mean we don't actually need this path hit,
            // in which case we need to remove the file results.
            //
            // TODO: We should potentially back out the qual_kind_group and
            // path_kind_group here or have the `compile` step notice that the
            // path_hits is empty and so on.
            if file_results.line_spans.len() == 0 {
                qual_kind_group.path_hits.remove(&path);
            } else {
                // ### path faceting (now that we know we're keeping the hits)
                let path_sans_filename = match path.rfind('/') {
                    Some(offset) => ustr(&path[0..offset + 1]),
                    None => ustr(""),
                };
                let mut path_pieces: Vec<Ustr> = path_sans_filename
                    .split_inclusive('/')
                    .map(|s| ustr(s))
                    .collect();
                // drop the filename portion.
                path_pieces.truncate(path_pieces.len() - 1);
                qual_kind_group
                    .path_facet
                    .place_item(path_pieces, path_sans_filename);
            }
        }
    }

    pub fn compile(self, _file_limit: usize, _line_limit: usize) -> FlattenedResultsBundle {
        let mut path_kind_results = vec![];
        for (path_kind, pk_group) in self.path_kind_groups {
            let mut kind_groups = vec![];
            for (descriptor, qk_group) in pk_group.qual_kind_groups {
                let mut facets = vec![];

                if let Some(facet) = qk_group.relation_facet.compile() {
                    facets.push(facet);
                }
                if let Some(facet) = qk_group.path_facet.compile() {
                    facets.push(facet);
                }

                let mut by_file: Vec<FlattenedResultsByFile> =
                    qk_group.path_hits.into_values().collect();
                // The path_hits within each file are not guaranteed to be sorted,
                // so we sort them now.
                for results in by_file.iter_mut() {
                    results.line_spans.sort_by_key(|x| x.line_range.clone());
                }

                kind_groups.push(FlattenedKindGroupResults {
                    kind: descriptor.kind,
                    pretty: descriptor.pretty,
                    facets,
                    by_file,
                });
            }

            path_kind_results.push(FlattenedPathKindGroupResults {
                path_kind,
                file_names: pk_group.file_names,
                kind_groups,
            });
        }

        FlattenedResultsBundle {
            path_kind_results,
            content_type: "text/plain".to_string(),
        }
    }
}

/// Faceting support logic; the ResultFacetKind bakes in rules.
pub struct MaybeFacetRoot {
    pub kind: ResultFacetKind,
    pub root: MaybeFacetGroup,
}

impl MaybeFacetRoot {
    pub fn new(kind: ResultFacetKind) -> MaybeFacetRoot {
        MaybeFacetRoot {
            kind,
            root: MaybeFacetGroup::default(),
        }
    }

    /// Place the value within a fully built-out hierarchy.  We don't do dynamic
    /// hierarchy creation as things collide; instead we just create it all and
    /// then collapse it out of existence during the `compile` phase.
    pub fn place_item(&mut self, mut pieces: Vec<Ustr>, value: Ustr) {
        pieces.reverse();
        self.root.place_item(pieces, value);
    }

    /// Determine whether there's enough variety that faceting is appropriate,
    /// and if so, return a fully populated `ResultFacetRoot` according to the
    /// rules for this root's `ResultFacetKind`.
    ///
    /// The general algorithm here is:
    /// - Determine if each `MaybeFacetGroup` is "sole" (just one group),
    ///   "clumped" (has multiple sub-groups that meet the clump threshold),
    ///   "clump-able" (has one sub-group that meets the clump threshold and the
    ///   other groups can be clumped into an "Other" catch-all, if allowed)
    ///   or "sparse" (has sub-groups that don't meet the clump threshold).
    /// - A "sole" group that has a "sole" child gets merged with the child.
    ///   This happens for path-based faceting where multiple directory segments
    ///   may be shared in common with no deviation.
    /// -
    pub fn compile(self) -> Option<ResultFacetRoot> {
        if self.root.count == 0 {
            return None;
        }

        let (label, clump_thresh, other) = match self.kind {
            ResultFacetKind::SymbolByRelation => ("Relation".to_string(), 0, None),
            ResultFacetKind::PathByPath => ("Path".to_string(), 3, Some("*".to_string())),
        };
        let (compiled, breadth) = self.root.compile("".to_string(), clump_thresh, other);
        if breadth > 1 {
            return Some(ResultFacetRoot {
                label,
                kind: self.kind,
                groups: compiled.nested_groups,
            });
        } else {
            return None;
        }
    }
}

#[derive(Default)]
pub struct MaybeFacetGroup {
    pub nested_groups: BTreeMap<Ustr, MaybeFacetGroup>,
    pub values: Vec<Ustr>,
    /// Count of the values stored in this group in `values` and any nested
    /// groups.  This value will always be at least 1.
    pub count: u32,
}

impl MaybeFacetGroup {
    pub fn place_item(&mut self, mut reversed_pieces: Vec<Ustr>, value: Ustr) {
        self.count += 1;
        if let Some(next_piece) = reversed_pieces.pop() {
            self.nested_groups
                .entry(next_piece)
                .or_insert_with(|| MaybeFacetGroup::default())
                .place_item(reversed_pieces, value);
        } else {
            self.values.push(value);
        }
    }

    pub fn flatten(mut self) -> Vec<Ustr> {
        for subgroup in self.nested_groups.into_values() {
            let mut sub_flattened = subgroup.flatten();
            self.values.append(&mut sub_flattened);
        }

        return self.values;
    }

    /// Compiles the current group, returning the compiled result and the
    /// maximum number of nested groups known in the returned sub-tree which
    /// we're going to call the breadth.
    pub fn compile(
        mut self,
        prefix: String,
        clump_thresh: u32,
        other: Option<String>,
    ) -> (ResultFacetGroup, u32) {
        if self.nested_groups.len() == 0 {
            // No sub-groups means we are a leaf node and should return as-is.
            return (
                ResultFacetGroup {
                    label: prefix,
                    values: self.values,
                    nested_groups: vec![],
                    count: self.count,
                },
                1,
            );
        } else if self.nested_groups.len() == 1 {
            let (sole_name, sole_group) = self.nested_groups.into_iter().next().unwrap();
            let (mut sole_compiled, breadth) =
                sole_group.compile(prefix.clone() + sole_name.as_str(), clump_thresh, other.clone());

            if self.values.len() == 0 {
                // Collapse us into the nested group
                return (sole_compiled, breadth);
            }

            // We have values of our own and so we either want to fold the nested
            // group's contents into our own or retain our group and it as a
            // nested group.
            if breadth > 1 {
                // There's a tree somewhere down there, so just nest.
                return (
                    ResultFacetGroup {
                        label: prefix,
                        values: self.values,
                        nested_groups: vec![sole_compiled],
                        count: self.count,
                    },
                    breadth,
                );
            } else {
                // There's no tree below us, so fold its contents into us.  Note
                // that inductively according to this heuristic, we know the
                // sole_compiled will have no nested_groups and instead only
                // values.
                self.values.append(&mut sole_compiled.values);
                return (
                    ResultFacetGroup {
                        label: prefix,
                        values: self.values,
                        nested_groups: vec![],
                        count: self.count,
                    },
                    1,
                );
            }
        }

        // So there must be multiple nested_groups; the question is now how many
        // meet our clump criteria.
        let mut clump_hit_count: u32 = 0;
        let mut clump_miss_count: u32 = 0;

        for group in self.nested_groups.values() {
            if group.count >= clump_thresh {
                clump_hit_count += 1;
            } else {
                clump_miss_count += 1;
            }
        }

        if clump_hit_count >= 2 || (clump_hit_count >= 1 && other.is_some()) {
            let mut nested_groups = vec![];
            let mut breadth: u32;

            // Yes, we're going to materialize this group and some sub-groups.
            if other.is_none() || clump_miss_count == 0 {
                breadth = self.nested_groups.len() as u32;
                // We don't need to worry about building up an "other" group.
                for (name, group) in self.nested_groups {
                    let (sub_compiled, sub_breadth) =
                        group.compile(prefix.clone() + name.as_str(), clump_thresh, other.clone());
                    nested_groups.push(sub_compiled);
                    if sub_breadth > breadth {
                        breadth = sub_breadth;
                    }
                }
            } else {
                let mut other_group = ResultFacetGroup {
                    label: prefix.clone() + other.as_ref().unwrap().as_str(),
                    values: vec![],
                    nested_groups: vec![],
                    count: 0,
                };
                breadth = clump_hit_count + 1;

                for (name, group) in self.nested_groups {
                    if group.count >= clump_thresh {
                        let (sub_compiled, sub_breadth) =
                            group.compile(prefix.clone() + name.as_str(), clump_thresh, other.clone());
                        nested_groups.push(sub_compiled);
                        if sub_breadth > breadth {
                            breadth = sub_breadth;
                        }
                    } else {
                        let mut sub_flattened = group.flatten();
                        other_group.values.append(&mut sub_flattened);
                    }
                }
                nested_groups.push(other_group);
            }

            return (
                ResultFacetGroup {
                    label: prefix,
                    values: self.values,
                    nested_groups,
                    count: self.count,
                },
                breadth,
            );
        } else {
            // We're not going to materialize this group; just fold everything
            // in to ourselves.
            for subgroup in self.nested_groups.into_values() {
                let mut sub_flattened = subgroup.flatten();
                self.values.append(&mut sub_flattened);
            }

            return (
                ResultFacetGroup {
                    label: prefix,
                    values: self.values,
                    nested_groups: vec![],
                    count: self.count,
                },
                1,
            );
        }
    }
}

#[derive(Debug)]
pub struct CompileResultsCommand {
    pub args: CompileResults,
}

#[async_trait]
impl PipelineJunctionCommand for CompileResultsCommand {
    async fn execute(
        &self,
        _server: &Box<dyn AbstractServer + Send + Sync>,
        input: Vec<(String, PipelineValues)>,
    ) -> Result<PipelineValues> {
        let mut results = SearchResults::default();

        // We currently don't care about the name of the input because we only
        // match by type, but one could imagine a scenario in which they serve
        // as labels we want to propagate.
        for (_, pipe_value) in input {
            match pipe_value {
                PipelineValues::FileMatches(fm) => {
                    results.ingest_file_match_hits(fm.file_matches);
                }
                PipelineValues::SymbolCrossrefInfoList(scil) => {
                    for info in scil.symbol_crossref_infos {
                        results.ingest_symbol(info)?;
                    }
                }
                PipelineValues::TextMatches(tm) => {
                    results.ingest_fulltext_hits(tm.by_file);
                }
                _ => {
                    return Err(ServerError::StickyProblem(ErrorDetails {
                        layer: ErrorLayer::ConfigLayer,
                        message: "compile-results got something weird".to_string(),
                    }));
                }
            }
        }

        let results_bundle = results.compile(self.args.file_limit, self.args.line_limit);

        Ok(PipelineValues::FlattenedResultsBundle(results_bundle))
    }
}
