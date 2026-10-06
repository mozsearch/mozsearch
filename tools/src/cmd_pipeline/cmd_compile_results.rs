use std::collections::{BTreeMap, BTreeSet, HashSet};

use async_trait::async_trait;
use clap::Args;
use ustr::{Ustr, UstrMap, ustr};

use super::facets::MaybeFacetRoot;
use super::interface::{
    FlattenedKindGroupResults, FlattenedLineSpan, FlattenedPathKindGroupResults,
    FlattenedResultsBundle, FlattenedResultsByFile, PipelineJunctionCommand, PipelineValues,
    PresentationKind, ResultFacetKind, SymbolCrossrefInfo, SymbolQuality, SymbolRelation,
};

use crate::{
    abstract_server::{
        AbstractServer, ErrorDetails, ErrorLayer, FileMatch, Result, ServerError, TextMatchesByFile,
    },
    file_format::analysis::{AnalysisStructured, PathSearchResult},
};

/// Process file, crossref, and fulltext search results into a classic
/// mozsearch mixed results representation consisting of groups of results
/// clustered by "path kind" (normal, test, generated, third party), and then
/// by key/kind precedence (files, IDL, defs, override stuff, super/subclass
/// stuff, assignments, uses, declarations, text matches), noting that
/// precedences will likely change.
///
/// The limits are applied in that order (with the path kinds in the order of
/// the tree's per-file-info.toml: core code, third-party, tests, generated),
/// and then by path, so the results kept are core code's first, like
/// `/search/`'s (router.py's `sort_compiled`), and the later commands (ex:
/// augment-results) only do the work for the results that are shown.  The
/// limits hit are listed in the results.
#[derive(Debug, Args)]
pub struct CompileResults {
    /// Maximum number of file results (file name matches) to list, truncating
    /// at the limit.  These also count toward `line_limit`, like `/search/`'s.
    #[clap(short, long, value_parser, default_value = "4000")]
    file_limit: usize,

    /// Maximum number of result lines to limit, truncating at the limit.
    /// Context lines don't impact this limit.  (`/search/`'s is 4000 too.)
    #[clap(short, long, value_parser, default_value = "4000")]
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
    pub sym_to_meta: UstrMap<AnalysisStructured>,
    pub path_kind_groups: UstrMap<PathKindGroup>,
    /// Every key_line gets added to this set like `{path}:{key_line}` to
    /// suppress redundant hits on the line (from fulltext matches).
    pub path_line_suppressions: HashSet<String>,
    /// The limits the inputs hit (ex: livegrep's on matches).
    pub limits_hit: BTreeSet<String>,
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
        self.sym_to_pretty.insert(info.symbol, info.get_pretty());

        // Skip symbols that are only here for class relationship purposes.
        let (root_sym, relation_facet): (Ustr, &'static Ustr) = match &info.relation {
            SymbolRelation::SubclassOf(_, _)
            | SymbolRelation::SuperclassOf(_, _)
            | SymbolRelation::CousinClassOf(_, _) => {
                return Ok(());
            }
            SymbolRelation::Queried => (info.symbol, &SELF),
            SymbolRelation::OverrideOf(sym, _) => (*sym, &OVERRIDDEN_BY),
            SymbolRelation::OverriddenBy(sym, _) => (*sym, &OVERRIDES),
            SymbolRelation::CousinOverrideOf(sym, _) => (*sym, &COUSIN_OVERRIDES),
        };

        let root_pretty = *self.sym_to_pretty.get(&root_sym).ok_or_else(|| {
            ServerError::StickyProblem(ErrorDetails {
                layer: ErrorLayer::RuntimeInvariantViolation,
                message: format!("no pretty available for root_sym {}", root_sym),
            })
        })?;

        let mut ingest = |kind, path_containers| {
            let descriptor = QualKindDescriptor {
                kind: kind,
                quality: info.quality.clone(),
                pretty: root_pretty,
            };

            if let Some(path_containers) = path_containers {
                for path_container in path_containers {
                    self.ingest_path_hits(
                        &info.symbol,
                        descriptor.clone(),
                        relation_facet,
                        path_container,
                    );
                }
            }
        };
        ingest(PresentationKind::IDL, info.crossref_info.idl);
        ingest(PresentationKind::IDLPartial, info.crossref_info.idl_partial);
        ingest(
            PresentationKind::Definitions,
            info.crossref_info.definitions,
        );
        ingest(
            PresentationKind::Declarations,
            info.crossref_info.declarations,
        );
        ingest(
            PresentationKind::Assignments,
            info.crossref_info.assignments,
        );
        ingest(PresentationKind::Uses, info.crossref_info.uses);

        if let Some(meta) = info.crossref_info.meta {
            self.sym_to_meta.insert(info.symbol, meta);
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
            .or_default();
        let qual_kind_group = path_kind_group
            .qual_kind_groups
            .entry(descriptor)
            .or_insert_with(QualKindGroup::new);

        // ### path faceting
        let path_sans_filename = match path_container.path.rfind('/') {
            Some(offset) => ustr(&path_container.path[0..offset + 1]),
            None => ustr(""),
        };
        let mut path_pieces: Vec<Ustr> =
            path_sans_filename.split_inclusive('/').map(ustr).collect();
        // drop the filename portion.
        if !path_pieces.is_empty() {
            path_pieces.truncate(path_pieces.len() - 1);
        }
        qual_kind_group
            .path_facet
            .place_item(path_pieces, path_sans_filename);

        // ### symbol relation faceting
        qual_kind_group
            .relation_facet
            .place_item(vec![*relation_facet], *sym);

        // ### line results
        let file_results = qual_kind_group
            .path_hits
            .entry(path_container.path)
            .or_insert_with(|| FlattenedResultsByFile {
                file: path_container.path,
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
                .entry(file_match.concise.path_kind)
                .or_default();
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
                .or_default();
            let qual_kind_group = path_kind_group
                .qual_kind_groups
                .entry(descriptor.clone())
                .or_insert_with(QualKindGroup::new);

            // ### line results
            let file_results =
                qual_kind_group
                    .path_hits
                    .entry(path)
                    .or_insert_with(|| FlattenedResultsByFile {
                        file: path,
                        line_spans: vec![],
                    });
            for text_match in file_match.matches {
                if self
                    .path_line_suppressions
                    .insert(format!("{}:{}", path, text_match.line_num))
                {
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
            if file_results.line_spans.is_empty() {
                qual_kind_group.path_hits.remove(&path);
            } else {
                // ### path faceting (now that we know we're keeping the hits)
                let path_sans_filename = match path.rfind('/') {
                    Some(offset) => ustr(&path[0..offset + 1]),
                    None => ustr(""),
                };
                let mut path_pieces: Vec<Ustr> =
                    path_sans_filename.split_inclusive('/').map(ustr).collect();
                // drop the filename portion.
                path_pieces.truncate(path_pieces.len() - 1);
                qual_kind_group
                    .path_facet
                    .place_item(path_pieces, path_sans_filename);
            }
        }
    }

    /// The results, limited to `file_limit` file name matches and `line_limit`
    /// lines in all, keeping those of the path kinds in `path_kind_order`
    /// first (and then any others by name), then by kind, then by path.
    pub fn compile(
        self,
        file_limit: usize,
        line_limit: usize,
        path_kind_order: &[Ustr],
    ) -> FlattenedResultsBundle {
        let mut path_kind_groups: Vec<(Ustr, PathKindGroup)> =
            self.path_kind_groups.into_iter().collect();
        path_kind_groups.sort_by_key(|(path_kind, _)| {
            let order = path_kind_order.iter().position(|kind| kind == path_kind);
            (
                order.unwrap_or(path_kind_order.len()),
                path_kind.to_string(),
            )
        });
        let mut limits_hit = self.limits_hit;
        let (mut files, mut lines) = (0, 0);

        let mut path_kind_results = vec![];
        for (path_kind, pk_group) in path_kind_groups {
            let mut file_names = pk_group.file_names;
            let room = file_limit
                .saturating_sub(files)
                .min(line_limit.saturating_sub(lines));
            if file_names.len() > room {
                file_names.truncate(room);
                limits_hit.insert(if files + room >= file_limit {
                    format!("file limit ({} files)", file_limit)
                } else {
                    format!("result count limit ({} results)", line_limit)
                });
            }
            files += file_names.len();
            lines += file_names.len();

            let mut kind_groups = vec![];
            for (descriptor, qk_group) in pk_group.qual_kind_groups {
                let mut facets = vec![];

                if let Some(facet) = qk_group.relation_facet.compile() {
                    facets.push(facet);
                }
                if let Some(facet) = qk_group.path_facet.compile() {
                    facets.push(facet);
                }

                let mut by_file: Vec<FlattenedResultsByFile> = vec![];
                for mut results in qk_group.path_hits.into_values() {
                    // Several results can be the same line (ex: several
                    // structured records of a declaration), which is one
                    // result, with the union of their ranges.
                    results
                        .line_spans
                        .sort_by_key(|x| (x.key_line, x.line_range));
                    results.line_spans.dedup_by(|later, earlier| {
                        if later.key_line != earlier.key_line {
                            return false;
                        }
                        earlier.line_range = (
                            earlier.line_range.0.min(later.line_range.0),
                            earlier.line_range.1.max(later.line_range.1),
                        );
                        true
                    });
                    // The path_hits within each file are not guaranteed to be
                    // sorted, so we sort them now.
                    results.line_spans.sort_by_key(|x| x.line_range);
                    let room = line_limit.saturating_sub(lines);
                    if results.line_spans.len() > room {
                        results.line_spans.truncate(room);
                        limits_hit.insert(format!("result count limit ({} results)", line_limit));
                    }
                    lines += results.line_spans.len();
                    if !results.line_spans.is_empty() {
                        by_file.push(results);
                    }
                }

                if !by_file.is_empty() {
                    kind_groups.push(FlattenedKindGroupResults {
                        kind: descriptor.kind,
                        pretty: descriptor.pretty,
                        facets,
                        by_file,
                    });
                }
            }

            if !file_names.is_empty() || !kind_groups.is_empty() {
                path_kind_results.push(FlattenedPathKindGroupResults {
                    path_kind,
                    file_names,
                    kind_groups,
                });
            }
        }

        FlattenedResultsBundle {
            path_kind_results,
            content_type: "text/plain".to_string(),
            limits_hit: limits_hit.into_iter().collect(),
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
        server: &(dyn AbstractServer + Send + Sync),
        input: Vec<(String, PipelineValues)>,
    ) -> Result<PipelineValues> {
        let mut results = SearchResults::default();

        // We currently don't care about the name of the input because we only
        // match by type, but one could imagine a scenario in which they serve
        // as labels we want to propagate.
        for (_, pipe_value) in input {
            match pipe_value {
                PipelineValues::FileMatches(fm) => {
                    if fm.limit_hit {
                        results
                            .limits_hit
                            .insert("file search hit limit".to_string());
                    }
                    results.ingest_file_match_hits(fm.file_matches);
                }
                PipelineValues::SymbolCrossrefInfoList(scil) => {
                    for info in scil.symbol_crossref_infos {
                        results.ingest_symbol(info)?;
                    }
                }
                PipelineValues::TextMatches(tm) => {
                    if tm.limit_hit {
                        results
                            .limits_hit
                            .insert("fulltext search hit limit".to_string());
                    }
                    if tm.timed_out {
                        results
                            .limits_hit
                            .insert("fulltext search timeout".to_string());
                    }
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

        // The tree's path kinds' order (see per-file-info.toml), or router.py's.
        let mut path_kind_order: Vec<Ustr> = server
            .path_kinds()
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        if path_kind_order.is_empty() {
            path_kind_order = ["normal", "third_party", "test", "generated"]
                .into_iter()
                .map(ustr)
                .collect();
        }
        let results_bundle =
            results.compile(self.args.file_limit, self.args.line_limit, &path_kind_order);

        Ok(PipelineValues::FlattenedResultsBundle(results_bundle))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abstract_server::{TextBounds, TextMatchInFile};
    use crate::file_format::analysis::{LineRange, SearchResult};

    fn text_hits(path: &str, path_kind: &str, lines: u32) -> TextMatchesByFile {
        TextMatchesByFile {
            file: ustr(path),
            path_kind: ustr(path_kind),
            matches: (1..=lines)
                .map(|line_num| TextMatchInFile {
                    line_num,
                    bounds: TextBounds {
                        start: 0,
                        end_exclusive: 1,
                    },
                    line_str: format!("line {}", line_num),
                })
                .collect(),
        }
    }

    #[test]
    fn test_compile_limits() {
        let mut results = SearchResults::default();
        results.ingest_fulltext_hits(vec![
            text_hits("tests/a.js", "test", 2),
            text_hits("b.cpp", "normal", 5),
            text_hits("__GENERATED__/g.h", "generated", 1),
            text_hits("third_party/t.c", "third_party", 2),
        ]);
        let order: Vec<Ustr> = ["normal", "third_party", "test", "generated"]
            .into_iter()
            .map(ustr)
            .collect();
        let bundle = results.compile(10, 6, &order);
        // Core code's 5 lines, then 1 of third-party's 2, and none of the
        // tests' or generated code's.
        let kept: Vec<(&str, usize)> = bundle
            .path_kind_results
            .iter()
            .map(|pk| {
                let lines = pk.kind_groups.iter().flat_map(|kg| &kg.by_file);
                (
                    pk.path_kind.as_str(),
                    lines.map(|f| f.line_spans.len()).sum(),
                )
            })
            .collect();
        assert_eq!(kept, vec![("normal", 5), ("third_party", 1)]);
        assert_eq!(bundle.limits_hit, vec!["result count limit (6 results)"]);

        // Under the limits, everything is kept in that order.
        let mut results = SearchResults::default();
        results.ingest_fulltext_hits(vec![
            text_hits("tests/a.js", "test", 2),
            text_hits("b.cpp", "normal", 1),
        ]);
        let bundle = results.compile(10, 10, &order);
        let kinds: Vec<&str> = bundle
            .path_kind_results
            .iter()
            .map(|pk| pk.path_kind.as_str())
            .collect();
        assert_eq!(kinds, vec!["normal", "test"]);
        assert!(bundle.limits_hit.is_empty());
    }

    #[test]
    fn test_compile_merges_same_lines() {
        let line = |lineno: u32, peek: (u32, u32)| SearchResult {
            lineno,
            bounds: (0, 1),
            line: format!("line {}", lineno),
            context: ustr("C"),
            contextsym: ustr("#C"),
            peek_range: LineRange {
                start_lineno: peek.0,
                end_lineno: peek.1,
            },
        };
        let mut results = SearchResults::default();
        // A declaration's several records on line 7 (ex: one with a peek
        // range, one without), and another result on line 9.
        results.ingest_path_hits(
            &ustr("#C"),
            QualKindDescriptor {
                kind: PresentationKind::Declarations,
                quality: SymbolQuality::ExplicitSymbol,
                pretty: ustr("C"),
            },
            &ustr("Self"),
            PathSearchResult {
                path: ustr("a.h"),
                path_kind: ustr("normal"),
                lines: vec![
                    line(9, (0, 0)),
                    line(7, (7, 12)),
                    line(7, (0, 0)),
                    line(7, (6, 8)),
                ],
            },
        );
        let bundle = results.compile(10, 10, &[ustr("normal")]);
        let spans: Vec<(u32, (u32, u32))> = bundle.path_kind_results[0].kind_groups[0].by_file[0]
            .line_spans
            .iter()
            .map(|span| (span.key_line, span.line_range))
            .collect();
        assert_eq!(spans, vec![(7, (6, 12)), (9, (9, 9))]);
    }
}
