use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::env;
use std::fmt::Write as _;
use std::io::Write;
use std::ops::{Deref, Range, RangeInclusive};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::time::Instant;

use crate::abstract_server::{AbstractServer, FileMatch, ServerError};
use crate::blame;
use crate::cmd_pipeline::facets::{FacetFile, FacetView, PathKinds, file_facets};
use crate::commit_index::{CommitIndex, CommitRef};
use crate::file_format::analysis_manglings::make_file_sym_from_path;
use crate::file_format::bisectable_mmap::BisectableMmap;
use crate::file_format::chunked_gzip::LINES_PER_CHUNK;
use crate::file_format::code_coverage_report;
use crate::file_format::coverage::InterpolatedCoverage;
use crate::file_format::crossref::CrossrefData;
use crate::file_format::doc_trees::{DocTrees, read_doc_trees};
use crate::file_format::jumpref::{JumprefData, JumprefSource, JumprefTraversals, TreeJumprefs};
use crate::file_format::per_file_info::{PathFacetMap, path_facet_map};
use crate::file_format::repo_data_ingestion::ConcisePerFileInfo;
use crate::git_ops::{self, coverage_history, coverage_summary, git_time_to_chrono};
use crate::hyperblame::explore::{self, ExploreCommit, ExploreFile, blot_files, commit_changes};
use crate::hyperblame::file_changes::{ScopeChange, file_changes};
use crate::hyperblame::future;
use crate::hyperblame::interdiff::{
    self, Counts as InterdiffCounts, IdToken, Mark as InterdiffMark, Side as InterdiffSideTokens,
    TokenId,
};
use crate::hyperblame::outline_diff;
use crate::hyperblame::page_blame::{
    CommitMeta, PageBlame, StripLine, interpolate_blank_lines, page_blame,
};
use crate::hyperblame::page_data_cache::{PAGE_DATA_CACHE, PageData};
use crate::hyperblame::peephole::{self, Cursor, peephole_page};
use crate::hyperblame::token_blame::{TreeHistory, blame_tokens};
use crate::languages;
use crate::languages::FormatAs;
use crate::links;
use crate::rendered_docs::{self, Rendered};
use crate::rendered_specs::{self, SpecLinks};
use crate::spec_analyze::{SpecEntry, SpecsConfig};
use crate::templating::builder::{
    build_and_parse_coverage_history, build_and_parse_dir_listing, build_and_parse_explore,
    build_and_parse_interdiff_files, build_and_parse_outline_diff,
};
use crate::tokenize;
use crate::tree_sitter_support::config_tokenizer::unescape_name;
use crate::utils::OwnedOrBorrowed;

use crate::file_format::analysis::{
    AnalysisSource, BindingImplKind, BindingSlotLang, ExpansionInfo, WithLocation,
    collect_file_syms_from_source,
};
use crate::file_format::config::{self, Config, GitData, SearchEndpoint, TreeConfig};
use crate::file_format::history::syntax_files::{split_token_line, token_file_lines};
use crate::output::{
    self, BreadcrumbsLinksTo, F, Options, PanelItem, PanelItemLabel, PanelSection, RevisionData,
};
use crate::url_encode_path::url_encode_path;

use git2::{Oid, Repository, Tree, TreeEntry};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use serde_json::{json, to_string, to_string_pretty};
use ustr::{Ustr, UstrMap, ustr};

#[derive(Debug)]
pub struct FormattedLine {
    pub line: String,
    // If this line should open a new <div> and its <code> line should be position: sticky, this
    // has a String which is the symbol starting the nest.
    pub sym_starts_nest: Option<Ustr>,
    // This line should close this many <div>'s.
    pub pop_nest_count: u32,
}

/// Splits string literal tokens at the analysis records within them (ex: a
/// data-l10n-id in markup in a JS string, or a quoted JS object key's
/// property), so that the records get tokens, and so spans, of their own, and
/// likewise comments' lines (ex: SpiderMonkey's `[SMDOC]` references) and
/// plain text's (ex: a document's references to Sphinx labels), whose
/// records' tokens are like identifiers'.
fn split_strings_at_records(
    input: &str,
    tokens: Vec<tokenize::Token>,
    analysis: &[WithLocation<Vec<AnalysisSource>>],
) -> Vec<tokenize::Token> {
    let mut result = Vec::with_capacity(tokens.len());
    let mut line_start = 0;
    let mut cur_line = 1;
    let mut cur_datum = 0;
    for token in tokens {
        if token.kind == tokenize::TokenKind::Newline {
            cur_line += 1;
            line_start = token.end;
            result.push(token);
            continue;
        }
        let record_kind = match token.kind {
            tokenize::TokenKind::StringLiteral => tokenize::TokenKind::StringLiteral,
            tokenize::TokenKind::Comment => tokenize::TokenKind::Comment,
            tokenize::TokenKind::PlainText => tokenize::TokenKind::Identifier(None),
            _ => {
                result.push(token);
                continue;
            }
        };
        let column = (token.start - line_start) as u32;
        let end_column = (token.end - line_start) as u32;
        while cur_datum < analysis.len()
            && (analysis[cur_datum].loc.lineno < cur_line
                || (analysis[cur_datum].loc.lineno == cur_line
                    && analysis[cur_datum].loc.col_start < column))
        {
            cur_datum += 1;
        }
        let mut pos = token.start;
        while cur_datum < analysis.len()
            && analysis[cur_datum].loc.lineno == cur_line
            && analysis[cur_datum].loc.col_start < end_column
        {
            let loc = &analysis[cur_datum].loc;
            let start = line_start + loc.col_start as usize;
            let end = (line_start + loc.col_end as usize).min(token.end);
            // (Overlapping records, or ones whose columns aren't byte offsets,
            // stay in the string.)
            if start >= pos
                && end > start
                && input.is_char_boundary(start)
                && input.is_char_boundary(end)
            {
                if start > pos {
                    result.push(tokenize::Token {
                        start: pos,
                        end: start,
                        kind: token.kind.clone(),
                    });
                }
                result.push(tokenize::Token {
                    start,
                    end,
                    kind: record_kind.clone(),
                });
                pos = end;
            }
            cur_datum += 1;
        }
        if pos < token.end {
            result.push(tokenize::Token {
                start: pos,
                end: token.end,
                kind: token.kind,
            });
        }
    }
    result
}

/// Renders source code into a Vec of HTML-formatted lines wrapped in `FormattedLine` objects that
/// provide the metadata for the position:sticky post-processing step.  Caller is responsible
/// for generating line numbers and any blame information.
pub fn format_code(
    cfg: Option<&Config>,
    jumpref_lookup: &dyn JumprefSource,
    format: FormatAs,
    path: &str,
    input: &str,
    analysis: &[WithLocation<Vec<AnalysisSource>>],
) -> (Vec<FormattedLine>, String) {
    let tokens = match format {
        FormatAs::Binary => panic!("Unexpected binary file"),
        FormatAs::CSS => tokenize::tokenize_css(input),
        FormatAs::Plain => tokenize::tokenize_plain(input),
        FormatAs::YAML => tokenize::tokenize_yaml(input),
        FormatAs::Fluent => tokenize::tokenize_fluent(input),
        FormatAs::FormatCLike(spec) => tokenize::tokenize_c_like(input, spec),
        FormatAs::FormatXPIDL(spec, cdata_spec) => {
            tokenize::tokenize_xpidl(input, spec, cdata_spec)
        }
        FormatAs::FormatTagLike(script_spec) => tokenize::tokenize_tag_like(input, script_spec),
    };
    let tokens = split_strings_at_records(input, tokens, analysis);

    let mut output_lines = Vec::new();
    let mut output = String::new();
    let mut last = 0;

    // The stack of AnalysisSource records that had a valid, non-redundant nesting_range.
    // (It's possible for a single source line to start multiple nesting ranges, but since our
    // use case is making the entire line position:sticky, it only makes sense to create a single
    // range in that case.)
    let mut nesting_stack: Vec<&AnalysisSource> = Vec::new();
    let mut starts_nest: Option<Ustr> = None;

    fn fixup(s: String) -> String {
        s.replace("\r", "\u{21A9}") // U+21A9 = LEFTWARDS ARROW WITH HOOK.
    }

    let mut line_start = 0;
    let mut cur_line = 1;

    let mut cur_datum = 0;

    // The analysis records for the file itself are generated at the beginning.
    // They shouldn't be associated with the actual tokens.
    while cur_datum < analysis.len() && analysis[cur_datum].loc.is_file_target() {
        cur_datum += 1;
    }

    fn entity_replace(s: String) -> String {
        s.replace("&", "&amp;").replace("<", "&lt;")
    }

    // The SYM_INFO dictionary we output into the HTML which provides the symbol
    // information required to populate the context menu as well as providing
    // additional metadata for the super navigation panel.  This replaces the
    // previous ANALYSIS_DATA array which combined information from the crossref
    // generated "jumps" file as well as "source records" at the point of each
    // token.
    let mut generated_sym_info = BTreeMap::new();
    let mut jumpref_traversed: UstrMap<JumprefTraversals> = UstrMap::default();

    // Stuff the file's own info in the symbol info map.
    let file_sym = make_file_sym_from_path(path);
    if let Ok(jumpref) = jumpref_lookup.lookup(&file_sym) {
        generated_sym_info.insert(ustr(&file_sym), jumpref);
        jumpref_traversed.insert(ustr(&file_sym), JumprefTraversals::empty());
    }

    let mut last_pos = 0;

    for token in tokens {
        //let word = &input[token.start .. token.end];
        //println!("TOK {:?} '{}' {}", token, word, last_pos);

        assert!(last_pos <= token.start);
        assert!(token.start <= token.end);
        last_pos = token.end;

        if token.kind == tokenize::TokenKind::Newline {
            output.push_str(&input[last..token.start]);

            // Pop nesting symbols whose end is on the NEXT line.  That is, it doesn't make
            // sense for the position:sticky overlay to cover up the line that contains the
            // token that closes the nesting range.
            //
            // The check below accomplishes this by scanning until we find an (endline - 1)
            // that is beyond the current line.
            let truncate_to = match nesting_stack
                .iter()
                .rposition(|a| a.nesting_range.end_lineno - 1 > cur_line)
            {
                Some(first_keep) => first_keep + 1,
                None => 0,
            };
            let pop_count = nesting_stack.len() - truncate_to;
            nesting_stack.truncate(truncate_to);

            output_lines.push(FormattedLine {
                line: fixup(output),
                sym_starts_nest: starts_nest.take(),
                pop_nest_count: pop_count as u32,
            });
            output = String::new();

            cur_line += 1;
            line_start = token.end;
            last = token.end;

            continue;
        }

        let column = (token.start - line_start) as u32;

        // Advance cur_datum as long as analysis[cur_datum] is pointing
        // to tokens we've already gone past. This effectively advances
        // cur_datum such that `analysis[cur_datum]` is the analysis data
        // for our current token (if there is any).
        while cur_datum < analysis.len() && cur_line > analysis[cur_datum].loc.lineno {
            cur_datum += 1
        }
        while cur_datum < analysis.len()
            && cur_line == analysis[cur_datum].loc.lineno
            && column > analysis[cur_datum].loc.col_start
        {
            cur_datum += 1
        }

        let datum = if cur_datum < analysis.len()
            && cur_line == analysis[cur_datum].loc.lineno
            && column == analysis[cur_datum].loc.col_start
        {
            let r = &analysis[cur_datum].data;
            cur_datum += 1;
            Some(r)
        } else {
            None
        };

        match (&token.kind, datum) {
            (&tokenize::TokenKind::Identifier(_), Some(d))
            | (&tokenize::TokenKind::StringLiteral, Some(d))
            | (&tokenize::TokenKind::Comment, Some(d)) => {
                for a in d.iter() {
                    // If this symbol starts a relevant nesting range and we haven't already pushed a
                    // symbol for this line, push it onto our stack.  Note that the nesting_range
                    // identifies the start/end brace which may not be on the same line as the symbol,
                    // but since we want the symbol to be the thing that's sticky, we start the range
                    // on the symbol.
                    //
                    // A range is "relevant" if:
                    // - It has a valid nesting_range.  (Empty ranges have 0 lineno's for start/end.)
                    // - The range start is on this line or after this line.
                    // - Its end line is not on the current line or the next line and therefore will
                    //   actually trigger the "position:sticky" display scenario.
                    let nests = match (a.nesting_range.start_lineno, nesting_stack.last()) {
                        (0, _) => false,
                        (_, None) => true,
                        (a_start, Some(top)) => {
                            a_start >= cur_line
                                && a_start != top.nesting_range.start_lineno
                                && a.nesting_range.end_lineno > cur_line + 1
                        }
                    };
                    if nests {
                        starts_nest = Some(*a.sym.first().unwrap());
                        nesting_stack.push(a);
                    }

                    for sym in &a.sym {
                        if generated_sym_info.contains_key(sym) {
                            continue;
                        }

                        // The Clang plugin provides type information which could be used here,
                        // especially in the non-crossref case.
                        // See bug 2047542 to either use or remove that data.

                        if !a.no_crossref {
                            jumpref_lookup.add_sym_info(
                                *sym,
                                &mut generated_sym_info,
                                &mut jumpref_traversed,
                            );
                        }
                    }
                }
            }
            _ => {}
        }

        let get_symbols =
            |token: &tokenize::Token, datum: &mut dyn Iterator<Item = &AnalysisSource>| {
                match &token.kind {
                    &tokenize::TokenKind::Identifier(_)
                    | &tokenize::TokenKind::StringLiteral
                    | &tokenize::TokenKind::Comment => {
                        // Build the list of symbols for the highlighter.  We do this for all source
                        // records, even ones marked "no_crossref" because we still want to highlight
                        // locals.  These will be emitted into a `data-symbols` attribute below.
                        let (syms, confidences) = {
                            let mut syms = String::new();
                            let mut confidences = Vec::new();
                            // Suppress including the symbol multiple times.  This was possible under the
                            // ANALYSIS_DATA regime where "source" records mapped directly to "searches",
                            // but this may now be moot.
                            let mut seen_syms = Vec::new();
                            for (sym, confidence) in
                                datum.flat_map(|item| item.sym.iter().zip(item.confidences()))
                            {
                                if let Some(index) = seen_syms.iter().position(|s| s == sym) {
                                    confidences[index] = confidence.max(confidences[index]);
                                    continue;
                                }
                                if !seen_syms.is_empty() {
                                    syms.push(',');
                                }
                                seen_syms.push(*sym);
                                syms.push_str(sym);
                                confidences.push(confidence);
                            }
                            (syms, confidences)
                        };

                        if !syms.is_empty() {
                            format!(
                                "data-symbols=\"{}\" data-confidences=\"{}\"",
                                syms.replace('"', "&quot;"),
                                serde_json::to_string(&confidences)
                                    .unwrap()
                                    .replace('"', "&quot;")
                            )
                        } else {
                            "".to_owned()
                        }
                    }
                    _ => String::new(),
                }
            };

        let get_style = |token: &tokenize::Token,
                         datum: &mut dyn Iterator<Item = &AnalysisSource>| {
            let datum: Vec<&AnalysisSource> = datum.collect();
            // (A citation of a spec by an outdated URL, ex: an obsolete spec's
            // or an old host's, or of an anchor its spec no longer has, says
            // so; see `spec_analyze::citations`.)
            let has_syntax = |syntax: &str| {
                datum
                    .iter()
                    .any(|a| a.syntax.iter().any(|s| AsRef::<str>::as_ref(s) == syntax))
            };
            let mut marks = vec![];
            if has_syntax("outdated") {
                marks.push((
                    "syn_outdated",
                    "This cites an outdated URL: a moved or obsolete spec's",
                ));
            }
            if has_syntax("stale") {
                marks.push(("syn_stale", "This cites an anchor its spec no longer has"));
            }
            let style = match token.kind {
                tokenize::TokenKind::Identifier(ref maybe_style) => {
                    let mut has_datum = false;
                    let classes = datum.iter().flat_map(|a| {
                        has_datum = true;
                        a.syntax.iter().flat_map(|s| match s.as_ref() {
                            "type" => vec!["syn_type"],
                            "def" | "decl" | "idl" => vec!["syn_def"],
                            "deflocal" => vec!["syn_deflocal"],
                            "key" => vec!["syn_key"],
                            _ => vec![],
                        })
                    });
                    let classes = classes.collect::<Vec<_>>();
                    if !classes.is_empty() {
                        format!("class=\"{}\" ", classes.join(" "))
                    } else if has_datum {
                        // If the token has analysis record, do not apply keyword.
                        "".to_owned()
                    } else if let Some(style) = maybe_style {
                        style.clone()
                    } else {
                        "".to_owned()
                    }
                }
                tokenize::TokenKind::StringLiteral => "class=\"syn_string\" ".to_owned(),
                tokenize::TokenKind::Comment => "class=\"syn_comment\" ".to_owned(),
                tokenize::TokenKind::TagName => "class=\"syn_tag\" ".to_owned(),
                tokenize::TokenKind::TagAttrName => "class=\"syn_tag\" ".to_owned(),
                tokenize::TokenKind::EndTagName => "class=\"syn_tag\" ".to_owned(),
                tokenize::TokenKind::RegularExpressionLiteral => "class=\"syn_regex\" ".to_owned(),
                _ => "".to_owned(),
            };
            if marks.is_empty() {
                return style;
            }
            let classes = marks.iter().map(|(class, _)| *class).join(" ");
            let title = format!(
                "title=\"{}\" ",
                marks.iter().map(|(_, title)| *title).join("; ")
            );
            match style.strip_prefix("class=\"") {
                Some(rest) => format!("class=\"{} {}{}", classes, rest, title),
                None => format!("class=\"{}\" {}{}", classes, title, style),
            }
        };

        // Only get the symbols and style of the symbols that appear directly in the source code, not in expansions
        // (but also those of other analyses' records there, without expansions, ex: an observer topic's, for
        // a macro naming it).
        let datum_outside_expansions = datum.iter().flat_map(|d| d.iter());
        let has_expansion = |data: &AnalysisSource| {
            matches!(data.expansion_info, Some(ExpansionInfo::ExpandsTo(_)))
        };
        let is_direct =
            |data: &AnalysisSource| has_expansion(data) || data.expansion_info.is_none();
        let (symbols, style) = if datum_outside_expansions.clone().any(has_expansion) {
            let symbols = get_symbols(
                &token,
                &mut datum_outside_expansions.clone().filter(|&a| is_direct(a)),
            );
            let style = get_style(
                &token,
                &mut datum_outside_expansions
                    .clone()
                    .filter(|&a| has_expansion(a)),
            );
            (symbols, style)
        } else {
            let symbols = get_symbols(&token, &mut datum_outside_expansions.clone());
            let style = get_style(&token, &mut datum_outside_expansions.clone());
            (symbols, style)
        };

        let expansion_to_html = |key: &str, platform: &str, input: &str| {
            let mut html = String::new();

            let tokens = match format {
                FormatAs::Binary => panic!("Unexpected binary file"),
                FormatAs::CSS => tokenize::tokenize_css(input),
                FormatAs::Plain => tokenize::tokenize_plain(input),
                FormatAs::YAML => tokenize::tokenize_yaml(input),
                FormatAs::Fluent => tokenize::tokenize_fluent(input),
                FormatAs::FormatCLike(spec) => tokenize::tokenize_c_like(input, spec),
                FormatAs::FormatXPIDL(spec, cdata_spec) => {
                    tokenize::tokenize_xpidl(input, spec, cdata_spec)
                }
                FormatAs::FormatTagLike(script_spec) => {
                    tokenize::tokenize_tag_like(input, script_spec)
                }
            };

            let datum_in_expansion: HashMap<_, _> = datum
                .iter()
                .flat_map(|d| d.iter())
                .flat_map(|data| match data.expansion_info {
                    Some(ExpansionInfo::InExpansionAt(ref offsets)) => Some(
                        offsets
                            .get(key)
                            .and_then(|o| o.get(platform))
                            .into_iter()
                            .flat_map(|v| v.iter())
                            .map(move |&offset| (offset, data)),
                    ),
                    _ => None,
                })
                .flatten()
                .into_group_map();

            let mut last = 0;

            for token in tokens {
                let token_symbols = datum_in_expansion
                    .get(&token.start)
                    .map(Deref::deref)
                    .unwrap_or(&[]);
                let style = get_style(&token, &mut token_symbols.iter().copied());
                let symbols = get_symbols(&token, &mut token_symbols.iter().copied());

                match token.kind {
                    tokenize::TokenKind::Punctuation | tokenize::TokenKind::PlainText => {
                        let mut sanitized = entity_replace(input[last..token.end].to_string());
                        if token.kind == tokenize::TokenKind::PlainText {
                            sanitized = links::linkify_comment(cfg, sanitized);
                        }
                        html.push_str(&sanitized);
                        last = token.end;
                    }
                    _ => {
                        if !style.is_empty() || !symbols.is_empty() {
                            html.push_str(&entity_replace(input[last..token.start].to_string()));
                            html.push_str(&format!("<span {}{}>", style, symbols));
                            let mut sanitized =
                                entity_replace(input[token.start..token.end].to_string());
                            if token.kind == tokenize::TokenKind::Comment
                                || token.kind == tokenize::TokenKind::StringLiteral
                            {
                                sanitized = links::linkify_comment(cfg, sanitized);
                            }
                            html.push_str(&sanitized);
                            html.push_str("</span>");
                            last = token.end;
                        }
                    }
                }
            }

            html.push_str(&entity_replace(input[last..].to_string()));
            html
        };

        let expansions: BTreeMap<_, _> = {
            let expansions = datum_outside_expansions.filter_map(|a| match a.expansion_info {
                Some(ExpansionInfo::ExpandsTo(ref e)) => Some(e),
                _ => None,
            });

            // Turn BTreeMap<String, BTreeMap<String, String>> into Vec<(key: String, (platform: String, expansion: String))> and sort by (key, expansion)
            let mut expansions: Vec<_> = expansions
                .flat_map(|e| {
                    e.iter().flat_map(|(key, expansions)| {
                        expansions.iter().map(move |(platform, expansion)| {
                            (key.to_owned(), (platform.to_owned(), expansion.to_owned()))
                        })
                    })
                })
                .collect();
            expansions.sort_unstable_by(|a, b| Ord::cmp(&(&a.0, &a.1.1), &(&b.0, &b.1.1)));

            // Format expansions into html
            let expansions = expansions.into_iter().map(|(key, (platform, expansion))| {
                let html = expansion_to_html(&key, &platform, &expansion);
                (key, (platform, html))
            });

            // Group by key again
            let expansions = expansions.chunk_by(|(key, _)| key.clone());

            // For each key: merge platforms that yielded the same expansion together
            expansions
                .into_iter()
                .map(|(key, expansions)| {
                    // First into a Vec<(platform: String, expansion: String)>
                    let expansions = expansions.fold(
                        Vec::<(String, String)>::new(),
                        |mut expansions, (_symbol, (platform, expansion))| {
                            if let Some((last_platform, last_expansion)) = expansions.last_mut()
                                && *last_expansion == expansion
                            {
                                last_platform.push(' ');
                                last_platform.push_str(&platform);
                                return expansions;
                            }

                            expansions.push((platform.to_owned(), expansion));
                            expansions
                        },
                    );

                    // Then into a BTreeMap<String, String> again
                    let expansions: BTreeMap<_, _> = expansions.into_iter().collect();
                    (key, expansions)
                })
                .collect()
        };

        let expansions = if !expansions.is_empty() {
            format!(
                "data-expansions=\"{}\" ",
                entity_replace(serde_json::to_string(&expansions).unwrap()).replace("\"", "&quot;")
            )
        } else {
            "".to_owned()
        };

        match token.kind {
            tokenize::TokenKind::Punctuation | tokenize::TokenKind::PlainText => {
                let mut sanitized = entity_replace(input[last..token.end].to_string());
                if token.kind == tokenize::TokenKind::PlainText {
                    sanitized = links::linkify_comment(cfg, sanitized);
                }
                output.push_str(&sanitized);
                last = token.end;
            }
            _ => {
                if !expansions.is_empty() || !style.is_empty() || !symbols.is_empty() {
                    output.push_str(&entity_replace(input[last..token.start].to_string()));
                    output.push_str(&format!("<span {}{}{}>", expansions, style, symbols));
                    let mut sanitized = entity_replace(input[token.start..token.end].to_string());
                    if token.kind == tokenize::TokenKind::Comment
                        || token.kind == tokenize::TokenKind::StringLiteral
                    {
                        sanitized = links::linkify_comment(cfg, sanitized);
                    }
                    output.push_str(&sanitized);
                    output.push_str("</span>");
                    last = token.end;
                }
            }
        }
    }

    output.push_str(&entity_replace(input[last..].to_string()));

    if !output.is_empty() {
        output_lines.push(FormattedLine {
            line: fixup(output),
            sym_starts_nest: starts_nest.take(),
            pop_nest_count: nesting_stack.len() as u32,
        });
    }

    // (The history digests are for `/query/`'s results, not the page.)
    for jumpref in generated_sym_info.values_mut().flatten() {
        jumpref.recency = None;
        jumpref.recency_from = None;
    }
    let sym_json = if env::var("MOZSEARCH_DIFFABLE").is_err() {
        to_string(&json!(generated_sym_info)).unwrap()
    } else {
        to_string_pretty(&json!(generated_sym_info)).unwrap()
    };
    (output_lines, sym_json)
}

#[derive(Default)]
pub struct FormatPerfInfo {
    pub format_code_duration_us: u64,
    pub blame_lines_duration_us: u64,
    pub commit_info_duration_us: u64,
    pub format_mixing_duration_us: u64,
}

pub struct FormattedFile {
    pub perf: FormatPerfInfo,
    /// The page's token-centric blame, if it has one, for writing its
    /// hyperblame data files (see `hyperblame_files`).
    pub token_blame: Option<PageBlame>,
}

/// Renders source code with blame annotations and semantic analysis data (if provided).
/// The caller provides the panel sections.  Currently used by `output-file.rs` to statically
/// generate the tip of whatever branch it's on with semantic analysis data, and `format_path` to
/// dynamically generate the contents of a file without semantic analysis data.
#[allow(clippy::too_many_arguments)]
/// The rows of a file's source listing, as `format_file_data` writes them but
/// without coverage or blame, and its SYM_INFO, for files whose listing isn't
/// rendered (ex: the files a try push changes; see
/// `abstract_server::TryOverlay`), so that their lines can be excerpted for
/// results like other files' (see `chunked_gzip::extract_rows`).
pub fn source_rows_html(
    jumprefs: &dyn JumprefSource,
    path: &str,
    source: &str,
    analysis: &[WithLocation<Vec<AnalysisSource>>],
) -> (String, String) {
    let format = languages::select_formatting(path);
    if let FormatAs::Binary = format {
        return (String::new(), "{}".to_string());
    }
    let (lines, sym_json) = format_code(None, jumprefs, format, path, source, analysis);
    let mut html = String::new();
    let mut nest_depth = 0;
    let mut nest_syms: Vec<Ustr> = vec![];
    for (i, line) in lines.iter().enumerate() {
        let lineno = i + 1;
        if let Some(nest_sym) = &line.sym_starts_nest {
            html.push_str(&format!(
                r#"<div class="nesting-container nesting-depth-{}" data-nesting-sym="{}">"#,
                nest_depth,
                nest_sym.replace('"', "&quot;")
            ));
            nest_depth += 1;
            nest_syms.push(*nest_sym);
        }
        let nesting = if lineno > 1 && (lineno as u32 - 1).is_multiple_of(LINES_PER_CHUNK) {
            let syms: Vec<&str> = nest_syms.iter().map(|sym| sym.as_str()).collect();
            format!(
                " data-nesting=\"{}\"",
                syms.join(",").replace('"', "&quot;")
            )
        } else {
            String::new()
        };
        html.push_str(&format!(
            "<div role=\"row\" id=\"line-{}\" class=\"source-line-with-number{}\"{}>\n\
             <div role=\"cell\"><div role=\"button\" aria-expanded=\"false\" class=\"cov-strip cov-no-data\" aria-label=\"uncovered\"></div></div>\n\
             <div role=\"cell\"><div class=\"blame-strip\"></div></div>\n\
             <div role=\"cell\" class=\"line-number\" data-line-number=\"{}\"></div>\n\
             <code role=\"cell\" class=\"source-line\">{}\n</code>\n\
             </div>\n",
            lineno,
            if line.sym_starts_nest.is_some() {
                " nesting-sticky-line"
            } else {
                ""
            },
            nesting,
            lineno,
            line.line
        ));
        for _ in 0..line.pop_nest_count {
            nest_depth -= 1;
            nest_syms.pop();
            html.push_str("</div>");
        }
    }
    (html, sym_json)
}

/// What a file's source listing shows besides its text and analysis (see
/// `format_file_data`): its revision's commit, its blame, its coverage, and
/// the try push whose context its searches are in, from the tree's git
/// repositories and history (see `file_extras`), so that a page can be
/// rendered over a tree's server (see `AbstractServer::rev_file`).
#[derive(Default, Deserialize, Serialize)]
pub struct FileExtras {
    /// The revision, its commit's header (HTML; see `blame::commit_header`),
    /// and its date (RFC 3339).
    pub revision: Option<(String, String, String)>,
    /// The token-centric blame, if the tree's history has it (see
    /// `page_token_blame`), with its data's URLs.
    pub token_blame: Option<PageBlame>,
    /// Else the classic line blame's lines (see `blame::LineData`).
    pub blame_lines: Option<Vec<String>>,
    pub coverage: Option<git_ops::FileCoverage>,
    pub coverage_history: Option<serde_json::Value>,
    /// The revision's coverage summary, if the file has a coverage history.
    pub coverage_summary: Option<code_coverage_report::NodeMetadata>,
    /// The try push whose context its searches are in (a try push's rebased
    /// head's; see `try_delta`).
    pub search_rev: Option<String>,
    /// For a try push's rebased head, the push's stack context (ex:
    /// "try/hg/abcdef012345"; see `try_import::push_url_path`), whose stack
    /// strip its pages show (see stack-strip.js), whether or not their URLs
    /// say `?stack=`.
    #[serde(default)]
    pub stack: Option<String>,
    /// For a try push's own commit, the push and the page of the file in its
    /// stack rebased onto the indexed revision, which has its semantic
    /// analysis (see `try_delta`): `(push, URL)`.
    #[serde(default)]
    pub rebased_file: Option<(String, String)>,
    #[serde(skip)]
    pub blame_lines_duration_us: u64,
    #[serde(skip)]
    pub commit_info_duration_us: u64,
}

/// The `FileExtras` of the file at `path`, whose text is `data`, in `commit`
/// (and its blame's and coverage's commits), for a page whose links go to
/// `breadcrumbs_links_to`.
#[allow(clippy::too_many_arguments)]
pub fn file_extras(
    cfg: &Config,
    tree_name: &str,
    commit: &Option<git2::Commit>,
    breadcrumbs_links_to: BreadcrumbsLinksTo,
    blame_commit: &Option<git2::Commit>,
    coverage_commit: Option<&git2::Commit>,
    path: &str,
    data: &str,
) -> Result<FileExtras, &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let coverage = git_ops::get_coverage(tree_config.git.as_ref(), coverage_commit, path);

    let pre_blame_lines = Instant::now();
    // We use the token-centric history if the tree has it (and it has the
    // file), and otherwise the classic line blame; trees only have one or the
    // other (see `config::git_data`).
    let mut token_blame = tree_config
        .git
        .as_ref()
        .zip(commit.as_ref())
        .and_then(|(git, commit)| page_token_blame(git, commit, path, data));
    if let (Some(page), Some(commit), Some(git)) = (&mut token_blame, commit, &tree_config.git) {
        set_token_blame_urls(page, tree_name, commit, path, breadcrumbs_links_to);
        set_file_changes(git, commit, path, page);
    }
    let blame_lines = match token_blame {
        Some(_) => None,
        None => git_ops::get_blame_lines(tree_config.git.as_ref(), blame_commit, path),
    };
    let blame_lines_duration_us = pre_blame_lines.elapsed().as_micros() as u64;

    let pre_commit = Instant::now();
    let revision = match *commit {
        Some(ref commit) => Some((
            commit.id().to_string(),
            blame::commit_header(commit)?,
            git_time_to_chrono(commit.time()).to_rfc3339(),
        )),
        None => None,
    };
    let commit_info_duration_us = pre_commit.elapsed().as_micros() as u64;

    // The head of a try push's stack rebased onto the indexed revision has
    // the push's semantic delta (see `try_delta`), and its pages' searches
    // are of the push.
    let search_rev = revision
        .as_ref()
        .map(|(rev, _, _)| rev.clone())
        .filter(|rev| {
            crate::try_delta::delta_dir(&tree_config.paths.index_path, rev)
                .join("meta.json")
                .exists()
        });

    let stack = search_rev
        .as_deref()
        .zip(tree_config.git.as_ref())
        .and_then(|(head, git)| crate::try_import::rebased_head_push(&git.repo, head))
        .map(|push| crate::try_import::push_url_path(&push));

    // (A try push's own commits' pages point to the push's rebased stack's,
    // which have its analysis.)
    let rebased_file = match (&search_rev, &revision, &tree_config.git) {
        (None, Some((rev, _, _)), Some(git)) => crate::try_import::commit_push(cfg, tree_name, rev)
            .and_then(|push| {
                let head = crate::try_import::semantic_head(cfg, tree_name, &push)?;
                git.history
                    .as_ref()
                    .filter(|history| history_has(history, &head))?;
                let tree = git
                    .repo
                    .find_commit(Oid::from_str(&head).ok()?)
                    .ok()?
                    .tree()
                    .ok()?;
                tree.get_path(Path::new(path)).ok()?;
                let url = format!(
                    "/{}/rev/{}/{}?stack={}",
                    tree_name,
                    head,
                    url_encode_path(path),
                    crate::try_import::push_url_path(&push)
                );
                Some((push, url))
            }),
        _ => None,
    };

    let coverage_history = coverage_history(tree_config.git.as_ref(), path);
    let coverage_summary = match coverage_history {
        Some(_) => coverage_commit.as_ref().and_then(|commit| {
            let coverage_rev = commit.id().to_string();
            git_ops::coverage_summary(tree_config.git.as_ref(), &coverage_rev, path)
        }),
        None => None,
    };
    Ok(FileExtras {
        revision,
        token_blame,
        blame_lines,
        coverage,
        coverage_history: coverage_history.and_then(|history| serde_json::to_value(history).ok()),
        coverage_summary,
        search_rev,
        stack,
        rebased_file,
        blame_lines_duration_us,
        commit_info_duration_us,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn format_file_data(
    cfg: &Config,
    tree_name: &str,
    mut panel: Vec<PanelSection>,
    info_boxes: String,
    extras: FileExtras,
    breadcrumbs_links_to: BreadcrumbsLinksTo,
    path: &str,
    data: String,
    jumpref_lookup: &dyn JumprefSource,
    analysis: &[WithLocation<Vec<AnalysisSource>>],
    writer: &mut dyn Write,
) -> Result<FormattedFile, &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;

    let coverage = extras.coverage;

    let mut format_perf = FormatPerfInfo {
        blame_lines_duration_us: extras.blame_lines_duration_us,
        commit_info_duration_us: extras.commit_info_duration_us,
        ..FormatPerfInfo::default()
    };

    let format = languages::select_formatting(path);
    if let FormatAs::Binary = format {
        write!(writer, "Binary file").unwrap();
        return Ok(FormattedFile {
            perf: format_perf,
            token_blame: None,
        });
    };

    let slug = format_to_slug_attribute(&format);
    let stack_attribute = extras.stack.as_ref().map_or_else(String::new, |stack| {
        format!(
            r#" data-stack="{}""#,
            stack.replace('&', "&amp;").replace('"', "&quot;")
        )
    });
    let pre_format_code = Instant::now();
    let (output_lines, sym_json) =
        format_code(Some(cfg), jumpref_lookup, format, path, &data, analysis);
    format_perf.format_code_duration_us = pre_format_code.elapsed().as_micros() as u64;

    let token_blame = extras.token_blame.filter(|page| {
        let matches = page.lines.len() == output_lines.len();
        if !matches {
            log::warn!(
                "Not using token blame for {}: {} lines but {} formatted lines",
                path,
                page.lines.len(),
                output_lines.len()
            );
        }
        matches
    });
    let blame_lines = extras.blame_lines;

    let revision = match &extras.revision {
        Some((rev, header, date)) => Some(RevisionData {
            rev: rev.as_str(),
            desc: header.as_str(),
            date: chrono::DateTime::parse_from_rfc3339(date).map_err(|_| "Bad date")?,
        }),
        None => None,
    };

    let pre_format_mixing = Instant::now();

    let path_wrapper = Path::new(path);
    let filename = path_wrapper.file_name().unwrap().to_str().unwrap();

    let title = format!("{} - mozsearch", filename);
    let search_rev = extras.search_rev.as_deref();
    let opt = Options {
        title: &title,
        tree_name,
        include_date: env::var("MOZSEARCH_DIFFABLE").is_err(),
        revision,
        breadcrumbs_links_to,
        extra_content_classes: "source-listing not-diff",
        search_rev,
    };

    output::generate_header(&opt, writer)?;

    let file_syms = collect_file_syms_from_source(path, analysis);

    output::generate_breadcrumbs(&opt, writer, path, &file_syms, !analysis.is_empty())?;

    if let Some(coverage_history) = &extras.coverage_history {
        let liquid_globals = liquid::object!({
            "tree": tree_name,
            "path": path,
            "coverage_history": liquid::model::to_value(coverage_history)
                .map_err(|_| "Template problems")?,
        });

        let template = build_and_parse_coverage_history();
        template
            .render_to(writer, &liquid_globals)
            .or(Err("Template problems"))?;

        add_coverage_panel_item(&mut panel, extras.coverage_summary.as_ref());
    }
    output::generate_panel(&opt, writer, &panel, false)?;

    let info_boxes = match &extras.rebased_file {
        Some((push, url)) => format!(
            r#"<section class="info-box info-box-info try-rebased-file">This revision is a commit of <a href="/{}/{}">{}</a>, whose stack rebased onto the indexed revision has its semantic analysis: <a href="{}">see this file there</a>, with jumps and references, the push's blame, and its stack strip.</section>{}"#,
            tree_name,
            crate::try_import::push_url_path(push),
            entity_escape(&crate::try_import::push_title(cfg, tree_name, push).0),
            url.replace('&', "&amp;").replace('"', "&quot;"),
            info_boxes
        ),
        None => info_boxes,
    };
    let info_boxes_container = F::Seq(vec![
        F::S(r#"<section class="info-boxes" id="info-boxes-container">"#),
        F::Indent(vec![F::T(info_boxes)]),
        F::S("</section>"),
    ]);
    output::generate_formatted(writer, &info_boxes_container, 0)?;

    if let Some(ext) = path_wrapper.extension()
        && ext.to_str().unwrap() == "svg"
        && let Some(url) = tree_config.paths.make_raw_resource_branch_url(path)
    {
        output::generate_svg_preview(writer, &url)?
    }

    let f = F::Seq(vec![F::T(format!(
        "<div id=\"file\" class=\"file\" role=\"table\"{}{}>",
        slug, stack_attribute
    ))]);

    output::generate_formatted(writer, &f, 0).unwrap();

    // Map blame revisions to consecutive integer identifiers so that our aria
    // labels for screen readers can have a more human friendly identifier than
    // (some portion of) the git hash.
    let mut blame_hash_to_human_id = HashMap::new();
    let mut next_human_id = 1;

    // Blame lines and source lines are now interleaved.  Since we already have fully rendered the
    // source above, we output the blame info, line number, and rendered HTML source as we process
    // each line for blame purposes.
    let mut last_revs = None;
    let mut last_color = false;
    let mut last_commit = None;
    let mut nest_depth = 0;
    // The symbols of the nesting containers around the current line (see
    // `chunked_gzip`'s use of `data-nesting`).
    let mut nest_syms: Vec<Ustr> = vec![];
    for (i, line) in output_lines.iter().enumerate() {
        let lineno = i + 1;

        // Compute the coverage data for this line (if any)
        let coverage_data: String = if let Some(ref coverage) = coverage {
            // There's 2 levels of not having data for a line here:
            // 1. We had no coverage data, coverage is None.  In that case,
            //    we'll take the else case.
            // 2. We have coverage data (coverage is Some(x)), but the array
            //    has no data for this line.  This should only happen if the
            //    coverage data is for a different revision control revision
            //    than the source code.
            use InterpolatedCoverage::*;
            match coverage.lines.get(i) {
                None => r#" class="cov-strip cov-uncovered cov-unknown" aria-label="missing data""#
                    .to_owned(),
                Some(InterpolatedMiss) => {
                    r#" class="cov-strip cov-miss cov-interpolated" aria-label="uncovered""#
                        .to_owned()
                }
                Some(InterpolatedHit) => {
                    r#" class="cov-strip cov-hit cov-interpolated" aria-label="uncovered""#
                        .to_owned()
                }
                Some(Uncovered) => {
                    r#" class="cov-strip cov-uncovered cov-known" aria-label="uncovered""#
                        .to_owned()
                }
                Some(Covered(0)) => {
                    r#" class="cov-strip cov-miss cov-known" aria-label="miss" data-coverage="0""#
                        .to_owned()
                }
                // Should this directly be a CSS variable?
                Some(&Covered(x)) => {
                    let (hit_count, log_hit_count, precision_class) = if coverage.exact {
                        (x, (x + 1).ilog10(), "cov-exact")
                    } else {
                        (10_u32.pow(x - 1), x, "cov-approx")
                    };

                    format!(
                        r#" class="cov-strip cov-hit cov-known cov-log10-{} {}" aria-label="hit {}{}" data-coverage="{}""#,
                        log_hit_count,
                        precision_class,
                        if hit_count < 1000 {
                            hit_count
                        } else {
                            hit_count / 1000
                        },
                        if hit_count < 1000 { "" } else { "k" },
                        x,
                    )
                }
            }
        } else {
            r#" class="cov-strip cov-no-data" aria-label="uncovered""#.to_owned()
        };

        // Compute the blame data for this line (if any)
        let blame_data = if let Some(ref page) = token_blame {
            let strip = &page.lines[i];
            // Like below, we alternate colors whenever the commit changes, but
            // lines without a commit don't get a color.
            let (color_class, aria_label) = match strip.commit {
                Some(commit) => {
                    let same_commit_as_last = last_commit == Some(commit);
                    if !same_commit_as_last {
                        last_color = !last_color;
                    }
                    last_commit = Some(commit);
                    let human_id = blame_hash_to_human_id
                        .entry(commit.to_string())
                        .or_insert_with(|| {
                            let id = next_human_id;
                            next_human_id += 1;
                            id
                        });
                    (
                        if last_color { " c1" } else { " c2" },
                        format!(
                            "{} hash {}",
                            if same_commit_as_last { "same" } else { "new" },
                            human_id
                        ),
                    )
                }
                None => ("", "no tokens".to_string()),
            };
            let (classes, data) = strip.strip_attrs();
            format!(
                r#" class="blame-strip{}{}"{} role="button" aria-label="{}" aria-expanded="false""#,
                color_class, classes, data, aria_label,
            )
        } else if let Some(ref lines) = blame_lines {
            let blame_line = blame::LineData::deserialize(&lines[i]);

            // These store the final data we ship to the front-end.
            // Each of these is a comma-separated list with one element
            // for each blame entry. Currently they only contain one
            // element ever, since the blame-skipping implementation wasn't
            // very good and was removed.
            let revs = blame_line.rev.to_string();
            let filespecs = blame_line.path.to_string();
            let blame_linenos = blame_line.lineno.to_string();

            let human_id = blame_hash_to_human_id
                .entry(revs.clone())
                .or_insert_with(|| {
                    let id = next_human_id;
                    next_human_id += 1;
                    id
                });

            let same_rev_as_last = last_revs.is_some_and(|last| last == revs);
            let color = if same_rev_as_last {
                last_color
            } else {
                !last_color
            };
            last_revs = Some(revs.clone());
            last_color = color;
            let class = if color { 1 } else { 2 };
            let data = format!(
                r#" class="blame-strip c{}" data-blame="{}#{}#{}" role="button" aria-label="{} hash {}" aria-expanded="false""#,
                class,
                revs,
                filespecs,
                blame_linenos,
                if same_rev_as_last { "same" } else { "new" },
                human_id,
            );
            data
        } else {
            " class=\"blame-strip\"".to_owned()
        };

        // If this line starts nesting, we need to create a div that exists strictly to contain the
        // position:sticky element.
        if let Some(nest_sym) = &line.sym_starts_nest {
            write!(
                writer,
                r#"<div class="nesting-container nesting-depth-{}" data-nesting-sym="{}">"#,
                nest_depth,
                nest_sym.replace('"', "&quot;")
            )
            .unwrap();
            nest_depth += 1;
            nest_syms.push(*nest_sym);
        }

        // The rows that can start the gzip's chunks (see `chunked_gzip`) say
        // which nesting containers they're in, since their opening tags are
        // in earlier chunks, so that `/query/`'s results can say which
        // symbol (ex: function) each of their lines is in.
        let nesting = if lineno > 1 && (lineno as u32 - 1).is_multiple_of(LINES_PER_CHUNK) {
            let syms: Vec<&str> = nest_syms.iter().map(|sym| sym.as_str()).collect();
            format!(
                " data-nesting=\"{}\"",
                syms.join(",").replace('"', "&quot;")
            )
        } else {
            String::new()
        };

        // Emit the actual source line here.
        let f = F::Seq(vec![
            F::T(format!(
                "<div role=\"row\" id=\"line-{}\" class=\"source-line-with-number{}\"{}>",
                lineno,
                if line.sym_starts_nest.is_some() {
                    " nesting-sticky-line"
                } else {
                    ""
                },
                nesting
            )),
            F::Indent(vec![
                // Coverage Info. Its contents go in a div nested inside the
                // "cell" role div because in order to make the hover UI
                // accessible we expose it as a role=button which needs its own
                // element.
                F::T(format!(
                    r#"<div role="cell"><div role="button" aria-expanded="false"{}></div></div>"#,
                    coverage_data
                )),
                // Blame info.  Contents are nested for the exact same reason as
                // the coverage info (role=button needs its own div).
                F::T(format!(
                    "<div role=\"cell\"><div{}></div></div>",
                    blame_data
                )),
                // The line number.
                F::T(format!(
                    "<div role=\"cell\" class=\"line-number\" data-line-number=\"{}\"></div>",
                    lineno
                )),
                // The source line.
                F::T(format!(
                    "<code role=\"cell\" class=\"source-line\">{}\n</code>",
                    line.line
                )),
            ]),
            F::S("</div>"),
        ]);
        output::generate_formatted(writer, &f, 0).unwrap();

        // And at the end of this line we need to pop off the appropriate number of position:sticky
        // containing elements.
        for _ in 0..line.pop_nest_count {
            nest_depth -= 1;
            nest_syms.pop();
            write!(writer, "</div>").unwrap();
        }
    }

    let f = F::Seq(vec![F::S("</div>")]);
    output::generate_formatted(writer, &f, 0).unwrap();

    writeln!(writer, "<script>var SYM_INFO = {};</script>", sym_json,).unwrap();
    if let Some(page) = &token_blame {
        let info_json = if env::var("MOZSEARCH_DIFFABLE").is_err() {
            to_string(&page.info).unwrap()
        } else {
            to_string_pretty(&page.info).unwrap()
        };
        // Author names and paths could contain "</script>".
        writeln!(
            writer,
            "<script>var BLAME_INFO = {};</script>",
            info_json.replace("</", "<\\/")
        )
        .unwrap();
    }

    output::generate_footer(&opt, tree_name, path, writer).unwrap();

    format_perf.format_mixing_duration_us = pre_format_mixing.elapsed().as_micros() as u64;

    Ok(FormattedFile {
        perf: format_perf,
        token_blame,
    })
}

/// The hyperblame data files for a page's token-centric blame, by file name:
/// `commits.json` has the `blame::commit_info_json` of each of the page's
/// commits (in `BlameInfo::commits` order), and `lines-K.json` has chunk K of
/// the popup's per-token data (see `LinesChunk`).
pub fn hyperblame_files(
    tree_config: &TreeConfig,
    git: &GitData,
    page: &PageBlame,
) -> Vec<(String, String)> {
    let revs = page.info.commits.iter().map(|(rev, _, _)| rev.as_str());
    let mut files = vec![(
        "commits.json".to_string(),
        hyperblame_commits_json(tree_config, git, revs, &page.scope_changes),
    )];
    for (k, chunk) in page.chunks.iter().enumerate() {
        files.push((format!("lines-{}.json", k), to_string(chunk).unwrap()));
    }
    files
}

fn hyperblame_commits_json<'a>(
    tree_config: &TreeConfig,
    git: &GitData,
    revs: impl Iterator<Item = &'a str>,
    scope_changes: &[Vec<ScopeChange>],
) -> String {
    let infos: Vec<serde_json::Value> = revs
        .enumerate()
        .map(|(i, rev)| {
            let mut info = Oid::from_str(rev)
                .and_then(|oid| git.repo.find_commit(oid))
                .ok()
                .and_then(|commit| blame::commit_info_json(tree_config, git, &commit).ok())
                .unwrap_or(serde_json::Value::Null);
            // (Its most changed definitions in the file, for the file
            // timeline's list.)
            if let (Some(info), Some(changes)) = (info.as_object_mut(), scope_changes.get(i))
                && !changes.is_empty()
            {
                info.insert("changes".to_string(), json!(changes));
            }
            info
        })
        .collect();
    to_string(&infos).unwrap()
}

/// One of the hyperblame data files (see `hyperblame_files`) for the file at
/// `path` in revision `rev`, for pages of revisions other than the tip.  We
/// use the data rendering the page left in `PAGE_DATA_CACHE`, if it's there.
/// A token-centric blame data file of the file at `path` in the revision
/// `rev` (`/{tree}/rev-hyperblame/{rev}/{path}/{file}`): `file` is
/// `commits.json` or `lines-K.json` (see `hyperblame_file`), or
/// `peephole/{tokens}.json` for comma-separated token indices,
/// `future/{token}.json` or `before/{token}.json` (see `peephole_json`,
/// `future_json` and `before_json`).  (For trees' servers; see
/// `AbstractServer::hyperblame_json`.)
pub fn hyperblame_json(
    cfg: &Config,
    tree_name: &str,
    rev: &str,
    path: &str,
    file: &str,
) -> Result<String, &'static str> {
    if let Some((kind, name)) = file.split_once('/') {
        let tokens: Vec<u32> = name
            .strip_suffix(".json")
            .and_then(|tokens| tokens.split(',').map(|t| t.parse::<u32>().ok()).collect())
            .filter(|tokens: &Vec<u32>| !tokens.is_empty())
            .ok_or("Bad tokens")?;
        return match kind {
            "peephole" => peephole_json(cfg, tree_name, rev, path, tokens),
            "future" => future_json(cfg, tree_name, rev, path, tokens[0]),
            "before" => before_json(cfg, tree_name, rev, path, tokens[0]),
            _ => Err("Unknown hyperblame data"),
        };
    }
    hyperblame_file(cfg, tree_name, rev, path, file)
}

pub fn hyperblame_file(
    cfg: &Config,
    tree_name: &str,
    rev: &str,
    path: &str,
    file_name: &str,
) -> Result<String, &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let git = tree_config.get_git()?;
    let commit = git
        .repo
        .revparse_single(rev)
        .and_then(|object| object.peel_to_commit())
        .map_err(|_| "Bad revision")?;
    let key = (
        tree_name.to_string(),
        commit.id().to_string(),
        path.to_string(),
    );
    let data = PAGE_DATA_CACHE
        .get_or_compute(key, || {
            let (repo, oid) = get_object_at(
                OwnedOrBorrowed::Borrowed(&git.repo),
                commit.id(),
                Path::new(path),
            )
            .ok()?;
            let object = repo.find_object(oid, Some(git2::ObjectType::Blob)).ok()?;
            let source = git_ops::read_blob_object(&object);
            let mut page = page_token_blame(git, &commit, path, &source)?;
            set_file_changes(git, &commit, path, &mut page);
            Some(PageData::new(&page))
        })
        .ok_or("No token blame for the file")?;

    // (And leave them for nginx; see `sidecars`.  Only for the full commit
    // id, which pages request, so that the files are at their URLs.)
    let full_rev = rev == commit.id().to_string();
    if file_name == "commits.json" {
        let revs = || data.revs.iter().map(String::as_str);
        let json = data
            .commits_json
            .get_or_init(|| hyperblame_commits_json(tree_config, git, revs(), &data.scope_changes))
            .clone();
        if full_rev {
            write_page_data_sidecars(tree_config, tree_name, rev, path, &data, Some(&json));
        }
        return Ok(json);
    }
    if full_rev {
        write_page_data_sidecars(tree_config, tree_name, rev, path, &data, None);
    }
    file_name
        .strip_prefix("lines-")
        .and_then(|name| name.strip_suffix(".json"))
        .and_then(|k| k.parse::<usize>().ok())
        .and_then(|k| data.chunks.get(k).cloned())
        .ok_or("No such hyperblame file")
}

/// Whether the file at `path` has a rendered view (see `format_rendered`).
pub fn has_rendered_view(path: &str) -> bool {
    rendered_docs::is_doc_path(path)
}

/// The URL of the rendered view of the document at `path`, at the tip or at
/// the revision `rev`.
pub fn rendered_url(tree_name: &str, rev: Option<&str>, path: &str) -> String {
    match rev {
        Some(rev) => format!(
            "/{}/rev-rendered/{}/{}",
            tree_name,
            rev,
            url_encode_path(path)
        ),
        None => format!("/{}/rendered/{}", tree_name, url_encode_path(path)),
    }
}

lazy_static! {
    /// Each tree's map of its documentation's staged layout (see
    /// `DocTrees`), if it has one.
    static ref DOC_TREES: std::sync::Mutex<HashMap<String, Option<std::sync::Arc<DocTrees>>>> =
        std::sync::Mutex::new(HashMap::new());
}

lazy_static! {
    /// Each tree's jumpref data, if it has it, for the rendered views' links
    /// to labels in other documents.
    static ref DOC_JUMPREFS: std::sync::Mutex<
        HashMap<String, Option<std::sync::Arc<BisectableMmap<JumprefData>>>>,
    > = std::sync::Mutex::new(HashMap::new());
}

fn tree_jumprefs(
    tree_name: &str,
    tree_config: &TreeConfig,
) -> Option<std::sync::Arc<BisectableMmap<JumprefData>>> {
    let mut cache = DOC_JUMPREFS.lock().unwrap();
    cache
        .entry(tree_name.to_string())
        .or_insert_with(|| {
            let inline_path = format!("{}/jumpref", tree_config.paths.index_path);
            let extra_path = format!("{}/jumpref-extra", tree_config.paths.index_path);
            if !Path::new(&inline_path).exists() || !Path::new(&extra_path).exists() {
                return None;
            }
            BisectableMmap::new(&inline_path, &extra_path).map(std::sync::Arc::new)
        })
        .clone()
}

fn tree_doc_trees(tree_name: &str, tree_config: &TreeConfig) -> Option<std::sync::Arc<DocTrees>> {
    let mut cache = DOC_TREES.lock().unwrap();
    cache
        .entry(tree_name.to_string())
        .or_insert_with(|| {
            let path = format!("{}/doc-trees.json", tree_config.paths.index_path);
            Path::new(&path)
                .exists()
                .then(|| std::sync::Arc::new(read_doc_trees(&path)))
        })
        .clone()
}

lazy_static! {
    /// Each tree's specs (its index's copy of its config's `specs.toml`; see
    /// scripts/spec-analyze.sh), if it has them, for their rendered views.
    static ref TREE_SPECS: std::sync::Mutex<HashMap<String, Option<std::sync::Arc<SpecsConfig>>>> =
        std::sync::Mutex::new(HashMap::new());
}

/// The tree's specs, if it has them (see `TREE_SPECS`).
pub fn tree_specs(
    tree_name: &str,
    tree_config: &TreeConfig,
) -> Option<std::sync::Arc<SpecsConfig>> {
    let mut cache = TREE_SPECS.lock().unwrap();
    cache
        .entry(tree_name.to_string())
        .or_insert_with(|| {
            let path = format!("{}/specs.toml", tree_config.paths.index_path);
            SpecsConfig::load(Path::new(&path))
                .ok()
                .map(std::sync::Arc::new)
        })
        .clone()
}

/// What a rendered view's path under a spec's source is of (see
/// `spec_view`).
#[derive(Debug, PartialEq)]
enum SpecView {
    /// A page (`/{page}.html`), or the first.
    Page(Option<String>),
    /// The page with an anchor (`/anchor/{anchor}`).
    Anchor(String),
    /// The page with a (1-based) line (`/line/{line}`, as source listings'
    /// links to the rendered view say, with whatever the line selection
    /// says after its first line).
    Line(usize),
}

/// The spec whose source (which has a rendered view) a rendered view's
/// path is under, with the source's path and what the path is of.
fn spec_view<'a>(specs: &'a SpecsConfig, path: &str) -> Option<(&'a SpecEntry, String, SpecView)> {
    specs.specs.iter().find_map(|spec| {
        let format = spec.format?;
        if !rendered_specs::has_rendered_view(format) {
            return None;
        }
        let source = spec.source_path()?;
        if path == source {
            return Some((spec, source, SpecView::Page(None)));
        }
        let rest = path.strip_prefix(&source)?.strip_prefix('/')?;
        let view = if let Some(anchor) = rest.strip_prefix("anchor/") {
            SpecView::Anchor(anchor.to_string())
        } else if let Some(line) = rest
            .strip_prefix("line/")
            .or((rest == "line").then_some(""))
        {
            let digits: String = line.chars().take_while(char::is_ascii_digit).collect();
            SpecView::Line(digits.parse().unwrap_or(1))
        } else {
            SpecView::Page(Some(rest.strip_suffix(".html")?.to_string()))
        };
        Some((spec, source, view))
    })
}

/// Whether the file at `path` of a tree with `specs` is a spec's source
/// with a rendered view (for Bikeshed's, if the index has its rendering).
pub fn has_rendered_spec_view(tree_config: &TreeConfig, specs: &SpecsConfig, path: &str) -> bool {
    spec_view(specs, path).is_some_and(|(spec, _, view)| {
        view == SpecView::Page(None)
            && (spec.format != Some(crate::spec_analyze::SpecFormat::Bikeshed)
                || bikeshed_rendering_path(tree_config, path).exists())
    })
}

/// Where the index has Bikeshed's rendering of the spec whose source is at
/// `path` (see scripts/spec-analyze.sh).
fn bikeshed_rendering_path(tree_config: &TreeConfig, path: &str) -> std::path::PathBuf {
    Path::new(&tree_config.paths.index_path).join(format!("rendered-specs/{}.html", path))
}

/// Why a Bikeshed spec has no rendered view at a revision.
const BIKESHED_RENDERINGS: &str = "Only the latest version of a Bikeshed spec, and the heads of imported pull requests changing it, have rendered views";

/// Where Bikeshed's rendering of the spec whose source is at `path` is, at the
/// tip (see `bikeshed_rendering_path`), or at revision `rev`, if a spec pull
/// request's import rendered it (its head's; see render-spec-pr).
fn bikeshed_rendering_path_at(
    tree_config: &TreeConfig,
    path: &str,
    rev: Option<Oid>,
) -> std::path::PathBuf {
    match rev {
        Some(rev) => Path::new(&tree_config.paths.index_path)
            .join("try-rendered")
            .join(rev.to_string())
            .join(format!("{}.html", path)),
        None => bikeshed_rendering_path(tree_config, path),
    }
}

/// The files which a tree's ecmarkup specs import (and which those import),
/// and which its Bikeshed specs include (by the index's renderings), which
/// have rendered views as parts of their specs, reading their sources with
/// `read`, with their specs' names.
pub fn imported_spec_files(
    tree_config: &TreeConfig,
    specs: &SpecsConfig,
    read: &dyn Fn(&str) -> Option<String>,
) -> HashMap<String, String> {
    let mut files: HashMap<String, String> = HashMap::new();
    for spec in &specs.specs {
        if spec.format != Some(crate::spec_analyze::SpecFormat::Ecmarkup) {
            continue;
        }
        let Some(source) = spec.source_path() else {
            continue;
        };
        for file in rendered_specs::ecmarkup::imported_files(&source, read) {
            files.entry(file).or_insert_with(|| spec.name.clone());
        }
    }
    for spec in &specs.specs {
        if spec.format != Some(crate::spec_analyze::SpecFormat::Bikeshed) {
            continue;
        }
        let Some(source) = spec.source_path() else {
            continue;
        };
        let Ok(output) = std::fs::read_to_string(bikeshed_rendering_path(tree_config, &source))
        else {
            continue;
        };
        let dir = Path::new(&source).parent().unwrap_or(Path::new(""));
        for file in rendered_specs::bikeshed::included_files(&output) {
            files
                .entry(dir.join(file).to_string_lossy().into_owned())
                .or_insert_with(|| spec.name.clone());
        }
    }
    files
}

/// The targets of a ReSpec spec's references (at `path`), by where they are in
/// its source (lines and byte columns), from the index's analysis of it, which
/// has those of references to other specs' terms (see
/// `rendered_specs::respec`).
fn respec_index_targets(tree_config: &TreeConfig, path: &str) -> Vec<(u32, u32, u32, String)> {
    // (Compressed, or not; see `read_analyses`.)
    let analysis = format!("{}/analysis/{}.gz", tree_config.paths.index_path, path);
    if !Path::new(&analysis).exists() && !Path::new(&analysis[..analysis.len() - 3]).exists() {
        return vec![];
    }
    crate::file_format::analysis::read_analysis(
        &analysis,
        &mut crate::file_format::analysis::read_target,
    )
    .into_iter()
    .flat_map(|target| {
        let loc = target.loc;
        target
            .data
            .into_iter()
            .filter(|data| data.kind == crate::file_format::analysis::AnalysisKind::Use)
            .map(move |data| (loc.lineno, loc.col_start, loc.col_end, data.sym.to_string()))
    })
    .collect()
}

/// The `spec` global of info boxes (see `templating::info_boxes`), of a spec
/// of the tree's: its name, shortname, and URL, and the specs which
/// superseded it (its `successors`, if it's obsolete), by their names and
/// their rendered views' URLs (or else their own).
pub fn spec_globals(
    tree_name: &str,
    tree_config: &TreeConfig,
    specs: &SpecsConfig,
    spec: &SpecEntry,
) -> liquid::model::Value {
    let successors: Vec<serde_json::Value> = spec
        .successors
        .iter()
        .map(|name| {
            let successor = specs
                .specs
                .iter()
                .find(|s| s.name == *name || s.shortname() == name);
            let view = successor.and_then(|s| {
                let source = s.source_path()?;
                has_rendered_spec_view(tree_config, specs, &source)
                    .then(|| rendered_url(tree_name, None, &source))
            });
            let url = view
                .or_else(|| successor.and_then(|s| s.rendered.clone()))
                .unwrap_or_default();
            json!({ "name": name, "url": url })
        })
        .collect();
    liquid::model::to_value(&json!({
        "name": spec.name,
        "shortname": spec.shortname(),
        "rendered": spec.rendered.clone().unwrap_or_default(),
        "successors": successors,
    }))
    .unwrap_or_default()
}

/// The Bikeshed spec which includes the file at `path` (by the index's
/// rendering of it), with its source's path.
fn bikeshed_including_spec<'a>(
    tree_config: &TreeConfig,
    specs: &'a SpecsConfig,
    path: &str,
) -> Option<(&'a SpecEntry, String)> {
    specs.specs.iter().find_map(|spec| {
        if spec.format != Some(crate::spec_analyze::SpecFormat::Bikeshed) {
            return None;
        }
        let source = spec.source_path()?;
        let dir = Path::new(&source).parent()?.to_str()?;
        let relative = match dir {
            "" => path,
            dir => path.strip_prefix(dir)?.strip_prefix('/')?,
        };
        if path == source {
            return None;
        }
        // (Its elements with the file's content have its `bs-pos`s.)
        let output = std::fs::read_to_string(bikeshed_rendering_path(tree_config, &source)).ok()?;
        output
            .contains(&format!(" bs-pos=\"{}:", relative))
            .then_some((spec, source))
    })
}

/// How a rendered spec links to the tree's other specs' rendered views, at
/// the tip or at the revision.
struct RenderedSpecLinks<'a> {
    tree_name: &'a str,
    rev: Option<&'a str>,
}

impl SpecLinks for RenderedSpecLinks<'_> {
    fn rendered_url(&self, path: &str) -> String {
        rendered_url(self.tree_name, self.rev, path)
    }

    fn algorithm_url(&self, sym: &str) -> Option<String> {
        // (Of the latest revision, whose citations the trees have.)
        Some(algorithm_url(self.tree_name, sym))
    }
}

/// The URL of the view of the algorithm that `sym` defines, beside the code
/// citing its steps (see `format_algorithm`).
pub fn algorithm_url(tree_name: &str, sym: &str) -> String {
    // (Symbols' characters are all a URL path's, see `spec_symbol`.)
    format!("/{}/algorithm/{}", tree_name, sym)
}

/// Where the tree defines `sym`: the path and line of its definition.
fn tree_definition(tree_name: &str, tree_config: &TreeConfig, sym: &str) -> Option<(String, u32)> {
    let jumprefs = tree_jumprefs(tree_name, tree_config)?;
    let def = jumprefs.lookup(sym).ok()??.jumps.definition?;
    let (path, line) = def.rsplit_once('#')?;
    Some((path.to_string(), line.parse().ok()?))
}

/// The spec whose rendered URL (or one of its aliases) the URL is, or is
/// under, the longest such (ex: a later level's, under its series' URL),
/// whatever the URLs' schemes and hosts' case, and with or without a trailing
/// slash.
pub fn spec_for_url<'a>(specs: &'a SpecsConfig, url: &str) -> Option<&'a SpecEntry> {
    fn normalize(url: &str) -> String {
        let url = url.trim();
        let url = url
            .strip_prefix("https://")
            .or_else(|| url.strip_prefix("http://"))
            .unwrap_or(url);
        let (host, path) = url.split_once('/').unwrap_or((url, ""));
        format!("{}/{}", host.to_ascii_lowercase(), path)
    }
    let url = normalize(url);
    let mut best: Option<(&SpecEntry, usize)> = None;
    for spec in &specs.specs {
        for candidate in spec.rendered.iter().chain(&spec.aliases) {
            let mut candidate = normalize(candidate);
            if !candidate.ends_with('/') {
                candidate.push('/');
            }
            let matches = url.starts_with(&candidate)
                || url.trim_end_matches('/') == candidate.trim_end_matches('/');
            if matches && best.is_none_or(|(_, len)| candidate.len() > len) {
                best = Some((spec, candidate.len()));
            }
        }
    }
    best.map(|(spec, _)| spec)
}

/// Where a web spec's URL (without its fragment) is: its spec's rendered
/// view, at the anchor (percent-encoded, as URLs have them), in the first of
/// the tree and its `SPEC_` linked tree with the spec's source (ex: from
/// firefox-disco, wubspecs'; see the DWIM rule "spec-url"), if it's a spec's
/// with a rendered view.
pub fn spec_url_target(
    cfg: &Config,
    tree_name: &str,
    url: &str,
    anchor: Option<&str>,
) -> Option<String> {
    let tree_config = cfg.trees.get(tree_name)?;
    let candidates = std::iter::once(tree_name).chain(
        tree_config
            .paths
            .linked_trees
            .get("SPEC_")
            .map(String::as_str),
    );
    for spec_tree in candidates {
        let Some(spec_tree_config) = cfg.trees.get(spec_tree) else {
            continue;
        };
        let Some(specs) = tree_specs(spec_tree, spec_tree_config) else {
            continue;
        };
        let Some(spec) = spec_for_url(&specs, url) else {
            continue;
        };
        let Some(source_path) = spec.source_path() else {
            return None;
        };
        if !Path::new(&spec_tree_config.paths.files_path)
            .join(&source_path)
            .is_file()
        {
            continue;
        }
        let base = rendered_url(spec_tree, None, &source_path);
        let anchor = anchor.filter(|anchor| !anchor.is_empty()).map(|anchor| {
            urlencoding::decode(anchor)
                .map(|anchor| anchor.into_owned())
                .unwrap_or_else(|_| anchor.to_string())
        });
        return Some(match anchor {
            Some(anchor) if spec.format.is_some_and(rendered_specs::is_paged) => {
                rendered_specs::anchor_url(&base, &anchor)
            }
            Some(anchor) => format!("{}{}", base, rendered_specs::fragment(&anchor)),
            None => base,
        });
    }
    None
}

/// Where a DWIM lookup rule's lookup (see query/dwim.toml) says its input is,
/// in the tree `tree_name` (or another), from the rule's captures:
/// - "spec": a web spec's URL's rendered view (see `spec_url_target`);
/// - "commit": a commit's page, in the tree (or its `SPEC_` linked tree, ex:
///   a spec's upstream commit, whose forest commit it goes to), or another
///   tree whose repository has it;
/// - "hg-commit": an hg commit's page, in a tree with an hg repository (by
///   its `hgcommit` route, which maps it);
/// - "explore-bug" and "explore-phab": the explore page of the commits which
///   mention a bug or a Phabricator revision, in a tree with a history;
/// - "spec-pr": a spec repository's pull request's try push's page, in the
///   tree (or its `SPEC_` linked tree) whose specs' repositories it's of;
/// - "treeherder-try": a try push's page (from Treeherder's `repo=try` and
///   `revision=` parameters), in a tree importing them (or whose sibling
///   does; see `try_import::importing_tree`).
pub fn dwim_lookup(
    cfg: &Config,
    tree_name: &str,
    kind: &str,
    capture: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    let tree_config = cfg.trees.get(tree_name)?;
    let with_specs = || {
        std::iter::once(tree_name.to_string()).chain(
            tree_config
                .paths
                .linked_trees
                .get("SPEC_")
                .filter(|linked| cfg.trees.contains_key(linked.as_str()))
                .cloned(),
        )
    };
    match kind {
        "spec" => spec_url_target(
            cfg,
            tree_name,
            &capture("url")?,
            capture("anchor").as_deref(),
        ),
        "commit" => {
            let rev = capture("rev")?;
            let has = |tree: &str| {
                let in_repo = cfg
                    .trees
                    .get(tree)
                    .and_then(|tree| tree.get_git().ok())
                    .is_some_and(|git| {
                        git.repo
                            .revparse_single(&rev)
                            .and_then(|o| o.peel_to_commit())
                            .is_ok()
                    });
                in_repo || forest_rev(cfg, tree, &rev).is_some()
            };
            let others = cfg
                .trees
                .keys()
                .filter(|name| name.as_str() != tree_name)
                .cloned();
            with_specs()
                .chain(others)
                .find(|tree| has(tree))
                .map(|tree| format!("/{}/commit/{}", tree, rev))
        }
        "hg-commit" => {
            let rev = capture("rev")?;
            tree_config.paths.hg_root.as_ref()?;
            Some(format!("/{}/hgcommit/{}", tree_name, rev))
        }
        // (The tree's files at its branch's head then, roughly; or else, ex:
        // not a time, the query, which says why.)
        "rev-at" => {
            let time = capture("time")?;
            let at = rev_at(cfg, tree_name, time.trim().trim_matches(['\'', '"'])).ok()??;
            Some(format!("/{}/rev/{}/", tree_name, at.rev))
        }
        "explore-bug" | "explore-phab" => {
            tree_config.get_git().ok()?.history.as_ref()?;
            let (page, key) = match kind {
                "explore-bug" => ("bug", capture("bug")?),
                _ => ("phab", capture("rev")?),
            };
            Some(format!("/{}/explore/{}/{}", tree_name, page, key))
        }
        "spec-pr" => {
            let repo = capture("repo")?.to_ascii_lowercase();
            let number = capture("number")?;
            for tree in with_specs() {
                let specs = crate::try_import::spec_repos(cfg, &tree);
                let spec = specs.iter().find(|spec| {
                    spec.url
                        .as_deref()
                        .and_then(|url| url.strip_prefix("https://github.com/"))
                        .map(|url| {
                            url.trim_end_matches('/')
                                .trim_end_matches(".git")
                                .to_ascii_lowercase()
                        })
                        == Some(repo.clone())
                });
                if let Some(spec) = spec {
                    return Some(format!("/{}/try/{}/{}", tree, spec.name, number));
                }
            }
            None
        }
        "treeherder-try" => {
            let params: HashMap<String, String> =
                url::form_urlencoded::parse(capture("params")?.as_bytes())
                    .into_owned()
                    .collect();
            if params.get("repo").map(String::as_str) != Some("try") {
                return None;
            }
            let rev = crate::try_import::normalize_rev(params.get("revision")?)?;
            let importing = crate::try_import::importing_tree(cfg, tree_name)
                .unwrap_or_else(|| tree_name.to_string());
            crate::try_import::hook(cfg, &importing)?;
            crate::try_import::url_push(cfg, &importing, "hg", &rev)?;
            Some(format!("/{}/try/hg/{}", tree_name, rev))
        }
        _ => None,
    }
}

/// Where the view of an algorithm (see `format_algorithm`) is if the tree
/// doesn't define it but a linked tree does (ex: from firefox-disco, for a
/// `SPEC_` symbol, wubspecs').
pub fn algorithm_redirect(cfg: &Config, tree_name: &str, sym: &str) -> Option<String> {
    let tree_config = cfg.trees.get(tree_name)?;
    if tree_definition(tree_name, tree_config, sym).is_some() {
        return None;
    }
    let (_, linked) = tree_config
        .paths
        .linked_trees
        .iter()
        .find(|(prefix, _)| sym.starts_with(prefix.as_str()))?;
    Some(algorithm_url(linked, sym))
}

/// A citation of a spec's algorithm, or one of its steps, in a tree's file:
/// the tree, the path, the (1-based) line, the function it's in, the step,
/// and the lines from it, up to the next step's or its block's end (see
/// `citation_extent`), of which the first `shown` show at first, and the
/// lines after them, up to its file's next citation (at most
/// `CITATION_AFTER_LINES`), which it can show.
struct Citation {
    tree: String,
    path: String,
    line: u32,
    function: String,
    step: Option<String>,
    excerpt: Vec<String>,
    shown: usize,
    after: Vec<String>,
    /// If it continues the citation before it in its column (in the same
    /// file and function, after it): how many lines are between them; and
    /// whether the one after it continues it closely, so that its lines
    /// after it are those between them, which show, lightened (see
    /// `mark_continuations`).
    continues: Option<u32>,
    continued_closely: bool,
    /// The excerpt's lines' rows in the tree's source listing, if it has them
    /// (see `citation_rows`).
    rows: Option<String>,
    /// If its step's text changed after it was written (by the trees'
    /// histories): when, and the spec's commit that changed it.
    drift: Option<(i64, String)>,
    /// What it is, if not a citation, ex: the C++ implementation of a WebIDL
    /// member reflecting the spec's definition.
    note: Option<String>,
    /// The try push's head it's at, if it's a push's (see `Citing::TryPush`),
    /// and whether it's in a file the push changes.
    rev: Option<String>,
    changed: bool,
    /// Where it goes among the algorithm's citations (of the same step): its
    /// own path and line, or for an implementation, its WebIDL member's, after
    /// it.
    order: (String, u32, u8),
}

/// The listing of a citation's file, at its revision.
fn citation_url(citation: &Citation) -> String {
    match &citation.rev {
        Some(rev) => format!(
            "/{}/rev/{}/{}",
            citation.tree,
            rev,
            url_encode_path(&citation.path)
        ),
        None => format!(
            "/{}/source/{}",
            citation.tree,
            url_encode_path(&citation.path)
        ),
    }
}

/// The newest change to a page's lines (0-based) by its token blame: its
/// time and revision.
fn newest_change(page: &PageBlame, lines: RangeInclusive<usize>) -> Option<(i64, String)> {
    let strip = page.merge_lines(lines);
    strip
        .entries
        .iter()
        .map(|entry| &page.info.commits[entry.commit])
        .max_by_key(|(_, time, _)| *time)
        .map(|(rev, time, _)| (*time, rev.clone()))
}

/// How many lines a citation's excerpt has at most (see `citation_extent`),
/// and how many show at first, at least (and through how many of its lines of
/// code, past its comments, at most this many), the rest folded.
const CITATION_EXCERPT_LINES: usize = 60;
const CITATION_SHOWN_LINES: usize = 12;
const CITATION_SHOWN_CODE_LINES: usize = 4;
const CITATION_SHOWN_MAX_LINES: usize = 30;
/// How many lines after a citation's excerpt it can show (up to its file's
/// next citation), in case its extent should have been longer.
const CITATION_AFTER_LINES: usize = 30;

/// How many of an excerpt's lines show at first (see `CITATION_SHOWN_LINES`):
/// all of them, if only a few more would fold.
fn citation_shown(excerpt: &[String]) -> usize {
    let mut code = 0;
    let mut through_code = excerpt.len();
    for (i, line) in excerpt.iter().enumerate() {
        if !line.trim().is_empty() && !is_comment_line(line) {
            code += 1;
            if code == CITATION_SHOWN_CODE_LINES {
                through_code = i + 1;
                break;
            }
        }
    }
    let shown = CITATION_SHOWN_LINES
        .max(through_code)
        .min(CITATION_SHOWN_MAX_LINES);
    if excerpt.len() <= shown + 3 {
        excerpt.len()
    } else {
        shown
    }
}

/// The lines after a citation's excerpt (ending at `end`, 0-based), up to
/// its file's `next` citation, or the end of the top-level definition it's in
/// (a `}` unindented, or before a line unindented, ex: Python's), at most
/// `CITATION_AFTER_LINES`.
fn citation_after(lines: &[String], end: usize, next: Option<usize>) -> Vec<String> {
    let until = next
        .unwrap_or(usize::MAX)
        .min(end + CITATION_AFTER_LINES)
        .min(lines.len());
    let mut after = vec![];
    for line in lines.get(end..until.max(end)).unwrap_or_default() {
        let unindented = !line.is_empty() && !line.starts_with(char::is_whitespace);
        if unindented && !line.starts_with(['}', ')', ']']) {
            break;
        }
        after.push(line.clone());
        if unindented {
            break;
        }
    }
    after
}

/// A step's key among its algorithm's (see `AlgorithmSection::steps`): a
/// range's or list's first step's (ex: "1-3"'s, "5 and 6"'s).
fn step_key(step: &str) -> &str {
    let end = step
        .find(|c: char| c.is_whitespace() || matches!(c, '-' | '–' | ','))
        .unwrap_or(step.len());
    step[..end].trim_end_matches(['.', ':'])
}

/// Whether a line of a file is a comment's (or a block comment's line).
fn is_comment_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("//")
        || trimmed.starts_with("/*")
        || trimmed.starts_with('*')
        || trimmed == "#"
        || trimmed.starts_with("# ")
}

/// How many of a file's lines from a citation's (0-based) `first` its
/// excerpt has, as the implementation of its step: up to (but not) its
/// file's `next` citation of the algorithm, or a comment starting with a
/// step (ex: "// Step 3."), or a line leaving the block it's in (closing it,
/// or indented less), or after the end of a definition it precedes (ex: a
/// function under its URL), and at most `CITATION_EXCERPT_LINES`, without
/// blank lines at its end.  (Blank lines and comments don't end it.)
fn citation_extent(lines: &[String], first: usize, next: Option<usize>) -> usize {
    lazy_static! {
        static ref STEP_START: regex::Regex =
            regex::Regex::new(r"(?i)^\s*(?://+|/?\*+|#+)\s*\(?steps?\s+[0-9]").unwrap();
        static ref CONTROL: regex::Regex = regex::Regex::new(
            r"^\s*(?:\}\s*)?(?:if|else|for|while|do|switch|case|default|try|catch|return|co_return)\b"
        )
        .unwrap();
    }
    let indent = |line: &str| line.len() - line.trim_start().len();
    let Some(start) = lines.get(first) else {
        return 0;
    };
    // (A block comment's lines' stars are after its indentation.)
    let level =
        indent(start) - usize::from(start.trim_start().starts_with('*') && indent(start) > 0);
    let mut depth: i64 = 0;
    // Whether its first code is a definition (ex: a function's, not a
    // statement's), whose end ends it: whether it opens a block before its
    // statement ends (and isn't control flow), once that's known.
    let mut definition: Option<bool> = None;
    let mut first_code: Option<bool> = None;
    let mut end = first + 1;
    for (i, line) in lines
        .iter()
        .enumerate()
        .skip(first)
        .take(CITATION_EXCERPT_LINES)
    {
        let trimmed = line.trim();
        if i > first && Some(i) == next {
            break;
        }
        if trimmed.is_empty() {
            continue;
        }
        if is_comment_line(line) {
            if i > first && STEP_START.is_match(line) {
                break;
            }
            end = i + 1;
            continue;
        }
        if i > first && indent(line) < level && !trimmed.starts_with(['}', ')', ']']) {
            break;
        }
        // (Its braces, but a trailing comment's.)
        let code = trimmed.split("//").next().unwrap_or_default();
        let control = *first_code.get_or_insert_with(|| CONTROL.is_match(line));
        let mut left = false;
        for c in code.chars() {
            match c {
                '{' => {
                    if depth == 0 {
                        definition.get_or_insert(!control);
                    }
                    depth += 1;
                }
                '}' => {
                    depth -= 1;
                    left |= depth < 0;
                }
                ';' if depth == 0 => {
                    definition.get_or_insert(false);
                }
                _ => {}
            }
        }
        if left {
            break;
        }
        end = i + 1;
        if definition == Some(true) && depth == 0 && code.contains('}') {
            break;
        }
    }
    end - first
}

/// Whose code citing an algorithm a column of its view has (see
/// `format_algorithm`).
#[derive(Clone)]
enum Citing {
    /// The code citing the spec tree's specs: its linked trees' for the
    /// symbol's prefix if it has them (ex: firefox-disco's, for wubspecs),
    /// whose code it's of, and otherwise the tree's own (ex: the test
    /// tree's, whose code cites its own specs), at their indexed revisions.
    /// (Not a spec tree's own other files, then, which are documents, ex:
    /// the HTML Standard's review drafts, copies of it whose references to
    /// other specs are citations of them, as code's would be, or specs'
    /// published copies.)
    Specs,
    /// A tree's code at the head of a try push's stack rebased onto its
    /// indexed revision, by the push's semantic delta (see `try_delta`).
    TryPush {
        tree: String,
        delta: std::sync::Arc<crate::try_delta::TryDelta>,
    },
}

impl Citing {
    /// The trees whose code it is, for the symbol, from the spec tree's.
    fn trees(&self, tree_name: &str, tree_config: &TreeConfig, sym: &str) -> Vec<String> {
        match self {
            Citing::Specs => {
                let trees: Vec<String> = tree_config
                    .paths
                    .linked_trees
                    .iter()
                    .filter(|(prefix, _)| sym.starts_with(prefix.as_str()))
                    .map(|(_, tree)| tree.clone())
                    .collect();
                if trees.is_empty() {
                    vec![tree_name.to_string()]
                } else {
                    trees
                }
            }
            Citing::TryPush { tree, .. } => vec![tree.clone()],
        }
    }

    fn delta(&self) -> Option<&crate::try_delta::TryDelta> {
        match self {
            Citing::Specs => None,
            Citing::TryPush { delta, .. } => Some(delta),
        }
    }
}

/// The citations of `sym` (its doc-uses, see `spec_analyze::citations`) in
/// the code `citing` it, with their excerpts from the code's revisions (the
/// trees' indexed ones, or a try push's head), but those in the files of the
/// code's trees which they leave out (`excluded_citation_paths`, ex:
/// firefox-disco's vendored tooling).  A WebIDL member reflecting
/// the spec's definition (its doc-use, see `spec_analyze::bcd`) is followed
/// by its C++ implementations (by its `impl` binding slots), up to their
/// ends.
fn algorithm_citations(
    cfg: &Config,
    tree_name: &str,
    tree_config: &TreeConfig,
    sym: &str,
    citing: &Citing,
) -> Vec<Citation> {
    let delta = citing.delta();
    let mut citations = vec![];
    for tree in citing.trees(tree_name, tree_config, sym) {
        let Some(index_path) = cfg.tree_index_paths.get(&tree) else {
            continue;
        };
        let (inline, extra) = (
            format!("{}/crossref", index_path),
            format!("{}/crossref-extra", index_path),
        );
        if !Path::new(&inline).exists() || !Path::new(&extra).exists() {
            continue;
        }
        let Some(crossref) = BisectableMmap::<CrossrefData>::new(&inline, &extra) else {
            continue;
        };
        // (A try push's, with its patches.)
        let lookup = |sym: &str| -> Option<CrossrefData> {
            let data = crossref.lookup(sym).ok().flatten();
            match delta.and_then(|delta| delta.crossref_patch(sym).ok().flatten()) {
                Some(patch) => patch.apply(data),
                None => data,
            }
        };
        let Some(data) = lookup(sym) else {
            continue;
        };
        let git = cfg.trees.get(&tree).and_then(|t| t.get_git().ok());
        let head = git.and_then(|git| {
            let object = match delta {
                Some(delta) => git.repo.revparse_single(&delta.meta.head).ok()?,
                None => git.repo.head().ok()?.peel(git2::ObjectType::Commit).ok()?,
            };
            object.peel_to_commit().ok()
        });
        let excluded = cfg
            .trees
            .get(&tree)
            .map_or(&[][..], |t| &t.paths.excluded_citation_paths[..]);
        for result in data.doc_uses.unwrap_or_default() {
            if excluded
                .iter()
                .any(|prefix| result.path.starts_with(prefix.as_str()))
            {
                continue;
            }
            let changed = delta.is_some_and(|delta| delta.change(&result.path).is_some());
            let lines: Vec<String> = match (git, &head) {
                (Some(git), Some(head)) => read_commit_file(&git.repo, head, &result.path)
                    .map_or(vec![], |text| text.lines().map(str::to_string).collect()),
                _ => vec![],
            };
            // (The file's citations of the algorithm, whose next ends one's
            // excerpt, 0-based.)
            let mut cited_lines: Vec<usize> = result
                .lines
                .iter()
                .map(|hit| hit.lineno as usize - 1)
                .collect();
            cited_lines.sort_unstable();
            for hit in result.lines {
                let (function, step) = match hit.context.rsplit_once(" step ") {
                    Some((function, step)) => (function.to_string(), Some(step.to_string())),
                    None => (hit.context.to_string(), None),
                };
                let first = hit.lineno as usize - 1;
                // (Up to the file's next citation of the algorithm, or step,
                // or the end of the block it's in; see `citation_extent`.)
                let next = cited_lines.iter().copied().find(|&line| line > first);
                let len = citation_extent(&lines, first, next);
                let excerpt: Vec<String> =
                    lines[first.min(lines.len())..(first + len).min(lines.len())].to_vec();
                let after = citation_after(&lines, first + len, next);
                let webidl = hit.contextsym.starts_with("WEBIDL_");
                // (A WebIDL member's is its line.)
                let (excerpt, after) = if excerpt.is_empty() || webidl {
                    let line = lines
                        .get(first)
                        .map_or_else(|| hit.line.to_string(), String::clone);
                    (vec![line], vec![])
                } else {
                    (excerpt, after)
                };
                citations.push(Citation {
                    tree: tree.clone(),
                    path: result.path.to_string(),
                    line: hit.lineno,
                    function,
                    step,
                    shown: citation_shown(&excerpt),
                    excerpt,
                    after,
                    continues: None,
                    continued_closely: false,
                    rows: None,
                    drift: None,
                    note: webidl.then(|| "WebIDL".to_string()),
                    rev: delta.map(|delta| delta.meta.head.clone()),
                    changed,
                    order: (result.path.to_string(), hit.lineno, 0),
                });
                if webidl {
                    citations.extend(webidl_implementations(
                        &lookup,
                        &tree,
                        git,
                        head.as_ref(),
                        delta,
                        &hit.contextsym,
                        &hit.context,
                        (result.path.to_string(), hit.lineno, 1),
                    ));
                }
            }
        }
    }
    citations
}

/// The C++ implementations of a WebIDL member (`webidl`, by its `impl` binding
/// slots, in the tree whose crossref data `lookup` has, at `head`, maybe a
/// try push's with `delta`), as citations: from their definitions' lines up
/// to their ends (a line closing a block at their indentation), at most
/// `CITATION_EXCERPT_LINES`.
#[allow(clippy::too_many_arguments)]
fn webidl_implementations(
    lookup: &dyn Fn(&str) -> Option<CrossrefData>,
    tree: &str,
    git: Option<&GitData>,
    head: Option<&git2::Commit>,
    delta: Option<&crate::try_delta::TryDelta>,
    webidl: &str,
    webidl_pretty: &str,
    order: (String, u32, u8),
) -> Vec<Citation> {
    let Some(data) = lookup(webidl) else {
        return vec![];
    };
    let Some(meta) = data.meta else {
        return vec![];
    };
    let mut implementations = vec![];
    for slot in &meta.binding_slots {
        if slot.props.impl_kind != Some(BindingImplKind::Impl)
            || slot.props.slot_lang != BindingSlotLang::Cpp
        {
            continue;
        }
        let Some(implementation) = lookup(&slot.sym) else {
            continue;
        };
        let Some(definition) = implementation
            .definitions
            .as_ref()
            .and_then(|defs| defs.first())
            .and_then(|def| Some((def.path, def.lines.first()?.lineno)))
        else {
            continue;
        };
        let lines: Vec<String> = match (git, head) {
            (Some(git), Some(head)) => read_commit_file(&git.repo, head, &definition.0)
                .map_or(vec![], |text| text.lines().map(str::to_string).collect()),
            _ => vec![],
        };
        let first = definition.1 as usize - 1;
        let Some(start) = lines.get(first) else {
            continue;
        };
        let indent = start.len() - start.trim_start().len();
        let mut excerpt = vec![];
        for line in lines.iter().skip(first).take(CITATION_EXCERPT_LINES) {
            excerpt.push(line.clone());
            let trimmed = line.trim_start();
            if excerpt.len() > 1 && trimmed.starts_with('}') && line.len() - trimmed.len() <= indent
            {
                break;
            }
            // (A one-line definition, ex: `void Method() {}`.)
            if excerpt.len() == 1 && trimmed.ends_with('}') && trimmed.contains('{') {
                break;
            }
        }
        let pretty = implementation
            .meta
            .as_ref()
            .map_or_else(|| slot.sym.to_string(), |meta| meta.pretty.to_string());
        let after = citation_after(&lines, first + excerpt.len(), None);
        implementations.push(Citation {
            tree: tree.to_string(),
            path: definition.0.to_string(),
            line: definition.1,
            function: pretty,
            step: None,
            shown: citation_shown(&excerpt),
            excerpt,
            after,
            continues: None,
            continued_closely: false,
            rows: None,
            drift: None,
            note: Some(format!(
                "C++ implementation of the WebIDL {}",
                webidl_pretty
            )),
            rev: delta.map(|delta| delta.meta.head.clone()),
            changed: delta.is_some_and(|delta| delta.change(definition.0.as_str()).is_some()),
            order: order.clone(),
        });
    }
    implementations
}

/// An algorithm's section of its view (see `format_algorithm`): its rendering
/// with its blocks' blame (or line markers), its steps' (1-based) lines, and
/// the citations of it and its steps, in each of the view's columns of code
/// citing it.
struct AlgorithmSection {
    sym: String,
    name: String,
    /// Its spec's shortname.
    shortname: String,
    /// Its anchor in its spec.
    anchor: Option<String>,
    def_path: String,
    def_line: u32,
    source_path: String,
    /// The file with the algorithm (its spec's source, or a file it imports).
    file: String,
    rendered_link: String,
    view: rendered_specs::RenderedSpec,
    strips: Vec<String>,
    steps: BTreeMap<String, usize>,
    columns: Vec<Vec<Citation>>,
    token_blame: Option<std::sync::Arc<PageBlame>>,
}

/// The section of the algorithm `sym` defines in one of the tree's specs, with
/// its blocks' blame (of the file with it, which the page's other sections may
/// share; see `write_algorithm_view`) and its citations' drift, and a column of
/// citations for each of `citing`.
#[allow(clippy::too_many_arguments)]
fn algorithm_section(
    cfg: &Config,
    tree_name: &str,
    tree_config: &TreeConfig,
    specs: &SpecsConfig,
    git: &GitData,
    commit: &git2::Commit,
    sym: &str,
    citing: &[Citing],
) -> Result<AlgorithmSection, &'static str> {
    let (def_path, def_line) =
        tree_definition(tree_name, tree_config, sym).ok_or("No definition of the symbol")?;
    let read = |file: &str| read_commit_file(&git.repo, commit, file);
    // (The spec whose source, or a file it imports, has the definition.)
    let (spec, source_path) = match spec_view(specs, &def_path) {
        Some((spec, source, SpecView::Page(None))) => (spec, source),
        _ => rendered_specs::ecmarkup::importing_spec(specs, &def_path, &read)
            .ok_or("The symbol isn't a spec's")?,
    };
    let source = read(&source_path).ok_or("No source")?;
    let output = (spec.format == Some(crate::spec_analyze::SpecFormat::Bikeshed))
        .then(|| std::fs::read_to_string(bikeshed_rendering_path(tree_config, &source_path)).ok())
        .flatten();
    let spec_links = RenderedSpecLinks {
        tree_name,
        rev: None,
    };
    // (A ReSpec spec's references to other specs' terms link by the index's
    // analysis, like its rendered view's.)
    let view = match spec.format {
        Some(crate::spec_analyze::SpecFormat::Respec) => rendered_specs::respec::render_algorithm(
            &source_path,
            spec,
            specs,
            &spec_links,
            &read,
            &crate::spec_analyze::xref::Terms::default(),
            &respec_index_targets(tree_config, &source_path),
            sym,
        ),
        _ => rendered_specs::render_algorithm(
            spec,
            &source,
            &source_path,
            specs,
            &spec_links,
            &read,
            output.as_deref(),
            sym,
        ),
    }
    .ok_or("The symbol isn't an algorithm's")?;
    // (The blocks are of the file with the algorithm.)
    let file = view.file.clone().unwrap_or_else(|| source_path.clone());
    let file_source = if file == source_path {
        source.clone()
    } else {
        read(&file).ok_or("No file")?
    };

    let token_blame = spec_page_token_blame(
        tree_name,
        git,
        commit,
        &file,
        &file_source,
        BreadcrumbsLinksTo::Latest,
    );
    let strips = match &token_blame {
        Some(page) => rendered_strips(&view.rendered, &file_source, page, &view.range),
        None => rendered_line_markers(&view.rendered, &file_source, &view.range),
    };
    // The steps' (1-based) lines.
    let mut steps: BTreeMap<String, usize> = BTreeMap::new();
    for (path, at) in &view.steps {
        let line = file_source[..(*at).min(file_source.len())]
            .matches('\n')
            .count()
            + 1;
        steps.entry(path.clone()).or_insert(line);
    }
    let mut columns: Vec<Vec<Citation>> = citing
        .iter()
        .map(|citing| algorithm_citations(cfg, tree_name, tree_config, sym, citing))
        .collect();
    // Drift: a citation of a step whose text changed (by the spec's history)
    // after the comment citing it last did (by the citing tree's), ex: after
    // Gecko implemented it.  A step's lines are up to the next step's.  (Not
    // in the files a try push changes, which the history doesn't have at the
    // head of its rebased stack.)
    if let Some(spec_page) = &token_blame {
        let last_line = file_source[..view.range.end.min(file_source.len())]
            .matches('\n')
            .count();
        let mut starts: Vec<usize> = steps.values().map(|&line| line - 1).collect();
        starts.sort_unstable();
        let step_lines = |step: &str| -> Option<RangeInclusive<usize>> {
            let first = steps.get(step)? - 1;
            let next = starts
                .get(starts.partition_point(|&start| start <= first))
                .copied()
                .unwrap_or(last_line + 1);
            Some(first..=next.saturating_sub(1).max(first))
        };
        // (The citing files' blame, once each.)
        let mut citing_pages: HashMap<(String, String), Option<PageBlame>> = HashMap::new();
        for citation in columns.iter_mut().flatten() {
            if citation.changed {
                continue;
            }
            let Some(lines) = citation.step.as_deref().and_then(step_lines) else {
                continue;
            };
            let Some(changed) = newest_change(spec_page, lines) else {
                continue;
            };
            let page = citing_pages
                .entry((citation.tree.clone(), citation.path.clone()))
                .or_insert_with(|| {
                    let git = cfg.trees.get(&citation.tree)?.get_git().ok()?;
                    let head = git.repo.head().ok()?.peel_to_commit().ok()?;
                    let source = read_commit_file(&git.repo, &head, &citation.path)?;
                    page_token_blame(git, &head, &citation.path, &source)
                });
            let line = citation.line as usize - 1;
            let written = page
                .as_ref()
                .and_then(|page| newest_change(page, line..=line));
            if written.is_some_and(|(time, _)| changed.0 > time) {
                citation.drift = Some(changed);
            }
        }
    }
    // (Those of the algorithm first, then by their steps' lines.)
    for citations in &mut columns {
        citations.sort_by_key(|c| {
            (
                c.step.as_ref().map_or(0, |step| {
                    steps.get(step_key(step)).copied().unwrap_or(usize::MAX)
                }),
                c.tree.clone(),
                c.order.clone(),
            )
        });
    }

    let name = tree_jumprefs(tree_name, tree_config)
        .and_then(|jumprefs| jumprefs.lookup(sym).ok()?)
        .map_or_else(|| sym.to_string(), |jumpref| jumpref.pretty.to_string());
    let anchor = crate::spec_analyze::spec_anchor(spec.shortname(), sym);
    let base = rendered_url(tree_name, None, &source_path);
    let rendered_link = match &anchor {
        Some(anchor) if spec.format.is_some_and(rendered_specs::is_paged) => {
            rendered_specs::anchor_url(&base, anchor)
        }
        Some(anchor) => format!("{}{}", base, rendered_specs::fragment(anchor)),
        None => base.clone(),
    };
    Ok(AlgorithmSection {
        sym: sym.to_string(),
        name,
        shortname: spec.shortname().to_string(),
        anchor,
        def_path,
        def_line,
        source_path,
        file,
        rendered_link,
        view,
        strips,
        steps,
        columns,
        token_blame,
    })
}

/// The most algorithms an algorithm's view shows which it calls.
const MAX_CALLED_ALGORITHMS: usize = 8;

/// The algorithms (of the tree's specs) which the algorithm `sym` calls (its
/// callees in the tree's crossref which are algorithms, abstract operations,
/// or IDL members with steps), in the order of their first calls, up to
/// `MAX_CALLED_ALGORITHMS`.
fn called_algorithms(tree_config: &TreeConfig, sym: &str) -> Vec<String> {
    let index_path = &tree_config.paths.index_path;
    let (inline, extra) = (
        format!("{}/crossref", index_path),
        format!("{}/crossref-extra", index_path),
    );
    let Some(crossref) = BisectableMmap::<CrossrefData>::new(&inline, &extra) else {
        return vec![];
    };
    let Ok(Some(data)) = crossref.lookup(sym) else {
        return vec![];
    };
    let is_algorithm = |callee: &str| {
        let Ok(Some(data)) = crossref.lookup(callee) else {
            return false;
        };
        data.meta.is_some_and(|meta| {
            &*meta.impl_kind == "spec"
                && (matches!(&*meta.kind, "specAlgorithm" | "specAbstractOp")
                    || meta.extra.get("algorithm").and_then(|a| a.as_bool()) == Some(true))
        })
    };
    let mut called: Vec<(u32, String)> = data
        .callees
        .unwrap_or_default()
        .into_iter()
        .filter(|callee| !callee.implements && callee.tree.is_none() && &*callee.sym != sym)
        .map(|callee| {
            // (Its first call's line, of "path#lines".)
            let first = callee
                .jump
                .rsplit_once('#')
                .and_then(|(_, lines)| lines.split(',').filter_map(|l| l.parse().ok()).min())
                .unwrap_or(u32::MAX);
            (first, callee.sym.to_string())
        })
        .collect();
    called.sort();
    called.dedup_by(|a, b| a.1 == b.1);
    called
        .into_iter()
        .map(|(_, sym)| sym)
        .filter(|sym| is_algorithm(sym))
        .take(MAX_CALLED_ALGORITHMS)
        .collect()
}

/// The view of the algorithm that `sym` defines in one of the tree's specs
/// (`/{tree}/algorithm/{sym}`), beside the code citing it and its steps (see
/// `Citing::Specs`, ex: Gecko's, in firefox-disco, for wubspecs' specs): the
/// citations' excerpts, as their source listings' rows (see
/// `citation_rows`), on the left, placed by the page's script next to their
/// steps' blocks, and the algorithm's rendering (see
/// `rendered_specs::render_algorithm`), with the blame of its blocks, on the
/// right.  Then the algorithms it calls (see `called_algorithms`), each
/// beside the code citing it, so that one can follow its calls (from its call
/// sites' "↓"s), without blame.
pub fn format_algorithm(
    cfg: &Config,
    tree_name: &str,
    sym: &str,
    writer: &mut dyn Write,
) -> Result<(), &'static str> {
    write_algorithm_view(cfg, tree_name, sym, None, writer)
}

/// An algorithm's view at a try push's head (see `format_try_algorithm`):
/// the tree whose push it is, the push's delta, and another push's, to
/// compare it with instead of the code the spec tree's view has.
struct TryAlgorithmView {
    tree: String,
    delta: std::sync::Arc<crate::try_delta::TryDelta>,
    versus: Option<std::sync::Arc<crate::try_delta::TryDelta>>,
}

/// Run `future` from the web-server's synchronous handlers (on tokio's
/// blocking threads), on the runtime's workers, where servers' synchronous
/// methods can block in place (see `abstract_server::RemoteServer`).
pub fn run_on_runtime<T: Send + 'static>(future: impl Future<Output = T> + Send + 'static) -> T {
    let handle = tokio::runtime::Handle::current();
    match handle.block_on(handle.spawn(future)) {
        Ok(output) => output,
        Err(err) => std::panic::resume_unwind(err.into_panic()),
    }
}

/// The model of the canvas of the try push of `server`'s tree whose rebased
/// stack's head is `head` (see `patch_canvas`).
fn canvas_model(
    server: &(dyn AbstractServer + Send + Sync),
    tree_name: &str,
    head: &str,
) -> Result<crate::patch_canvas::CanvasModel, &'static str> {
    let (server, tree_name, head) = (server.clonify(), tree_name.to_string(), head.to_string());
    run_on_runtime(async move {
        crate::patch_canvas::build_canvas(server.as_ref(), &tree_name, &head).await
    })
}

/// The model of a try push's canvas (see `patch_canvas`), as JSON
/// (`/{tree}/rev-canvas/{head}/canvas.json`).
pub fn try_canvas_json(
    server: &(dyn AbstractServer + Send + Sync),
    tree_name: &str,
    head: &str,
) -> Result<String, &'static str> {
    let model = canvas_model(server, tree_name, head)?;
    serde_json::to_string(&model).map_err(|_| "Canvas problems")
}

/// A try push's canvas (`/{tree}/rev-canvas/{head}`; see `patch_canvas`): its
/// model, which static/js/rev-canvas.js lays out with graph.js as cards of the
/// functions' code connected by their calls, beside the push's summary and its
/// links, with the head's searches (its pages', and its code's context
/// menus') in the push's context.
pub fn format_try_canvas(
    server: &(dyn AbstractServer + Send + Sync),
    tree_name: &str,
    head: &str,
    writer: &mut dyn Write,
) -> Result<(), &'static str> {
    let model = canvas_model(server, tree_name, head)?;
    let title = format!("Canvas of try push {} - mozsearch", model.push);
    let opt = Options {
        title: &title,
        tree_name,
        include_date: env::var("MOZSEARCH_DIFFABLE").is_err(),
        revision: None,
        breadcrumbs_links_to: BreadcrumbsLinksTo::Latest,
        extra_content_classes: "rev-canvas",
        search_rev: Some(&model.head),
    };
    output::generate_header(&opt, writer)?;
    writeln!(
        writer,
        r#"<div class="rc-header"><h2 class="rc-title">Try push <a href="/{}/{}">{}</a>: {}</h2><p class="rc-note">The functions it adds or changes, at <a href="/{}/rev/{}/">the head of its stack rebased onto the indexed revision</a>, and the calls among them and with the functions calling them or called by them.  Its arrangement here is yours, kept in this browser.</p><div id="rc-toolbar" class="rc-toolbar"></div></div>"#,
        tree_name,
        rendered_docs::escape(&crate::try_import::push_url_path(&model.push)),
        rendered_docs::escape(&model.push),
        rendered_docs::escape(&model.summary),
        tree_name,
        model.head,
    )
    .unwrap();
    writeln!(
        writer,
        r#"<div id="interactive-graph-base"><div id="interactive-graph-viewport"><div id="interactive-graph-container"></div></div></div>"#
    )
    .unwrap();
    let sym_info = to_string(&model.sym_info).unwrap();
    let mut page_model = serde_json::to_value(&model).map_err(|_| "Canvas problems")?;
    if let Some(object) = page_model.as_object_mut() {
        object.remove("sym_info");
    }
    writeln!(
        writer,
        "<script>var SYM_INFO = {};\nvar CANVAS = {};</script>",
        sym_info.replace("</", "<\\/"),
        page_model.to_string().replace("</", "<\\/")
    )
    .unwrap();
    writeln!(
        writer,
        r#"<script src="/{}/static/js/graph.js"></script><script src="/{}/static/js/rev-canvas.js"></script>"#,
        tree_name, tree_name
    )
    .unwrap();
    output::generate_footer(&opt, tree_name, "", writer)
}

/// The view of an algorithm (see `format_algorithm`) in a try push's context
/// (`/{tree}/rev-algorithm/{head}/{sym}`, ex: firefox-try's, for a `SPEC_`
/// symbol, whose specs are its linked tree's): its steps between the code
/// citing it at the head of the push's stack rebased onto the tree's indexed
/// revision, by the push's semantic delta (see `try_delta`), on the left,
/// and the code citing it that the spec tree's view has (ex: firefox-disco's,
/// at the same revision), or with `versus` (`?versus={head}`), the code at
/// the head of another push's, on the right.
pub fn format_try_algorithm(
    cfg: &Config,
    tree_name: &str,
    head: &str,
    sym: &str,
    versus: Option<&str>,
    writer: &mut dyn Write,
) -> Result<(), &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let open = |head: &str| {
        crate::try_delta::TryDelta::open(&tree_config.paths.index_path, head)
            .map(std::sync::Arc::new)
            .ok_or("No semantic analysis of the revision")
    };
    let delta = open(head)?;
    let versus = versus.map(open).transpose()?;
    let (_, spec_tree) = tree_config
        .paths
        .linked_trees
        .iter()
        .find(|(prefix, _)| sym.starts_with(prefix.as_str()))
        .ok_or("Not a linked tree's symbol")?;
    let view = TryAlgorithmView {
        tree: tree_name.to_string(),
        delta,
        versus,
    };
    write_algorithm_view(cfg, spec_tree, sym, Some(view), writer)
}

/// The try pushes of the tree with semantic deltas but `head`'s, as
/// (push, head), newest first, at most `MAX_VERSUS_PUSHES`, for comparing an
/// algorithm's view at `head` with (see `format_try_algorithm`).
fn other_try_deltas(index_path: &str, head: &str) -> Vec<(String, String)> {
    const MAX_VERSUS_PUSHES: usize = 30;
    let Ok(entries) = std::fs::read_dir(Path::new(index_path).join("try-deltas")) else {
        return vec![];
    };
    let mut deltas: Vec<(std::time::SystemTime, String, String)> = entries
        .flatten()
        .filter_map(|entry| {
            let meta_path = entry.path().join("meta.json");
            let modified = std::fs::metadata(&meta_path).ok()?.modified().ok()?;
            let meta: crate::try_delta::TryDeltaMeta =
                serde_json::from_str(&std::fs::read_to_string(&meta_path).ok()?).ok()?;
            (meta.head != head).then_some((modified, meta.push, meta.head))
        })
        .collect();
    deltas.sort_by(|a, b| b.0.cmp(&a.0));
    deltas
        .into_iter()
        .take(MAX_VERSUS_PUSHES)
        .map(|(_, push, head)| (push, head))
        .collect()
}

/// An algorithm's view, beside the code citing it at the spec tree's view's
/// trees' indexed revisions, or in a try push's context (see
/// `format_try_algorithm`).
fn write_algorithm_view(
    cfg: &Config,
    tree_name: &str,
    sym: &str,
    try_view: Option<TryAlgorithmView>,
    writer: &mut dyn Write,
) -> Result<(), &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let specs = tree_specs(tree_name, tree_config).ok_or("The tree has no specs")?;
    let git = tree_config.get_git()?;
    let commit = rendered_commit(git, None)?;
    let citing: Vec<Citing> = match &try_view {
        None => vec![Citing::Specs],
        Some(view) => vec![
            Citing::TryPush {
                tree: view.tree.clone(),
                delta: view.delta.clone(),
            },
            match &view.versus {
                Some(versus) => Citing::TryPush {
                    tree: view.tree.clone(),
                    delta: versus.clone(),
                },
                None => Citing::Specs,
            },
        ],
    };
    // (The sections' algorithms' views, in the try push's context.)
    let view_url = |sym: &str| match &try_view {
        Some(view) => format!(
            "/{}/rev-algorithm/{}/{}{}",
            view.tree,
            view.delta.meta.head,
            url_encode_path(sym),
            view.versus
                .as_ref()
                .map_or_else(String::new, |versus| format!(
                    "?versus={}",
                    versus.meta.head
                ))
        ),
        None => format!("/{}/algorithm/{}", tree_name, url_encode_path(sym)),
    };
    let main = algorithm_section(
        cfg,
        tree_name,
        tree_config,
        &specs,
        git,
        &commit,
        sym,
        &citing,
    )?;
    let mut sections = vec![main];
    let called = called_algorithms(tree_config, sym);
    for called in &called {
        if let Ok(section) = algorithm_section(
            cfg,
            tree_name,
            tree_config,
            &specs,
            git,
            &commit,
            called,
            &citing,
        ) {
            sections.push(section);
        }
    }
    // The calls among the sections' algorithms, for the map.
    let numbers: HashMap<&str, usize> = sections
        .iter()
        .enumerate()
        .map(|(n, section)| (section.sym.as_str(), n))
        .collect();
    let mut calls = vec![];
    for (n, section) in sections.iter().enumerate() {
        let callees = match n {
            0 => called.clone(),
            _ => called_algorithms(tree_config, &section.sym),
        };
        for callee in callees {
            match numbers.get(callee.as_str()) {
                Some(&m) if m != n => calls.push((n, m)),
                _ => {}
            }
        }
    }
    let map = algorithm_map(&sections, &calls);
    // The sections' blames, once for each file (their index in the page's
    // `RENDERED_BLAME_INFOS`, which the sections' strips say with
    // `data-hb-ctx`; see `HyperblameContexts` in hyperblame.js).
    let mut blame_files: Vec<(String, std::sync::Arc<PageBlame>)> = vec![];
    for section in &mut sections {
        let Some(page) = &section.token_blame else {
            continue;
        };
        let ctx = match blame_files
            .iter()
            .position(|(file, _)| *file == section.file)
        {
            Some(ctx) => ctx,
            None => {
                blame_files.push((section.file.clone(), page.clone()));
                blame_files.len() - 1
            }
        };
        for strip in &mut section.strips {
            *strip = strip.replacen(
                r#"<div class="blame-strip"#,
                &format!(r#"<div data-hb-ctx="{}" class="blame-strip"#, ctx),
                1,
            );
        }
    }
    let sym_infos: Vec<String> = citing
        .iter()
        .enumerate()
        .map(|(column, citing)| citation_rows(cfg, &mut sections, column, citing))
        .collect();
    let main = &sections[0];

    let title = format!("{} - mozsearch", rendered_docs::escape(&main.name));
    let opt = Options {
        title: &title,
        tree_name,
        include_date: env::var("MOZSEARCH_DIFFABLE").is_err(),
        revision: None,
        breadcrumbs_links_to: BreadcrumbsLinksTo::Latest,
        extra_content_classes: "rendered-page",
        search_rev: None,
    };
    let panel = vec![PanelSection {
        name: "Revision control".to_owned(),
        items: vec![
            PanelItem {
                label: PanelItemLabel::Plaintext("Rendered view".to_owned()),
                tooltip: "Open the algorithm in the rendered spec".to_owned(),
                id: "panel-rendered-view",
                link: main.rendered_link.clone(),
                update_link_lineno: "",
                accel_key: None,
                copyable: true,
                extras: Default::default(),
            },
            PanelItem {
                label: PanelItemLabel::Plaintext("Source listing".to_owned()),
                tooltip: "Open the algorithm's definition in the source listing".to_owned(),
                id: "panel-source-listing",
                link: format!(
                    "/{}/source/{}#{}",
                    tree_name,
                    url_encode_path(&main.def_path),
                    main.def_line
                ),
                update_link_lineno: "",
                accel_key: None,
                copyable: true,
                extras: Default::default(),
            },
        ],
        raw_items: vec![],
    }];

    output::generate_header(&opt, writer)?;
    output::generate_breadcrumbs(&opt, writer, &main.file, &[], false)?;
    output::generate_panel(&opt, writer, &panel, false)?;
    // The trees whose code a column has, as its description says.
    let column_trees = |column: usize| -> String {
        let trees: BTreeSet<&str> = sections
            .iter()
            .flat_map(|section| &section.columns[column])
            .map(|c| c.tree.as_str())
            .collect();
        match trees.len() {
            0 => citing[column]
                .trees(tree_name, tree_config, sym)
                .join(" and "),
            _ => trees.into_iter().collect::<Vec<_>>().join(" and "),
        }
    };
    // (A try push's view's columns, and what to compare the push with.)
    let describe_push = |tree: &str, delta: &crate::try_delta::TryDelta| {
        format!(
            r#"{} at <a href="/{}/rev/{}/">the head of try push {}'s stack</a>, rebased onto its indexed revision (with the files it changes marked)"#,
            rendered_docs::escape(tree),
            tree,
            delta.meta.head,
            rendered_docs::escape(&delta.meta.push),
        )
    };
    let columns_notice = match &try_view {
        None => {
            let citing_trees: BTreeSet<&str> =
                main.columns[0].iter().map(|c| c.tree.as_str()).collect();
            match citing_trees.len() {
                0 => " (none does)".to_string(),
                _ => format!(
                    ", in {}",
                    citing_trees
                        .iter()
                        .copied()
                        .collect::<Vec<_>>()
                        .join(" and ")
                ),
            }
        }
        Some(view) => format!(
            ": on the left, in {}; on the right, in {}",
            describe_push(&view.tree, &view.delta),
            match &view.versus {
                Some(versus) => describe_push(&view.tree, versus),
                None => format!("{}, at its indexed revision", column_trees(1)),
            }
        ),
    };
    writeln!(
        writer,
        r#"<div class="rd-notice">{} beside the code citing it and its steps{}.{}{}</div>"#,
        rendered_docs::escape(&main.name),
        columns_notice,
        if !blame_files.is_empty() {
            " Hover over the bar to the left of a block to see what last changed its text."
        } else {
            ""
        },
        match sections.len() {
            1 => String::new(),
            2 => " Below, the algorithm it calls, beside the code citing it.".to_string(),
            n => format!(
                " Below, the {} algorithms it calls, each beside the code citing it.",
                n - 1
            ),
        }
    )
    .unwrap();
    if let Some(map) = &map {
        writeln!(
            writer,
            r#"<details id="sp-algorithm-map" class="sp-algorithm-map" data-own-context-menu open><summary>Map of the page's algorithms and their calls</summary>{}</details>"#,
            map
        )
        .unwrap();
    }
    if let Some(view) = &try_view {
        let tree_config = cfg.trees.get(&view.tree).ok_or("Invalid tree")?;
        let others = other_try_deltas(&tree_config.paths.index_path, &view.delta.meta.head);
        let versus = view.versus.as_ref().map(|versus| versus.meta.head.as_str());
        let mut options = format!(
            r#"<option value="">{}, at its indexed revision</option>"#,
            rendered_docs::escape(&column_trees(1))
        );
        for (push, head) in &others {
            write!(
                options,
                r#"<option value="{}"{}>{} at try push {}</option>"#,
                head,
                if versus == Some(head.as_str()) {
                    " selected"
                } else {
                    ""
                },
                rendered_docs::escape(&view.tree),
                rendered_docs::escape(push)
            )
            .unwrap();
        }
        writeln!(
            writer,
            r#"<form class="sp-versus" method="get" action="{}"><label>Compare with <select name="versus" onchange="this.form.submit()">{}</select></label></form>"#,
            view_url(sym).split('?').next().unwrap_or_default(),
            options
        )
        .unwrap();
    }
    for (n, section) in sections.iter().enumerate() {
        write_algorithm_section(writer, tree_name, &view_url, n, section);
    }
    // Each citation goes next to its step's block (or after the ones before
    // it), and the ones of the algorithm at the top, in each section, and a
    // step's block goes down to its citations, if they're below it, with space
    // after the step before it, whose text sticks while its code shows; and the
    // links to the algorithms the sections after the first are of get "↓"s
    // to their sections, and "▸"s showing their steps in place, after the
    // calling blocks (copies of their sections' renderings, without their
    // ids and line markers, whose calls can show theirs).  The map's node of the section one has scrolled to
    // (the last one starting above the map's bottom) is the current one, and
    // a node's click scrolls to its section (whose top, like the anchors',
    // stays below the map, by its height in "--sp-map-height").
    let page_sections: Vec<serde_json::Value> = sections
        .iter()
        .enumerate()
        .map(|(n, section)| {
            json!({
                "steps": section.steps,
                "anchor": if n == 0 { None } else { section.anchor.clone() },
                "name": section.name,
            })
        })
        .collect();
    // (Its cards' placement is side-cards.js's.)
    writeln!(
        writer,
        r#"<script src="/{}/static/js/side-cards.js"></script>"#,
        tree_name
    )
    .unwrap();
    writeln!(
        writer,
        r##"<script>(() => {{
  const sections = {};
  // A step's block: its text's, by its tokens' lines (`data-l`, but blanks',
  // ex: a paragraph's last, of the next line; not in the copies of called
  // algorithms' steps), as blocks' line ranges overlap (ex: a list item's,
  // without its end tag, goes to the next one's line), or else the block
  // whose range starts last before it.
  const blockOfLine = (doc, line) => {{
    const token = [...doc.querySelectorAll(`[data-l="${{line}}"]`)]
      .find(token => token.textContent.trim() && !token.closest(".sp-inline-called"));
    const block = token && (token.closest("tr") || token.closest(".rd-block"));
    if (block) {{
      return block;
    }}
    let found = null;
    let start = -1;
    for (const mark of doc.querySelectorAll("[data-rd-lines]")) {{
      const [first, last] = mark.dataset.rdLines.split("-").map(Number);
      if (first <= line && line <= last && first > start) {{
        found = mark.closest("tr") || mark.parentElement;
        start = first;
      }}
    }}
    return found;
  }};
  sections.forEach(({{ steps }}, n) => {{
    const doc = document.getElementById(n ? `rendered-doc-${{n}}` : "rendered-doc");
    const asides = [...document.querySelectorAll(`#sp-algorithm-${{n}} > .sp-algorithm-view > .sp-citations`)];
    if (!doc || !asides.length) {{
      return;
    }}
    // Each column's cards, and their steps' blocks, which stick below the
    // page's header while their code shows.
    const columns = asides.map(aside => ({{
      aside,
      cards: [...aside.querySelectorAll(":scope > .sp-citation")].map(element => {{
        const line = steps[element.dataset.step];
        return {{ element, block: (line && blockOfLine(doc, line)) || null }};
      }}),
    }}));
    const cards = columns.flatMap(({{ cards }}) => cards);
    for (const {{ block }} of cards) {{
      block?.classList.add("sp-step-sticky");
    }}
    const blocks = [...new Set(cards.map(({{ block }}) => block).filter(Boolean))]
      .sort((a, b) => a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1);
    // (The space before a step's block: after the step before it which has
    // code, at its list item's end (so its text sticks while its code shows),
    // or before its own sub-steps if it's one of them, or else before it.)
    const spaceBefore = (block, previous) => {{
      const item = previous?.parentElement?.tagName == "LI" ? previous.parentElement : null;
      const [at, where] = !item ? [block, "before"]
        : item.contains(block) ? [previous, "after"] : [item, "append"];
      const row = (where == "append" ? at.lastElementChild : at)?.tagName == "TR";
      const space = document.createElement(row ? "tr" : "div");
      space.className = "sp-step-space";
      space.setAttribute("aria-hidden", "true");
      if (row) {{
        space.append(document.createElement("td"));
      }}
      at[where](space);
      return space;
    }};
    const place = () => {{
      // (Where the steps are, not where they're stuck.)
      doc.classList.add("sp-placing");
      placeSideCards({{
        doc,
        columns,
        blocks,
        spaceClass: "sp-step-space",
        spaceBefore,
        // (A card continuing the one before it adjoins it.)
        gap: card => (card.element.classList.contains("sp-citation-continued") ? 0 : 6),
      }});
      doc.classList.remove("sp-placing");
    }};
    // Again when the view's size changes (but by its spaces), or a card's
    // (ex: its folded lines showing).
    let placed = null;
    const size = () => `${{doc.clientWidth}}x${{doc.offsetHeight}}`;
    const replace = () => {{
      place();
      placed = size();
    }};
    replace();
    const observer = new ResizeObserver(entries => {{
      if (entries.some(entry => entry.target !== doc) || size() !== placed) {{
        replace();
      }}
    }});
    observer.observe(doc);
    for (const {{ element }} of cards) {{
      observer.observe(element);
    }}
    document.fonts?.ready.then(replace);
  }});
  sections.forEach(({{ anchor }}, n) => {{
    if (!anchor) {{
      return;
    }}
    for (const link of document.querySelectorAll(".sp-algorithm a[href*='#']")) {{
      const hash = link.getAttribute("href").split("#").pop();
      if (decodeURIComponent(hash) !== anchor || link.closest(`#sp-algorithm-${{n}}`)) {{
        continue;
      }}
      const down = document.createElement("a");
      down.className = "sp-called-link";
      down.href = `#sp-algorithm-${{n}}`;
      down.title = "Go to this algorithm's section below";
      down.textContent = "↓";
      const toggle = document.createElement("button");
      toggle.type = "button";
      toggle.className = "sp-inline-toggle";
      toggle.dataset.section = n;
      toggle.title = "Show this algorithm's steps here";
      toggle.setAttribute("aria-expanded", "false");
      toggle.textContent = "▸";
      link.after(down, toggle);
    }}
  }});
  // A card's folded lines, and the lines after it, show on its buttons'
  // clicks (the latter's with the former).
  document.addEventListener("click", event => {{
    const unfold = event.target.closest(".sp-citation-unfold");
    if (!unfold) {{
      return;
    }}
    const card = unfold.closest(".sp-citation");
    const groups = unfold.dataset.unfold == "after" ? ["fold", "after"] : ["fold"];
    for (const group of groups) {{
      card.querySelector(`.sp-citation-${{group}}`)?.removeAttribute("hidden");
      card.querySelector(`.sp-citation-unfold[data-unfold="${{group}}"]`)?.remove();
    }}
  }});
  const shown = new WeakMap();
  document.addEventListener("click", event => {{
    const toggle = event.target.closest(".sp-inline-toggle");
    if (!toggle) {{
      return;
    }}
    if (shown.has(toggle)) {{
      shown.get(toggle).remove();
      shown.delete(toggle);
      toggle.textContent = "▸";
      toggle.setAttribute("aria-expanded", "false");
      return;
    }}
    const n = Number(toggle.dataset.section);
    const called = document.createElement("div");
    called.className = "sp-inline-called";
    const head = document.createElement("div");
    head.className = "sp-inline-head";
    const go = document.createElement("a");
    go.href = `#sp-algorithm-${{n}}`;
    go.textContent = "↓ beside the code citing it";
    head.append(`${{sections[n].name}} `, go);
    called.append(head);
    for (const child of document.getElementById(`rendered-doc-${{n}}`).children) {{
      called.append(child.cloneNode(true));
    }}
    // (Without its section's spaces, nor sticking steps.)
    for (const elt of called.querySelectorAll(".blame-strip, [data-rd-lines], .sp-inline-called, .sp-step-space")) {{
      elt.remove();
    }}
    for (const elt of called.querySelectorAll(".sp-step-sticky")) {{
      elt.classList.remove("sp-step-sticky");
    }}
    for (const elt of called.querySelectorAll("[id]")) {{
      elt.removeAttribute("id");
    }}
    for (const copy of called.querySelectorAll(".sp-inline-toggle")) {{
      copy.textContent = "▸";
      copy.setAttribute("aria-expanded", "false");
    }}
    // (After the calling block, and the calls' already shown after it.)
    let at = toggle.closest(".rd-block") || toggle.parentElement;
    while (at.nextElementSibling?.classList.contains("sp-inline-called")) {{
      at = at.nextElementSibling;
    }}
    at.after(called);
    shown.set(toggle, called);
    toggle.textContent = "▾";
    toggle.setAttribute("aria-expanded", "true");
  }});
  const map = document.getElementById("sp-algorithm-map");
  if (map) {{
    const nodes = [...map.querySelectorAll("g.node[id^='sp-map-']")];
    const sectionOf = node => document.getElementById(`sp-algorithm-${{node.id.slice("sp-map-".length)}}`);
    const keepHeight = () => {{
      document.documentElement.style.setProperty("--sp-map-height", `${{map.offsetHeight}}px`);
    }};
    keepHeight();
    new ResizeObserver(keepHeight).observe(map);
    const showCurrent = () => {{
      const bottom = map.getBoundingClientRect().bottom + 1;
      let current = nodes[0];
      for (const node of nodes) {{
        if (sectionOf(node)?.getBoundingClientRect().top <= bottom) {{
          current = node;
        }}
      }}
      for (const node of nodes) {{
        node.classList.toggle("sp-map-current", node === current);
      }}
    }};
    let pending = false;
    document.addEventListener("scroll", () => {{
      if (!pending) {{
        pending = true;
        requestAnimationFrame(() => {{
          pending = false;
          showCurrent();
        }});
      }}
    }}, {{ passive: true }});
    showCurrent();
    // A node's click goes to its section (the map has no symbols' menus on
    // clicks: see `data-own-context-menu`), and its right-click is a menu of
    // its algorithm's pages.
    for (const node of nodes) {{
      node.addEventListener("click", () => {{
        keepHeight();
        sectionOf(node)?.scrollIntoView();
      }});
      node.addEventListener("contextmenu", event => {{
        const section = sectionOf(node);
        if (!section || typeof ContextMenu == "undefined") {{
          return;
        }}
        event.preventDefault();
        const n = Number(node.id.slice("sp-map-".length));
        const link = selector => section.querySelector(selector)?.getAttribute("href");
        const items = [
          ["Go to its section", `#${{section.id}}`],
          ["Show it beside the code citing it, and the algorithms it calls",
           n ? link(".sp-called-view") : null],
          ["Show it in its spec", n ? link(".sp-called-rendered") : null],
        ].filter(([, href]) => href).map(([html, href]) => new MenuItem({{
          html,
          href,
          preaction: () => ContextMenu.hide(),
          icon: "export-alt",
          section: "algorithm-map",
        }}));
        ContextMenu.showItems(items, event);
      }});
    }}
  }}
}})();</script>"##,
        serde_json::to_string(&page_sections)
            .unwrap()
            .replace("</", "<\\/")
    )
    .unwrap();
    // (A view with columns of code from different trees or revisions has
    // each's SYM_INFO, for context menus in it; see `citation_rows`.)
    writeln!(
        writer,
        "<script>var SYM_INFO = {};</script>",
        sym_infos[0].replace("</", "<\\/")
    )
    .unwrap();
    if sym_infos.len() > 1 {
        writeln!(
            writer,
            "<script>var SYM_INFOS = [{}];</script>",
            sym_infos.join(",").replace("</", "<\\/")
        )
        .unwrap();
    }
    if !blame_files.is_empty() {
        let infos: Vec<&crate::hyperblame::page_blame::BlameInfo> =
            blame_files.iter().map(|(_, page)| &page.info).collect();
        writeln!(
            writer,
            "<script>var RENDERED_BLAME_INFOS = {};</script>",
            to_string(&infos).unwrap().replace("</", "<\\/")
        )
        .unwrap();
    }
    output::generate_footer(&opt, tree_name, &main.file, writer)
}

/// The map of an algorithm's view (see `format_algorithm`), if it has more
/// than one section: a diagram of the sections' algorithms and the `calls`
/// among them (of their numbers), rendered like the query pages' diagrams (see
/// the "graph" command), but whose nodes are the sections' (`sp-map-{n}`) and
/// not symbols, so that the page's script can make them its sections' map;
/// their labels are their names, but the main algorithm's spec's shortname.
fn algorithm_map(sections: &[AlgorithmSection], calls: &[(usize, usize)]) -> Option<String> {
    use dot_generator::*;
    use dot_structures::*;
    use graphviz_rust::cmd::{CommandArg, Format, Layout};
    use graphviz_rust::printer::PrinterContext;

    if sections.len() < 2 {
        return None;
    }
    let mut graph = graph!(
        di id!("g");
        node!("graph"; attr!("ranksep", "0.25"), attr!("nodesep", "0.2")),
        node!("node"; attr!("shape","box"), attr!("fontname", esc "Courier New"), attr!("fontsize", "10"))
    );
    // (Not symbols' ids, which would make the nodes' "data-symbols".)
    let id = |n: usize| NodeId(Id::Plain(format!("SYN_MAP_{}", n)), None);
    let prefix = format!("{} ", sections[0].shortname);
    for (n, section) in sections.iter().enumerate() {
        let label = section.name.strip_prefix(&prefix).unwrap_or(&section.name);
        let label = label.replace('\\', "\\\\").replace('"', "\\\"");
        graph.add_stmt(stmt!(Node::new(
            id(n),
            vec![
                attr!("label", esc label),
                attr!("id", esc format!("sp-map-{}", n)),
            ]
        )));
    }
    for &(from, to) in calls {
        graph.add_stmt(stmt!(edge!(id(from) => id(to))));
    }
    let svg = graphviz_rust::exec(
        graph,
        &mut PrinterContext::default(),
        vec![
            CommandArg::Format(Format::Svg),
            CommandArg::Layout(Layout::Dot),
        ],
    )
    .ok()?;
    let svg = crate::cmd_pipeline::transform_svg(std::str::from_utf8(&svg).ok()?);
    // (Without its XML prolog and doctype.)
    Some(svg[svg.find("<svg")?..].to_string())
}

/// How much indentation an excerpt's lines have in common.
fn excerpt_indent(excerpt: &[String]) -> usize {
    excerpt
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0)
}

/// The rows (see `chunked_gzip`) of the lines `lines` of the tree's file at
/// `path` in its source listing, or with `delta`, at the try push's head:
/// rendered from the push's analysis (with `jumprefs`, which should have the
/// push's) if it changes the file's source or analysis, as the head's listings
/// are (see `abstract_server::TryOverlay`).
pub(crate) fn file_rows(
    cfg: &Config,
    tree: &str,
    delta: Option<&crate::try_delta::TryDelta>,
    jumprefs: &TreeJumprefs,
    path: &str,
    lines: &BTreeSet<u32>,
) -> Option<HashMap<u32, crate::file_format::chunked_gzip::Row>> {
    use crate::file_format::analysis::{read_analysis, read_source};
    use crate::file_format::chunked_gzip::{extract_rows, read_rows};
    use crate::try_delta::FileChange;

    let index_path = cfg.tree_index_paths.get(tree)?;
    let rendered_by = delta.filter(|delta| {
        matches!(
            delta.change(path),
            Some(FileChange::Added | FileChange::Modified)
        ) || delta.meta.analyzed.contains_key(path)
    });
    if let Some(delta) = rendered_by {
        let source = match delta.generated_source(path) {
            Some(file) => std::fs::read_to_string(file).ok(),
            None => cfg
                .trees
                .get(tree)
                .and_then(|tree_config| tree_config.get_git().ok())
                .and_then(|git| {
                    let head = git
                        .repo
                        .revparse_single(&delta.meta.head)
                        .ok()?
                        .peel_to_commit()
                        .ok()?;
                    read_commit_file(&git.repo, &head, path)
                }),
        }?;
        // (A file it changes which its analysis doesn't have has none.)
        let analysis = delta
            .analysis_file(path)
            .filter(|file| file.exists())
            .map(|file| read_analysis(&file.to_string_lossy(), &mut read_source))
            .unwrap_or_default();
        let (html, _) = source_rows_html(jumprefs, path, &source, &analysis);
        return Some(extract_rows(&html, lines));
    }
    let gz = format!("{}/file/{}.gz", index_path, path);
    read_rows(Path::new(&gz), lines).ok()
}

/// A row of a source listing (see `chunked_gzip::ROW_START`) as an excerpt on
/// another page, like `/query/`'s results' (see `excerpt_row`): without its id,
/// and the `indent` leading blanks of its code, with a link to its line (`link`,
/// with the line as its fragment) as its line number (with the class
/// `lineno_class`, not `.line-number`, which code-highlighter.js would select
/// in the page), and with `row_class`, if any, added to its classes.
pub(crate) fn excerpt_row_html(
    row: &str,
    line: u32,
    link: &str,
    lineno_class: &str,
    indent: usize,
    row_class: Option<&str>,
) -> String {
    const CODE: &str = r#"<code role="cell" class="source-line">"#;
    let mut row = crate::cmd_pipeline::excerpt_row(row)
        .replacen(&format!(r#" id="line-{}""#, line), "", 1)
        .replacen(
            &format!(
                r#"<div role="cell" class="line-number" data-line-number="{}"></div>"#,
                line
            ),
            &format!(
                r#"<a role="cell" class="{}" href="{}#{}" data-line-number="{}"></a>"#,
                lineno_class, link, line, line
            ),
            1,
        );
    if let Some(class) = row_class {
        row = row.replacen(
            r#"class="source-line-with-number"#,
            &format!(r#"class="source-line-with-number {}"#, class),
            1,
        );
    }
    match row.find(CODE) {
        Some(at) => {
            let code = at + CODE.len();
            let blank = row[code..]
                .bytes()
                .take(indent)
                .take_while(|b| *b == b' ' || *b == b'\t')
                .count();
            format!("{}{}", &row[..code], &row[code + blank..])
        }
        None => row,
    }
}

/// Add the SYM_INFO (see `format_code`) of the symbols in rows' `html` (their
/// `data-symbols`) to `sym_info`, by `jumprefs` (see `add_sym_info`).
pub(crate) fn add_rows_sym_info(
    jumprefs: &dyn JumprefSource,
    html: &str,
    sym_info: &mut BTreeMap<Ustr, Option<JumprefData>>,
    traversed: &mut UstrMap<JumprefTraversals>,
) {
    const DATA_SYMBOLS: &str = "data-symbols=\"";
    let mut rest = html;
    while let Some(start) = rest.find(DATA_SYMBOLS) {
        rest = &rest[start + DATA_SYMBOLS.len()..];
        let end = rest.find('"').unwrap_or(rest.len());
        for sym in rest[..end].split(',').filter(|sym| !sym.is_empty()) {
            let sym = ustr(sym);
            if !sym_info.contains_key(&sym) {
                jumprefs.add_sym_info(sym, sym_info, traversed);
            }
        }
        rest = &rest[end..];
    }
}

/// Mark the citations (of a section's column, in its order) which continue
/// the one before them (see `Citation::continues`): of the same tree's file,
/// at the same revision, and function, after it (not WebIDL members and their
/// implementations, which say what they are).
fn mark_continuations(citations: &mut [Citation]) {
    for i in 1..citations.len() {
        let (before, rest) = citations.split_at_mut(i);
        let (previous, citation) = (&mut before[i - 1], &mut rest[0]);
        let end = previous.line + previous.excerpt.len() as u32;
        if (
            &previous.tree,
            &previous.path,
            &previous.rev,
            &previous.function,
        ) != (
            &citation.tree,
            &citation.path,
            &citation.rev,
            &citation.function,
        ) || citation.line < end
            || previous.note.is_some()
            || citation.note.is_some()
        {
            continue;
        }
        let between = citation.line - end;
        citation.continues = Some(between);
        // (Its lines after it are those between them, if they're all there.)
        previous.continued_closely =
            between <= CLOSE_CONTINUATION_LINES && previous.after.len() == between as usize;
    }
}

/// How many lines can be between a citation and the one continuing it for
/// them to show, rather than a note of how many there are.
const CLOSE_CONTINUATION_LINES: u32 = 3;

/// The indentation the citations' excerpts lose (see `excerpt_row_html`):
/// their runs' (see `mark_continuations`) common indentation, so that their
/// code lines up.
fn run_indents(citations: &[Citation]) -> Vec<usize> {
    let mut indents = vec![];
    let mut start = 0;
    for i in 0..=citations.len() {
        if i == citations.len() || (i > start && citations[i].continues.is_none()) {
            let indent = citations[start..i]
                .iter()
                .map(|citation| excerpt_indent(&citation.excerpt))
                .min()
                .unwrap_or(0);
            indents.extend(std::iter::repeat_n(indent, i - start));
            start = i;
        }
    }
    indents
}

/// The buttons showing a citation's folded lines, and the lines after it.
fn unfold_buttons(folded: usize, after: usize) -> String {
    let mut html = String::new();
    if folded > 0 {
        html.push_str(&format!(
            r#"<button type="button" class="sp-citation-unfold" data-unfold="fold">Show {} more line{}</button>"#,
            folded,
            if folded == 1 { "" } else { "s" }
        ));
    }
    if after > 0 {
        html.push_str(&format!(
            r#"<button type="button" class="sp-citation-unfold" data-unfold="after" title="In case its step goes on">Show the {} line{} after it</button>"#,
            after,
            if after == 1 { "" } else { "s" }
        ));
    }
    if html.is_empty() {
        html
    } else {
        format!(r#"<div class="sp-citation-unfolds">{}</div>"#, html)
    }
}

/// The sections' citations' excerpts in their view's `column`, of the code
/// `citing` their algorithms, as their lines' rows in their trees' source
/// listings (see `chunked_gzip`), if the trees have them, like `/query/`'s
/// results' (see `excerpt_row`), so that they have the listings' syntax
/// highlighting and symbols: without their common indentation, in their
/// trees' (`data-tree`, and its search, which context-menu.js searches with),
/// and with links to their lines in the listings (not `.line-number`s, which
/// code-highlighter.js would select in this page).  A try push's files whose
/// sources or analysis it changes are rendered by its delta, as its head's
/// listings are.  And the SYM_INFO (see `format_code`) of their symbols, whose
/// jumps are to their trees' files (by their `tree`s), for their context
/// menus.
fn citation_rows(
    cfg: &Config,
    sections: &mut [AlgorithmSection],
    column: usize,
    citing: &Citing,
) -> String {
    use crate::file_format::chunked_gzip::Row;

    let delta = citing.delta();
    // Each tree's jumprefs (a try push's with its delta's).
    let mut tree_jumprefs: HashMap<String, TreeJumprefs> = HashMap::new();
    let mut jumprefs_of = |tree: &str| -> TreeJumprefs {
        tree_jumprefs
            .entry(tree.to_string())
            .or_insert_with(|| {
                let mut jumprefs = cfg
                    .trees
                    .get(tree)
                    .map(|tree_config| {
                        TreeJumprefs::open(&tree_config.paths, &cfg.tree_index_paths)
                    })
                    .unwrap_or_default();
                jumprefs.delta = delta.and_then(|delta| delta.jumprefs());
                jumprefs
            })
            .clone()
    };
    for section in sections.iter_mut() {
        mark_continuations(&mut section.columns[column]);
    }
    let mut lines: BTreeMap<(String, String), BTreeSet<u32>> = BTreeMap::new();
    for citation in sections.iter().flat_map(|section| &section.columns[column]) {
        lines
            .entry((citation.tree.clone(), citation.path.clone()))
            .or_default()
            .extend(
                citation.line
                    ..citation.line + (citation.excerpt.len() + citation.after.len()) as u32,
            );
    }
    let mut rows: HashMap<(String, String), HashMap<u32, Row>> = HashMap::new();
    for ((tree, path), lines) in lines {
        let jumprefs = jumprefs_of(&tree);
        if let Some(file_rows) = file_rows(cfg, &tree, delta, &jumprefs, &path, &lines) {
            rows.insert((tree, path), file_rows);
        }
    }

    // Each tree's SYM_INFO, and its traversals (see `add_sym_info`).
    type TreeSymInfo = (
        TreeJumprefs,
        BTreeMap<Ustr, Option<JumprefData>>,
        UstrMap<JumprefTraversals>,
    );
    let mut tree_sym_info: BTreeMap<String, TreeSymInfo> = BTreeMap::new();
    for section in sections.iter_mut() {
        let indents = run_indents(&section.columns[column]);
        for (citation, indent) in section.columns[column].iter_mut().zip(indents) {
            let Some(file_rows) = rows.get(&(citation.tree.clone(), citation.path.clone())) else {
                continue;
            };
            // (All of their rows, or the excerpt's text.)
            let url = citation_url(citation);
            let row_lines = |from: usize, to: usize, class: Option<&str>| -> Option<String> {
                (from..to)
                    .map(|i| {
                        let line = citation.line + i as u32;
                        let row = file_rows.get(&line)?;
                        Some(excerpt_row_html(
                            &row.html,
                            line,
                            &url,
                            "sp-citation-lineno",
                            indent,
                            class,
                        ))
                    })
                    .collect()
            };
            let (len, after) = (citation.excerpt.len(), citation.after.len());
            let Some(shown) = row_lines(0, citation.shown, None) else {
                continue;
            };
            let Some(folded) = row_lines(citation.shown, len, None) else {
                continue;
            };
            // (The lines up to the citation continuing it closely show,
            // lightened; others' are folded.)
            let Some(after_rows) = row_lines(
                len,
                len + after,
                citation.continued_closely.then_some("sp-citation-between"),
            ) else {
                continue;
            };
            let mut html = shown;
            if !folded.is_empty() {
                html.push_str(&format!(
                    r#"<div class="sp-citation-fold" role="rowgroup" hidden>{}</div>"#,
                    folded
                ));
            }
            if citation.continued_closely {
                html.push_str(&after_rows);
            } else if !after_rows.is_empty() {
                html.push_str(&format!(
                    r#"<div class="sp-citation-after" role="rowgroup" hidden>{}</div>"#,
                    after_rows
                ));
            }
            let (jumprefs, sym_info, traversed) = tree_sym_info
                .entry(citation.tree.clone())
                .or_insert_with(|| {
                    (
                        jumprefs_of(&citation.tree),
                        BTreeMap::new(),
                        UstrMap::default(),
                    )
                });
            add_rows_sym_info(jumprefs, &html, sym_info, traversed);
            citation.rows = Some(format!(
                r#"<div class="file sp-citation-rows" role="table" data-tree="{}" data-search="{}"{}>{}</div>{}"#,
                rendered_docs::escape(&citation.tree),
                config::search_url(&citation.tree),
                citation
                    .rev
                    .as_ref()
                    .map_or_else(String::new, |rev| format!(r#" data-search-rev="{}""#, rev)),
                html,
                unfold_buttons(
                    len - citation.shown,
                    if citation.continued_closely { 0 } else { after }
                ),
            ));
        }
    }

    let mut page_sym_info: BTreeMap<Ustr, Option<JumprefData>> = BTreeMap::new();
    for (tree, (_, sym_info, _)) in tree_sym_info {
        let tree = ustr(&tree);
        for (sym, mut jumpref) in sym_info {
            if let Some(jumpref) = jumpref.as_mut() {
                // (The tree's own symbols' jumps are to its files.)
                jumpref.tree.get_or_insert(tree);
                // (The history digests are for `/query/`'s results.)
                jumpref.recency = None;
                jumpref.recency_from = None;
            }
            page_sym_info.entry(sym).or_insert(jumpref);
        }
    }
    to_string(&page_sym_info).unwrap()
}

/// An algorithm's section of its view (the `n`th): its heading (but the
/// first's), with a link to its own view (by `view_url`), and its rendering
/// with its strips, after its first column of citations and before the
/// others.  (Columns of code from different trees or revisions have their
/// SYM_INFOs, see `write_algorithm_view`.)
fn write_algorithm_section(
    writer: &mut dyn Write,
    tree_name: &str,
    view_url: &dyn Fn(&str) -> String,
    n: usize,
    section: &AlgorithmSection,
) {
    let suffix = if n == 0 {
        String::new()
    } else {
        format!("-{}", n)
    };
    write!(
        writer,
        r#"<section class="sp-algorithm-section" id="sp-algorithm-{}">"#,
        n
    )
    .unwrap();
    if n > 0 {
        writeln!(
            writer,
            r#"<h2 class="sp-called-heading">{} <a class="sp-called-view" href="{}" title="Show it beside the code citing it, and the algorithms it calls">⇄</a> <a class="sp-called-rendered" href="{}">{}</a></h2>"#,
            rendered_docs::escape(&section.name),
            view_url(&section.sym),
            rendered_docs::escape(&section.rendered_link),
            rendered_docs::escape(&section.source_path),
        )
        .unwrap();
    }
    let several = section.columns.len() > 1;
    let write_article = |writer: &mut dyn Write| {
        write!(
            writer,
            r#"<article id="rendered-doc{}" class="rendered-doc rendered-spec sp-algorithm">"#,
            suffix
        )
        .unwrap();
        let mut pos = 0;
        let html = &section.view.rendered.html;
        for (block, strip) in section.view.rendered.blocks.iter().zip(&section.strips) {
            let at = block.strip_at.clamp(pos, html.len());
            writer.write_all(&html.as_bytes()[pos..at]).unwrap();
            writer.write_all(strip.as_bytes()).unwrap();
            pos = at;
        }
        writer.write_all(&html.as_bytes()[pos..]).unwrap();
        writeln!(writer, "</article>").unwrap();
    };
    // (The code on the left, and the steps it implements on the right, and
    // the other columns' code after them.)
    writeln!(
        writer,
        r#"<div class="sp-algorithm-view{}">"#,
        if several { " sp-algorithm-columns" } else { "" }
    )
    .unwrap();
    for (column, citations) in section.columns.iter().enumerate() {
        if column == 1 {
            write_article(writer);
        }
        writeln!(
            writer,
            r#"<aside id="sp-citations{}{}" class="sp-citations"{}>"#,
            suffix,
            if column == 0 {
                String::new()
            } else {
                format!("-c{}", column)
            },
            if several {
                format!(r#" data-sym-info="{}""#, column)
            } else {
                String::new()
            }
        )
        .unwrap();
        for citation in citations {
            let step = citation.step.as_deref().unwrap_or("");
            let key = step_key(step);
            let known = citation.step.is_none() || section.steps.contains_key(key);
            write!(
                writer,
                r#"<div class="sp-citation{}{}" data-step="{}">"#,
                if citation.changed {
                    " sp-citation-changed"
                } else {
                    ""
                },
                if citation.continues.is_some() {
                    " sp-citation-continued"
                } else {
                    ""
                },
                rendered_docs::escape(key),
            )
            .unwrap();
            // Its file and function, unless it continues the one before it
            // (then how many lines are between them, if they don't show).
            match citation.continues {
                None => write!(
                    writer,
                    r#"<div class="sp-citation-head"><a href="{}#{}">{}{}:{}</a> <span class="sp-citation-fn">{}</span>{}{}</div>"#,
                    citation_url(citation),
                    citation.line,
                    if citation.tree == tree_name {
                        String::new()
                    } else {
                        format!("{}: ", rendered_docs::escape(&citation.tree))
                    },
                    rendered_docs::escape(&citation.path),
                    citation.line,
                    rendered_docs::escape(&citation.function),
                    citation.note.as_deref().map_or_else(String::new, |note| format!(
                        r#" <span class="sp-citation-note">{}</span>"#,
                        rendered_docs::escape(note)
                    )),
                    if citation.changed {
                        r#" <span class="sp-citation-pushed" title="The try push changes this file">changed by the push</span>"#
                    } else {
                        ""
                    },
                )
                .unwrap(),
                Some(between) if between > CLOSE_CONTINUATION_LINES => write!(
                    writer,
                    r#"<a class="sp-citation-skip" href="{}#{}">⋮ {} lines</a>"#,
                    citation_url(citation),
                    citation.line - between,
                    between
                )
                .unwrap(),
                Some(_) => {}
            }
            // Its step, in the gutter beside its code.
            let label = match (&citation.step, known) {
                (Some(step), true) => format!(
                    r#"<span class="sp-citation-step">step {}</span>{}"#,
                    rendered_docs::escape(step),
                    match &citation.drift {
                        Some((time, rev)) => format!(
                            r#" <a class="sp-drift" href="/{}/commit/{}" title="The spec changed this step's text on {}, after this comment was written">changed since</a>"#,
                            tree_name,
                            rev,
                            chrono::DateTime::from_timestamp(*time, 0)
                                .map_or_else(String::new, |date| date
                                    .format("%Y-%m-%d")
                                    .to_string())
                        ),
                        None => String::new(),
                    }
                ),
                (Some(step), false) => format!(
                    r#"<span class="sp-citation-step sp-citation-unknown" title="The spec's algorithm has no such step now">step {}?</span>"#,
                    rendered_docs::escape(step)
                ),
                (None, _) => String::new(),
            };
            write!(
                writer,
                r#"<div class="sp-citation-body"><div class="sp-citation-gutter">{}</div><div class="sp-citation-lines">"#,
                label
            )
            .unwrap();
            if let Some(rows) = &citation.rows {
                writeln!(writer, "{}</div></div></div>", rows).unwrap();
                continue;
            }
            // (Without their common indentation.)
            let indent = excerpt_indent(&citation.excerpt);
            write!(writer, r#"<pre class="sp-citation-code">"#).unwrap();
            for line in &citation.excerpt {
                writeln!(
                    writer,
                    "{}",
                    rendered_docs::escape(line.get(indent..).unwrap_or(line.trim_start()))
                )
                .unwrap();
            }
            writeln!(writer, "</pre></div></div></div>").unwrap();
        }
        if citations.is_empty() {
            writeln!(
                writer,
                r#"<div class="sp-citations-none">No code cites this algorithm.</div>"#
            )
            .unwrap();
        }
        writeln!(writer, "</aside>").unwrap();
    }
    if !several {
        write_article(writer);
    }
    writeln!(writer, "</div></section>").unwrap();
}

/// A file of a commit, as text (ex: an ecmarkup spec's imports).
fn read_commit_file(repo: &Repository, commit: &git2::Commit, path: &str) -> Option<String> {
    let entry = commit.tree().ok()?.get_path(Path::new(path)).ok()?;
    let blob = repo.find_blob(entry.id()).ok()?;
    Some(String::from_utf8_lossy(blob.content()).into_owned())
}

/// The commit of a rendered view: the revision's, or the tip's.
fn rendered_commit<'a>(
    git: &'a GitData,
    rev: Option<&str>,
) -> Result<git2::Commit<'a>, &'static str> {
    match rev {
        Some(rev) => git
            .repo
            .revparse_single(rev)
            .and_then(|object| object.peel_to_commit())
            .map_err(|_| "Bad revision"),
        None => git
            .repo
            .head()
            .and_then(|head| head.peel_to_commit())
            .map_err(|_| "No revision"),
    }
}

/// Where a rendered view's path redirects to, if it's to a spec's anchor or
/// line, which are on one of its pages: the page, with the anchor or line as
/// its fragment (or the first page, if the source doesn't have the anchor).
pub fn rendered_redirect(
    cfg: &Config,
    tree_name: &str,
    rev: Option<&str>,
    path: &str,
) -> Option<String> {
    let tree_config = cfg.trees.get(tree_name)?;
    let specs = tree_specs(tree_name, tree_config)?;
    let (spec, source_path, view) = spec_view(&specs, path.trim_end_matches('/'))?;
    if matches!(view, SpecView::Page(_)) {
        return None;
    }
    let git = tree_config.get_git().ok()?;
    let commit = rendered_commit(git, rev).ok()?;
    let (repo, oid) = get_object_at(
        OwnedOrBorrowed::Borrowed(&git.repo),
        commit.id(),
        Path::new(&source_path),
    )
    .ok()?;
    let object = repo.find_object(oid, Some(git2::ObjectType::Blob)).ok()?;
    let source = git_ops::read_blob_object(&object);
    let read = |file: &str| read_commit_file(&git.repo, &commit, file);
    let base = rendered_url(tree_name, rev, &source_path);
    let (page, hash) = match view {
        SpecView::Anchor(anchor) => {
            match rendered_specs::anchor_page(spec, &source, &source_path, &specs, &read, &anchor) {
                Some(page) => (page.name, rendered_specs::fragment(&anchor)),
                // (A paged spec's first page would send it back here.)
                None if spec.format.is_some_and(rendered_specs::is_paged) => (None, String::new()),
                None => (None, rendered_specs::fragment(&anchor)),
            }
        }
        SpecView::Line(line) => {
            let offset = match line {
                0 | 1 => 0,
                _ => source
                    .match_indices('\n')
                    .nth(line - 2)
                    .map_or(source.len(), |(i, _)| i + 1),
            };
            let page =
                rendered_specs::offset_page(spec, &source, &source_path, &specs, &read, offset);
            (page.and_then(|page| page.name), format!("#{}", line))
        }
        SpecView::Page(_) => return None,
    };
    Some(format!(
        "{}{}",
        rendered_specs::page_url(&base, page.as_deref()),
        hash
    ))
}

/// How a rendered document links to the tree's other files, at the tip or at
/// a revision.
struct DocLinks<'a> {
    tree_name: &'a str,
    tree_config: &'a TreeConfig,
    /// The revision, for pages other than the tip's.
    rev: Option<String>,
    commit: &'a git2::Commit<'a>,
    tree: Tree<'a>,
    doc_trees: Option<std::sync::Arc<DocTrees>>,
    jumprefs: Option<std::sync::Arc<BisectableMmap<JumprefData>>>,
}

impl rendered_docs::Links for DocLinks<'_> {
    fn exists(&self, path: &str) -> bool {
        !path.is_empty() && self.tree.get_path(Path::new(path)).is_ok()
    }

    fn source_url(&self, path: &str) -> String {
        match &self.rev {
            Some(rev) => format!("/{}/rev/{}/{}", self.tree_name, rev, url_encode_path(path)),
            None => format!("/{}/source/{}", self.tree_name, url_encode_path(path)),
        }
    }

    fn rendered_url(&self, path: &str) -> String {
        rendered_url(self.tree_name, self.rev.as_deref(), path)
    }

    fn raw_url(&self, path: &str) -> Option<String> {
        let hg_rev = self
            .tree_config
            .git
            .as_ref()
            .and_then(|git| git.hg_rev(self.commit.id()))
            .unwrap_or_else(|| "default".to_string());
        self.tree_config.paths.make_raw_resource_rev_url(
            &self.commit.id().to_string(),
            &hg_rev,
            path,
        )
    }

    fn search_url(&self, query: &str) -> String {
        config::search_link(self.tree_name, query, true)
    }

    fn tree_name(&self) -> &str {
        self.tree_name
    }

    fn staged_doc(&self, staged: &str) -> Option<String> {
        self.doc_trees.as_ref()?.find_source(staged)
    }

    /// The label's documentation (its doc-def, if it has one), at the tip,
    /// whose line numbers the cross-reference has: the line of its rendered
    /// view (see `RenderedBlocks` in hyperblame.js), or of its source listing.
    fn label_url(&self, label: &str) -> Option<String> {
        let jumprefs = self.jumprefs.as_ref()?;
        let lookup = |label: &str| {
            let jumpref = jumprefs
                .lookup(&rendered_docs::label_symbol(label))
                .ok()??;
            jumpref.jumps.doc
        };
        // (Labels can be prefixed by their document; see `Doc::find_label`.)
        let doc = lookup(label).or_else(|| lookup(label.rsplit_once(':')?.1))?;
        let (path, line) = doc.rsplit_once('#')?;
        let url = if has_rendered_view(path) {
            rendered_url(self.tree_name, None, path)
        } else {
            format!("/{}/source/{}", self.tree_name, url_encode_path(path))
        };
        Some(format!("{}#{}", url, line))
    }
}

/// The blame strips of a rendered document's blocks, which merge the strips
/// of their lines (see `rendered_docs::block_lines`), with the same classes
/// and data attributes as source listings' strips, plus the block's index
/// (`data-rd-block`) and (1-based) lines (`data-rd-lines`).  Only the lines
/// of the bytes `range` of the source count (ex: a page's).
fn rendered_strips(
    rendered: &Rendered,
    source: &str,
    page: &PageBlame,
    range: &Range<usize>,
) -> Vec<String> {
    let mut line_starts = vec![0];
    line_starts.extend(source.match_indices('\n').map(|(i, _)| i + 1));
    let line_of = |offset: usize| line_starts.partition_point(|&start| start <= offset) - 1;
    let lines = line_of(range.start)..=line_of(range.end.saturating_sub(1).max(range.start));
    let has_tokens = |line: usize| {
        lines.contains(&line) && page.info.token_counts.get(line).is_some_and(|&n| n > 0)
    };
    let ranges =
        rendered_docs::block_lines(&rendered.blocks, line_of, page.lines.len(), has_tokens);
    let mut strips: Vec<StripLine> = ranges
        .iter()
        .map(|range| match range {
            Some(lines) => page.merge_lines(lines.clone()),
            None => StripLine::default(),
        })
        .collect();
    interpolate_blank_lines(&mut strips);

    // Like source listings, we alternate colors whenever the commit changes.
    let mut last_commit = None;
    let mut color = false;
    strips
        .iter()
        .zip(&ranges)
        .enumerate()
        .map(|(i, (strip, lines))| {
            let color_class = match strip.commit {
                Some(commit) => {
                    if last_commit != Some(commit) {
                        color = !color;
                    }
                    last_commit = Some(commit);
                    if color { " c1" } else { " c2" }
                }
                None => "",
            };
            let (classes, data) = strip.strip_attrs();
            let lines = lines.as_ref().map_or(String::new(), |lines| {
                format!("{}-{}", lines.start() + 1, lines.end() + 1)
            });
            format!(
                r#"<div class="blame-strip{}{}"{} data-rd-block="{}" data-rd-lines="{}" role="button" aria-label="blame of this block" aria-expanded="false"></div>"#,
                color_class, classes, data, i, lines
            )
        })
        .collect()
}

/// The (0-based) lines of a rendered document's blocks (see
/// `rendered_docs::block_lines`), and of the bytes `range` of the source it
/// covers (ex: a page's).  Only the lines of `range` count.
fn rendered_block_lines(
    rendered: &Rendered,
    source: &str,
    range: &Range<usize>,
) -> Vec<Option<std::ops::RangeInclusive<usize>>> {
    let lines: Vec<&str> = source.lines().collect();
    let in_range = rendered_range_lines(source, range);
    let mut line_starts = vec![0];
    line_starts.extend(source.match_indices('\n').map(|(i, _)| i + 1));
    let line_of = |offset: usize| line_starts.partition_point(|&start| start <= offset) - 1;
    let has_text = |line: usize| {
        in_range.contains(&line) && lines.get(line).is_some_and(|line| !line.trim().is_empty())
    };
    rendered_docs::block_lines(&rendered.blocks, line_of, lines.len(), has_text)
}

/// The (0-based) lines of the bytes `range` of a source.
fn rendered_range_lines(source: &str, range: &Range<usize>) -> Range<usize> {
    let line_of = |offset: usize| {
        source.as_bytes()[..offset.min(source.len())]
            .iter()
            .filter(|&&b| b == b'\n')
            .count()
    };
    line_of(range.start)..line_of(range.end.saturating_sub(1).max(range.start)) + 1
}

/// For a rendered document without blame, markers of its blocks' lines, like
/// their strips' `data-rd-lines`, for links to its lines.  Only the lines of
/// the bytes `range` of the source count (ex: a page's).
fn rendered_line_markers(rendered: &Rendered, source: &str, range: &Range<usize>) -> Vec<String> {
    rendered_block_lines(rendered, source, range)
        .iter()
        .map(|range| match range {
            Some(range) => format!(
                r#"<span class="rd-lines" data-rd-lines="{}-{}" hidden></span>"#,
                range.start() + 1,
                range.end() + 1
            ),
            None => String::new(),
        })
        .collect()
}

lazy_static! {
    /// A rendered spec's definitions' and links' opening tags, and its
    /// headings' tags (see `annotate_spec_symbols`).
    static ref SPEC_DFN_OR_LINK: regex::Regex =
        regex::Regex::new(r#"<(dfn|a|h[1-6])(\s[^>]*)?>|</h[1-6]>"#).unwrap();
    static ref ATTR_ID: regex::Regex = regex::Regex::new(r#"\sid="([^"]*)""#).unwrap();
    static ref ATTR_HREF: regex::Regex = regex::Regex::new(r#"\shref="([^"]*)""#).unwrap();
}

/// An attribute's value without its HTML escapes.
fn unescape_attr(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// A page of a spec's rendered view, with its definitions and its links to
/// definitions annotated for the context menu (see context-menu.js's rendered
/// specs' symbols), if the tree has their symbols (`has_sym`; see
/// `spec_symbol`), which `syms` gets: a definition (`<dfn id>`) gets its
/// anchor's symbol (`data-symbols`), and a link to an anchor of the tree's
/// specs' rendered views (on the page, `#{anchor}`, or another,
/// `/{tree}/rendered/{path}#{anchor}` or `.../anchor/{anchor}`) its
/// definition's symbol and its URL (`data-rendered-def`), where "Go to
/// definition" goes, rather than to the definition's source.  A heading's
/// `<dfn>` without an id (ex: Wattsi's, whose headings have their `<dfn>`s'
/// ids, ex: the HTML parser's tokenizer states) gets the heading's.
fn annotate_spec_symbols(
    html: &str,
    tree_name: &str,
    specs: &SpecsConfig,
    spec: &SpecEntry,
    has_sym: &dyn Fn(&str) -> bool,
    syms: &mut Vec<String>,
) -> String {
    let rendered_prefix = format!("/{}/rendered/", tree_name);
    let decode = |text: &str| {
        urlencoding::decode(text)
            .map(|text| text.into_owned())
            .unwrap_or_else(|_| text.to_string())
    };
    // (The id of the heading the tags are in, if any.)
    let mut heading_id: Option<String> = None;
    SPEC_DFN_OR_LINK
        .replace_all(html, |caps: &regex::Captures| {
            let tag = &caps[0];
            let Some(name) = caps.get(1).map(|m| m.as_str()) else {
                heading_id = None;
                return tag.to_string();
            };
            let attrs = caps.get(2).map_or("", |m| m.as_str());
            if name.starts_with('h') {
                heading_id = ATTR_ID.captures(attrs).map(|id| id[1].to_string());
                return tag.to_string();
            }
            if attrs.contains("data-symbols=") {
                return tag.to_string();
            }
            let (sym, rendered_def) = if name == "dfn" {
                let Some(id) = ATTR_ID
                    .captures(attrs)
                    .map(|id| id[1].to_string())
                    .or_else(|| heading_id.clone())
                else {
                    return tag.to_string();
                };
                (
                    crate::spec_analyze::spec_symbol(spec.shortname(), &unescape_attr(&id)),
                    None,
                )
            } else {
                let Some(href) = ATTR_HREF.captures(attrs) else {
                    return tag.to_string();
                };
                let raw_href = href[1].to_string();
                let href = unescape_attr(&raw_href);
                let target = if let Some(anchor) = href.strip_prefix('#') {
                    Some((spec, decode(anchor)))
                } else if let Some(rest) = href.strip_prefix(&rendered_prefix) {
                    let (path, fragment) = rest.split_once('#').unwrap_or((rest, ""));
                    spec_view(specs, &decode(path)).and_then(|(target, _, view)| match view {
                        SpecView::Anchor(anchor) => Some((target, decode(&anchor))),
                        SpecView::Page(_) if !fragment.is_empty() => {
                            Some((target, decode(fragment)))
                        }
                        _ => None,
                    })
                } else {
                    None
                };
                let Some((target, anchor)) = target else {
                    return tag.to_string();
                };
                (
                    crate::spec_analyze::spec_symbol(target.shortname(), &anchor),
                    Some(raw_href),
                )
            };
            if !has_sym(&sym) {
                return tag.to_string();
            }
            let annotated = format!(
                r#"<{} data-symbols="{}"{}{}>"#,
                name,
                rendered_docs::escape(&sym),
                rendered_def.map_or_else(String::new, |href| format!(
                    r#" data-rendered-def="{}""#,
                    href
                )),
                attrs
            );
            syms.push(sym);
            annotated
        })
        .into_owned()
}

/// The rendered view of a document (Markdown or reStructuredText) or of a
/// spec's source (see `rendered_specs`) at `path`, at the tip
/// (`/{tree}/rendered/{path}`) or at the revision `rev`
/// (`/{tree}/rev-rendered/{rev}/{path}`), with a blame strip for each of its
/// blocks if the tree's history has the file.  A spec's source can be split
/// into pages, `/{tree}/rendered/{path}/{page}.html` (and see
/// `rendered_redirect` for links to its anchors and lines).  See
/// `rendered_docs`.
///
/// With `with_prs`, a spec's page at the tip has its spec repository's open
/// pull requests' changes of it beside it (`/{tree}/rendered-prs/{path}`; see
/// `spec_prs`): the pull requests changing its source, with facets filtering
/// them, whose changes of the page spec-pr-view.js fetches (see
/// `spec_pr_blocks_json`) and shows beside the blocks they change.
pub fn format_rendered(
    cfg: &Config,
    tree_name: &str,
    rev: Option<&str>,
    path: &str,
    with_prs: bool,
    writer: &mut dyn Write,
) -> Result<(), &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let git = tree_config.get_git()?;
    let commit = rendered_commit(git, rev)?;
    let path = path.trim_end_matches('/');
    // (The page's path, for its pull requests' changes of it.)
    let view_path = path.to_string();
    let specs = tree_specs(tree_name, tree_config);
    let spec_view = specs.as_deref().and_then(|specs| spec_view(specs, path));
    // (A file an ecmarkup spec imports, or a Bikeshed spec includes (at the
    // tip), is rendered as its part of the spec, whose main source is `main`.)
    let read = |file: &str| read_commit_file(&git.repo, &commit, file);
    let (path, spec, page, main) = match spec_view {
        Some((spec, source, SpecView::Page(page))) => (source, Some(spec), page, None),
        Some(_) => return Err("Not a page of the spec"),
        // (A file a Bikeshed spec includes is its part even if it's a
        // document, ex: Markdown.)
        None => match specs.as_deref().and_then(|specs| {
            rev.is_none()
                .then(|| bikeshed_including_spec(tree_config, specs, path))
                .flatten()
        }) {
            Some((spec, main)) => (path.to_string(), Some(spec), None, Some(main)),
            None if has_rendered_view(path) => (path.to_string(), None, None, None),
            None => match specs
                .as_deref()
                .and_then(|specs| rendered_specs::ecmarkup::importing_spec(specs, path, &read))
            {
                Some((spec, main)) => (path.to_string(), Some(spec), None, Some(main)),
                None => {
                    return Err(
                        "Only documents (Markdown and reStructuredText) and specs' sources have rendered views",
                    );
                }
            },
        },
    };
    let path = path.as_str();
    // (The spec repository's summary, and its pull requests changing the
    // source; see `spec_prs`.)
    let pr_view = match with_prs {
        false => None,
        true => {
            // (A file an ecmarkup spec imports has one, but not a file a
            // Bikeshed spec includes; see `spec_pr_blocks_json`.)
            let ecmarkup = spec
                .is_some_and(|spec| spec.format == Some(crate::spec_analyze::SpecFormat::Ecmarkup));
            if rev.is_some() || spec.is_none() || (main.is_some() && !ecmarkup) {
                return Err(
                    "Only the latest version of a spec's source has a view with its pull requests",
                );
            }
            let spec_repo = path.split('/').next().unwrap_or_default().to_string();
            let summary = crate::spec_prs::repo_summary(cfg, tree_name, &spec_repo);
            // (And those changing the files an ecmarkup spec's source imports,
            // which have their own views.)
            let imported = match main {
                None if ecmarkup => rendered_specs::ecmarkup::imported_files(path, &read),
                _ => vec![],
            };
            let prs: Vec<crate::spec_prs::PrSummary> = summary
                .iter()
                .flat_map(|summary| &summary.prs)
                .filter(|pr| {
                    pr.files
                        .iter()
                        .any(|file| file.path == path || imported.contains(&file.path))
                })
                .cloned()
                .collect();
            Some((spec_repo, summary, prs, imported))
        }
    };
    let source = {
        let (repo, oid) = get_object_at(
            OwnedOrBorrowed::Borrowed(&git.repo),
            commit.id(),
            Path::new(path),
        )?;
        let object = repo
            .find_object(oid, Some(git2::ObjectType::Blob))
            .map_err(|_| "Not a file")?;
        git_ops::read_blob_object(&object)
    };
    let links_to = match rev {
        Some(_) => BreadcrumbsLinksTo::Historical,
        None => BreadcrumbsLinksTo::Latest,
    };
    let commit_id = commit.id().to_string();
    let page_rev = rev.map(|_| commit_id.clone());

    // (A spec's pages are each of the whole source's blame, which is cached;
    // see `spec_page_token_blame`.)
    let token_blame = if spec.is_some() {
        spec_page_token_blame(tree_name, git, &commit, path, &source, links_to)
    } else {
        page_token_blame(git, &commit, path, &source).map(|mut page| {
            set_token_blame_urls(&mut page, tree_name, &commit, path, links_to);
            std::sync::Arc::new(page)
        })
    };
    if let Some(page) = &token_blame {
        // The page will request its hyperblame data right away; see
        // `hyperblame_file`.  (The tip's are static files.)
        if rev.is_some() {
            keep_page_data(tree_config, tree_name, &commit_id, path, page);
        }
    }

    let doc_trees = tree_doc_trees(tree_name, tree_config);
    let links = DocLinks {
        tree_name,
        tree_config,
        rev: page_rev.clone(),
        commit: &commit,
        tree: commit.tree().map_err(|_| "Git commit with no tree")?,
        doc_trees: doc_trees.clone(),
        jumprefs: tree_jumprefs(tree_name, tree_config),
    };
    // A spec's page (its index, the pages, the page, and the bytes of the
    // source it covers).
    let mut spec_page = None;
    let rendered = if let Some(spec) = spec {
        let spec_links = RenderedSpecLinks {
            tree_name,
            rev: page_rev.as_deref(),
        };
        let specs = specs.as_deref().unwrap();
        let rendered_spec = match &main {
            Some(main) if spec.format == Some(crate::spec_analyze::SpecFormat::Bikeshed) => {
                let at = rev.is_some().then(|| commit.id());
                let output =
                    std::fs::read_to_string(bikeshed_rendering_path_at(tree_config, main, at))
                        .map_err(|_| BIKESHED_RENDERINGS)?;
                rendered_specs::bikeshed::render_file(
                    &output,
                    &source,
                    main,
                    path,
                    spec,
                    specs,
                    &spec_links,
                )
            }
            Some(main) => {
                rendered_specs::ecmarkup::render_file(main, spec, specs, &spec_links, &read, path)
            }
            // (A ReSpec spec's references to other specs' terms link by the
            // index's analysis, at the tip.)
            None if spec.format == Some(crate::spec_analyze::SpecFormat::Respec) => {
                let targets = if rev.is_none() {
                    respec_index_targets(tree_config, path)
                } else {
                    vec![]
                };
                rendered_specs::respec::render(
                    path,
                    spec,
                    specs,
                    &spec_links,
                    &read,
                    &crate::spec_analyze::xref::Terms::default(),
                    &targets,
                )
            }
            // (Bikeshed's rendering is the index's, of the latest revision, or
            // a spec pull request's import's, of its head.)
            None if spec.format == Some(crate::spec_analyze::SpecFormat::Bikeshed) => {
                let at = rev.is_some().then(|| commit.id());
                let output =
                    std::fs::read_to_string(bikeshed_rendering_path_at(tree_config, path, at))
                        .map_err(|_| BIKESHED_RENDERINGS)?;
                rendered_specs::bikeshed::render(&output, &source, path, spec, specs, &spec_links)
            }
            None => rendered_specs::render(
                spec,
                &source,
                path,
                specs,
                &spec_links,
                &read,
                page.as_deref(),
            ),
        }
        .ok_or("No such page")?;
        spec_page = Some((rendered_spec.pages, rendered_spec.page, rendered_spec.range));
        rendered_spec.rendered
    } else if path.ends_with(".rst") {
        rendered_docs::rst::render(&source, path, &links)
    } else {
        rendered_docs::markdown::render(&source, path, &links)
    };
    let range = spec_page
        .as_ref()
        .map_or(0..source.len(), |(_, _, range)| range.clone());
    let strips = match &token_blame {
        Some(page) => rendered_strips(&rendered, &source, page, &range),
        None => rendered_line_markers(&rendered, &source, &range),
    };
    // The URL of the page at a revision (or the tip).
    let page_name = spec_page
        .as_ref()
        .and_then(|(pages, page, _)| pages[*page].name.clone());
    let view_url = |rev: Option<&str>| {
        rendered_specs::page_url(&rendered_url(tree_name, rev, path), page_name.as_deref())
    };
    let first_line = source[..range.start].matches('\n').count() + 1;

    let filename = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path);
    let title = format!(
        "{} - mozsearch",
        rendered_docs::escape(rendered.title.as_deref().unwrap_or(filename))
    );
    let header = blame::commit_header(&commit)?;
    let revision = page_rev.as_deref().map(|rev| RevisionData {
        rev,
        desc: &header,
        date: git_time_to_chrono(commit.time()),
    });
    let opt = Options {
        title: &title,
        tree_name,
        include_date: env::var("MOZSEARCH_DIFFABLE").is_err(),
        revision,
        breadcrumbs_links_to: links_to,
        extra_content_classes: match pr_view {
            Some(_) => "rendered-page rendered-prs-page",
            None => "rendered-page",
        },
        search_rev: None,
    };

    let source_url = rendered_docs::Links::source_url(&links, path);
    let encoded_path = url_encode_path(path);
    let mut vcs_items = vec![PanelItem {
        label: PanelItemLabel::Plaintext("Source listing".to_owned()),
        tooltip: "Open the source listing of the document".to_owned(),
        id: "panel-source-listing",
        link: if first_line > 1 {
            format!("{}#{}", source_url, first_line)
        } else {
            source_url.clone()
        },
        update_link_lineno: "",
        accel_key: None,
        copyable: true,
        extras: Default::default(),
    }];
    match rev {
        Some(_) => vcs_items.push(PanelItem {
            label: PanelItemLabel::Plaintext("Go to latest version".to_owned()),
            tooltip: "Open the rendered view of the latest version of the document".to_owned(),
            id: "panel-vcs-latest",
            link: view_url(None),
            update_link_lineno: "",
            accel_key: None,
            copyable: true,
            extras: Default::default(),
        }),
        None if spec
            .is_some_and(|spec| spec.format == Some(crate::spec_analyze::SpecFormat::Bikeshed)) => {
        }
        None => vcs_items.push(PanelItem {
            label: PanelItemLabel::Plaintext("Permalink".to_owned()),
            tooltip: "Create a revision-specific link of the rendered view".to_owned(),
            id: "panel-permalink",
            link: view_url(Some(&commit_id)),
            update_link_lineno: "",
            accel_key: Some('Y'),
            copyable: true,
            extras: Default::default(),
        }),
    }
    // (The view with the spec repository's open pull requests' changes, in
    // trees which sync their summaries, and back; also for the files an
    // ecmarkup spec imports.)
    if rev.is_none()
        && spec.is_some_and(|spec| {
            main.is_none() || spec.format == Some(crate::spec_analyze::SpecFormat::Ecmarkup)
        })
        && crate::spec_prs::hook(cfg, tree_name).is_some()
    {
        let (label, tooltip, route) = match pr_view {
            Some(_) => (
                "Without pull requests",
                "Open the rendered view without its open pull requests' changes",
                "rendered",
            ),
            None => (
                "With pull requests",
                "Open the rendered view with its open pull requests' changes beside it",
                "rendered-prs",
            ),
        };
        vcs_items.push(PanelItem {
            label: PanelItemLabel::Plaintext(label.to_owned()),
            tooltip: tooltip.to_owned(),
            id: "panel-rendered-prs",
            link: format!("/{}/{}/{}", tree_name, route, url_encode_path(&view_path)),
            update_link_lineno: "",
            accel_key: None,
            copyable: true,
            extras: Default::default(),
        });
    }
    if let Some(link) = rendered_docs::Links::raw_url(&links, path) {
        vcs_items.push(PanelItem {
            label: PanelItemLabel::Plaintext("Raw".to_owned()),
            tooltip: "Open a raw file of the current file".to_owned(),
            id: "panel-raw",
            link,
            update_link_lineno: "",
            accel_key: Some('R'),
            copyable: true,
            extras: Default::default(),
        });
    }
    let mut panel = vec![PanelSection {
        name: "Revision control".to_owned(),
        items: vcs_items,
        raw_items: vec![],
    }];
    if let Some(target) = doc_trees
        .as_ref()
        .and_then(|doc_trees| doc_trees.find(path))
    {
        panel.push(PanelSection {
            name: "Other Tools".to_owned(),
            items: vec![PanelItem {
                label: PanelItemLabel::Plaintext("Source Docs".to_owned()),
                tooltip: "Open the rendered documentation on Source Docs".to_owned(),
                id: "panel-source-docs",
                link: format!("https://firefox-source-docs.mozilla.org/{}", target),
                update_link_lineno: "",
                accel_key: None,
                copyable: false,
                extras: Default::default(),
            }],
            raw_items: vec![],
        });
    }

    // (In the view with its pull requests' changes, their facets and list are
    // the panel's first section; see rendered_prs.liquid.)
    let prs_globals = match &pr_view {
        Some((spec_repo, summary, prs, imported)) => Some(rendered_prs_globals(
            tree_name,
            spec_repo,
            summary.as_ref(),
            prs,
            (path, &view_path),
            imported,
        )?),
        None => None,
    };
    if let Some(globals) = &prs_globals {
        let mut html = vec![];
        render_rendered_prs(globals, "panel", &mut html)?;
        let html = String::from_utf8(html).map_err(|_| "Template problems")?;
        if !html.trim().is_empty() {
            panel.insert(
                0,
                PanelSection {
                    name: String::new(),
                    items: vec![],
                    raw_items: vec![html],
                },
            );
        }
    }

    output::generate_header(&opt, writer)?;
    output::generate_breadcrumbs(&opt, writer, path, &[], false)?;
    output::generate_panel(&opt, writer, &panel, false)?;

    // Info boxes, ex: an obsolete spec's (see `templating::info_boxes`).
    let mut globals = liquid::object!({
        "tree": tree_name,
        "path": path,
        "config": {
            "github_repo_url": tree_config.paths.github_repo.as_deref().unwrap_or(""),
            "hg_repo_url": tree_config.paths.hg_root.as_deref().unwrap_or(""),
            "wpt_root": tree_config.paths.wpt_root.as_deref().unwrap_or(""),
        },
        "concise": { "info": {} },
        "detailed": {},
    });
    globals.insert(
        "spec".into(),
        match (spec, specs.as_deref()) {
            (Some(spec), Some(specs)) => spec_globals(tree_name, tree_config, specs, spec),
            _ => liquid::model::Value::Nil,
        },
    );
    let info_boxes = crate::templating::info_boxes::InfoBoxes::cached(cfg).render(&globals);
    if !info_boxes.trim().is_empty() {
        writeln!(
            writer,
            r#"<section class="info-boxes" id="info-boxes-container">{}</section>"#,
            info_boxes
        )
        .unwrap();
    }

    writeln!(
        writer,
        r#"<div class="rd-notice">Searchfox's rendering of <a href="{}">{}</a>, which {} may render differently.{}</div>"#,
        rendered_docs::escape(&source_url),
        rendered_docs::escape(&encoded_path),
        match spec.and_then(|spec| spec.format) {
            Some(crate::spec_analyze::SpecFormat::Wattsi) => "Wattsi",
            Some(crate::spec_analyze::SpecFormat::Ecmarkup) => "ecmarkup",
            Some(crate::spec_analyze::SpecFormat::Bikeshed) => "Bikeshed",
            Some(crate::spec_analyze::SpecFormat::Respec) => "ReSpec",
            None => "Sphinx",
        },
        if token_blame.is_some() {
            " Hover over the bar to the left of a block to see what last changed its text."
        } else {
            ""
        }
    )
    .unwrap();
    // (In the view with its pull requests' changes, its pages'.)
    let view_base = match pr_view {
        Some(_) => format!("/{}/rendered-prs/{}", tree_name, url_encode_path(path)),
        None => rendered_url(tree_name, page_rev.as_deref(), path),
    };
    // A spec's pages link to the ones before and after them, and the first.
    let page_nav = spec_page
        .as_ref()
        .filter(|(pages, _, _)| pages.len() > 1)
        .map(|(pages, page, _)| {
            let base = &view_base;
            let link = |i: usize, class: &str, text: &str| {
                format!(
                    r#"<a class="{}" href="{}">{}</a>"#,
                    class,
                    rendered_docs::escape(&rendered_specs::page_url(
                        &base,
                        pages[i].name.as_deref()
                    )),
                    rendered_docs::escape(text)
                )
            };
            let mut nav = String::from(r#"<nav class="sp-pages">"#);
            if *page > 0 {
                nav.push_str(&link(
                    *page - 1,
                    "sp-prev",
                    &format!("← {}", pages[*page - 1].title),
                ));
                nav.push_str(&link(0, "sp-index", "Table of contents"));
            }
            if *page + 1 < pages.len() {
                nav.push_str(&link(
                    *page + 1,
                    "sp-next",
                    &format!("{} →", pages[*page + 1].title),
                ));
            }
            nav.push_str("</nav>");
            nav
        });
    if let Some(nav) = &page_nav {
        writeln!(writer, "{}", nav).unwrap();
    }
    if let Some(globals) = &prs_globals {
        render_rendered_prs(globals, "header", writer)?;
        writeln!(writer, r#"<div class="sp-prs-view">"#).unwrap();
    }
    // (In the view with its pull requests' changes, with the page's lines,
    // beside which spec-pr-view.js shows their changes of the source.)
    let page_lines = rendered_range_lines(&source, &range);
    write!(
        writer,
        r#"<article id="rendered-doc" class="rendered-doc{}"{}>"#,
        if spec.is_some() { " rendered-spec" } else { "" },
        match pr_view {
            Some(_) => format!(
                r#" data-rd-range="{}-{}""#,
                page_lines.start + 1,
                page_lines.end
            ),
            None => String::new(),
        }
    )
    .unwrap();
    // (A spec's definitions and links to definitions have their symbols, for
    // the context menu, whose SYM_INFO the page has; see
    // `annotate_spec_symbols`.  The segments between the blocks' strips have
    // whole tags.)
    let spec_jumprefs = spec
        .is_some()
        .then(|| TreeJumprefs::open(&tree_config.paths, &cfg.tree_index_paths));
    let has_sym = |sym: &str| {
        spec_jumprefs
            .as_ref()
            .is_some_and(|jumprefs| matches!(jumprefs.lookup(sym), Ok(Some(_))))
    };
    let mut page_syms: Vec<String> = vec![];
    let mut write_segment = |writer: &mut dyn Write, segment: &str| match (spec, specs.as_deref()) {
        (Some(spec), Some(specs)) => {
            let annotated =
                annotate_spec_symbols(segment, tree_name, specs, spec, &has_sym, &mut page_syms);
            writer.write_all(annotated.as_bytes()).unwrap();
        }
        _ => writer.write_all(segment.as_bytes()).unwrap(),
    };
    let mut pos = 0;
    for (block, strip) in rendered.blocks.iter().zip(&strips) {
        let at = block.strip_at.clamp(pos, rendered.html.len());
        write_segment(writer, &rendered.html[pos..at]);
        writer.write_all(strip.as_bytes()).unwrap();
        pos = at;
    }
    write_segment(writer, &rendered.html[pos..]);
    writeln!(writer, "</article>").unwrap();
    if pr_view.is_some() {
        writeln!(
            writer,
            r#"<aside class="sp-pr-column" aria-label="The pull requests' changes"></aside></div>
<script defer src="/{}/static/js/side-cards.js"></script>
<script defer src="/{}/static/js/spec-pr-view.js"></script>"#,
            tree_name, tree_name
        )
        .unwrap();
    }
    if let Some(jumprefs) = &spec_jumprefs
        && !page_syms.is_empty()
    {
        let mut sym_info: BTreeMap<Ustr, Option<JumprefData>> = BTreeMap::new();
        let mut traversed = UstrMap::default();
        for sym in &page_syms {
            if !sym_info.contains_key(&ustr(sym)) {
                jumprefs.add_sym_info(ustr(sym), &mut sym_info, &mut traversed);
            }
        }
        writeln!(
            writer,
            "<script>var SYM_INFO = {};</script>",
            to_string(&sym_info)
                .unwrap_or_else(|_| "{}".to_string())
                .replace('<', "\\u003c")
        )
        .unwrap();
    }
    if let Some(nav) = &page_nav {
        writeln!(writer, "{}", nav).unwrap();
        // Links to anchors that aren't on the page (ex: to the first page,
        // from outside, or from the page, which only changes its fragment,
        // and doesn't reload it; see search.js's popstate handler) go to
        // their pages.
        writeln!(
            writer,
            r#"<script>(() => {{ const go = () => {{ const id = decodeURIComponent(location.hash.slice(1)); if (id && !/^\d+$/.test(id) && !document.getElementById(id)) {{ location.replace({} + encodeURIComponent(id)); }} }}; go(); addEventListener("hashchange", go); }})();</script>"#,
            serde_json::to_string(&format!("{}/anchor/", view_base))
            .unwrap()
            .replace("</", "<\\/")
        )
        .unwrap();
    }

    if let Some(page) = &token_blame {
        let info_json = to_string(&page.info).unwrap();
        // Author names and paths could contain "</script>".
        writeln!(
            writer,
            "<script>var BLAME_INFO = {};</script>",
            info_json.replace("</", "<\\/")
        )
        .unwrap();
    }

    output::generate_footer(&opt, tree_name, path, writer)
}

/// What the view's entry of a pull request whose first commits are another's
/// says (see `spec_prs::Containment`), before and after the other's link
/// (ex: "Its first commit is #1940's; only its own changes are shown here.").
fn containment_note(c: &crate::spec_prs::Containment) -> (String, String) {
    use crate::spec_prs::ContainedHow;
    let all_theirs = c.matched == c.own;
    let one = c.matched == 1;
    let mut before = match (all_theirs, one) {
        (true, true) => "Its commit is ".to_string(),
        (true, false) => "Its commits are ".to_string(),
        (false, true) => "Its first commit is ".to_string(),
        (false, false) => format!("Its first {} commits are ", c.matched),
    };
    before.push_str(match (c.how, one) {
        (ContainedHow::Commits, _) => "",
        (ContainedHow::Changes, true) => "a version of ",
        (ContainedHow::Changes, false) => "versions of ",
        (ContainedHow::Messages, true) => "an earlier version of ",
        (ContainedHow::Messages, false) => "earlier versions of ",
    });
    let mut after = "'s".to_string();
    if c.matched < c.of {
        after.push_str(&match one {
            true => format!(" first (of {})", c.of),
            false => format!(" first {} (of {})", c.matched, c.of),
        });
    }
    after.push_str(match (c.how, one) {
        (ContainedHow::Commits, _) => "",
        (ContainedHow::Changes, _) => ", with the same changes",
        (ContainedHow::Messages, true) => ", by its message",
        (ContainedHow::Messages, false) => ", by their messages",
    });
    after.push_str(match all_theirs {
        true => "; it has no changes of its own.",
        false => "; only its own changes are shown here.",
    });
    (before, after)
}

/// What the view's entry of a pull request whose commits others have first
/// says (see `containment_note`), before and after their links.
fn contained_by_note(containments: &[&crate::spec_prs::Containment]) -> (String, String) {
    use crate::spec_prs::ContainedHow;
    let some = containments.iter().any(|c| c.matched < c.of);
    let versions = containments.iter().any(|c| c.how != ContainedHow::Commits);
    (
        format!(
            "{}{} in ",
            if some {
                "Some of its commits"
            } else {
                "Its commits"
            },
            if versions {
                " (or versions of them) are"
            } else {
                " are"
            }
        ),
        match containments.len() {
            1 => " too, which shows only its own changes.",
            _ => " too, which show only their own changes.",
        }
        .to_string(),
    )
}

/// The pull requests changing a spec's page, in its view with their changes
/// (see `format_rendered`'s `with_prs`), for rendered_prs.liquid: their list,
/// in the panel, with facets filtering them, and each's data, for
/// spec-pr-view.js, and which they are, before the page (see
/// `render_rendered_prs`).  A pull request whose first commits are another's
/// (ex: stacked on it; see `spec_prs::containments`, of all of the spec
/// repository's pull requests) has only its own changes (see
/// `spec_pr_blocks_json`'s `after`), and both entries say so.
fn rendered_prs_globals(
    tree_name: &str,
    spec_repo: &str,
    summary: Option<&crate::spec_prs::RepoSummary>,
    prs: &[crate::spec_prs::PrSummary],
    (source_path, view_path): (&str, &str),
    imported: &[String],
) -> Result<liquid::Object, &'static str> {
    use crate::spec_prs::{pr_facet_values, pr_facets, review_name};
    let try_path_of = |number: u32| format!("/{}/try/{}/{}", tree_name, spec_repo, number);
    let contained = summary
        .map(|summary| crate::spec_prs::containments(&summary.prs))
        .unwrap_or_default();
    let mut contained_by: BTreeMap<u32, Vec<(u32, &crate::spec_prs::Containment)>> =
        BTreeMap::new();
    for (number, containment) in &contained {
        contained_by
            .entry(containment.number)
            .or_default()
            .push((*number, containment));
    }
    let entries: Vec<serde_json::Value> = prs
        .iter()
        .map(|pr| {
            let try_path = try_path_of(pr.number);
            let contains = contained.get(&pr.number);
            let data = json!({
                "number": pr.number,
                "title": pr.title,
                "url": pr.url,
                "author": pr.author,
                "review": pr.review.as_ref().map(|_| review_name(pr.review.as_deref())),
                "review_class": pr
                    .review
                    .as_deref()
                    .unwrap_or("none")
                    .to_lowercase()
                    .replace('_', "-"),
                "draft": pr.draft,
                "try": try_path,
                // (If it changes the page's source; only its own changes, if
                // its first commits are another's.)
                "blocks": pr
                    .head
                    .as_ref()
                    .filter(|_| pr.files.iter().any(|file| file.path == source_path))
                    .map(|head| format!(
                        "/{}/spec-pr-blocks/{}/{}/{}/{}{}",
                        tree_name,
                        spec_repo,
                        pr.number,
                        head,
                        url_encode_path(view_path),
                        contains.map_or(String::new(), |c| format!("?after={}", c.base))
                    )),
                "contains": contains.map(|c| c.number),
                // (The files the page's source imports which it changes, and
                // their views with the pull requests' changes.)
                "imported": pr
                    .files
                    .iter()
                    .filter(|file| imported.contains(&file.path))
                    .map(|file| json!({
                        "path": file.path,
                        "view": format!("/{}/rendered-prs/{}", tree_name, url_encode_path(&file.path)),
                    }))
                    .collect::<Vec<_>>(),
            });
            json!({
                "number": pr.number,
                "title": pr.title,
                "try_path": try_path,
                "facets": serde_json::to_string(&pr_facet_values(pr, chrono::Utc::now())).unwrap_or_default(),
                "data": data.to_string(),
                "error": pr.error.clone().unwrap_or_else(|| "Its head couldn't be fetched".to_string()),
                "loads": pr.head.is_some(),
                "contains": contains.map(|c| {
                    let (before, after) = containment_note(c);
                    json!({
                        "number": c.number,
                        "try_path": try_path_of(c.number),
                        "before": before,
                        "after": after,
                    })
                }),
                "contained_by": contained_by.get(&pr.number).map(|list| {
                    let (before, after) = contained_by_note(
                        &list.iter().map(|(_, c)| *c).collect::<Vec<_>>(),
                    );
                    json!({
                        "prs": list
                            .iter()
                            .map(|(number, _)| json!({
                                "number": number,
                                "try_path": try_path_of(*number),
                            }))
                            .collect::<Vec<_>>(),
                        "before": before,
                        "after": after,
                    })
                }),
            })
        })
        .collect();
    liquid::to_object(&json!({
        "tree": tree_name,
        "spec_repo": spec_repo,
        "summary": summary.map(|summary| json!({
            "repo": summary.repo,
            "synced": summary.synced.get(..16).map_or(summary.synced.clone(), |minute| format!("{} UTC", minute.replace('T', " "))),
        })),
        "prs": entries,
        "facets": pr_facets(prs, chrono::Utc::now()),
    }))
    .map_err(|_| "Template problems")
}

/// Render `part` of rendered_prs.liquid ("panel" or "header") with
/// `rendered_prs_globals`.
fn render_rendered_prs(
    globals: &liquid::Object,
    part: &str,
    writer: &mut dyn Write,
) -> Result<(), &'static str> {
    let mut globals = globals.clone();
    globals.insert(
        "part".into(),
        liquid::model::Value::scalar(part.to_string()),
    );
    crate::templating::builder::build_and_parse_rendered_prs()
        .render_to(writer, &globals)
        .map_err(|_| "Template problems")
}

lazy_static! {
    /// The trunk's lines of paged specs' pages, by tree, revision, source and
    /// page (see `trunk_page_lines`).
    static ref TRUNK_PAGE_LINES: std::sync::Mutex<
        HashMap<(String, Oid, String, Option<String>), Option<std::ops::Range<usize>>>,
    > = Default::default();
}

/// The (0-based) lines of the page `page` (or the first) of the paged spec
/// `spec`'s source at `source_path`, whose text at the tip `tip` is `trunk`,
/// at the tip (reading its other files with `read`), if it has the page; as
/// rendering it finds them, once per revision (for `spec_pr_blocks_json`).
#[allow(clippy::too_many_arguments)]
fn trunk_page_lines(
    tree_name: &str,
    tip: &git2::Commit,
    spec: &SpecEntry,
    specs: &SpecsConfig,
    source_path: &str,
    trunk: &str,
    read: &dyn Fn(&str) -> Option<String>,
    page: Option<&str>,
) -> Option<std::ops::Range<usize>> {
    let key = (
        tree_name.to_string(),
        tip.id(),
        source_path.to_string(),
        page.map(str::to_string),
    );
    if let Some(lines) = TRUNK_PAGE_LINES.lock().unwrap().get(&key) {
        return lines.clone();
    }
    let links = RenderedSpecLinks {
        tree_name,
        rev: None,
    };
    let lines = rendered_specs::render(spec, trunk, source_path, specs, &links, read, page)
        .filter(|rendered| rendered.file.is_none())
        .map(|rendered| rendered_range_lines(trunk, &rendered.range));
    TRUNK_PAGE_LINES.lock().unwrap().insert(key, lines.clone());
    lines
}

/// The changes of a spec pull request (`spec_repo`'s pull request `number`,
/// at its head `head`) to a spec's rendered page (`path`, as for
/// `format_rendered`) at the tip, for the page's view with its open pull
/// requests' changes beside it (`/{tree}/rendered-prs/{path}`; see
/// `format_rendered`), as JSON (`/{tree}/spec-pr-blocks/{spec}/{number}/{head}/{path}`):
/// its `state`, and if it's "ok", its `changes` (see
/// `spec_prs::block_changes`): each's trunk line to go beside (`anchor`,
/// 1-based, as `data-rd-lines`: the first it replaces, or the one it adds
/// lines after), the trunk's lines it replaces (`trunk`, first and last, or
/// null if it only adds lines), its `kind` ("change", "insert"
/// or "remove"), and the HTML of the blocks with the lines replacing them.
/// Returns whether it's final, which it is until the index changes (so it
/// can be cached), unless the pull request's head isn't there yet, or for a
/// Bikeshed spec, which the web server can't render, the pull request isn't
/// imported at that head (see `try_import`).
///
/// The pull request's version is the tip's with its changes: its head merged
/// with the tip's spec repository's directory (as `git merge-tree` would,
/// from their merge base, with the spec's upstream commit at the tip; see
/// `spec_forest`), rendered as the tip's is, but for a Bikeshed spec, its
/// import's rendering of its head rebased onto the tip (which must be the
/// tree's revision).  Its states otherwise: "conflicts" (its changes to the
/// file conflict with the trunk's since its base), "removes-file",
/// "needs-import" (with a `message`), and "missing-head".  Without its
/// rendering, the "needs-import" and "conflicts" states have its `changes` of
/// the source (see `spec_prs::source_changes`), with the lines each removes
/// and adds (`removed` and `added`, and how many more there are) instead of
/// HTML: its changes of the trunk's, or for "conflicts", of its base's, beside
/// where the base's lines are in the trunk's.
///
/// With `after` (`?after=` in the URL), one of the pull request's commits
/// (an ancestor of its head) whose changes are another pull request's (ex:
/// it's stacked on the other; see `spec_prs::containments`), its changes are
/// only those after it: from the trunk with that commit's changes merged (as
/// its head's are), whose lines are placed where they are in the trunk's
/// (`spec_prs::map_lines`), or for "conflicts", from that commit's version.
/// If that commit's changes conflict with the trunk's, its changes are all
/// of them, and `after_unused` says so.
pub fn spec_pr_blocks_json(
    cfg: &Config,
    tree_name: &str,
    spec_repo: &str,
    number: u32,
    head: &str,
    path: &str,
    after: Option<&str>,
) -> Result<(serde_json::Value, bool), &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let git = tree_config.get_git()?;
    let repo = &git.repo;
    let tip = rendered_commit(git, None)?;
    let specs = tree_specs(tree_name, tree_config).ok_or("Not a tree of specs")?;
    // (Or a file an ecmarkup spec imports, whose view is its part of the
    // spec, whose main source is `main`; see `format_rendered`.)
    let (spec, source_path, page, main) = match spec_view(&specs, path) {
        Some((spec, source, SpecView::Page(page))) => (spec, source, page, None),
        Some(_) => return Err("Not a page of a spec"),
        None => {
            let read = |file: &str| read_commit_file(repo, &tip, file);
            match rendered_specs::ecmarkup::importing_spec(&specs, path, &read) {
                Some((spec, main)) => (spec, path.to_string(), None, Some(main)),
                None => return Err("Not a page of a spec"),
            }
        }
    };
    let upstream_path = source_path
        .strip_prefix(&format!("{}/", spec_repo))
        .ok_or("Not a file of the spec repository")?
        .to_string();
    let state = |state: &str, message: Option<String>, final_: bool| {
        Ok((json!({ "state": state, "message": message }), final_))
    };
    // (A state without the pull request's rendering, with its changes of the
    // source; see `spec_prs::source_changes`.)
    let with_source = |state: &str,
                       message: Option<String>,
                       final_: bool,
                       changes: Vec<crate::spec_prs::SourceChange>| {
        let changes: Vec<serde_json::Value> = changes
            .into_iter()
            .map(|change| {
                let removes = !change.removed.is_empty() || change.more_removed > 0;
                let adds = !change.added.is_empty() || change.more_added > 0;
                json!({
                    "anchor": change.anchor + 1,
                    "trunk": (!change.trunk.is_empty())
                        .then(|| [change.trunk.start + 1, change.trunk.end]),
                    "kind": match (removes, adds) {
                        (true, false) => "remove",
                        (false, true) => "insert",
                        _ => "change",
                    },
                    "removed": change.removed,
                    "added": change.added,
                    "more_removed": change.more_removed,
                    "more_added": change.more_added,
                })
            })
            .collect();
        Ok((
            json!({ "state": state, "message": message, "changes": changes }),
            final_,
        ))
    };
    let head_oid = match Oid::from_str(head) {
        Ok(oid) if head.len() == 40 => oid,
        _ => return Err("Not a revision"),
    };
    let Ok(head_commit) = repo.find_commit(head_oid) else {
        return state("missing-head", None, false);
    };
    let after = match after {
        Some(after) => {
            let oid = Oid::from_str(after)
                .ok()
                .filter(|_| after.len() == 40)
                .ok_or("Not a revision")?;
            if oid != head_oid && !repo.graph_descendant_of(head_oid, oid).unwrap_or(false) {
                return Err("Not a commit of the pull request's");
            }
            Some(repo.find_commit(oid).map_err(|_| "Not a commit")?)
        }
        None => None,
    };
    // (All of its commits are the other's.)
    if after.as_ref().is_some_and(|after| after.id() == head_oid) {
        return state("ok", None, true).map(|(mut value, final_)| {
            value["changes"] = json!([]);
            (value, final_)
        });
    }

    // The pull request's changes merged with the tip's.
    let trunk_upstream = repo
        .find_reference(&crate::spec_forest::included_ref(spec_repo))
        .and_then(|reference| reference.peel_to_commit())
        .map_err(|_| "The spec repository's upstream commit isn't there")?;
    let base = repo
        .merge_base(trunk_upstream.id(), head_oid)
        .and_then(|base| repo.find_commit(base))
        .map_err(|_| "The pull request has no merge base with the trunk")?;
    let ours = tip
        .tree()
        .and_then(|tree| tree.get_path(Path::new(spec_repo)))
        .and_then(|entry| entry.to_object(repo))
        .and_then(|object| object.peel_to_tree())
        .map_err(|_| "The spec repository isn't at the tip")?;
    let (Ok(ancestor), Ok(theirs)) = (base.tree(), head_commit.tree()) else {
        return Err("Git commit with no tree");
    };
    let merged_index = repo
        .merge_trees(&ancestor, &ours, &theirs, None)
        .map_err(|_| "The pull request couldn't be merged")?;
    let trunk = read_commit_file(repo, &tip, &source_path).ok_or("Not a file at the tip")?;
    let trunk_lines = trunk.lines().count();
    if merged_index.has_conflicts() {
        let conflicting = merged_index.conflicts().is_ok_and(|mut conflicts| {
            conflicts.any(|conflict| {
                conflict.is_ok_and(|conflict| {
                    [conflict.ancestor, conflict.our, conflict.their]
                        .iter()
                        .flatten()
                        .any(|entry| entry.path == upstream_path.as_bytes())
                })
            })
        });
        if conflicting {
            // (Its changes of its base's lines, or after `after`, of that
            // commit's, beside where those lines are in the trunk's.)
            let ours = read_commit_file(repo, after.as_ref().unwrap_or(&base), &upstream_path)
                .unwrap_or_default();
            let theirs = read_commit_file(repo, &head_commit, &upstream_path).unwrap_or_default();
            let to_trunk = crate::spec_prs::line_hunks(&ours, &trunk);
            return with_source(
                "conflicts",
                None,
                true,
                crate::spec_prs::source_changes(
                    &ours,
                    &theirs,
                    |lines| crate::spec_prs::map_lines(&to_trunk, lines),
                    trunk_lines,
                ),
            );
        }
    }
    let merged_file = |file: &str| {
        let entry = merged_index.get_path(Path::new(file), 0)?;
        let blob = repo.find_blob(entry.id).ok()?;
        Some(String::from_utf8_lossy(blob.content()).into_owned())
    };
    let Some(mut merged) = merged_file(&upstream_path) else {
        return state("removes-file", None, true);
    };
    // The version its changes are from: the trunk's, or after `after`, the
    // trunk's with that commit's changes merged (as its head's are), whose
    // lines are where `to_trunk` puts them in the trunk's, unless they
    // conflict.
    let after_text: Option<String> = after.as_ref().and_then(|after| {
        let ancestor = repo
            .merge_base(trunk_upstream.id(), after.id())
            .and_then(|base| repo.find_commit(base))
            .and_then(|base| base.tree())
            .ok()?;
        let index = repo
            .merge_trees(&ancestor, &ours, &after.tree().ok()?, None)
            .ok()?;
        if index.has_conflicts() {
            return None;
        }
        let entry = index.get_path(Path::new(&upstream_path), 0)?;
        let blob = repo.find_blob(entry.id).ok()?;
        Some(String::from_utf8_lossy(blob.content()).into_owned())
    });
    let after_unused = after.is_some() && after_text.is_none();
    let to_trunk = after_text
        .as_ref()
        .map(|text| crate::spec_prs::line_hunks(text, &trunk));
    let own_hunks = |merged: &str| -> Vec<crate::spec_prs::LineHunk> {
        match (&after_text, &to_trunk) {
            (Some(text), Some(to_trunk)) => crate::spec_prs::line_hunks(text, merged)
                .into_iter()
                .map(|hunk| crate::spec_prs::LineHunk {
                    trunk: crate::spec_prs::map_lines(to_trunk, &hunk.trunk),
                    merged: hunk.merged,
                })
                .collect(),
            _ => crate::spec_prs::line_hunks(&trunk, merged),
        }
    };
    let own_source_changes = |merged: &str| match (&after_text, &to_trunk) {
        (Some(text), Some(to_trunk)) => crate::spec_prs::source_changes(
            text,
            merged,
            |lines| crate::spec_prs::map_lines(to_trunk, lines),
            trunk_lines,
        ),
        _ => crate::spec_prs::source_changes(&trunk, merged, |lines| lines.clone(), trunk_lines),
    };

    let spec_links = RenderedSpecLinks {
        tree_name,
        rev: None,
    };
    let prefix = format!("{}/", spec_repo);
    let read = |file: &str| match file.strip_prefix(&prefix) {
        Some(file) => merged_file(file),
        None => read_commit_file(repo, &tip, file),
    };
    // (A paged spec's page which none of its changes are near has none of
    // them, without rendering it: rendering a big spec's page parses its whole
    // source (ex: the HTML Standard's, ~1 s), for each of its pull requests
    // (300 of the HTML Standard's), but most change a page or two.  Near: in
    // the trunk's lines of the page, or at its ends; see `trunk_page_lines`.)
    let mut hunks = None;
    if main.is_none() && spec.format.is_some_and(rendered_specs::is_paged) {
        let read_trunk = |file: &str| read_commit_file(repo, &tip, file);
        if let Some(lines) = trunk_page_lines(
            tree_name,
            &tip,
            spec,
            &specs,
            &source_path,
            &trunk,
            &read_trunk,
            page.as_deref(),
        ) {
            let near = |hunk: &crate::spec_prs::LineHunk| {
                hunk.trunk.start <= lines.end && hunk.trunk.end >= lines.start
            };
            let all = own_hunks(&merged);
            if !all.iter().any(near) {
                return state("ok", None, true).map(|(mut value, final_)| {
                    value["changes"] = json!([]);
                    (value, final_)
                });
            }
            hunks = Some(all);
        }
    }
    let rendered_spec = match spec.format {
        // (With its spec's other files at their merged versions.)
        _ if main.is_some() => rendered_specs::ecmarkup::render_file(
            main.as_deref().unwrap_or_default(),
            spec,
            &specs,
            &spec_links,
            &read,
            &source_path,
        ),
        // (Its import's rendering of its head rebased onto the tip.)
        Some(crate::spec_analyze::SpecFormat::Bikeshed) => {
            // (Until then, its changes of the source.)
            let needs_import = |message: &str| {
                with_source(
                    "needs-import",
                    Some(message.to_string()),
                    false,
                    own_source_changes(&merged),
                )
                .map(|(mut value, final_)| {
                    value["after_unused"] = json!(after_unused);
                    (value, final_)
                })
            };
            let imported = repo
                .find_reference(&format!("refs/try/{}/{}", spec_repo, number))
                .and_then(|reference| reference.peel_to_commit())
                .ok();
            let imported_head = imported.as_ref().and_then(|commit| {
                crate::spec_forest::parse_upstream_trailer(commit.message().ok()?)
                    .map(|(_, oid)| oid)
            });
            if imported_head != Some(head_oid) {
                return needs_import(match imported {
                    Some(_) => {
                        "Its import is of an older head; import it again to see its changes to this Bikeshed spec rendered."
                    }
                    None => {
                        "Import it to see its changes to this Bikeshed spec rendered, which only its import renders."
                    }
                });
            }
            let rebased = repo
                .find_reference(&format!("refs/try-rebased/{}/{}", spec_repo, number))
                .and_then(|reference| reference.peel_to_commit())
                .ok()
                .filter(|rebased| rebased.parent_id(0).ok() == Some(tip.id()));
            let Some(rebased) = rebased else {
                return needs_import(
                    "Its import isn't of this revision of the tree; import it again to see its changes to this Bikeshed spec rendered.",
                );
            };
            let Ok(output) = std::fs::read_to_string(bikeshed_rendering_path_at(
                tree_config,
                &source_path,
                Some(rebased.id()),
            )) else {
                return needs_import("Its import has no rendering of this Bikeshed spec.");
            };
            merged = read_commit_file(repo, &rebased, &source_path).unwrap_or(merged);
            rendered_specs::bikeshed::render(
                &output,
                &merged,
                &source_path,
                spec,
                &specs,
                &spec_links,
            )
        }
        Some(crate::spec_analyze::SpecFormat::Respec) => rendered_specs::respec::render(
            &source_path,
            spec,
            &specs,
            &spec_links,
            &read,
            &crate::spec_analyze::xref::Terms::default(),
            &respec_index_targets(tree_config, &source_path),
        ),
        _ => rendered_specs::render(
            spec,
            &merged,
            &source_path,
            &specs,
            &spec_links,
            &read,
            page.as_deref(),
        ),
    };
    // (A page the pull request removes, or a part of another file's.)
    let Some(rendered_spec) = rendered_spec.filter(|rendered| rendered.file.is_none()) else {
        return state("ok", None, true).map(|(mut value, final_)| {
            value["changes"] = json!([]);
            (value, final_)
        });
    };
    let rendered = &rendered_spec.rendered;
    let block_lines = rendered_block_lines(rendered, &merged, &rendered_spec.range);
    let page_lines = rendered_range_lines(&merged, &rendered_spec.range);
    let hunks = hunks.unwrap_or_else(|| own_hunks(&merged));
    let changes: Vec<serde_json::Value> =
        crate::spec_prs::block_changes(&hunks, &block_lines, &page_lines, trunk_lines)
            .into_iter()
            .map(|change| {
                // (The blocks' ids are the trunk's.)
                let html: String = change
                    .blocks
                    .iter()
                    .filter_map(|&block| {
                        rendered_docs::block_html(&rendered.html, rendered.blocks[block].strip_at)
                    })
                    .collect();
                json!({
                    "anchor": change.anchor + 1,
                    "trunk": (!change.trunk.is_empty())
                        .then(|| [change.trunk.start + 1, change.trunk.end]),
                    "kind": if change.blocks.is_empty() {
                        "remove"
                    } else if change.trunk.is_empty() {
                        "insert"
                    } else {
                        "change"
                    },
                    "html": SPEC_PR_BLOCK_ID.replace_all(&html, "${1}data-pr-id="),
                })
            })
            .collect();
    state("ok", None, true).map(|(mut value, final_)| {
        value["changes"] = json!(changes);
        value["after_unused"] = json!(after_unused);
        (value, final_)
    })
}

lazy_static! {
    /// The `id` attributes of a spec pull request's changed blocks (see
    /// `spec_pr_blocks_json`), which are the trunk's.
    static ref SPEC_PR_BLOCK_ID: regex::Regex = regex::Regex::new(r#"(\s)id="#).unwrap();
}

/// How many steps a page of a peephole history has at most.
const PEEPHOLE_STEPS: usize = 12;
/// How many bytes of blobs we read for a page of a peephole history before
/// stopping.
const PEEPHOLE_COST: usize = 64_000_000;

/// A page of the peephole history (see `hyperblame::peephole`) of the tokens
/// with the (1-based) indices `tokens` in the file at `path` in revision `rev`:
/// the steps, `commits` with the `blame::commit_info_json` of each step's
/// commit, `next` with the URL of the next page (if any), `end` (if the
/// history ended), and `cost` (the bytes of blobs read).
pub fn peephole_json(
    cfg: &Config,
    tree_name: &str,
    rev: &str,
    path: &str,
    tokens: Vec<u32>,
) -> Result<String, &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let git = tree_config.get_git()?;
    let history = git.history.as_ref().ok_or("No history")?;
    let commit = git
        .repo
        .revparse_single(rev)
        .and_then(|object| object.peel_to_commit())
        .map_err(|_| "Bad revision")?;
    let start = Cursor {
        rev: commit.id().to_string(),
        path: path.to_string(),
        tokens,
    };
    let page = peephole_page(history, &git.repo, start, PEEPHOLE_STEPS, PEEPHOLE_COST);

    let mut commits = serde_json::Map::new();
    for step in &page.steps {
        if commits.contains_key(&step.rev) {
            continue;
        }
        let info = Oid::from_str(&step.rev)
            .and_then(|oid| git.repo.find_commit(oid))
            .ok()
            .and_then(|commit| blame::commit_info_json(tree_config, git, &commit).ok())
            .unwrap_or(serde_json::Value::Null);
        commits.insert(step.rev.clone(), info);
    }
    let next = page.next.as_ref().map(|cursor| {
        format!(
            "/{}/rev-hyperblame/{}/{}/peephole/{}.json",
            tree_name,
            cursor.rev,
            url_encode_path(&cursor.path),
            cursor.tokens.iter().map(|t| t.to_string()).join(",")
        )
    });
    Ok(to_string(&json!({
        "steps": page.steps,
        "commits": commits,
        "next": next,
        "end": page.end,
        "cost": page.cost,
    }))
    .unwrap())
}

/// Where the token with the (1-based) index `token` in the file at `path` in
/// revision `rev` was just before the commit which introduced it (see
/// `hyperblame::peephole::before`): `rev` (that commit's parent), `path`, and
/// `token`, plus `introduced`, the commit.
pub fn before_json(
    cfg: &Config,
    tree_name: &str,
    rev: &str,
    path: &str,
    token: u32,
) -> Result<String, &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let git = tree_config.get_git()?;
    let history = git.history.as_ref().ok_or("No history")?;
    let commit = git
        .repo
        .revparse_single(rev)
        .and_then(|object| object.peel_to_commit())
        .map_err(|_| "Bad revision")?;
    let cursor = Cursor {
        rev: commit.id().to_string(),
        path: path.to_string(),
        tokens: vec![token],
    };
    let (introduced, before) =
        peephole::before(history, &git.repo, &cursor).ok_or("Couldn't find the token before")?;
    Ok(to_string(&json!({
        "rev": before.rev,
        "path": before.path,
        "token": before.tokens[0],
        "introduced": introduced,
    }))
    .unwrap())
}

/// Where the token with the (1-based) index `token` in the file at `path` in
/// revision `rev` is now, or the commit which removed it (see
/// `hyperblame::future`): `future` with the result, and `commits` with the
/// `blame::commit_info_json` of the commits which changed it.
pub fn future_json(
    cfg: &Config,
    tree_name: &str,
    rev: &str,
    path: &str,
    token: u32,
) -> Result<String, &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let git = tree_config.get_git()?;
    let history = git.history.as_ref().ok_or("No history")?;
    let commit = git
        .repo
        .revparse_single(rev)
        .and_then(|object| object.peel_to_commit())
        .map_err(|_| "Bad revision")?;
    let future = future::follow(history, commit.id(), path, token).map_err(|e| {
        log::warn!(
            "Couldn't follow {}:{}:{} into the future: {}",
            rev,
            path,
            token,
            e
        );
        "Couldn't follow the token"
    })?;

    let mut commits = serde_json::Map::new();
    let revs = future
        .changes
        .iter()
        .map(|change| &change.rev)
        .chain(future.rev.iter());
    for rev in revs {
        let info = Oid::from_str(rev)
            .and_then(|oid| git.repo.find_commit(oid))
            .ok()
            .and_then(|commit| blame::commit_info_json(tree_config, git, &commit).ok())
            .unwrap_or(serde_json::Value::Null);
        commits.insert(rev.clone(), info);
    }
    Ok(to_string(&json!({ "future": future, "commits": commits })).unwrap())
}

/// How the outline of the file at `path` in revision `to` changed since
/// revision `from` (see `outline_diff::outline_diff`), where `from` may be
/// `since:YYYY-MM-DD` for the newest revision before that day on `to`'s
/// first-parent chain (see `outline_diff::first_parent_before`), as JSON.
/// The outline diff of the file at `path` between the revisions `from` (or
/// `since:YYYY-MM-DD`, the first-parent revision before that day) and `to`
/// (`/{tree}/rev-outline/{from}/{to}/{path}`, as JSON, and the outline diff
/// page's; see `render_outline_diff`), with all of the "from" file's
/// definitions' lines if `all_from_lines`.  (For trees' servers; see
/// `AbstractServer::outline_diff`.)
pub fn outline_diff_data(
    cfg: &Config,
    tree_name: &str,
    from: &str,
    to: &str,
    path: &str,
    all_from_lines: bool,
) -> Result<outline_diff::OutlineDiff, &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let git = tree_config.get_git()?;
    let history = git.history.as_ref().ok_or("No history")?;
    let parse = |rev: &str| {
        git.repo
            .revparse_single(rev)
            .and_then(|object| object.peel_to_commit())
            .map_err(|_| "Bad revision")
    };
    let to = parse(to)?;
    let (from, reached) = match from.strip_prefix("since:") {
        Some(day) => {
            let time = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
                .map_err(|_| "Bad date")?
                .and_hms_opt(0, 0, 0)
                .ok_or("Bad date")?
                .and_utc()
                .timestamp();
            outline_diff::first_parent_before(history, &to, time)
                .ok_or("The history doesn't have the revision")?
        }
        None => (parse(from)?, true),
    };
    let mut diff =
        outline_diff::outline_diff(history, &git.repo, &from, &to, path, all_from_lines)?;
    diff.complete &= reached;
    Ok(diff)
}

/// Tokens added and removed, as outline diffs' pages show them (ex: "+3 −1").
fn outline_delta(tokens: &outline_diff::OutlineTokens) -> String {
    [
        (tokens.added > 0).then(|| format!("+{}", tokens.added)),
        (tokens.removed > 0).then(|| format!("\u{2212}{}", tokens.removed)),
    ]
    .into_iter()
    .flatten()
    .join(" ")
}

/// The rows of the outline diff `diff` of the file at `path` (see
/// `render_outline_diff`), one per entry, as outline_diff.liquid and
/// explore_file.liquid take them, and how many entries have each status.
/// Their names in the "from" and "to" files, linking to their lines, and as
/// one column (`name`, `link` and `note`; ex: a renamed one's earlier name),
/// and the commits which changed them, newest first, linking to
/// `commit_link(index, commit)`'s link and label.
fn outline_rows(
    tree_name: &str,
    path: &str,
    diff: &outline_diff::OutlineDiff,
    commit_link: impl Fn(usize, &outline_diff::OutlineCommit) -> (String, String),
) -> (Vec<serde_json::Value>, BTreeMap<&'static str, usize>) {
    use outline_diff::OutlineStatus;

    let day = |date: &str| date[..10.min(date.len())].to_string();
    let from_path = diff.from_path.as_deref().unwrap_or(path);
    let rev_link = |rev: &str, path: &str, lines: Option<(u32, u32)>| {
        format!(
            "/{}/rev/{}/{}{}",
            tree_name,
            rev,
            url_encode_path(path),
            lines.map_or(String::new(), |(first, _)| format!("#{}", first))
        )
    };

    // (The entries which the changed ones are in show with them.)
    let mut context = vec![false; diff.entries.len()];
    let mut ancestors: Vec<usize> = vec![];
    for (index, entry) in diff.entries.iter().enumerate() {
        ancestors.truncate(entry.depth);
        if entry.status != OutlineStatus::Unchanged {
            for ancestor in &ancestors {
                context[*ancestor] = true;
            }
        }
        ancestors.push(index);
    }

    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let rows: Vec<serde_json::Value> = diff
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let delta = outline_delta(&entry.tokens);
            let with_delta = |label: &str| match delta.is_empty() {
                true => label.to_string(),
                false => format!("{} {}", label, delta),
            };
            // (A renamed one's earlier name, past the ancestors it shares.)
            let parent = &entry.pretty[..entry.pretty.len() - entry.name.len()];
            let earlier = |pretty: &str| pretty.strip_prefix(parent).unwrap_or(pretty).to_string();
            let both = (Some(entry.name.clone()), Some(entry.name.clone()));
            let ((from_name, to_name), from_note, to_note, badge, status) = match entry.status {
                OutlineStatus::Unchanged => (both, None, None, String::new(), "unchanged"),
                OutlineStatus::Changed => (both, None, None, delta.clone(), "changed"),
                OutlineStatus::Added => (
                    (None, Some(entry.name.clone())),
                    None,
                    None,
                    with_delta("added"),
                    "added",
                ),
                OutlineStatus::Removed => match &entry.evolved_into {
                    Some(into) => (
                        (Some(entry.name.clone()), Some(into.pretty.clone())),
                        None,
                        into.path.clone(),
                        "moved out".to_string(),
                        "removed",
                    ),
                    None => (
                        (Some(entry.name.clone()), None),
                        None,
                        None,
                        "removed".to_string(),
                        "removed",
                    ),
                },
                OutlineStatus::Renamed => (
                    (
                        entry
                            .evolved_from
                            .as_ref()
                            .map(|from| earlier(&from.pretty)),
                        Some(entry.name.clone()),
                    ),
                    None,
                    None,
                    with_delta("renamed"),
                    "renamed",
                ),
                OutlineStatus::Moved => (
                    (
                        entry.evolved_from.as_ref().map(|from| from.pretty.clone()),
                        Some(entry.name.clone()),
                    ),
                    entry
                        .evolved_from
                        .as_ref()
                        .and_then(|from| from.path.clone()),
                    None,
                    with_delta("moved in"),
                    "moved",
                ),
            };
            *counts.entry(status).or_default() += 1;
            let from_link = match entry.status {
                OutlineStatus::Moved => None,
                _ => from_name
                    .as_ref()
                    .map(|_| rev_link(&diff.from.rev, from_path, entry.from_lines)),
            };
            let to_link = match (&entry.status, &to_note) {
                (OutlineStatus::Removed, Some(into_path)) => {
                    Some(rev_link(&diff.to.rev, into_path, None))
                }
                (OutlineStatus::Removed, None) => None,
                _ => Some(rev_link(&diff.to.rev, path, entry.lines)),
            };
            // (As one column: removed ones as they were, and where moved ones
            // came from or went, by their names there if they differ.)
            let elsewhere = |name: &Option<String>, path: &Option<String>| match (name, path) {
                (Some(name), Some(path)) if *name != entry.pretty => {
                    Some(format!("{} in {}", unescape_name(name), path))
                }
                (_, Some(path)) => Some(path.clone()),
                (Some(name), None) => Some(unescape_name(name)),
                (None, None) => None,
            };
            let (name, link, note) = match entry.status {
                OutlineStatus::Removed => (
                    &from_name,
                    &from_link,
                    elsewhere(&to_name, &to_note).map(|to| format!("to {}", to)),
                ),
                OutlineStatus::Renamed => (
                    &to_name,
                    &to_link,
                    from_name
                        .as_deref()
                        .map(|from| format!("was {}", unescape_name(from))),
                ),
                OutlineStatus::Moved => (
                    &to_name,
                    &to_link,
                    elsewhere(&from_name, &from_note).map(|from| format!("from {}", from)),
                ),
                _ => (&to_name, &to_link, None),
            };
            // (Its lines, by which explore pages' panel finds its excerpt;
            // see `outline_excerpts_json`.)
            let range = match (entry.status, entry.lines, entry.from_lines) {
                (OutlineStatus::Removed, _, Some(from)) => Some(excerpt_range_key(from, true)),
                (OutlineStatus::Removed, _, None) => None,
                (_, lines, _) => lines.map(|lines| excerpt_range_key(lines, false)),
            };
            json!({
                "status": status,
                "depth": entry.depth,
                "context": context[index],
                "range": range,
                "title": format!(
                    "{} ({})",
                    unescape_name(&entry.pretty),
                    if entry.decl { "declaration" } else { &entry.kind }
                ),
                "from_name": from_name.as_deref().map(unescape_name),
                "from_link": from_link,
                "from_note": from_note,
                "to_name": to_name.as_deref().map(unescape_name),
                "to_link": to_link,
                "to_note": to_note,
                "name": name.as_deref().map(unescape_name),
                "link": link,
                "note": note,
                "badge": badge,
                "commits": entry.changes.iter().sorted().map(|(commit, added, removed)| {
                    let commit_info = &diff.commits[*commit];
                    let (link, label) = commit_link(*commit, commit_info);
                    json!({
                        "link": link,
                        "label": label,
                        "title": format!(
                            "{} {}: +{} \u{2212}{}",
                            day(&commit_info.date),
                            commit_info.subject,
                            added,
                            removed
                        ),
                    })
                }).collect::<Vec<_>>(),
            })
        })
        .collect();
    (rows, counts)
}

/// The page of an outline diff (`/{tree}/outline-diff/{from}/{to}/{path}`,
/// with `from` as for `outline_diff_data`): the definitions of the "from" file and
/// of the "to" file side by side, with their changes between them (and the
/// commits which made them), linking to their lines, and the commits.  It
/// shows the definitions which changed (and the ones they're in), and the
/// unchanged ones if asked.
pub fn render_outline_diff(
    tree_name: &str,
    path: &str,
    diff: outline_diff::OutlineDiff,
    writer: &mut dyn Write,
) -> Result<(), &'static str> {
    let short = |rev: &str| rev[..12.min(rev.len())].to_string();
    let day = |date: &str| date[..10.min(date.len())].to_string();
    let from_path = diff.from_path.as_deref().unwrap_or(path);
    let rev_link =
        |rev: &str, path: &str| format!("/{}/rev/{}/{}", tree_name, rev, url_encode_path(path));
    // (Its commits are numbered as in its list of them.)
    let (rows, counts) = outline_rows(tree_name, path, &diff, |index, _| {
        (format!("#commit-{}", index + 1), (index + 1).to_string())
    });

    let commits: Vec<serde_json::Value> = diff
        .commits
        .iter()
        .enumerate()
        .map(|(index, commit)| {
            json!({
                "n": index + 1,
                "day": day(&commit.date),
                "subject": commit.subject,
                "count": commit.count,
                "link": format!("/{}/commit/{}", tree_name, commit.rev),
                "diff": format!("/{}/diff/{}/{}", tree_name, commit.rev, url_encode_path(path)),
            })
        })
        .collect();
    let changed: Vec<String> = [
        ("changed", "changed"),
        ("added", "added"),
        ("removed", "removed"),
        ("renamed", "renamed"),
        ("moved", "moved in"),
    ]
    .iter()
    .filter_map(|(status, label)| counts.get(status).map(|n| format!("{} {}", n, label)))
    .collect();
    let commit_count: usize = diff.commits.iter().map(|commit| commit.count).sum();
    let summary = format!(
        "{} of {} definitions, in {} commit{}",
        match changed.is_empty() {
            true => "No changes".to_string(),
            false => changed.join(", "),
        },
        diff.entries.len(),
        commit_count,
        if commit_count == 1 { "" } else { "s" }
    );

    let title = format!(
        "Outline of {} from {} to {} - mozsearch",
        path,
        short(&diff.from.rev),
        short(&diff.to.rev)
    );
    let opt = Options {
        title: &title,
        tree_name,
        include_date: env::var("MOZSEARCH_DIFFABLE").is_err(),
        revision: None,
        breadcrumbs_links_to: BreadcrumbsLinksTo::Latest,
        extra_content_classes: "outline-diff-page",
        search_rev: None,
    };
    output::generate_header(&opt, writer)?;
    let globals = liquid::to_object(&json!({
        "tree": tree_name,
        "path": path,
        "source_link": format!("/{}/source/{}", tree_name, url_encode_path(path)),
        "from": {
            "short": short(&diff.from.rev),
            "day": day(&diff.from.date),
            "link": rev_link(&diff.from.rev, from_path),
            "path": diff.from_path,
        },
        "to": {
            "short": short(&diff.to.rev),
            "day": day(&diff.to.date),
            "link": rev_link(&diff.to.rev, path),
        },
        "complete": diff.complete,
        "summary": summary,
        "rows": rows,
        "commits": commits,
    }))
    .map_err(|_| "Template problems")?;
    build_and_parse_outline_diff()
        .render_to(writer, &globals)
        .map_err(|_| "Template problems")?;
    output::generate_footer(&opt, tree_name, "", writer)
}

/// Give a source page's token-centric blame what each of its commits changed
/// in the file (see `hyperblame::file_changes`), for the file timeline: the
/// sizes of their changes in `BLAME_INFO`, and their most changed definitions
/// for `commits.json`.
fn set_file_changes(git: &GitData, commit: &git2::Commit, path: &str, page: &mut PageBlame) {
    let Some(history) = git.history.as_ref() else {
        return;
    };
    let Some(timeline_commit) = history.timeline_commit(commit.id()) else {
        return;
    };
    let revs: Vec<&str> = page
        .info
        .commits
        .iter()
        .map(|(rev, _, _)| rev.as_str())
        .collect();
    let changes = file_changes(history, &timeline_commit, path, &revs);
    if changes.iter().all(Option::is_none) {
        return;
    }
    page.info.changes = changes
        .iter()
        .map(|change| change.as_ref().map(|change| change.size))
        .collect();
    page.scope_changes = changes
        .into_iter()
        .map(|change| change.map(|change| change.scopes).unwrap_or_default())
        .collect();
}

/// Set the URLs of a page's token-centric blame data: the page loads the
/// popup's data from the tip's static files, or from the web-server for other
/// revisions (see `hyperblame_files`), and the peephole histories are always
/// generated by the web-server.
fn set_token_blame_urls(
    page: &mut PageBlame,
    tree_name: &str,
    commit: &git2::Commit,
    path: &str,
    links_to: BreadcrumbsLinksTo,
) {
    let rev_url = format!(
        "/{}/rev-hyperblame/{}/{}",
        tree_name,
        commit.id(),
        url_encode_path(path)
    );
    page.info.peephole_url = Some(format!("{}/peephole", rev_url));
    page.info.history_url = Some(format!(
        "/{}/history/{}/{}",
        tree_name,
        commit.id(),
        url_encode_path(path)
    ));
    page.info.data_url = Some(match links_to {
        BreadcrumbsLinksTo::Latest => {
            format!("/{}/hyperblame/{}", tree_name, url_encode_path(path))
        }
        BreadcrumbsLinksTo::Historical => rev_url,
    });
}

/// The token-centric blame of the file at `path` in `commit`, if the commit has
/// the file and the tree's history has its blame, with the URLs of its data
/// for a page about the revision (ex: a diff), which the page will request
/// right away, so we leave the data in `PAGE_DATA_CACHE` (see
/// `hyperblame_file`).
fn revision_token_blame(
    tree_config: &TreeConfig,
    tree_name: &str,
    git: &GitData,
    commit: &git2::Commit,
    path: &str,
) -> Option<PageBlame> {
    let (repo, oid) = get_object_at(
        OwnedOrBorrowed::Borrowed(&git.repo),
        commit.id(),
        Path::new(path),
    )
    .ok()?;
    let object = repo.find_object(oid, Some(git2::ObjectType::Blob)).ok()?;
    let source = git_ops::read_blob_object(&object);
    let mut page = page_token_blame(git, commit, path, &source)?;
    set_token_blame_urls(
        &mut page,
        tree_name,
        commit,
        path,
        BreadcrumbsLinksTo::Historical,
    );
    keep_page_data(
        tree_config,
        tree_name,
        &commit.id().to_string(),
        path,
        &page,
    );
    Some(page)
}

/// Keep the hyperblame data of a page we rendered for the file at `path` in
/// revision `rev` (a full commit id), which the page will request right away
/// (see `hyperblame_file`): as sidecar files of its popup chunks (see
/// `sidecars`), which nginx serves, and in `PAGE_DATA_CACHE`, for its
/// `commits.json` (which we only generate if it's requested), and for requests
/// before the files are written.
fn keep_page_data(
    tree_config: &TreeConfig,
    tree_name: &str,
    rev: &str,
    path: &str,
    page: &PageBlame,
) {
    let data = PageData::new(page);
    write_page_data_sidecars(tree_config, tree_name, rev, path, &data, None);
    PAGE_DATA_CACHE.insert(
        (tree_name.to_string(), rev.to_string(), path.to_string()),
        data,
    );
}

/// Write the sidecar files (see `sidecars`) of a page's hyperblame data, with
/// its `commits.json` too if it's been generated.
fn write_page_data_sidecars(
    tree_config: &TreeConfig,
    tree_name: &str,
    rev: &str,
    path: &str,
    data: &PageData,
    commits_json: Option<&str>,
) {
    let dir = format!("rev-hyperblame/{}/{}", rev, path);
    for (k, chunk) in data.chunks.iter().enumerate() {
        crate::sidecars::write(
            &tree_config.paths.index_path,
            tree_name,
            &format!("{}/lines-{}.json", dir, k),
            chunk,
        );
    }
    if let Some(commits_json) = commits_json {
        crate::sidecars::write(
            &tree_config.paths.index_path,
            tree_name,
            &format!("{}/commits.json", dir),
            commits_json,
        );
    }
}

/// The token-centric blame of the file at `path` in `commit` whose contents are
/// `source`, if the tree's history has it.  See `hyperblame::token_blame`.
/// How many spec sources' token blame `spec_page_token_blame` keeps.
const SPEC_BLAME_ENTRIES: usize = 2;

/// (tree, revision, path, whether the page is the tip's), and the blame.
type SpecBlameEntry = (
    (String, Oid, String, bool),
    Option<std::sync::Arc<PageBlame>>,
);

lazy_static! {
    /// The token blame of the spec sources whose rendered views (or
    /// algorithms' views) were shown last, newest first.
    static ref SPEC_BLAME: std::sync::Mutex<Vec<SpecBlameEntry>> =
        std::sync::Mutex::new(Vec::new());
}

/// `page_token_blame` of a spec's source, with its URLs for the tip's page
/// or a revision's (see `set_token_blame_urls`), cached: a spec's pages (ex:
/// the HTML Standard's 61) are each of the whole source's blame, which takes
/// over 2s for the HTML Standard's.
fn spec_page_token_blame(
    tree_name: &str,
    git: &GitData,
    commit: &git2::Commit,
    path: &str,
    source: &str,
    links_to: BreadcrumbsLinksTo,
) -> Option<std::sync::Arc<PageBlame>> {
    let key = (
        tree_name.to_string(),
        commit.id(),
        path.to_string(),
        matches!(links_to, BreadcrumbsLinksTo::Latest),
    );
    if let Some((_, page)) = SPEC_BLAME.lock().unwrap().iter().find(|(k, _)| *k == key) {
        return page.clone();
    }
    let page = page_token_blame(git, commit, path, source).map(|mut page| {
        set_token_blame_urls(&mut page, tree_name, commit, path, links_to);
        std::sync::Arc::new(page)
    });
    let mut cache = SPEC_BLAME.lock().unwrap();
    cache.retain(|(k, _)| *k != key);
    cache.insert(0, (key, page.clone()));
    cache.truncate(SPEC_BLAME_ENTRIES);
    page
}

fn page_token_blame(
    git: &GitData,
    commit: &git2::Commit,
    path: &str,
    source: &str,
) -> Option<PageBlame> {
    let history = git.history.as_ref()?;
    let timeline_commit = history.timeline_commit(commit.id())?;
    let file_history = match history.file_history(&timeline_commit, path) {
        Ok(file_history) => file_history?,
        Err(e) => {
            log::warn!("Not using token blame for {}: {}", path, e);
            return None;
        }
    };
    let blame = match blame_tokens(source, &file_history) {
        Ok(blame) => blame,
        Err(e) => {
            log::warn!("Not using token blame for {}: {}", path, e);
            return None;
        }
    };
    // (When the commits landed, from the commit index if there is one, since
    // commit dates are hg's authors'.)
    let positions = history.commit_index().map(|index| index.position_lookup());
    Some(page_blame(&blame, source, path, |rev| {
        let commit = Oid::from_str(rev)
            .and_then(|oid| git.repo.find_commit(oid))
            .ok();
        match commit {
            Some(commit) => {
                let author = commit.author();
                let (name, _email) = git
                    .mailmap
                    .lookup(author.name().unwrap_or(""), author.email().unwrap_or(""));
                let position = positions.as_ref().and_then(|positions| positions.get(rev));
                let landed = position.as_ref().and_then(|position| {
                    chrono::DateTime::parse_from_rfc3339(&position.landed).ok()
                });
                CommitMeta {
                    time: landed.map_or(commit.time().seconds(), |date| date.timestamp()),
                    rank: position.map(|position| (!position.base, position.position)),
                    author: name.to_string(),
                }
            }
            None => CommitMeta {
                time: 0,
                rank: None,
                author: String::new(),
            },
        }
    }))
}

/// Panel item linking to the CI results of a test, or of the tests in a
/// directory, on tests.firefox.dev.  Only relevant if the per-file info has
/// "test_results".
pub fn tests_firefox_dev_panel_item(path: &str, is_dir: bool) -> PanelItem {
    let (tooltip, link) = if is_dir {
        (
            "Open the CI results of the tests in the current directory on tests.firefox.dev",
            format!(
                "https://tests.firefox.dev/tests.html?path={}",
                url_encode_path(path)
            ),
        )
    } else {
        (
            "Open the CI results of the current test on tests.firefox.dev",
            format!(
                "https://tests.firefox.dev/test.html?test={}",
                url_encode_path(path)
            ),
        )
    };
    PanelItem {
        label: PanelItemLabel::Plaintext("tests.firefox.dev".to_owned()),
        tooltip: tooltip.to_owned(),
        id: "panel-tests-firefox-dev",
        link,
        update_link_lineno: "",
        accel_key: Some('T'),
        copyable: false,
        extras: Default::default(),
    }
}

pub fn add_coverage_panel_item(
    panel: &mut Vec<PanelSection>,
    coverage_summary: Option<&code_coverage_report::NodeMetadata>,
) {
    let coverage_percentage = match coverage_summary {
        Some(summary) => {
            let coverage_bucket = (summary.coverage_percent / 10.).round();
            let coverage_percent = summary.coverage_percent.round();
            format!(
                r#"&nbsp;<span class="cov-percentage cov-percentage-{coverage_bucket}">{coverage_percent} %</span>"#
            )
        }
        None => String::new(),
    };

    let panel_item = PanelItem {
        label: PanelItemLabel::Html(format!(
            r#"Coverage:{}<span id="coverage-sparkline"></span>"#,
            coverage_percentage
        )),
        tooltip: "Show the test coverage graph".to_owned(),
        id: "panel-coverage",
        link: "javascript:CoverageGraph.open()".to_owned(),
        update_link_lineno: "",
        accel_key: None,
        copyable: false,
        extras: Default::default(),
    };

    const SECTION_NAME: &str = "Revision control";
    let section = panel
        .iter_mut()
        .find(|section| section.name == SECTION_NAME);
    match section {
        Some(section) => section.items.push(panel_item),
        None => panel.push(PanelSection {
            name: SECTION_NAME.to_owned(),
            items: vec![panel_item],
            raw_items: vec![],
        }),
    }
}

fn format_to_slug_attribute(format: &FormatAs) -> String {
    let slug = match format {
        FormatAs::FormatTagLike(spec) => spec.markdown_slug,
        FormatAs::FormatCLike(spec) => spec.markdown_slug,
        FormatAs::FormatXPIDL(_, _) => "",
        _ => "",
    };

    if slug.is_empty() {
        return String::new();
    }

    format!(r#" data-markdown-slug="{}""#, slug)
}

fn get_submodule_object_at<'a>(
    repo: &OwnedOrBorrowed<'a, Repository>,
    entry: TreeEntry,
    submodule_path: &Path,
    full_path: &Path,
) -> Result<(OwnedOrBorrowed<'a, Repository>, Oid), &'static str> {
    let submodule_path_str = submodule_path.to_str().ok_or("UTF-8 error")?;
    let submodule = repo
        .find_submodule(submodule_path_str)
        .or(Err("Can't find submodule"))?;
    let subrepo = submodule.open().or(Err("Can't open submodule"))?;
    let path_in_submodule = full_path
        .strip_prefix(submodule_path_str)
        .expect("submodule path is a always an ancestor of full path");
    get_object_at(
        OwnedOrBorrowed::Owned(subrepo),
        entry.id(),
        path_in_submodule,
    )
}

fn get_object_at<'a>(
    repo: OwnedOrBorrowed<'a, Repository>,
    commit: Oid,
    path: &Path,
) -> Result<(OwnedOrBorrowed<'a, Repository>, Oid), &'static str> {
    let commit = repo.find_commit(commit).or(Err("Bad revision"))?;
    let tree = commit.tree().or(Err("Git commit with no tree"))?;

    if path == "" {
        let tree_id = tree.id();
        drop(commit);
        drop(tree);
        return Ok((repo, tree_id));
    }

    if let Ok(entry) = tree.get_path(path) {
        let kind = entry.kind().ok_or("Unknown git object kind")?;

        return match kind {
            // If the path was exactly for the root of a submodule, handle it
            // here. Paths inside the submodule will be handled below after
            // first walking the ancestors to find any submodules.
            git2::ObjectType::Commit => get_submodule_object_at(&repo, entry, path, path),
            git2::ObjectType::Tree | git2::ObjectType::Blob => {
                drop(commit);
                drop(tree);
                Ok((repo, entry.id()))
            }
            _ => Err("Unsupported git object kind"),
        };
    }

    let (submodule_path, entry) = path
        .ancestors()
        .skip(1)
        .find_map(|ancestor| match tree.get_path(ancestor) {
            Ok(entry) => match entry.kind() {
                Some(git2::ObjectType::Commit) => Some((ancestor, entry)),
                _ => None,
            },
            Err(_) => None,
        })
        .ok_or("File, directory, or parent git submodule not found")?;

    get_submodule_object_at(&repo, entry, submodule_path, path)
}

/// Dynamically renders the contents of a specific file with blame annotations but without any
/// semantic analysis data available.  Used by the "rev" display and the "diff" mechanism when
/// there aren't actually any changes in the diff.
/// Whether a tree's blame or history has a revision (see `tree_for_rev`).
#[derive(Debug, PartialEq, Eq)]
enum RevKnowledge {
    /// The tree's blame or history has it.
    Known,
    /// The tree's repo has it, but its blame and history don't.
    Unknown,
    /// The tree's repo doesn't have it.
    Missing,
    /// The tree has no blame or history to tell.
    CantTell,
}

fn rev_knowledge(tree_config: &TreeConfig, rev: &str) -> RevKnowledge {
    let Some(git) = tree_config.git.as_ref() else {
        return RevKnowledge::CantTell;
    };
    if git.blame_repo.is_none() && git.history.is_none() {
        return RevKnowledge::CantTell;
    }
    let Some(oid) = git
        .repo
        .revparse_single(rev)
        .and_then(|object| object.peel_to_commit())
        .ok()
        .map(|commit| commit.id())
    else {
        return RevKnowledge::Missing;
    };
    let in_history = || {
        git.history
            .as_ref()
            .is_some_and(|history| history.timeline_commit(oid).is_some())
    };
    if git.blame_rev(oid).is_some() || in_history() {
        RevKnowledge::Known
    } else {
        RevKnowledge::Unknown
    }
}

/// For `/{tree}/rev/{rev}/` pages, another tree to show `rev` in, if this
/// tree's repo has it but its blame and history don't, and another tree's
/// do: ex: crash-stats links to firefox-main for crashes on every branch, and
/// a revision only on beta is in firefox-beta's blame and history, not
/// firefox-main's (the firefox trees share their repos, with a branch each).
/// This tree's notes are checked first, so the usual case costs a notes
/// lookup; the other trees' only when that fails.  (Like GitHub, any revision
/// we know works with any of the trees' URLs, by redirecting.)
pub fn tree_for_rev(cfg: &Config, tree_name: &str, rev: &str) -> Option<String> {
    let tree_config = cfg.trees.get(tree_name)?;
    redirect_target(
        rev_knowledge(tree_config, rev),
        cfg.trees
            .iter()
            .filter(|(name, _)| *name != tree_name)
            .map(|(name, other)| (name, rev_knowledge(other, rev))),
    )
}

/// For a tree whose repo is a forest of specs' repos (see `spec_forest`), the
/// forest commit of an upstream commit (ex: from a permalink to a spec's own
/// repository, `/{tree}/rev/{upstream rev}/fetch/fetch.bs`), and its spec.
pub fn forest_rev(cfg: &Config, tree_name: &str, rev: &str) -> Option<(String, String)> {
    let git = cfg.trees.get(tree_name)?.get_git().ok()?;
    // (Only forests have upstream notes.)
    git.repo
        .find_reference(crate::spec_forest::UPSTREAM_NOTES_REF)
        .ok()?;
    let upstream = git.repo.revparse_single(rev).ok()?.peel_to_commit().ok()?;
    let forest = crate::spec_forest::forest_commit(&git.repo, upstream.id())?;
    let commit = git.repo.find_commit(forest).ok()?;
    let message = String::from_utf8_lossy(commit.message_bytes()).into_owned();
    let (spec, _) = crate::spec_forest::parse_upstream_trailer(&message)?;
    Some((forest.to_string(), spec.to_string()))
}

/// `tree_for_rev`'s choice, given what this tree and the others (lazily, in
/// order) know of the revision.  Not when this tree's repo doesn't have the
/// revision at all, ex: the tree switcher keeping the revision of an
/// unrelated repo, which would bounce back to where it came from.
fn redirect_target<'a>(
    this: RevKnowledge,
    mut others: impl Iterator<Item = (&'a String, RevKnowledge)>,
) -> Option<String> {
    if this != RevKnowledge::Unknown {
        return None;
    }
    others
        .find(|(_, knowledge)| *knowledge == RevKnowledge::Known)
        .map(|(name, _)| name.clone())
}

/// Whether a file (not a directory) is the same at a revision as at the
/// tree's indexed revision, whose source listing then serves for it (see the
/// web server's `/rev/` route).
/// For each line of the file at `path` in revision `rev`, its line in the
/// indexed revision's version (the repository's head's), if it's unchanged
/// since (see `rev_lines::line_map`), or None if the tree's repository doesn't
/// have the revision or the file (ex: a crash report's frame's; see
/// `AbstractServer::line_map`).
pub fn rev_line_map(
    cfg: &Config,
    tree_name: &str,
    rev: &str,
    path: &str,
) -> Option<Vec<Option<u32>>> {
    let git = cfg.trees.get(tree_name)?.get_git().ok()?;
    let commit = git
        .repo
        .revparse_single(rev)
        .and_then(|o| o.peel_to_commit())
        .ok()?;
    let head = git.repo.head().and_then(|h| h.peel_to_commit()).ok()?;
    let blob_at = |commit: git2::Oid| -> Option<Vec<u8>> {
        let (repo, oid) = get_object_at(
            OwnedOrBorrowed::Borrowed(&git.repo),
            commit,
            Path::new(path),
        )
        .ok()?;
        let blob = repo.find_blob(oid).ok()?;
        Some(blob.content().to_vec())
    };
    let old = blob_at(commit.id())?;
    let new = blob_at(head.id()).unwrap_or_default();
    Some(crate::rev_lines::line_map(&old, &new))
}

pub fn same_as_head(cfg: &Config, tree_name: &str, rev: &str, path: &str) -> bool {
    let Some(tree_config) = cfg.trees.get(tree_name) else {
        return false;
    };
    let Ok(git) = tree_config.get_git() else {
        return false;
    };
    let path = Path::new(path.trim_end_matches('/'));
    let blob_at = |commit: git2::Oid| {
        let (repo, oid) = get_object_at(OwnedOrBorrowed::Borrowed(&git.repo), commit, path).ok()?;
        let kind = repo.find_object(oid, None).ok()?.kind();
        (kind == Some(git2::ObjectType::Blob)).then_some(oid)
    };
    let (Ok(commit), Ok(head)) = (
        git.repo
            .revparse_single(rev)
            .and_then(|o| o.peel_to_commit()),
        git.repo.head().and_then(|h| h.peel_to_commit()),
    ) else {
        return false;
    };
    // (A try push's head's pages are the push's, whose searches and jumps are
    // of it; see `format_file_data`'s `search_rev`.)
    let is_try_push =
        crate::try_delta::delta_dir(&tree_config.paths.index_path, &commit.id().to_string())
            .join("meta.json")
            .exists();
    commit.id() != head.id()
        && !is_try_push
        && blob_at(commit.id()).is_some_and(|blob| Some(blob) == blob_at(head.id()))
}

pub fn format_path(
    cfg: &Config,
    tree_name: &str,
    rev: &str,
    path: &str,
    writer: &mut dyn Write,
) -> Result<(), &'static str> {
    // (Directories' listings; see `rev_dir_data`.)
    if let Some(data) = rev_dir_data(cfg, tree_name, rev, path)? {
        return render_dir_listing(tree_name, rev, path, data, writer);
    }

    // Get the file data.
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let git = tree_config.get_git()?;
    let commit_obj = git.repo.revparse_single(rev).map_err(|_| "Bad revision")?;
    let path = Path::new(path.trim_end_matches('/'));

    let (repo, oid) = get_object_at(OwnedOrBorrowed::Borrowed(&git.repo), commit_obj.id(), path)?;

    let object = repo
        .find_object(oid, None)
        .map_err(|_| "Failed to retrieve git object from id")?;

    let commit = commit_obj.into_commit().or(Err("Bad revision"))?;

    match object.kind() {
        Some(git2::ObjectType::Blob) => {
            let path = path.to_str().expect("This Path was built from a str");
            let file = rev_file_data(cfg, tree_name, rev, path)?.ok_or("Invalid path")?;
            // (A try push's rebased head's jumprefs are the push's; see
            // `try_delta`.)
            let mut jumprefs = TreeJumprefs::default();
            if let Some(delta) = crate::try_delta::TryDelta::open(
                &tree_config.paths.index_path,
                &commit.id().to_string(),
            ) {
                jumprefs = TreeJumprefs::open(&tree_config.paths, &cfg.tree_index_paths);
                jumprefs.delta = delta.jumprefs();
            }
            render_rev_file(cfg, tree_name, path, file, &jumprefs, writer)
        }
        _ => Err("Invalid path"),
    }
}

/// The data of the listing of the directory at `path` in the revision `rev`
/// (see `render_dir_listing`), if it's a directory there: the revision's
/// commit (`rev`, `desc_html`, and `date`), its `files` (as `FileMatch`es,
/// with their sizes and coverage), its `coverage` and `coverage_history`,
/// the coverage summary for its panel (`coverage_panel`), and the try push
/// whose context its searches are in (`search_rev`).  (For trees' servers;
/// see `AbstractServer::rev_dir`.)
pub fn rev_dir_data(
    cfg: &Config,
    tree_name: &str,
    rev: &str,
    path: &str,
) -> Result<Option<serde_json::Value>, &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let git = tree_config.get_git()?;
    let commit_obj = git.repo.revparse_single(rev).map_err(|_| "Bad revision")?;
    let path = Path::new(path.trim_end_matches('/'));
    let (repo, oid) = get_object_at(OwnedOrBorrowed::Borrowed(&git.repo), commit_obj.id(), path)?;
    let object = repo
        .find_object(oid, None)
        .map_err(|_| "Failed to retrieve git object from id")?;
    if object.kind() != Some(git2::ObjectType::Tree) {
        return Ok(None);
    }
    let tree = object
        .into_tree()
        .expect("Should really be a tree, we just checked the object kind.");
    let commit = commit_obj.into_commit().or(Err("Bad revision"))?;

    // (A try push's rebased head's searches are of the push; see
    // `try_delta`.)
    let search_rev = Some(commit.id().to_string()).filter(|head| {
        crate::try_delta::delta_dir(&tree_config.paths.index_path, head)
            .join("meta.json")
            .exists()
    });
    let desc_html = blame::commit_header(&commit)?;
    let coverage_rev = format!("refs/tags/reverse/all/all/{}", commit.id());
    let files = tree
        .iter()
        .map(|entry| {
            let filename = entry.name().map_err(|_| "Utf-8 error")?;

            let is_dir = matches!(
                entry.kind(),
                Some(git2::ObjectType::Tree | git2::ObjectType::Commit)
            );
            let size = if is_dir {
                0
            } else {
                let object = entry
                    .to_object(&repo)
                    .or(Err("Failed to map git TreeEntry to Object"))?;
                object.into_blob().map_or(0, |blob| blob.size())
            };

            let path = path.join(filename);

            let coverage_info = coverage_summary(Some(git), &coverage_rev, &path);

            let concise = ConcisePerFileInfo {
                path_kind: "".into(),
                is_dir,
                file_size: size as u64,
                bugzilla_component: None,
                subsystem: None,
                tags: vec![],
                description: None,
                info: serde_json::Value::Null,
                coverage: coverage_info,
            };

            Ok(FileMatch {
                path: path.to_str().unwrap().into(),
                concise,
            })
        })
        .collect::<Result<Vec<_>, &str>>()?;

    let coverage_history = coverage_history(Some(git), path);
    let coverage = git_ops::coverage_summary(Some(git), &coverage_rev, path);
    let coverage_panel = match coverage_history {
        Some(_) => git
            .coverage_repo
            .as_ref()
            .and_then(|repo| {
                repo.revparse_single(&format!("refs/tags/reverse/all/all/{}", commit.id()))
                    .ok()
                    .and_then(|object| object.peel_to_commit().ok())
            })
            .and_then(|commit| {
                git_ops::coverage_summary(Some(git), &commit.id().to_string(), path)
            }),
        None => None,
    };
    let date = git_time_to_chrono(commit.time())
        .format("%F %T %z")
        .to_string();
    Ok(Some(json!({
        "rev": commit.id().to_string(),
        "desc_html": desc_html,
        "date": date,
        "files": files,
        "coverage": coverage,
        "has_coverage_history": coverage_history.is_some(),
        "coverage_history": coverage_history,
        "coverage_panel": coverage_panel,
        "search_rev": search_rev,
    })))
}

/// The listing of the directory at `path` in the revision `rev`, with its
/// `data` (see `rev_dir_data`).
pub fn render_dir_listing(
    tree_name: &str,
    rev: &str,
    path: &str,
    data: serde_json::Value,
    writer: &mut dyn Write,
) -> Result<(), &'static str> {
    let path = Path::new(path.trim_end_matches('/'));
    let mut panel = vec![PanelSection {
        name: "Revision control".to_owned(),
        items: vec![PanelItem {
            label: PanelItemLabel::Plaintext("Go to latest revision".to_owned()),
            tooltip: "Open the latest revision-agnostic link of the current file".to_owned(),
            id: "panel-vcs-latest",
            link: format!("/{}/source/{}", tree_name, path.to_string_lossy()),
            update_link_lineno: "",
            accel_key: None,
            copyable: true,
            extras: Default::default(),
        }],
        raw_items: vec![],
    }];
    if data["has_coverage_history"].as_bool() == Some(true) {
        let summary: Option<code_coverage_report::NodeMetadata> =
            serde_json::from_value(data["coverage_panel"].clone()).ok();
        add_coverage_panel_item(&mut panel, summary.as_ref());
    }

    let commit_hash = data["rev"].as_str().unwrap_or_default();
    let date = liquid::model::DateTime::from_str(data["date"].as_str().unwrap_or_default())
        .ok_or("Bad date")?;
    let search_rev = data["search_rev"].as_str().map(str::to_string);
    let liquid_globals = liquid::object!({
        "tree": tree_name,
        // the header always needs this
        "query": "",
        "path": path,
        "files": liquid::model::to_value(&data["files"]).map_err(|_| "Template problems")?,
        "rev": rev,
        "rev_box": {
            "long": commit_hash,
            "short": &commit_hash[..commit_hash.len().min(8)],
            "desc_html": data["desc_html"].as_str().unwrap_or_default(),
            "date": date,
        },
        "coverage": liquid::model::to_value(&data["coverage"]).map_err(|_| "Template problems")?,
        "coverage_history": liquid::model::to_value(&data["coverage_history"])
            .map_err(|_| "Template problems")?,
        "panel": panel,
        "search_url": match &search_rev {
            Some(_) => format!("/{}/query/default", tree_name),
            None => config::search_url(tree_name),
        },
        "query_search": search_rev.is_some()
            || config::search_endpoint(tree_name) == SearchEndpoint::Query,
        "search_box_url": config::search_box_url(tree_name, search_rev.is_some()),
        "search_rev": search_rev,
    });

    let template = build_and_parse_dir_listing();
    template
        .render_to(writer, &liquid_globals)
        .or(Err("Template problems"))
}

/// A file's page at a revision (see `rev_file_data` and `render_rev_file`).
#[derive(Deserialize, Serialize)]
pub struct RevFile {
    /// The revision (its full id).
    pub rev: String,
    /// Its hg revision, if known, else "default".
    pub hg_rev: String,
    pub text: String,
    /// Its analysis records, as `AbstractServer::fetch_raw_analysis` gives
    /// them: a try push's rebased head's (see `try_delta`), none otherwise.
    pub analysis: Vec<serde_json::Value>,
    pub extras: FileExtras,
}

/// The data of the page of the file at `path` in the revision `rev` (see
/// `render_rev_file`), if it's a file there.  (For trees' servers; see
/// `AbstractServer::rev_file`.)
pub fn rev_file_data(
    cfg: &Config,
    tree_name: &str,
    rev: &str,
    path: &str,
) -> Result<Option<RevFile>, &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let git = tree_config.get_git()?;
    let commit_obj = git.repo.revparse_single(rev).map_err(|_| "Bad revision")?;
    let path_ = Path::new(path.trim_end_matches('/'));
    let (repo, oid) = get_object_at(OwnedOrBorrowed::Borrowed(&git.repo), commit_obj.id(), path_)?;
    let object = repo
        .find_object(oid, None)
        .map_err(|_| "Failed to retrieve git object from id")?;
    if object.kind() != Some(git2::ObjectType::Blob) {
        return Ok(None);
    }
    let text = git_ops::read_blob_object(&object);
    let commit = commit_obj.into_commit().or(Err("Bad revision"))?;
    let path = path_.to_str().expect("This Path was built from a str");

    // Get blame.
    let blame_commit = if let Some(ref blame_repo) = git.blame_repo {
        let blame_oid = git
            .blame_rev(commit.id())
            .ok_or("Unable to find blame for revision")?;
        Some(
            blame_repo
                .find_commit(blame_oid)
                .map_err(|_| "Blame is not a blob")?,
        )
    } else {
        None
    };

    let coverage_commit = git.coverage_repo.as_ref().and_then(|repo| {
        repo.revparse_single(&format!("refs/tags/reverse/all/all/{}", commit.id()))
            .ok()
            .and_then(|object| object.peel_to_commit().ok())
    });

    // The head of a try push's rebased stack has the push's analysis (see
    // `try_delta`): its analysis of the files whose analysis it changes, and
    // the base's of the files it doesn't change.
    let analysis = match crate::try_delta::TryDelta::open(
        &tree_config.paths.index_path,
        &commit.id().to_string(),
    ) {
        Some(delta) => {
            let file = match delta.analysis_file(path) {
                Some(file) => Some(file),
                None if delta.change(path).is_none() => Some(PathBuf::from(format!(
                    "{}/analysis/{}.gz",
                    tree_config.paths.index_path, path
                ))),
                None => None,
            };
            file.and_then(|file| {
                let file = if file.exists() {
                    file
                } else {
                    file.with_extension("")
                };
                crate::file_format::analysis::read_analysis_values(&file.to_string_lossy())
            })
            .unwrap_or_default()
        }
        None => vec![],
    };

    let hg_rev = git
        .hg_rev(commit.id())
        .unwrap_or_else(|| "default".to_string());
    let extras = file_extras(
        cfg,
        tree_name,
        &Some(commit.clone()),
        BreadcrumbsLinksTo::Historical,
        &blame_commit,
        coverage_commit.as_ref(),
        path,
        &text,
    )?;
    Ok(Some(RevFile {
        rev: commit.id().to_string(),
        hg_rev,
        text,
        analysis,
        extras,
    }))
}

/// The page of the file at `path` in a revision, with its `file` data (see
/// `rev_file_data`) and the jumprefs of its analysis's symbols.
pub fn render_rev_file(
    cfg: &Config,
    tree_name: &str,
    path: &str,
    file: RevFile,
    jumprefs: &dyn JumprefSource,
    writer: &mut dyn Write,
) -> Result<(), &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let path = path.trim_end_matches('/');
    let hg_rev: &str = &file.hg_rev;

    let encoded_path = url_encode_path(path);

    let mut vcs_panel_items = vec![];
    vcs_panel_items.push(PanelItem {
        label: PanelItemLabel::Plaintext("Go to latest version".to_owned()),
        tooltip: "Open the latest revision-agnostic link of the current file".to_owned(),
        id: "panel-vcs-latest",
        link: format!("/{}/source/{}", tree_name, encoded_path),
        update_link_lineno: "#{}",
        accel_key: None,
        copyable: true,
        extras: Default::default(),
    });

    let gh_log_link = tree_config
        .paths
        .github_repo
        .as_ref()
        .map(|gh_root| format!("{}/commits/{}/{}", gh_root, file.rev, encoded_path));
    let hg_log_link = tree_config
        .paths
        .hg_root
        .as_ref()
        .map(|hg_root| format!("{}/log/{}/{}", hg_root, hg_rev, encoded_path));
    if let Some(link) = gh_log_link {
        vcs_panel_items.push(PanelItem {
            label: PanelItemLabel::Plaintext("Git log".to_owned()),
            tooltip: "Open git log of the current file".to_owned(),
            id: "panel-log-git",
            link,
            update_link_lineno: "",
            accel_key: hg_log_link.is_none().then_some('L'),
            copyable: true,
            extras: Default::default(),
        });
    }
    if let Some(link) = hg_log_link {
        vcs_panel_items.push(PanelItem {
            label: PanelItemLabel::Plaintext("Mercurial log".to_owned()),
            tooltip: "Open mercurial log of the current file".to_owned(),
            id: "panel-log-hg",
            link,
            update_link_lineno: "",
            accel_key: Some('L'),
            copyable: true,
            extras: Default::default(),
        });
    }

    if let Some(link) = tree_config
        .paths
        .make_raw_resource_rev_url(&file.rev, hg_rev, path)
    {
        vcs_panel_items.push(PanelItem {
            label: PanelItemLabel::Plaintext("Raw".to_owned()),
            tooltip: "Open a raw file of the current file".to_owned(),
            id: "panel-raw",
            link,
            update_link_lineno: "",
            accel_key: Some('R'),
            copyable: true,
            extras: Default::default(),
        });
    }

    if tree_config.paths.has_blame() {
        vcs_panel_items.push(PanelItem {
            label: PanelItemLabel::Plaintext("Blame".to_owned()),
            tooltip: "Hover over the gray bar on the left to see blame information".to_owned(),
            id: "panel-blame",
            link:
                "javascript:alert('Hover over the gray bar on the left to see blame information.')"
                    .to_owned(),
            update_link_lineno: "",
            accel_key: None,
            copyable: false,
            extras: Default::default(),
        });
    }
    if has_rendered_view(path) {
        vcs_panel_items.push(PanelItem {
            label: PanelItemLabel::Plaintext("Rendered view".to_owned()),
            tooltip: "Show the rendered document, with the blame of its blocks".to_owned(),
            id: "panel-rendered-view",
            link: rendered_url(tree_name, Some(&file.rev), path),
            update_link_lineno: "",
            accel_key: None,
            copyable: true,
            extras: Default::default(),
        });
    }
    let panel = vec![
        PanelSection {
            name: "Revision control".to_owned(),
            items: vcs_panel_items,
            raw_items: vec![],
        },
        create_markdown_panel_section(false, None),
    ];

    let analysis = crate::file_format::analysis::analysis_from_values(
        file.analysis,
        &mut crate::file_format::analysis::read_source,
    );
    let formatted = format_file_data(
        cfg,
        tree_name,
        panel,
        "".to_string(),
        file.extras,
        BreadcrumbsLinksTo::Historical,
        path,
        file.text,
        jumprefs,
        &analysis,
        writer,
    )?;
    // The page will request its hyperblame data right away; see
    // `hyperblame_file`.
    if let Some(page) = formatted.token_blame {
        keep_page_data(tree_config, tree_name, &file.rev, path, &page);
    }
    Ok(())
}

/// The symbols whose jumprefs a page of a file with the records `analysis`
/// (see `fetch_raw_analysis`) at `path` may need (see `format_code`): its
/// file's and its records', for looking them up ahead of time (see
/// `jumpref::PrefetchedJumprefs`).
pub fn page_symbols(path: &str, analysis: &[serde_json::Value]) -> Vec<Ustr> {
    let mut syms: BTreeSet<Ustr> = BTreeSet::new();
    syms.insert(ustr(&make_file_sym_from_path(path)));
    for record in analysis {
        if record.get("source").is_none() {
            continue;
        }
        if let Some(sym) = record["sym"].as_str() {
            syms.extend(sym.split(',').filter(|s| !s.is_empty()).map(ustr));
        }
    }
    syms.into_iter().collect()
}

/// The "Copy as" panel section: "Copy as..." for a popup of the formats to
/// copy the page as (Markdown links and code blocks, previewed, and the raw
/// file's URL, if any; see panel.js), with buttons beside it for the usual
/// ones: a link to the selected symbol (or the file and its selected lines),
/// that link leading a code block of the selected lines, and the code block
/// with the URL.  (The file link and the raw file are only in the popup, but
/// keep their keyboard shortcuts.)
pub fn create_markdown_panel_section(add_symbol_link: bool, raw_url: Option<&str>) -> PanelSection {
    let escape = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    let button = |id: &str, accel: char, label: &str, tooltip: &str| {
        format!(
            r#"<button type="button" id="{}" class="item" data-accel="{}" title="{}">{}<span class="accel">{}</span></button>"#,
            id, accel, tooltip, label, accel
        )
    };
    let mut modes = String::new();
    if add_symbol_link {
        modes.push_str(&button(
            "panel-copy-symbol-link",
            'S',
            "sym",
            "Copy a Markdown link whose text is the selected symbol (or the file and its selected lines)",
        ));
        modes.push_str(&button(
            "panel-copy-symbol-block",
            'B',
            "sym+block",
            "Copy a Markdown code block of the selected lines, led by the link to the selected symbol (or the file and lines)",
        ));
    }
    modes.push_str(&button(
        "panel-copy-code-block",
        'C',
        "block",
        "Copy a Markdown code block of the selected lines, led by their URL",
    ));
    let raw = raw_url.map_or(String::new(), |url| {
        format!(
            r#"<a hidden id="panel-raw" class="item" data-accel="R" href="{}">Raw</a>"#,
            escape(url)
        )
    });
    let html = format!(
        r#"<li class="copy-as"><button type="button" id="panel-copy-as" class="copy-as-more" title="Copy the page as one of several formats, previewed" aria-haspopup="dialog" aria-expanded="false">Copy as&hellip;</button><span class="copy-as-modes" role="group" aria-label="Copy as">{}</span><button type="button" hidden id="panel-copy-filename-link" class="item" data-accel="F">Filename link</button>{}</li>"#,
        modes, raw
    );
    PanelSection {
        name: "Copy as".to_owned(),
        items: vec![],
        raw_items: vec![html],
    }
}

fn split_lines(s: &str) -> Vec<&str> {
    let mut split = s.split('\n').collect::<Vec<_>>();
    if split[split.len() - 1].is_empty() {
        split.pop();
    }
    split
}

/// Dynamically renders a specific diff with blame annotations but without any semantic analysis
/// data available.
/// The data of a file's diff in a revision (see `rev_diff_data` and
/// `render_diff`).
#[derive(Deserialize, Serialize)]
pub struct RevDiff {
    /// The commit's header (HTML; see `blame::commit_header`), and its date
    /// (RFC 3339).
    pub header: String,
    pub date: String,
    /// `git diff-tree --cc`'s, with all of the file as its context.
    pub diff: String,
    pub parents: usize,
    /// The classic line blame of the file in the commit, and in its parents,
    /// if the tree has it.
    pub self_blame: Option<Vec<String>>,
    pub parent_blames: Option<Vec<Option<Vec<String>>>>,
    /// Or the token-centric blame of the file in the commit and then in each
    /// parent, if the tree has a history.
    pub token_blames: Vec<Option<PageBlame>>,
}

/// The data of the diff of the file at `path` in the revision `rev` (see
/// `render_diff`), unless the revision doesn't change it (whose page is then
/// the file's at the revision).  (For trees' servers; see
/// `AbstractServer::rev_diff`.)
pub fn rev_diff_data(
    cfg: &Config,
    tree_name: &str,
    rev: &str,
    path: &str,
) -> Result<Option<RevDiff>, &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;

    let git_path = tree_config.get_git_path()?;
    // (The system git, with SHA-1 collision detection, since this works on a
    // source repo; see `fast_import_git` for our git.)
    let output = Command::new("git")
        .arg("diff-tree")
        .arg("-p")
        .arg("--cc")
        .arg("--patience")
        .arg("--full-index")
        .arg("--no-prefix")
        .arg("-U100000")
        .arg(rev)
        .arg("--")
        .arg(path)
        .current_dir(git_path)
        .output()
        .map_err(|_| "Diff failed 1")?;
    if !output.status.success() {
        println!("ERR\n{}", git_ops::decode_bytes(output.stderr));
        return Err("Diff failed 2");
    }
    let difftxt = git_ops::decode_bytes(output.stdout);

    if difftxt.is_empty() {
        return Ok(None);
    }

    let git = tree_config.get_git()?;
    let commit_obj = git.repo.revparse_single(rev).map_err(|_| "Bad revision")?;
    let commit = commit_obj.as_commit().ok_or("Bad revision")?;

    // The classic line blame of the file in the commit and in its parents, if
    // the tree has it.
    let get_blame_for_oid = |oid: git2::Oid| -> Result<Option<Vec<String>>, &'static str> {
        let Some(blame_repo) = &git.blame_repo else {
            return Ok(None);
        };

        let blame_oid = git.blame_rev(oid).ok_or("Unable to find blame")?;
        let blame_commit = blame_repo
            .find_commit(blame_oid)
            .map_err(|_| "Blame is not a blob")?;
        let blame_tree = blame_commit.tree().map_err(|_| "Bad revision")?;
        let blame_lines = blame_tree
            .get_path(Path::new(path))
            .map(|blame_entry| {
                let blame = git_ops::read_blob_entry(blame_repo, &blame_entry);

                blame.lines().map(|s| s.to_owned()).collect::<Vec<_>>()
            })
            .ok();
        Ok(blame_lines)
    };

    let parent_blames: Vec<_> = commit
        .parent_ids()
        .map(get_blame_for_oid)
        .collect::<Result<_, _>>()?;
    let self_blame = get_blame_for_oid(commit.id())?;
    // Or the token-centric blame of the file in the commit and then in each
    // parent, if the tree has the history (trees only have one or the other;
    // see `config::git_data`).  Their strips name which of these (its index in
    // the page's `BLAME_INFOS`) and which line of that file they're for with
    // `data-hb-ctx` and `data-hb-line`, since the rows' ids are only the
    // commit's lines; see hyperblame.js.
    let token_blames: Vec<Option<PageBlame>> = match &git.history {
        Some(_) => std::iter::once(commit.clone())
            .chain(commit.parents())
            .map(|c| revision_token_blame(tree_config, tree_name, git, &c, path))
            .collect(),
        None => vec![],
    };
    Ok(Some(RevDiff {
        header: blame::commit_header(commit)?,
        date: git_time_to_chrono(commit.time()).to_rfc3339(),
        diff: difftxt,
        parents: commit.parents().count(),
        self_blame,
        parent_blames: git.blame_repo.as_ref().map(|_| parent_blames),
        token_blames,
    }))
}

/// The diff of the file at `path` in the revision `rev`
/// (`/{tree}/diff/{rev}/{path}`), with its `data` (see `rev_diff_data`).
pub fn render_diff(
    cfg: &Config,
    tree_name: &str,
    rev: &str,
    path: &str,
    data: RevDiff,
    writer: &mut dyn Write,
) -> Result<(), &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let mut token_blames = data.token_blames;
    let (rows, new_lines, num_lines) = parse_diff_rows(
        &data.diff,
        data.parents,
        data.self_blame.as_ref(),
        data.parent_blames.as_deref(),
    )?;
    drop_mismatched_token_blames(path, &mut token_blames, &num_lines);

    if let FormatAs::Binary = languages::select_formatting(path) {
        return Err("Cannot diff binary file");
    };

    let header = data.header;
    let date = chrono::DateTime::parse_from_rfc3339(&data.date).map_err(|_| "Bad date")?;

    let filename = Path::new(path).file_name().unwrap().to_str().unwrap();
    let title = format!("{} - mozsearch", filename);
    let opt = Options {
        title: &title,
        tree_name,
        include_date: true,
        revision: Some(RevisionData {
            rev,
            desc: &header,
            date,
        }),
        breadcrumbs_links_to: BreadcrumbsLinksTo::Historical,
        extra_content_classes: "source-listing diff",
        search_rev: None,
    };

    output::generate_header(&opt, writer)?;

    // Given this is a diff, the path shouldn't be a generated file and
    // the file symbol should never contain the platform.
    let file_syms = vec![make_file_sym_from_path(path)];

    output::generate_breadcrumbs(&opt, writer, path, &file_syms, false)?;

    let encoded_path = url_encode_path(path);

    let mut vcs_panel_items = vec![
        PanelItem {
            label: PanelItemLabel::Plaintext("Show changeset".to_owned()),
            tooltip: "Open the changeset information hosted on searchfox".to_owned(),
            id: "panel-vcs-changeset",
            link: format!("/{}/commit/{}", tree_name, rev),
            update_link_lineno: "",
            accel_key: None,
            copyable: true,
            extras: Default::default(),
        },
        PanelItem {
            label: PanelItemLabel::Plaintext("Show file without diff".to_owned()),
            tooltip: "Open the last revision without the current diff".to_owned(),
            id: "panel-vcs-without-diff",
            link: format!("/{}/rev/{}/{}", tree_name, rev, encoded_path),
            update_link_lineno: "#{}",
            accel_key: None,
            copyable: true,
            extras: Default::default(),
        },
        PanelItem {
            label: PanelItemLabel::Plaintext("Go to latest version".to_owned()),
            tooltip: "Open the latest revision-agnostic link of the current file".to_owned(),
            id: "panel-vcs-latest",
            link: format!("/{}/source/{}", tree_name, encoded_path),
            update_link_lineno: "#{}",
            accel_key: None,
            copyable: false,
            extras: Default::default(),
        },
    ];

    let gh_log_link = tree_config.paths.github_repo.as_ref().map(|gh_root| {
        format!(
            "{}/commits/{}/{}",
            gh_root,
            tree_config.paths.git_branch.as_deref().unwrap_or("HEAD"),
            encoded_path
        )
    });
    let hg_log_link = tree_config
        .paths
        .hg_root
        .as_ref()
        .map(|hg_root| format!("{}/log/default/{}", hg_root, encoded_path));
    if let Some(link) = gh_log_link {
        vcs_panel_items.push(PanelItem {
            label: PanelItemLabel::Plaintext("Git log".to_owned()),
            tooltip: "Open git log of the current file".to_owned(),
            id: "panel-log-git",
            link,
            update_link_lineno: "",
            accel_key: hg_log_link.is_none().then_some('L'),
            copyable: true,
            extras: Default::default(),
        });
    }
    if let Some(link) = hg_log_link {
        vcs_panel_items.push(PanelItem {
            label: PanelItemLabel::Plaintext("Mercurial log".to_owned()),
            tooltip: "Open mercurial log of the current file".to_owned(),
            id: "panel-log-hg",
            link,
            update_link_lineno: "",
            accel_key: Some('L'),
            copyable: true,
            extras: Default::default(),
        });
    }

    let sections = vec![PanelSection {
        name: "Revision control".to_owned(),
        items: vcs_panel_items,
        raw_items: vec![],
    }];
    output::generate_panel(&opt, writer, &sections, false)?;

    write_diff_rows(
        writer,
        cfg,
        path,
        &rows,
        &new_lines,
        &token_blames,
        &DiffRowExtras::default(),
    )?;

    output::generate_footer(&opt, tree_name, path, writer).unwrap();

    Ok(())
}

/// A row of a diff page: a line of the file in the commit, or a line of the
/// file in a parent which the commit removed.
struct DiffRow<'a> {
    /// The (1-based) line of the file in the commit, or -1 for removed lines.
    lineno: isize,
    /// The row's classic line blame, if the tree has it.
    blame: Option<&'a String>,
    /// The file and (1-based) line in it that the row's token blame is for:
    /// 0 for the file in the commit, and i + 1 for the file in parent i (see
    /// `render_diff`'s `token_blames`).
    token_line: Option<(usize, usize)>,
    origin: Vec<char>,
    content: &'a str,
}

/// The rows of the unified diff `difftxt` of a file in a commit with
/// `num_parents` parents (a combined diff, if more than one) which has a
/// single hunk with all of the file's lines, the contents of the file in the
/// commit, and the number of lines of the file in the commit and then in each
/// parent.  `parent_blames` (with `self_blame`) has the files' classic line
/// blames, if the tree has it.
fn parse_diff_rows<'a>(
    difftxt: &'a str,
    num_parents: usize,
    self_blame: Option<&'a Vec<String>>,
    parent_blames: Option<&'a [Option<Vec<String>>]>,
) -> Result<(Vec<DiffRow<'a>>, String, Vec<usize>), &'static str> {
    let mut new_lineno = 1;
    let mut old_lineno = vec![1; num_parents];

    let mut lines = split_lines(difftxt);
    for i in 0..lines.len() {
        if lines[i].starts_with('@') && i + 1 < lines.len() {
            lines = lines.split_off(i + 1);
            break;
        }
    }

    let mut new_lines = String::new();

    let mut rows = Vec::new();
    for line in lines {
        if line.is_empty() || line.starts_with('\\') {
            continue;
        }

        let (origin, content) = line.split_at(num_parents);
        let origin = origin.chars().collect::<Vec<_>>();
        let mut cur_blame = None;
        let mut token_line = None;
        for i in 0..num_parents {
            let has_minus = origin.contains(&'-');
            if (has_minus && origin[i] == '-') || (!has_minus && origin[i] != '+') {
                if let Some(parent_blames) = parent_blames {
                    cur_blame = match parent_blames[i] {
                        Some(ref lines) => Some(&lines[old_lineno[i] - 1]),
                        None => return Err("expected blame for '-' line, none found"),
                    };
                }
                if has_minus && token_line.is_none() {
                    token_line = Some((i + 1, old_lineno[i]));
                }
                old_lineno[i] += 1;
            }
        }

        let mut lno = -1;
        if !origin.contains(&'-') {
            new_lines.push_str(content);
            new_lines.push('\n');
            cur_blame = self_blame.map(|blame_lines| &blame_lines[new_lineno - 1]);
            token_line = Some((0, new_lineno));

            lno = new_lineno as isize;
            new_lineno += 1;
        }

        rows.push(DiffRow {
            lineno: lno,
            blame: cur_blame,
            token_line,
            origin,
            content,
        });
    }

    let num_lines = std::iter::once(new_lineno - 1)
        .chain(old_lineno.iter().map(|n| n - 1))
        .collect();
    Ok((rows, new_lines, num_lines))
}

/// Drop the token blames which aren't for the lines the diff has (as source
/// listings do; see `format_file_data`).  `num_lines` has the number of lines
/// of the file in the commit and then in each parent, like `token_blames`.
fn drop_mismatched_token_blames(
    path: &str,
    token_blames: &mut [Option<PageBlame>],
    num_lines: &[usize],
) {
    for (blame, num_lines) in token_blames.iter_mut().zip(num_lines) {
        if let Some(page) = blame
            && page.lines.len() != *num_lines
        {
            log::warn!(
                "Not using token blame for {} in a diff: {} lines but {} in the diff",
                path,
                page.lines.len(),
                num_lines
            );
            *blame = None;
        }
    }
}

/// What interdiffs add to the rows of a diff page (see `interdiff_file_data`):
/// attributes for the code of the rows of the lines of the file in the commit
/// and of the removed lines of the file in its (one) parent, by their
/// (1-based) lines, and rows to insert before the row of a line of the file in
/// the commit (or after the last row, for the number of lines + 1).
#[derive(Default, Deserialize, Serialize)]
pub struct DiffRowExtras {
    line_attrs: HashMap<usize, String>,
    removed_line_attrs: HashMap<usize, String>,
    rows_before: BTreeMap<usize, Vec<String>>,
}

/// Write the rows of a diff page (see `parse_diff_rows`), with their blame
/// strips, and its `BLAME_INFOS` (see `render_diff`).  `new_lines` is the
/// contents of the file in the commit, for its syntax highlighting.
fn write_diff_rows(
    writer: &mut dyn Write,
    cfg: &Config,
    path: &str,
    rows: &[DiffRow],
    new_lines: &str,
    token_blames: &[Option<PageBlame>],
    extras: &DiffRowExtras,
) -> Result<(), &'static str> {
    let format = languages::select_formatting(path);
    let analysis = Vec::new();
    let slug = format_to_slug_attribute(&format);
    let (formatted_lines, _) = format_code(
        Some(cfg),
        &TreeJumprefs::default(),
        format,
        path,
        new_lines,
        &analysis,
    );

    let f = F::Seq(vec![F::T(format!(
        "<div id=\"file\" class=\"file\" role=\"table\"{}>",
        slug
    ))]);

    output::generate_formatted(writer, &f, 0).unwrap();

    fn entity_replace(s: String) -> String {
        s.replace("&", "&amp;").replace("<", "&lt;")
    }

    // Like source listings, we number the commits in order for the aria
    // labels, and alternate the strip's colors whenever the commit changes.
    let mut blame_hash_to_human_id = HashMap::new();
    let mut last_rev = String::new();
    let mut last_color = false;
    for row in rows {
        let &DiffRow {
            lineno,
            blame,
            token_line,
            ref origin,
            content,
        } = row;
        if lineno > 0 {
            for extra_row in extras
                .rows_before
                .get(&(lineno as usize))
                .into_iter()
                .flatten()
            {
                writeln!(writer, "{}", extra_row).unwrap();
            }
        }
        let token_strip = token_line.and_then(|(blame_index, line)| {
            let page = token_blames.get(blame_index)?.as_ref()?;
            Some((blame_index, line, page, &page.lines[line - 1]))
        });
        let blame_data = match (token_strip, blame) {
            (Some((blame_index, line, page, strip)), _) => {
                let (color_class, aria_label) = match strip.commit {
                    Some(commit) => {
                        let rev = &page.info.commits[commit].0;
                        let same_commit_as_last = *rev == last_rev;
                        if !same_commit_as_last {
                            last_color = !last_color;
                        }
                        last_rev = rev.clone();
                        let next_human_id = blame_hash_to_human_id.len() + 1;
                        let human_id = *blame_hash_to_human_id
                            .entry(rev.clone())
                            .or_insert(next_human_id);
                        (
                            if last_color { " c1" } else { " c2" },
                            format!(
                                "{} hash {}",
                                if same_commit_as_last { "same" } else { "new" },
                                human_id
                            ),
                        )
                    }
                    None => ("", "no tokens".to_string()),
                };
                let (classes, data) = strip.strip_attrs();
                format!(
                    r#" class="blame-strip{}{}" data-hb-ctx="{}" data-hb-line="{}"{} role="button" aria-label="{}" aria-expanded="false""#,
                    color_class, classes, blame_index, line, data, aria_label,
                )
            }
            (None, Some(blame)) => {
                let line_data = blame::LineData::deserialize(blame);

                let color = if last_rev == line_data.rev {
                    last_color
                } else {
                    !last_color
                };
                last_rev = line_data.rev.to_string();
                last_color = color;
                let class = if color { 1 } else { 2 };
                format!(
                    r#" class="blame-strip c{}" data-blame="{}#{}#{}" role="button" aria-label="blame" aria-expanded="false""#,
                    class, line_data.rev, line_data.path, line_data.lineno
                )
            }
            (None, None) => " class=\"blame-strip\"".to_owned(),
        };

        let content = entity_replace(content.to_owned());
        let content = if lineno > 0 && (lineno as usize) < formatted_lines.len() + 1 {
            &formatted_lines[(lineno as usize) - 1].line
        } else {
            &content
        };

        let origin = origin.iter().cloned().collect::<String>();
        let extra_attrs = match token_line {
            _ if lineno > 0 => extras.line_attrs.get(&(lineno as usize)),
            Some((1, line)) => extras.removed_line_attrs.get(&line),
            _ => None,
        };

        let class = if origin.contains('-') {
            " minus-line"
        } else if origin.contains('+') {
            " plus-line"
        } else {
            ""
        };

        let f = F::Seq(vec![
            F::T(format!(
                "<div role=\"row\" id=\"line-{}\" class=\"source-line-with-number\">",
                // note: this can be -1 but that's the way it's always been.
                lineno
            )),
            F::Indent(vec![
                F::S(r#"<div class="line-strip">"#),
                F::Indent(vec![
                    // Blame info.
                    F::T(format!(
                        "<div role=\"cell\" class=\"blame-container\"><div{}></div></div>",
                        blame_data
                    )),
                ]),
                F::S("</div>"),
                // The line number.
                F::T(format!(
                    "<div role=\"cell\" class=\"line-number\" data-line-number=\"{}\"></div>",
                    if lineno > 0 {
                        format!("{}", lineno)
                    } else {
                        "".to_owned()
                    },
                )),
                // The source line, after the origin, which the token blame
                // popup skips with `data-hb-offset`.
                F::T(format!(
                    "<code role=\"cell\" class=\"source-line{}\"{}{}>{} {}\n</code>",
                    class,
                    if token_strip.is_some() {
                        format!(r#" data-hb-offset="{}""#, origin.len() + 1)
                    } else {
                        String::new()
                    },
                    extra_attrs.map_or("", String::as_str),
                    origin,
                    content
                )),
            ]),
            F::S("</div>"),
        ]);

        output::generate_formatted(writer, &f, 0).unwrap();
    }

    let last_line = rows.iter().map(|row| row.lineno).max().unwrap_or(0).max(0) as usize;
    for extra_row in extras
        .rows_before
        .range(last_line + 1..)
        .flat_map(|(_, rows)| rows)
    {
        writeln!(writer, "{}", extra_row).unwrap();
    }

    let f = F::Seq(vec![F::S("</div>")]);
    output::generate_formatted(writer, &f, 0).unwrap();

    if token_blames.iter().any(Option::is_some) {
        let infos: Vec<_> = token_blames
            .iter()
            .map(|page| page.as_ref().map(|page| &page.info))
            .collect();
        let infos_json = if env::var("MOZSEARCH_DIFFABLE").is_err() {
            to_string(&infos).unwrap()
        } else {
            to_string_pretty(&infos).unwrap()
        };
        // Author names and paths could contain "</script>".  `BLAME_INFO` is the
        // blame of the file in the commit, for the `#tokens=` hash (see
        // code-highlighter.js).
        writeln!(
            writer,
            "<script>var BLAME_INFOS = {};{}</script>",
            infos_json.replace("</", "<\\/"),
            if token_blames[0].is_some() {
                " var BLAME_INFO = BLAME_INFOS[0];"
            } else {
                ""
            }
        )
        .unwrap();
    }

    Ok(())
}

/// The most commits a side of an interdiff can have.
const MAX_INTERDIFF_COMMITS: usize = 50;

/// A side of an interdiff (see `hyperblame::interdiff`): its commits, oldest
/// first (as on explore pages), and the revision before the first.
struct InterdiffSide<'r> {
    commits: Vec<git2::Commit<'r>>,
    refs: Vec<CommitRef>,
    base: git2::Commit<'r>,
}

impl InterdiffSide<'_> {
    /// The side's commits as a URL component.
    fn key(&self) -> String {
        self.commits.iter().map(|c| c.id().to_string()).join(",")
    }

    fn post(&self) -> &git2::Commit<'_> {
        self.commits.last().unwrap()
    }
}

/// The side of an interdiff with the comma-separated revisions `revs`.
fn interdiff_side<'r>(repo: &'r Repository, revs: &str) -> Result<InterdiffSide<'r>, &'static str> {
    let mut refs = vec![];
    for rev in revs
        .split(',')
        .map(str::trim)
        .filter(|rev| !rev.is_empty())
        .take(MAX_INTERDIFF_COMMITS)
    {
        let commit = repo
            .revparse_single(rev)
            .and_then(|object| object.peel_to_commit())
            .map_err(|_| "Bad revision")?;
        // (The commit index's dates, which `explore::order_commits` orders.)
        let iso_date = chrono::DateTime::from_timestamp(commit.time().seconds(), 0)
            .map(|date| date.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .unwrap_or_default();
        refs.push(CommitRef {
            rev: commit.id().to_string(),
            iso_date,
            backout: false,
        });
    }
    if refs.is_empty() {
        return Err("No revisions");
    }
    explore::order_commits(repo, &mut refs, None);
    let commits: Vec<_> = refs
        .iter()
        .map(|commit_ref| {
            Oid::from_str(&commit_ref.rev)
                .and_then(|oid| repo.find_commit(oid))
                .map_err(|_| "Bad revision")
        })
        .collect::<Result<_, _>>()?;
    let base = commits[0]
        .parent(0)
        .map_err(|_| "The first commit of a side has no parent")?;
    Ok(InterdiffSide {
        commits,
        refs,
        base,
    })
}

/// The file at `path` in revisions, with its tokens and their identities in the
/// history (see `hyperblame::interdiff`), or nothing for revisions without the
/// file, by revision.
struct InterdiffFiles<'g> {
    git: &'g GitData,
    history: &'g TreeHistory,
    path: &'g str,
    files: HashMap<Oid, Rc<(String, Vec<IdToken>)>>,
}

impl InterdiffFiles<'_> {
    fn get(&mut self, commit: &git2::Commit) -> Result<Rc<(String, Vec<IdToken>)>, &'static str> {
        if let Some(file) = self.files.get(&commit.id()) {
            return Ok(file.clone());
        }
        let file = Rc::new(self.read(commit)?);
        self.files.insert(commit.id(), file.clone());
        Ok(file)
    }

    fn read(&self, commit: &git2::Commit) -> Result<(String, Vec<IdToken>), &'static str> {
        let Ok((repo, oid)) = get_object_at(
            OwnedOrBorrowed::Borrowed(&self.git.repo),
            commit.id(),
            Path::new(self.path),
        ) else {
            return Ok((String::new(), vec![]));
        };
        let object = repo
            .find_object(oid, Some(git2::ObjectType::Blob))
            .map_err(|_| "Not a file")?;
        let source = git_ops::read_blob_object(&object);
        let timeline_commit = self
            .history
            .timeline_commit(commit.id())
            .ok_or("The token-centric history doesn't have one of the revisions")?;
        let file_history = self
            .history
            .file_history(&timeline_commit, self.path)
            .map_err(|_| "Couldn't read the file's history")?
            .ok_or("The token-centric history doesn't have the file (ex: it's binary)")?;
        let tokens = {
            let blame = blame_tokens(&source, &file_history)
                .map_err(|_| "The file's history doesn't match it")?;
            // The syntax file has a line for each token, with its context.
            let syntax_lines = token_file_lines(&file_history.syntax);
            blame
                .tokens
                .iter()
                .enumerate()
                .map(|(i, t)| IdToken {
                    id: (
                        t.data.introduced.source_rev.to_string(),
                        t.data.introduced.path.to_string(),
                        t.data.introduced.lineno,
                    ),
                    text: source[t.range.clone()].to_string(),
                    context: syntax_lines
                        .get(i)
                        .map_or("%", |line| split_token_line(line).context)
                        .to_string(),
                    range: t.range.clone(),
                    line: t.line,
                })
                .collect()
        };
        Ok((source, tokens))
    }

    /// The tokens of the file at the side's base which the side's commits
    /// removed, and the tokens of the file after the side's last commit which
    /// they added: those each commit which changed the file removed from it or
    /// added to it.
    fn changes(
        &mut self,
        side: &InterdiffSide,
    ) -> Result<(HashSet<TokenId>, HashSet<TokenId>), &'static str> {
        let entry = |commit: &git2::Commit| {
            commit
                .tree()
                .ok()
                .and_then(|tree| tree.get_path(Path::new(self.path)).ok())
                .map(|entry| entry.id())
        };
        let (mut removed, mut added) = (HashSet::new(), HashSet::new());
        for commit in &side.commits {
            let parent = commit.parent(0).map_err(|_| "A commit has no parent")?;
            if entry(&parent) == entry(commit) {
                continue;
            }
            let (before, after) = (self.get(&parent)?, self.get(commit)?);
            removed.extend(interdiff::removed(&before.1, &after.1));
            added.extend(interdiff::added(&before.1, &after.1));
        }
        let (base, post) = (self.get(&side.base)?, self.get(side.post())?);
        let ids = |file: &(String, Vec<IdToken>)| -> HashSet<TokenId> {
            file.1.iter().map(|t| t.id.clone()).collect()
        };
        let (base_ids, post_ids) = (ids(&base), ids(&post));
        removed.retain(|id| base_ids.contains(id));
        added.retain(|id| post_ids.contains(id));
        Ok((removed, added))
    }
}

/// The byte offset of the start of each line of `source`.
fn line_starts(source: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(source.match_indices('\n').map(|(i, _)| i + 1))
        .collect()
}

/// The `data-idiff` attributes of the code of the rows of the lines of a file,
/// by (1-based) line, which interdiff.js marks the tokens with: "START:END:MARK"
/// for each marked token (see `interdiff::Mark::class`), separated by ";",
/// where START and END are UTF-16 offsets in the row's code, which starts with
/// the row's origin (ex: "+ ").
fn interdiff_attrs(
    source: &str,
    tokens: &[IdToken],
    marks: &[Option<InterdiffMark>],
) -> HashMap<usize, String> {
    let starts = line_starts(source);
    let utf16_len = |s: &str| s.encode_utf16().count();
    let mut attrs: HashMap<usize, Vec<String>> = HashMap::new();
    for (token, mark) in tokens.iter().zip(marks) {
        let Some(mark) = mark else {
            continue;
        };
        let start = 2 + utf16_len(&source[starts[token.line]..token.range.start]);
        let end = start + utf16_len(&token.text);
        attrs.entry(token.line + 1).or_default().push(format!(
            "{}:{}:{}",
            start,
            end,
            mark.class()
        ));
    }
    attrs
        .into_iter()
        .map(|(line, marks)| (line, format!(r#" data-idiff="{}""#, marks.join(";"))))
        .collect()
}

/// A's lines with the tokens which only A added (see
/// `hyperblame::interdiff::Interdiff::only_a`), to go before the rows of B's
/// lines (see `DiffRowExtras::rows_before`), each after the row of B's line
/// with the nearest matched token before A's tokens, by the line of B's
/// version they go before.
fn interdiff_only_a_lines(
    a_source: &str,
    a_tokens: &[IdToken],
    only_a: &[(usize, Option<usize>)],
) -> BTreeMap<usize, Vec<OnlyALine>> {
    let starts = line_starts(a_source);
    // A's lines with the tokens, with where they go, the tokens' byte ranges in
    // the line, and the first token's context.
    type Line<'t> = (Option<usize>, Vec<Range<usize>>, &'t str);
    let mut lines: BTreeMap<usize, Line> = BTreeMap::new();
    for &(i, anchor) in only_a {
        let token = &a_tokens[i];
        let start = starts[token.line];
        lines
            .entry(token.line)
            .or_insert((anchor, vec![], &token.context))
            .1
            .push(token.range.start - start..token.range.end - start);
    }
    let mut rows: BTreeMap<usize, Vec<OnlyALine>> = BTreeMap::new();
    for (line, (anchor, ranges, context)) in lines {
        let end = starts.get(line + 1).map_or(a_source.len(), |next| next - 1);
        let text = &a_source[starts[line]..end];
        let mut content = String::new();
        let mut pos = 0;
        for range in ranges {
            content.push_str(&entity_escape(&text[pos..range.start]));
            write!(
                content,
                r#"<span class="idiff-only-a">{}</span>"#,
                entity_escape(&text[range.clone()])
            )
            .unwrap();
            pos = range.end;
        }
        content.push_str(&entity_escape(&text[pos..]));
        // After B's line `anchor` (0-based), so before line `anchor` + 2.
        rows.entry(anchor.map_or(1, |anchor| anchor + 2))
            .or_default()
            .push(OnlyALine {
                line,
                content,
                context: context.to_string(),
            });
    }
    rows
}

/// One of A's lines with tokens which only A added; see
/// `interdiff_only_a_lines`.
struct OnlyALine {
    /// The (0-based) line of A's version.
    line: usize,
    /// The line as HTML, with the tokens marked.
    content: String,
    /// The context of the line's first such token.
    context: String,
}

impl OnlyALine {
    fn title(&self) -> String {
        format!(
            "Line {} of A's version, whose highlighted tokens B doesn't have",
            self.line + 1
        )
    }

    /// The line's row, for diff pages (see `DiffRowExtras::rows_before`).
    fn row_html(&self) -> String {
        format!(
            r#"<div role="row" class="source-line-with-number interdiff-only-a"><div class="line-strip"><div role="cell" class="blame-container"><div class="blame-strip"></div></div></div><div role="cell" class="line-number" data-line-number=""></div><code role="cell" class="source-line" title="{}">~ {}
</code></div>"#,
            self.title(),
            self.content
        )
    }
}

fn entity_escape(s: &str) -> String {
    s.replace("&", "&amp;").replace("<", "&lt;")
}

/// The interdiff of a file (see `interdiff_file_data`): B's diff of it, what marks
/// its rows' tokens, and the contexts of the differences (see
/// `interdiff::Mark::differs`) and of B's lines, by line (1-based).
struct FileInterdiff {
    difftxt: String,
    /// The `data-idiff` attributes of the rows of B's version and of B's base,
    /// and A's lines with tokens B doesn't have (see `DiffRowExtras`).
    line_attrs: HashMap<usize, String>,
    removed_line_attrs: HashMap<usize, String>,
    only_a: BTreeMap<usize, Vec<OnlyALine>>,
    counts: InterdiffCounts,
    /// The contexts of the first differences on lines of B's version and of
    /// B's base.
    post_differences: BTreeMap<usize, String>,
    base_differences: BTreeMap<usize, String>,
    /// The contexts of the first tokens of the lines of B's version and of B's
    /// base which have tokens.
    post_contexts: HashMap<usize, String>,
    base_contexts: HashMap<usize, String>,
}

impl FileInterdiff {
    /// Whether the patches differ in the file.
    fn differs(&self) -> bool {
        !(self.post_differences.is_empty()
            && self.base_differences.is_empty()
            && self.only_a.is_empty())
    }

    /// The contexts with differences.
    fn difference_contexts(&self) -> BTreeSet<&str> {
        self.post_differences
            .values()
            .chain(self.base_differences.values())
            .map(String::as_str)
            .chain(
                self.only_a
                    .values()
                    .flatten()
                    .map(|line| line.context.as_str()),
            )
            .collect()
    }

    fn extras(&self) -> DiffRowExtras {
        DiffRowExtras {
            line_attrs: self.line_attrs.clone(),
            removed_line_attrs: self.removed_line_attrs.clone(),
            rows_before: self
                .only_a
                .iter()
                .map(|(&key, lines)| (key, lines.iter().map(OnlyALine::row_html).collect()))
                .collect(),
        }
    }
}

/// The contexts of the first tokens of the lines of a file (1-based) with
/// tokens, and of its first tokens with marks which are differences.
fn interdiff_line_contexts(
    tokens: &[IdToken],
    marks: &[Option<InterdiffMark>],
) -> (HashMap<usize, String>, BTreeMap<usize, String>) {
    let mut contexts = HashMap::new();
    let mut differences = BTreeMap::new();
    for (token, mark) in tokens.iter().zip(marks) {
        contexts
            .entry(token.line + 1)
            .or_insert_with(|| token.context.clone());
        if mark.is_some_and(InterdiffMark::differs) {
            differences
                .entry(token.line + 1)
                .or_insert_with(|| token.context.clone());
        }
    }
    (contexts, differences)
}

/// The interdiff of the file at `path` between the sides `a` and `b`.
fn file_interdiff(
    git: &GitData,
    history: &TreeHistory,
    git_path: &str,
    a: &InterdiffSide,
    b: &InterdiffSide,
    path: &str,
) -> Result<FileInterdiff, &'static str> {
    let mut files = InterdiffFiles {
        git,
        history,
        path,
        files: HashMap::new(),
    };
    let (a_base, a_post) = (files.get(&a.base)?, files.get(a.post())?);
    let (b_base, b_post) = (files.get(&b.base)?, files.get(b.post())?);
    if [&a_base, &a_post, &b_base, &b_post]
        .iter()
        .all(|file| file.1.is_empty() && file.0.is_empty())
    {
        return Err("Neither side has the file");
    }
    let ((a_removed, a_added), (b_removed, b_added)) = (files.changes(a)?, files.changes(b)?);
    let result = interdiff::interdiff(
        &InterdiffSideTokens {
            base: &a_base.1,
            post: &a_post.1,
            removed: &a_removed,
            added: &a_added,
        },
        &InterdiffSideTokens {
            base: &b_base.1,
            post: &b_post.1,
            removed: &b_removed,
            added: &b_added,
        },
    );

    // B's diff of the file.  (The system git, with SHA-1 collision detection,
    // since this works on a source repo; see `fast_import_git` for our git.)
    let output = Command::new("git")
        .arg("diff")
        .arg("-p")
        .arg("--patience")
        .arg("--full-index")
        .arg("--no-prefix")
        .arg("-U100000")
        .arg(b.base.id().to_string())
        .arg(b.post().id().to_string())
        .arg("--")
        .arg(path)
        .current_dir(git_path)
        .output()
        .map_err(|_| "Diff failed 1")?;
    if !output.status.success() {
        return Err("Diff failed 2");
    }
    let mut difftxt = git_ops::decode_bytes(output.stdout);
    if difftxt.is_empty() {
        // B didn't change the file, so its diff is all context.
        difftxt = String::from("@@ @@\n");
        for line in b_post.0.lines() {
            writeln!(difftxt, " {}", line).unwrap();
        }
    }

    let (post_contexts, post_differences) = interdiff_line_contexts(&b_post.1, &result.post_marks);
    let (base_contexts, base_differences) = interdiff_line_contexts(&b_base.1, &result.base_marks);
    Ok(FileInterdiff {
        difftxt,
        line_attrs: interdiff_attrs(&b_post.0, &b_post.1, &result.post_marks),
        removed_line_attrs: interdiff_attrs(&b_base.0, &b_base.1, &result.base_marks),
        only_a: interdiff_only_a_lines(&a_post.0, &a_post.1, &result.only_a),
        counts: result.counts,
        post_differences,
        base_differences,
        post_contexts,
        base_contexts,
    })
}

/// Excerpts of a function show all of its rows if it has at most this many,
/// and otherwise its first row and the rows around its differences.
const MAX_EXCERPT_FUNCTION_ROWS: usize = 40;
/// How many rows around differences excerpts show.
const EXCERPT_CONTEXT_ROWS: usize = 3;
/// The most rows of excerpts a file shows, and an interdiff's summary.
const MAX_EXCERPT_ROWS: usize = 300;
const MAX_SUMMARY_EXCERPT_ROWS: usize = 3000;

/// Excerpts of B's diff of a file (see `FileInterdiff`) where the patches
/// differ, as HTML: for each function with differences, all of its rows if
/// it's short, and otherwise its first row (its declaration) and the rows
/// around the differences, as for differences outside of any function.  A
/// function's rows are those whose first token has its context (or a nested
/// one).  `link` is the file's interdiff, which the line numbers link to.  The
/// excerpts stop after `max_rows` rows (and at most `MAX_EXCERPT_ROWS`), and
/// this also returns how many they have.
fn interdiff_excerpts<'f>(
    cfg: &Config,
    path: &str,
    file: &'f FileInterdiff,
    link: &str,
    max_rows: usize,
) -> Result<(String, usize), &'static str> {
    let (rows, new_lines, _) = parse_diff_rows(&file.difftxt, 1, None, None)?;
    let format = languages::select_formatting(path);
    if let FormatAs::Binary = format {
        return Ok((String::new(), 0));
    }
    let max_rows = max_rows.min(MAX_EXCERPT_ROWS);
    let (formatted_lines, _) = format_code(
        Some(cfg),
        &TreeJumprefs::default(),
        format,
        path,
        &new_lines,
        &[],
    );

    /// A row of the excerpts (a row of B's diff, or one of A's lines), with
    /// its context and the context of its first difference.
    struct Item<'a> {
        html: String,
        context: Option<&'a str>,
        difference: Option<&'a str>,
    }
    // (Not `.line-number`, which would make code-highlighter.js select the
    // line in this page.)
    let row_html = |lineno: Option<usize>, class: &str, attrs: &str, code: &str| {
        let number = match lineno {
            Some(lineno) => format!(
                r#"<a class="interdiff-excerpt-lineno" href="{}#{}">{}</a>"#,
                link, lineno, lineno
            ),
            None => r#"<span class="interdiff-excerpt-lineno"></span>"#.to_string(),
        };
        format!(
            r#"<div role="row" class="source-line-with-number{}">{}<code role="cell" class="source-line"{}>{}
</code></div>"#,
            class, number, attrs, code
        )
    };
    let only_a_item = |line: &'f OnlyALine| Item {
        html: row_html(
            None,
            " interdiff-only-a",
            &format!(r#" title="{}""#, line.title()),
            &format!("~ {}", line.content),
        ),
        context: Some(line.context.as_str()),
        difference: Some(line.context.as_str()),
    };
    let mut items: Vec<Item> = vec![];
    let mut last_line = 0;
    for row in &rows {
        let origin = row.origin.iter().collect::<String>();
        let item = if row.lineno > 0 {
            let lineno = row.lineno as usize;
            items.extend(
                file.only_a
                    .get(&lineno)
                    .into_iter()
                    .flatten()
                    .map(only_a_item),
            );
            last_line = lineno;
            let content = formatted_lines
                .get(lineno - 1)
                .map_or_else(|| entity_escape(row.content), |line| line.line.clone());
            Item {
                html: row_html(
                    Some(lineno),
                    if origin.contains('+') {
                        " plus-line"
                    } else {
                        ""
                    },
                    file.line_attrs.get(&lineno).map_or("", String::as_str),
                    &format!("{} {}", origin, content),
                ),
                context: file.post_contexts.get(&lineno).map(String::as_str),
                difference: file.post_differences.get(&lineno).map(String::as_str),
            }
        } else {
            let base_line = row.token_line.map_or(0, |(_, line)| line);
            Item {
                html: row_html(
                    None,
                    " minus-line",
                    file.removed_line_attrs
                        .get(&base_line)
                        .map_or("", String::as_str),
                    &format!("{} {}", origin, entity_escape(row.content)),
                ),
                context: file.base_contexts.get(&base_line).map(String::as_str),
                difference: file.base_differences.get(&base_line).map(String::as_str),
            }
        };
        items.push(item);
    }
    items.extend(
        file.only_a
            .range(last_line + 1..)
            .flat_map(|(_, lines)| lines)
            .map(only_a_item),
    );

    // Each difference's range of items (its function's, or the items around
    // it), with its function's first item if the range doesn't start there,
    // merged with the ranges they overlap or touch.
    let in_context = |item: &Item, context: &str| {
        item.context.is_some_and(|c| {
            c == context
                || c.strip_prefix(context)
                    .is_some_and(|rest| rest.starts_with("::"))
        })
    };
    let mut extents: HashMap<&str, Option<(usize, usize)>> = HashMap::new();
    let mut ranges: Vec<(usize, usize, Option<usize>, Vec<&str>)> = vec![];
    for (i, item) in items.iter().enumerate() {
        let Some(context) = item.difference else {
            continue;
        };
        let extent = *extents.entry(context).or_insert_with(|| {
            if context == "%" {
                return None;
            }
            let first = items.iter().position(|item| in_context(item, context))?;
            let last = items.iter().rposition(|item| in_context(item, context))?;
            Some((first, last))
        });
        let around = (
            i.saturating_sub(EXCERPT_CONTEXT_ROWS),
            (i + EXCERPT_CONTEXT_ROWS).min(items.len() - 1),
        );
        let (start, end, first) = match extent {
            Some((first, last)) if last - first < MAX_EXCERPT_FUNCTION_ROWS => (first, last, None),
            Some((first, _)) => (around.0, around.1, Some(first)),
            None => (around.0, around.1, None),
        };
        match ranges.last_mut() {
            Some(last) if start <= last.1 + 1 => {
                last.1 = last.1.max(end);
                if !last.3.contains(&context) {
                    last.3.push(context);
                }
            }
            _ => ranges.push((start, end, first.filter(|&f| f < start), vec![context])),
        }
    }

    let mut html = String::new();
    let mut shown = 0;
    for (start, end, first, contexts) in &ranges {
        if shown >= max_rows {
            write!(
                html,
                r#"<p class="interdiff-excerpt-more"><a href="{}">More differences are in the file's interdiff.</a></p>"#,
                link
            )
            .unwrap();
            break;
        }
        let names: Vec<String> = contexts
            .iter()
            .map(|&context| match context {
                "%" => "the file, outside of any function".to_string(),
                _ => format!("<code>{}</code>", entity_escape(context)),
            })
            .collect();
        write!(
            html,
            r#"<div class="interdiff-excerpt"><div class="interdiff-excerpt-header">In {}</div><div class="interdiff-excerpt-rows" role="table">"#,
            names.join(", "),
        )
        .unwrap();
        if let Some(first) = first {
            html.push_str(&items[*first].html);
            html.push_str(r#"<div class="interdiff-excerpt-gap">⋯</div>"#);
        }
        for item in &items[*start..=*end] {
            html.push_str(&item.html);
        }
        html.push_str("</div></div>\n");
        shown += end - start + 1;
    }
    Ok((html, shown))
}

/// The legend of an interdiff's marks (see `interdiff::Mark`) as HTML, with how
/// many tokens each marks.
fn interdiff_legend(counts: &InterdiffCounts) -> String {
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    format!(
        r#"B's diff, compared with A's: <span class="idiff-new">{} token{} new in B</span>, <span class="idiff-same">{} the same as A's</span>, <span class="idiff-only-a">{} of A's which B doesn't have</span> (on rows of A's lines), <span class="idiff-rm-new">{} removed only by B</span>, <span class="idiff-rm-same">{} removed by both</span>, <span class="idiff-kept">{} removed by A but kept by B</span>, and <span class="idiff-base">{} from neither patch</span>."#,
        counts.new,
        plural(counts.new),
        counts.same,
        counts.only_a,
        counts.removed_new,
        counts.removed_same,
        counts.kept,
        counts.base,
    )
}

/// How B's diff of a file differs from A's, as HTML: the counts of the marks
/// which are differences (see `interdiff::Mark::differs`).
fn interdiff_differences(counts: &InterdiffCounts) -> String {
    [
        (counts.new, "idiff-new", "new in B"),
        (counts.only_a, "idiff-only-a", "only in A"),
        (counts.removed_new, "idiff-rm-new", "removed only by B"),
        (counts.kept, "idiff-kept", "removed by A but kept by B"),
    ]
    .into_iter()
    .filter(|(n, _, _)| *n > 0)
    .map(|(n, class, what)| format!(r#"<span class="{}">{} {}</span>"#, class, n, what))
    .join(", ")
}

/// The commits of a side of an interdiff as an HTML list, with when they
/// landed.
fn interdiff_commits_html(tree_name: &str, side: &InterdiffSide) -> String {
    let mut html = String::from("<ol>");
    for (commit, commit_ref) in side.commits.iter().zip(&side.refs) {
        let header = blame::commit_header(commit).unwrap_or_default();
        write!(
            html,
            r#"<li><span class="explore-commit-date" title="When it landed">{} UTC</span> <a href="/{}/commit/{}">{}</a> {}</li>"#,
            commit_ref.iso_date.get(..16).unwrap_or("").replace('T', " "),
            tree_name,
            commit.id(),
            &commit.id().to_string()[..12],
            header
        )
        .unwrap();
    }
    html.push_str("</ol>");
    html
}

/// The words, runs of whitespace, and other characters of `text`, for diffs of
/// commit messages.
fn message_words(text: &str) -> Vec<&str> {
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            0
        } else if c.is_whitespace() {
            1
        } else {
            2
        }
    };
    let mut words = vec![];
    let mut chars = text.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        let mut end = start + c.len_utf8();
        if class(c) != 2 {
            while let Some(&(i, next)) = chars.peek() {
                if class(next) != class(c) {
                    break;
                }
                end = i + next.len_utf8();
                chars.next();
            }
        }
        words.push(&text[start..end]);
    }
    words
}

/// How many unchanged lines diffs of commit messages show around changes, and
/// the fewest they elide.
const MESSAGE_CONTEXT_LINES: usize = 3;

/// A word diff of the sides' commit messages (each side's, oldest first) as
/// HTML, with A's words which B doesn't have struck out and B's words which A
/// doesn't have highlighted (ex: a reland's new reviewers), or an empty string
/// if they're the same.
fn interdiff_messages_html(a: &InterdiffSide, b: &InterdiffSide) -> String {
    let messages = |side: &InterdiffSide| {
        side.commits
            .iter()
            .map(|commit| {
                String::from_utf8_lossy(commit.message_bytes())
                    .trim_end()
                    .to_string()
            })
            .join("\n\n")
    };
    message_diff_html(&messages(a), &messages(b))
}

/// A word diff of the texts as HTML; see `interdiff_messages_html`.
fn message_diff_html(a_text: &str, b_text: &str) -> String {
    if a_text == b_text {
        return String::new();
    }
    let (a_words, b_words) = (message_words(a_text), message_words(b_text));

    // The diff's lines, as pieces which are in both messages, only A's, or
    // only B's.
    use similar::DiffTag;
    let mut lines: Vec<Vec<(DiffTag, String)>> = vec![vec![]];
    for op in similar::capture_diff_slices(similar::Algorithm::Patience, &a_words, &b_words) {
        let (tag, old, new) = op.as_tag_tuple();
        let pieces = match tag {
            DiffTag::Equal => vec![(DiffTag::Equal, &b_words[new])],
            _ => vec![
                (DiffTag::Delete, &a_words[old]),
                (DiffTag::Insert, &b_words[new]),
            ],
        };
        for (tag, words) in pieces {
            for (i, piece) in words.concat().split('\n').enumerate() {
                if i > 0 {
                    lines.push(vec![]);
                }
                if !piece.is_empty() {
                    lines.last_mut().unwrap().push((tag, piece.to_string()));
                }
            }
        }
    }

    // The changed lines and the lines around them, with the other runs of
    // unchanged lines elided (ex: a stack's other commits).
    let changed: Vec<bool> = lines
        .iter()
        .map(|line| line.iter().any(|(tag, _)| *tag != DiffTag::Equal))
        .collect();
    let near_change = |i: usize| {
        let around = i.saturating_sub(MESSAGE_CONTEXT_LINES)..(i + MESSAGE_CONTEXT_LINES + 1);
        changed[around.start..around.end.min(lines.len())]
            .iter()
            .any(|&c| c)
    };
    let mut html = vec![];
    let mut i = 0;
    while i < lines.len() {
        let hidden = (i..lines.len()).take_while(|&j| !near_change(j)).count();
        if hidden >= MESSAGE_CONTEXT_LINES {
            html.push(format!(
                r#"<span class="interdiff-messages-gap">⋯ {} unchanged lines</span>"#,
                hidden
            ));
            i += hidden;
            continue;
        }
        let mut line = String::new();
        for (tag, piece) in &lines[i] {
            match tag {
                DiffTag::Delete => write!(
                    line,
                    r#"<del class="idiff-only-a">{}</del>"#,
                    entity_escape(piece)
                ),
                DiffTag::Insert => write!(
                    line,
                    r#"<ins class="idiff-new">{}</ins>"#,
                    entity_escape(piece)
                ),
                _ => write!(line, "{}", entity_escape(piece)),
            }
            .unwrap();
        }
        html.push(line);
        i += 1;
    }
    html.join("\n")
}

/// The interdiff of the file at `path` between two versions of a patch (ex: a
/// landing and its reland), each a commit or a stack of them (comma-separated
/// revisions): B's diff of the file, from before its first commit to its last,
/// with its added and removed tokens marked by how they compare with A's (see
/// `hyperblame::interdiff`).
/// The data of a file's interdiff (see `interdiff_file_data` and
/// `render_interdiff_file`).
#[derive(Deserialize, Serialize)]
pub struct InterdiffFile {
    /// The diff of B's diff from A's, as `parse_diff_rows` takes it.
    pub diff: String,
    pub extras: DiffRowExtras,
    /// The token-centric blame of the file after B, and before it.
    pub token_blames: Vec<Option<PageBlame>>,
    /// B's last commit, its header (HTML) and its date (RFC 3339).
    pub b_rev: String,
    pub header: String,
    pub date: String,
    /// The sides' keys (their revisions, comma-separated) and commits (HTML;
    /// see `interdiff_commits_html`), and the legend of the differences.
    pub a_key: String,
    pub b_key: String,
    pub a_commits: String,
    pub b_commits: String,
    pub legend: String,
}

/// The data of the interdiff of the file at `path` between the sides `a_revs`
/// and `b_revs` (comma-separated revisions; see `render_interdiff_file`).
/// (For trees' servers; see `AbstractServer::interdiff_file`.)
pub fn interdiff_file_data(
    cfg: &Config,
    tree_name: &str,
    a_revs: &str,
    b_revs: &str,
    path: &str,
) -> Result<InterdiffFile, &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let git = tree_config.get_git()?;
    let history = git
        .history
        .as_ref()
        .ok_or("Interdiffs need the token-centric history")?;
    let repo: &Repository = &git.repo;
    let a = interdiff_side(repo, a_revs)?;
    let b = interdiff_side(repo, b_revs)?;

    let file = file_interdiff(git, history, tree_config.get_git_path()?, &a, &b, path)?;
    Ok(InterdiffFile {
        extras: file.extras(),
        legend: interdiff_legend(&file.counts),
        diff: file.difftxt,
        token_blames: vec![
            revision_token_blame(tree_config, tree_name, git, b.post(), path),
            revision_token_blame(tree_config, tree_name, git, &b.base, path),
        ],
        b_rev: b.post().id().to_string(),
        header: blame::commit_header(b.post())?,
        date: git_time_to_chrono(b.post().time()).to_rfc3339(),
        a_key: a.key(),
        b_key: b.key(),
        a_commits: interdiff_commits_html(tree_name, &a),
        b_commits: interdiff_commits_html(tree_name, &b),
    })
}

/// A file's interdiff (`/{tree}/interdiff/{A}/{B}/{path}`), with its `data`
/// (see `interdiff_file_data`).
pub fn render_interdiff_file(
    cfg: &Config,
    tree_name: &str,
    path: &str,
    data: InterdiffFile,
    writer: &mut dyn Write,
) -> Result<(), &'static str> {
    let (rows, new_lines, num_lines) = parse_diff_rows(&data.diff, 1, None, None)?;
    let mut token_blames = data.token_blames;
    drop_mismatched_token_blames(path, &mut token_blames, &num_lines);
    if let FormatAs::Binary = languages::select_formatting(path) {
        return Err("Cannot diff binary file");
    };
    let extras = data.extras;

    let b_rev = data.b_rev;
    let header = data.header;
    let filename = Path::new(path).file_name().unwrap().to_str().unwrap();
    let title = format!("{} interdiff - mozsearch", filename);
    let opt = Options {
        title: &title,
        tree_name,
        include_date: true,
        revision: Some(RevisionData {
            rev: &b_rev,
            desc: &header,
            date: chrono::DateTime::parse_from_rfc3339(&data.date).map_err(|_| "Bad date")?,
        }),
        breadcrumbs_links_to: BreadcrumbsLinksTo::Historical,
        extra_content_classes: "source-listing diff interdiff",
        search_rev: None,
    };
    output::generate_header(&opt, writer)?;
    let file_syms = vec![make_file_sym_from_path(path)];
    output::generate_breadcrumbs(&opt, writer, path, &file_syms, false)?;

    let encoded_path = url_encode_path(path);
    let (a_key, b_key) = (data.a_key, data.b_key);
    let sections = vec![PanelSection {
        name: "Interdiff".to_owned(),
        items: vec![
            PanelItem {
                label: PanelItemLabel::Plaintext("Swap A and B".to_owned()),
                tooltip: "Show A's diff compared with B's".to_owned(),
                id: "panel-interdiff-swap",
                link: format!(
                    "/{}/interdiff/{}/{}/{}",
                    tree_name, b_key, a_key, encoded_path
                ),
                update_link_lineno: "",
                accel_key: None,
                copyable: true,
                extras: Default::default(),
            },
            PanelItem {
                label: PanelItemLabel::Plaintext("All files".to_owned()),
                tooltip: "The files A or B changed".to_owned(),
                id: "panel-interdiff-files",
                link: format!("/{}/interdiff/{}/{}", tree_name, a_key, b_key),
                update_link_lineno: "",
                accel_key: None,
                copyable: true,
                extras: Default::default(),
            },
            PanelItem {
                label: PanelItemLabel::Plaintext("Show B's version".to_owned()),
                tooltip: "Open the file as of B's last commit".to_owned(),
                id: "panel-interdiff-b",
                link: format!("/{}/rev/{}/{}", tree_name, b_rev, encoded_path),
                update_link_lineno: "#{}",
                accel_key: None,
                copyable: true,
                extras: Default::default(),
            },
        ],
        raw_items: vec![],
    }];
    output::generate_panel(&opt, writer, &sections, false)?;

    write!(
        writer,
        r#"<section class="interdiff-summary">
<div class="interdiff-sides"><div><h3>A</h3>{}</div><div><h3>B</h3>{}</div></div>
<p class="interdiff-legend">{}</p>
</section>
"#,
        data.a_commits, data.b_commits, data.legend,
    )
    .unwrap();

    write_diff_rows(writer, cfg, path, &rows, &new_lines, &token_blames, &extras)?;

    output::generate_footer(&opt, tree_name, path, writer).unwrap();
    Ok(())
}

/// The most files an interdiff's summary compares (see
/// `interdiff_files_data`).
const MAX_INTERDIFF_SUMMARY_FILES: usize = 100;

/// An interdiff's summary (see `interdiff_file_data`): the sides' commits, with a
/// diff of their messages, and the files either side changed, with the explore
/// pages' sparklines (see `hyperblame::explore`) of A's commits and then B's,
/// faded for the files and symbols where B's changes are the same as A's, and
/// excerpts of B's diff where they differ (see `interdiff_excerpts`).
/// The data of the interdiff of the files the sides `a_revs` and `b_revs`
/// (comma-separated revisions) changed (see `render_interdiff_files`), as
/// interdiff_files.liquid takes it.  (For trees' servers; see
/// `AbstractServer::interdiff_files`.)
pub fn interdiff_files_data(
    cfg: &Config,
    tree_name: &str,
    a_revs: &str,
    b_revs: &str,
) -> Result<serde_json::Value, &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let git = tree_config.get_git()?;
    let history = git
        .history
        .as_ref()
        .ok_or("Interdiffs need the token-centric history")?;
    let git_path = tree_config.get_git_path()?;
    let repo: &Repository = &git.repo;
    let a = interdiff_side(repo, a_revs)?;
    let b = interdiff_side(repo, b_revs)?;

    let refs: Vec<CommitRef> = a.refs.iter().chain(&b.refs).cloned().collect();
    let commits: Vec<ExploreCommit> = refs
        .iter()
        .enumerate()
        .map(|(i, commit_ref)| explore_commit(tree_config, git, i + 1, commit_ref))
        .collect();
    let changes: Vec<_> = refs
        .iter()
        .map(|commit_ref| commit_changes(Some(history), repo, &commit_ref.rev))
        .collect();
    // The files' interdiffs (of the first MAX_INTERDIFF_SUMMARY_FILES, in the
    // page's order), and the contexts of their differences, which say which
    // symbols' rows of the sparklines to fade: a symbol (or its nested
    // symbols) has differences if their tokens' contexts are it.
    let paths: BTreeSet<&String> = changes.iter().flat_map(|c| c.keys()).collect();
    let interdiffs: HashMap<&str, Result<FileInterdiff, &str>> = paths
        .into_iter()
        .take(MAX_INTERDIFF_SUMMARY_FILES)
        .map(|path| {
            let interdiff = file_interdiff(git, history, git_path, &a, &b, path);
            (path.as_str(), interdiff)
        })
        .collect();
    let contexts: HashMap<&str, BTreeSet<&str>> = interdiffs
        .iter()
        .filter_map(|(&path, interdiff)| {
            Some((path, interdiff.as_ref().ok()?.difference_contexts()))
        })
        .collect();
    let same = |path: &str, pretty: &str| {
        let Some(contexts) = contexts.get(path) else {
            return false;
        };
        let pretty = if pretty == explore::TOP_LEVEL {
            "%"
        } else {
            pretty
        };
        !contexts.iter().any(|context| {
            *context == pretty
                || context
                    .strip_prefix(pretty)
                    .is_some_and(|rest| rest.starts_with("::"))
        })
    };
    let (a_key, b_key) = (a.key(), b.key());
    let link = |path: &str, _: &str| {
        format!(
            "/{}/interdiff/{}/{}/{}",
            tree_name,
            a_key,
            b_key,
            url_encode_path(path)
        )
    };
    let mut files = blot_files(&refs, &changes, link, same);

    let mut totals = InterdiffCounts::default();
    let mut excerpt_rows = 0;
    for file in &mut files {
        let changed = |commits: Range<usize>| {
            changes[commits]
                .iter()
                .any(|changes| changes.contains_key(&file.path))
        };
        let only = match (changed(0..a.refs.len()), changed(a.refs.len()..refs.len())) {
            (true, false) => "Only A changed it.  ",
            (false, true) => "Only B changed it.  ",
            _ => "",
        };
        let interdiff = match interdiffs.get(file.path.as_str()) {
            Some(Ok(interdiff)) => interdiff,
            Some(Err(err)) => {
                file.note = format!("{}{}.", only, err);
                continue;
            }
            None => {
                file.note = format!("{}Not compared here, since there are so many files.", only);
                continue;
            }
        };
        totals += &interdiff.counts;
        if !interdiff.differs() {
            file.same = true;
            file.note = format!("{}B's changes are the same as A's.", only);
            continue;
        }
        file.note = format!("{}{}", only, interdiff_differences(&interdiff.counts));
        file.excerpts = if excerpt_rows < MAX_SUMMARY_EXCERPT_ROWS {
            let (html, rows) = interdiff_excerpts(
                cfg,
                &file.path,
                interdiff,
                &file.link,
                MAX_SUMMARY_EXCERPT_ROWS - excerpt_rows,
            )?;
            excerpt_rows += rows;
            html
        } else {
            format!(
                r#"<p class="interdiff-excerpt-more"><a href="{}">The differences are in the file's interdiff.</a></p>"#,
                file.link
            )
        };
    }

    let (facets, file_groups) = explore_facets(cfg, tree_name, files);
    Ok(json!({
        "tree": tree_name,
        "sides": [
            {"name": "A", "start": 1, "commits": &commits[..a.refs.len()]},
            {"name": "B", "start": a.refs.len() + 1, "commits": &commits[a.refs.len()..]},
        ],
        "facets": facets,
        "file_groups": file_groups,
        "slot": explore::slot_width(commits.len()),
        "legend": interdiff_legend(&totals),
        "messages": interdiff_messages_html(&a, &b),
        "swap": format!("/{}/interdiff/{}/{}", tree_name, b_key, a_key),
    }))
}

/// The interdiff of the files two sides changed
/// (`/{tree}/interdiff/{A}/{B}`), with its `data` (see
/// `interdiff_files_data`).
pub fn render_interdiff_files(
    tree_name: &str,
    mut data: serde_json::Value,
    writer: &mut dyn Write,
) -> Result<(), &'static str> {
    unescape_explore_names(&mut data["file_groups"]);
    order_explore_files(&mut data["file_groups"]);
    // (Arranged here, if another server's data; see `rearranged_facets`.)
    data["facets"] = crate::cmd_pipeline::facets::rearranged_facets(&data["facets"], &[]);
    let title = "Interdiff - mozsearch";
    let opt = Options {
        title,
        tree_name,
        include_date: true,
        revision: None,
        breadcrumbs_links_to: BreadcrumbsLinksTo::Historical,
        extra_content_classes: "explore interdiff-files",
        search_rev: None,
    };
    output::generate_header(&opt, writer)?;
    output::generate_panel(&opt, writer, &[], true)?;
    let globals = liquid::to_object(&data).map_err(|_| "Template problems")?;
    build_and_parse_interdiff_files()
        .render_to(writer, &globals)
        .map_err(|_| "Template problems")?;
    output::generate_footer(&opt, tree_name, "", writer).unwrap();
    Ok(())
}

fn generate_commit_info(
    tree_name: &str,
    tree_config: &TreeConfig,
    writer: &mut dyn Write,
    commit: &git2::Commit,
) -> Result<(), &'static str> {
    let (header, remainder) = blame::commit_header_remainder(commit)?;

    fn format_rev(tree_name: &str, oid: git2::Oid) -> String {
        format!("<a href=\"/{}/commit/{}\">{}</a>", tree_name, oid, oid)
    }

    fn format_sig(sig: git2::Signature, git: &GitData) -> String {
        let (name, email) = git
            .mailmap
            .lookup(sig.name().unwrap(), sig.email().unwrap());
        format!("{} &lt;{}>", name, email)
    }

    let parents = commit
        .parent_ids()
        .map(|p| {
            F::T(format!(
                "<tr><td>parent</td><td>{}</td></tr>",
                format_rev(tree_name, p)
            ))
        })
        .collect::<Vec<_>>();

    let git = tree_config.get_git()?;
    let oldgit = match git.oldrevs(commit.id()) {
        Some(oldrevs) => vec![F::T(format!(
            "<tr><td>old {} git revs:</td><td>{}</td></tr>",
            tree_config.paths.oldtree_name.clone().unwrap_or_default(),
            oldrevs
        ))],
        None => vec![],
    };

    let hg = match git.hg_rev(commit.id()) {
        Some(hg_id) => {
            let hg_link = format!(
                "<a href=\"{}/rev/{}\">{}</a>",
                tree_config.paths.hg_root.as_ref().unwrap(),
                hg_id,
                hg_id
            );
            vec![F::T(format!("<tr><td>hg</td><td>{}</td></tr>", hg_link))]
        }

        None => vec![],
    };

    let id_string = format!("{}", commit.id());
    let gitstr = tree_config.paths.github_repo.as_ref().map(|ref ghurl| {
        format!(
            "<a href=\"{}/commit/{}\">{}</a>",
            ghurl, id_string, id_string
        )
    });

    let t = git_ops::git_time_to_chrono(commit.time());
    let t = t.to_rfc2822();

    let f = F::Seq(vec![
        F::S("<div class=\"commit-content\">"),
        F::Indent(vec![
            F::T(format!("<h3>{}</h3>", header)),
            F::T(format!("<pre><code>{}</code></pre>", remainder)),
            F::S("<table>"),
            F::Indent(vec![
                F::T(format!(
                    "<tr><td>commit</td><td>{}</td></tr>",
                    format_rev(tree_name, commit.id())
                )),
                F::Seq(parents),
                F::Seq(hg),
                F::T(gitstr.map_or(String::new(), |g| {
                    format!("<tr><td>git</td><td>{}</td></tr>", g)
                })),
                F::Seq(oldgit),
                F::T(format!(
                    "<tr><td>author</td><td>{}</td></tr>",
                    format_sig(commit.author(), git)
                )),
                F::T(format!(
                    "<tr><td>committer</td><td>{}</td></tr>",
                    format_sig(commit.committer(), git)
                )),
                F::T(format!("<tr><td>commit time</td><td>{}</td></tr>", t)),
            ]),
            F::S("</table>"),
        ]),
        F::S("</div>"),
    ]);

    output::generate_formatted(writer, &f, 0)?;

    let git_path = tree_config.get_git_path()?;
    // (The system git, with SHA-1 collision detection, since this works on a
    // source repo; see `fast_import_git` for our git.)
    let output = Command::new("git")
        .arg("show")
        .arg("--cc")
        .arg("--pretty=format:")
        .arg("--raw")
        .arg(id_string)
        .current_dir(git_path)
        .output()
        .map_err(|_| "Diff failed 1")?;
    if !output.status.success() {
        println!("ERR\n{}", git_ops::decode_bytes(output.stderr));
        return Err("Diff failed 2");
    }
    let difftxt = git_ops::decode_bytes(output.stdout);

    let lines = split_lines(&difftxt);
    let mut changes = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }

        let suffix = &line[commit.parents().count()..];
        let prefix_size = 2 * (commit.parents().count() + 1);
        let mut data = suffix.splitn(prefix_size + 1, ' ');
        let data = data.nth(prefix_size).ok_or("Invalid diff output 3")?;
        let file_info = data.split('\t').take(2).collect::<Vec<_>>();

        let f = F::T(format!(
            "<li>{} <a href=\"/{}/diff/{}/{}\">{}</a>",
            file_info[0],
            tree_name,
            commit.id(),
            url_encode_path(file_info[1]),
            file_info[1]
        ));
        changes.push(f);
    }

    let f = F::Seq(vec![F::S("<ul>"), F::Indent(changes), F::S("</ul>")]);
    output::generate_formatted(writer, &f, 0)?;

    Ok(())
}

/// A group of the files of an `/explore/` page or interdiff summary (ex: its
/// test files), as the server renders them; facets.js can regroup them.
#[derive(Serialize)]
struct FileGroupView {
    key: String,
    name: String,
    files: Vec<ExploreFile>,
}

/// The facets of the files of an `/explore/` page or interdiff summary, like
/// `/query/`'s results' (see `cmd_pipeline::facets::file_facets`).  Path kinds
/// and subsystems are the indexed revision's (see `PathFacetMap`), so the path
/// kinds of files it doesn't have (ex: deleted ones) are guessed from their
/// paths, and their subsystems are unknown.
///
/// Also returns the files grouped by path kind (in the order of the tree's
/// per-file-info.toml, as `/search/` shows them) and then sorted by path, and
/// sets their `facets`, `groups`, and `title` for facets.js, which filters
/// them by the facets' values and can group them by subsystem or directory
/// instead.
fn explore_facets(
    cfg: &Config,
    tree_name: &str,
    files: Vec<ExploreFile>,
) -> (Vec<FacetView>, Vec<FileGroupView>) {
    explore_facets_with(path_facet_map(cfg, tree_name).as_deref(), files)
}

/// `explore_facets` with the tree's `PathFacetMap`, if it has one.
fn explore_facets_with(
    map: Option<&PathFacetMap>,
    files: Vec<ExploreFile>,
) -> (Vec<FacetView>, Vec<FileGroupView>) {
    let kinds = PathKinds(map.map_or_else(Vec::new, |map| map.kinds()));
    let facet_files: Vec<FacetFile> = files
        .iter()
        .map(|file| {
            let (kind, known) = map.map_or((ustr(""), false), |map| map.path_kind(&file.path));
            FacetFile {
                path: file.path.clone(),
                kind,
                known,
                subsystem: map.and_then(|map| map.subsystem(&file.path)),
            }
        })
        .collect();
    let expanded_subsystems = map.map_or_else(Vec::new, |map| map.expanded_subsystems.clone());
    let (facets, mut data) = file_facets(&facet_files, &kinds, &expanded_subsystems);

    let mut groups: BTreeMap<(usize, String), FileGroupView> = BTreeMap::new();
    for (mut file, facts) in files.into_iter().zip(&facet_files) {
        let file_data = data.remove(&file.path).unwrap_or_default();
        file.facets = file_data.facets;
        file.groups = file_data.groups;
        file.title = file_data.title;
        groups
            .entry((kinds.order(&facts.kind), facts.kind.to_string()))
            .or_insert_with(|| FileGroupView {
                key: facts.kind.to_string(),
                name: kinds.name(&facts.kind),
                files: vec![],
            })
            .files
            .push(file);
    }
    let mut groups: Vec<FileGroupView> = groups.into_values().collect();
    for group in &mut groups {
        group.files.sort_by(|a, b| a.path.cmp(&b.path));
    }
    (facets, groups)
}

/// A commit of an `/explore/` page or an interdiff's summary, numbered from 1.
fn explore_commit(
    tree_config: &TreeConfig,
    git: &GitData,
    number: usize,
    commit_ref: &CommitRef,
) -> ExploreCommit {
    let header = Oid::from_str(&commit_ref.rev)
        .and_then(|oid| git.repo.find_commit(oid))
        .ok()
        .and_then(|commit| blame::commit_info_json(tree_config, git, &commit).ok())
        .and_then(|info| info["header"].as_str().map(str::to_string))
        .unwrap_or_else(|| commit_ref.rev.clone());
    // See `blame::commit_info_json` for the header's format.
    let summary = header.split("\n<br>").next().unwrap_or("").to_string();
    ExploreCommit {
        number,
        rev: commit_ref.rev.clone(),
        iso_date: commit_ref.iso_date.clone(),
        backout: commit_ref.backout,
        interdiff: None,
        header,
        summary,
        rebased: None,
    }
}

/// The most bugs or Phabricator revisions an `/explore/` page shows.
const MAX_EXPLORE_KEYS: usize = 10;

/// The `/explore/bug/BUGS` and `/explore/phab/REVS` pages, where `kind` is
/// "bug" or "phab" and `keys` is comma-separated: the commits which mention the
/// bugs (or Phabricator revisions), oldest first, and what they changed (see
/// `hyperblame::explore`).
/// The data of an `/explore/bug/BUGS` or `/explore/phab/REVS` page (see
/// `render_explore_page`): its `title`, the keys no commit mentions
/// (`missing`), whether it has fewer commits than there are (`truncated`),
/// the keys' commits on other branches (`other_branches`), its URL in the
/// tree (`explore_url`), and its explore page's data (`explore`; see
/// `explore_data`).  (For trees' servers; see `AbstractServer::explore_keys`.)
pub fn explore_page_data(
    cfg: &Config,
    tree_name: &str,
    kind: &str,
    keys: &str,
) -> Result<serde_json::Value, &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let git = tree_config.get_git()?;
    let history = git.history.as_ref();
    let ExploreRefs {
        index,
        keys,
        mut refs,
        own_revs,
        missing,
        truncated,
    } = explore_refs(cfg, tree_name, kind, keys)?;
    explore::mark_history_backouts(history.map(|h| Path::new(&h.path)), &mut refs);

    // The commits for the keys on other branches (ex: uplifts to beta, or
    // main's after beta's cut), linked to the trees for those branches (with
    // the same history), if any.
    let mut other_branches = vec![];
    for (branch, other_index) in index.other_branches() {
        let mut found: Vec<CommitRef> = keys
            .iter()
            .flat_map(|key| match kind {
                "bug" => other_index.bug_commits(key),
                _ => other_index.phab_commits(key),
            })
            .filter(|r| !own_revs.contains(&r.rev))
            .collect();
        if found.is_empty() {
            continue;
        }
        explore::order_commits(&git.repo, &mut found, Some(&other_index));
        found.truncate(explore::MAX_COMMITS);
        let branch_tree = cfg
            .trees
            .iter()
            .find(|(_, other)| {
                other.paths.history_path == tree_config.paths.history_path
                    && other.paths.git_branch.as_deref().unwrap_or("HEAD") == branch
            })
            .map(|(name, _)| name.clone());
        let commits: Vec<ExploreCommit> = found
            .iter()
            .enumerate()
            .map(|(i, commit_ref)| explore_commit(tree_config, git, i + 1, commit_ref))
            .collect();
        other_branches.push(json!({
            "branch": branch,
            "tree": branch_tree,
            "link_tree": branch_tree.as_deref().unwrap_or(tree_name),
            "commits": commits,
        }));
    }

    let title = match kind {
        "bug" if keys.len() == 1 => format!("Bug {}", keys[0]),
        "bug" => format!("Bugs {}", keys.join(", ")),
        _ => keys.join(", "),
    };
    let explore_url = format!("explore/{}/{}", kind, keys.join(","));
    let explore = explore_data(cfg, tree_name, &refs, None, Some(&explore_url))?;
    Ok(json!({
        "title": title,
        "missing": missing,
        "truncated": truncated,
        "other_branches": other_branches,
        "explore_url": explore_url,
        "explore": explore,
    }))
}

/// An `/explore/bug/BUGS` or `/explore/phab/REVS` page, with its `data` (see
/// `explore_page_data`), and its files' outline diffs from `server`.
pub fn render_explore_page(
    server: &(dyn AbstractServer + Send + Sync),
    tree_name: &str,
    data: serde_json::Value,
    writer: &mut dyn Write,
) -> Result<(), &'static str> {
    let strings = |value: &serde_json::Value| -> Vec<String> {
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect()
    };
    let extras = ExploreExtras {
        missing: strings(&data["missing"]),
        truncated: data["truncated"].as_bool().unwrap_or(false),
        other_branches: data["other_branches"]
            .as_array()
            .cloned()
            .unwrap_or_default(),
        explore_url: data["explore_url"].as_str().unwrap_or_default().to_string(),
        note: None,
        scopes: vec![],
        semantic_head: None,
        semantic_failure: None,
        landed: vec![],
        pull_request: None,
    };
    let title = data["title"].as_str().unwrap_or_default().to_string();
    render_explore(
        Some(server),
        tree_name,
        &title,
        data["explore"].clone(),
        extras,
        writer,
    )
}

/// The head `tree_name`'s branch had at `time` (see
/// `commit_index::parse_time`), roughly: its main line's last commit which had
/// landed by then (see `CommitIndex::rev_at`).
pub fn rev_at(
    cfg: &Config,
    tree_name: &str,
    time: &str,
) -> Result<Option<crate::commit_index::RevAt>, &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let git = tree_config.get_git()?;
    let branch = tree_config.paths.git_branch.as_deref().unwrap_or("HEAD");
    let index = git
        .history
        .as_ref()
        .and_then(|history| {
            CommitIndex::open_branch(&Path::new(&history.path).join("commit-index"), branch)
        })
        .ok_or("This tree has no commit index")?;
    let time = crate::commit_index::parse_time(time)
        .ok_or("Not a time (ex: 2024-08-30, 2024-08-30 07:26, or 2024-08-30T07:26:47Z)")?;
    Ok(index.rev_at(&time))
}

/// The commits of an explore page (see `explore_page_data`): its keys (with
/// Phabricator revisions as "DNNN"), the commits mentioning them in the tree's
/// commit index, oldest first, up to `explore::MAX_COMMITS`, and the keys no
/// commit mentions.
struct ExploreRefs {
    index: CommitIndex,
    keys: Vec<String>,
    refs: Vec<CommitRef>,
    /// The revisions of all of the commits, including those past the limit.
    own_revs: HashSet<String>,
    missing: Vec<String>,
    /// Whether there were more commits than `refs` has.
    truncated: bool,
}

fn explore_refs(
    cfg: &Config,
    tree_name: &str,
    kind: &str,
    keys: &str,
) -> Result<ExploreRefs, &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let git = tree_config.get_git()?;
    let history = git.history.as_ref();
    let branch = tree_config.paths.git_branch.as_deref().unwrap_or("HEAD");
    let index = history
        .and_then(|history| {
            CommitIndex::open_branch(&Path::new(&history.path).join("commit-index"), branch)
        })
        .ok_or("This tree has no commit index")?;

    let keys: Vec<String> = keys
        .split(',')
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .take(MAX_EXPLORE_KEYS)
        .map(|key| match kind {
            // Accept "D123" or "123" for Phabricator revisions.
            "phab" => format!("D{}", key.trim_start_matches(['D', 'd'])),
            _ => key.to_string(),
        })
        .collect();
    let mut refs: Vec<CommitRef> = vec![];
    let mut missing = vec![];
    for key in &keys {
        let found = match kind {
            "bug" => index.bug_commits(key),
            "phab" => index.phab_commits(key),
            _ => return Err("Unknown kind of exploration"),
        };
        if found.is_empty() {
            missing.push(key.clone());
        }
        refs.extend(found);
    }
    let own_revs: HashSet<String> = refs.iter().map(|r| r.rev.clone()).collect();
    explore::order_commits(&git.repo, &mut refs, Some(&index));
    let truncated = refs.len() > explore::MAX_COMMITS;
    refs.truncate(explore::MAX_COMMITS);
    Ok(ExploreRefs {
        index,
        keys,
        refs,
        own_revs,
        missing,
        truncated,
    })
}

/// The most commits a history page (see `history_data`) shows.
const MAX_HISTORY_COMMITS: usize = 100;

/// The most commits a history page looks at for changes to its scope (ex: of a
/// method a busy file's weeks of commits rarely changed; see `history_data`).
const MAX_HISTORY_CANDIDATES: usize = 1000;

/// The most moves to other files a history page follows its scope back
/// through (see `history_data`).
const MAX_HISTORY_MOVES: usize = 8;

/// Whether a history context (see `syntax_files`) is `scope` or nested in it,
/// "" being the whole file.
fn in_scope(context: &str, scope: &str) -> bool {
    scope.is_empty()
        || context == scope
        || context
            .strip_prefix(scope)
            .is_some_and(|rest| rest.starts_with("::"))
}

/// The history of a scope of the file at `path` (`/{tree}/history/{rev}/{path}`),
/// as of `rev`: the file, or one of its classes, methods or other contexts of
/// the history's (`?scope=Foo::Bar`, or `?token=N`, the context of its Nth
/// token), as the explore page (see `render_explore`) of the newest commits
/// which changed it (up to `MAX_HISTORY_COMMITS`), oldest first, with the
/// scope's changes, and links to its broader scopes (ex: the class of a
/// method).
///
/// The commits come from the file's files-delta journal (whose records follow
/// it through renames), newest first: its detail records' revisions, and the
/// revisions of its weekly summaries (see `consolidation`) which changed the
/// scope (whose own changes the summaries don't keep), which their
/// rev-summaries then say (see `explore::commit_changes`), and which also say
/// the file's earlier paths (see `explore::commit_moves`).  If the scope moved
/// to the file from another (see `explore::commit_symbol_moves`), the commits
/// before the move are the other file's which changed it (by its name there),
/// from the other file's journal as of the move's parent, and so on.
/// The data of the history page of the scope `scope` (or else the context of
/// the token `token`, or else the whole file) of the file at `path` as of the
/// revision `rev` (see `render_history`): its `title`, `note` and `scopes`,
/// and its explore page's data (`explore`; see `explore_data`).  (For trees'
/// servers; see `AbstractServer::history`.)
pub fn history_data(
    cfg: &Config,
    tree_name: &str,
    rev: &str,
    path: &str,
    scope: Option<&str>,
    token: Option<usize>,
) -> Result<serde_json::Value, &'static str> {
    use crate::file_format::history::syntax_files::{split_token_line, token_file_lines};
    use crate::file_format::history::timeline_common::JournalVersionRef;
    use crate::file_format::history::timeline_files_delta::FileDeltaRecord;
    use crate::hyperblame::journals::{JournalKind, JournalReader};

    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let git = tree_config.get_git()?;
    let history = git.history.as_ref().ok_or("This tree has no history")?;
    let commit = git
        .repo
        .revparse_single(rev)
        .and_then(|object| object.peel_to_commit())
        .map_err(|_| "Bad revision")?;
    let timeline_commit = history
        .timeline_commit(commit.id())
        .ok_or("The history doesn't have the revision")?;

    let scope = match (scope, token) {
        (Some(scope), _) => scope.to_string(),
        (None, Some(token)) => {
            let file = history
                .file_history(&timeline_commit, path)
                .ok()
                .flatten()
                .ok_or("The history doesn't have the file")?;
            let lines = token_file_lines(&file.syntax);
            let line = token
                .checked_sub(1)
                .and_then(|index| lines.get(index))
                .ok_or("No such token")?;
            split_token_line(line).context.to_string()
        }
        (None, None) => String::new(),
    };
    let scope = if scope == "%" { String::new() } else { scope };

    // The scope's history in the file, and if it moved there from another
    // file (see `explore::commit_symbol_moves`), in that one before then, as
    // it was named there, and so on: as segments of a file and a scope in it,
    // as of a timeline commit (the move's parent's, for the earlier files).
    let mut segment = Some((path.to_string(), scope.clone(), timeline_commit.id()));
    // The moves, newest first: (commit, earlier file, earlier name, name).
    let mut moves: Vec<(String, String, String, String)> = vec![];
    let mut kept: Vec<(CommitRef, explore::CommitChanges)> = vec![];
    let mut seen = HashSet::new();
    let mut more = false;
    let mut searched_all = true;
    'segments: while let Some((segment_path, segment_scope, timeline)) = segment.take() {
        let records: Vec<FileDeltaRecord> =
            match JournalReader::new(&history.timeline).records(&JournalVersionRef {
                timeline_rev: timeline.to_string(),
                path: JournalKind::FilesDelta.journal_path(&segment_path),
            }) {
                Ok(records) => records,
                Err(_) if !moves.is_empty() => break,
                Err(_) => return Err("The history's journal of the file is broken"),
            };
        // The revisions which may have changed the scope, newest first, and
        // those which may have moved it into the file (whose tokens moved,
        // rather than changed).
        let mut candidates: Vec<&str> = vec![];
        let mut move_candidates: HashSet<&str> = HashSet::new();
        for record in &records {
            let (revs, group) = match record {
                FileDeltaRecord::Detail(detail) => (
                    vec![detail.desc.source_rev.as_str()],
                    &detail.delta.symbol_group,
                ),
                FileDeltaRecord::Summary(summary) => (
                    summary
                        .desc
                        .source_revs
                        .iter()
                        .map(String::as_str)
                        .collect(),
                    &summary.symbol_group,
                ),
            };
            let touched = group.symbol_deltas.iter().any(|(context, delta)| {
                let totals = &delta.token_totals;
                in_scope(context, &segment_scope) && totals.added + totals.removed > 0
            });
            let moved = !segment_scope.is_empty()
                && group
                    .symbol_deltas
                    .get(&segment_scope)
                    .is_some_and(|delta| delta.evolved_from_path.is_some());
            if moved {
                move_candidates.extend(revs.iter().copied());
            }
            if touched || moved {
                candidates.extend(revs.into_iter().filter(|rev| seen.insert(rev.to_string())));
            }
        }

        // Their changes to the scope, in the file's paths at the time.
        let mut paths: HashSet<String> = HashSet::from([segment_path.clone()]);
        searched_all &= candidates.len() <= MAX_HISTORY_CANDIDATES;
        candidates.truncate(MAX_HISTORY_CANDIDATES);
        for candidate in candidates {
            let mut scoped: BTreeMap<String, u32> = BTreeMap::new();
            for (changed, symbols) in commit_changes(Some(history), &git.repo, candidate) {
                if !paths.contains(&changed) {
                    continue;
                }
                for (context, tokens) in symbols {
                    if tokens > 0 && in_scope(&context, &segment_scope) {
                        *scoped.entry(context).or_default() += tokens;
                    }
                }
            }
            for (to, from) in explore::commit_moves(history, candidate) {
                if paths.contains(&to) {
                    paths.insert(from);
                }
            }
            // (The commit which moved the scope into the file, after which
            // its history goes on in the file it was in.)
            let moved = match move_candidates.contains(candidate) {
                true => explore::commit_symbol_moves(history, candidate)
                    .into_iter()
                    .find(|m| m.to == segment_scope && paths.contains(&m.to_path)),
                false => None,
            };
            if let Some(m) = &moved {
                let tokens = scoped.entry(segment_scope.clone()).or_default();
                *tokens = (*tokens).max(m.tokens.max(1));
            }
            if scoped.is_empty() {
                continue;
            }
            if kept.len() == MAX_HISTORY_COMMITS {
                more = true;
                break 'segments;
            }
            let Some(found) = Oid::from_str(candidate)
                .ok()
                .and_then(|oid| git.repo.find_commit(oid).ok())
            else {
                continue;
            };
            kept.push((
                crate::commit_index::commit_ref(&found),
                BTreeMap::from([(segment_path.clone(), scoped)]),
            ));
            if let Some(m) = moved {
                let parent_timeline = found
                    .parent_id(0)
                    .ok()
                    .and_then(|parent| history.timeline_commit(parent))
                    .map(|commit| commit.id());
                moves.push((
                    candidate.to_string(),
                    m.from_path.clone(),
                    m.from.clone(),
                    m.to.clone(),
                ));
                if let Some(parent_timeline) = parent_timeline
                    && moves.len() <= MAX_HISTORY_MOVES
                {
                    segment = Some((m.from_path, m.from, parent_timeline));
                }
                break;
            }
        }
    }
    kept.reverse();
    let (refs, changes): (Vec<CommitRef>, Vec<explore::CommitChanges>) = kept.into_iter().unzip();

    // The scope and the broader ones (the file, then each enclosing context).
    let url = |scope: Option<&str>| {
        let base = format!(
            "/{}/history/{}/{}",
            tree_name,
            commit.id(),
            url_encode_path(path)
        );
        match scope {
            Some(scope) => format!("{}?scope={}", base, urlencoding::encode(scope)),
            None => base,
        }
    };
    let mut scopes = vec![json!({"label": path, "url": url(None)})];
    if !scope.is_empty() {
        let segments: Vec<&str> = scope.split("::").collect();
        for n in 1..=segments.len() {
            scopes.push(json!({
                "label": unescape_name(segments[n - 1]),
                "url": url(Some(&segments[..n].join("::"))),
            }));
        }
    }
    if let Some(last) = scopes.last_mut() {
        last["url"] = serde_json::Value::Null;
    }
    let title = if scope.is_empty() {
        format!("History of {}", path)
    } else {
        format!("History of {}", unescape_name(&scope))
    };
    let at = &commit.id().to_string()[..12];
    let note = if more {
        format!(
            "The newest {} commits which changed it, as of {}, oldest first.",
            MAX_HISTORY_COMMITS, at
        )
    } else if !searched_all {
        format!(
            "The commits which changed it among the newest {} commits which changed the \
             file, as of {}, oldest first.",
            MAX_HISTORY_CANDIDATES, at
        )
    } else {
        format!("The commits which changed it, as of {}, oldest first.", at)
    };
    // (And where it was before it moved to its file.)
    let note = moves.iter().fold(note, |note, (rev, from_path, from, to)| {
        format!(
            "{}  {} moved from {}{} in {}, and the commits before that are its changes there.",
            note,
            to,
            from_path,
            match from == to {
                true => String::new(),
                false => format!(" (as {})", from),
            },
            &rev[..12.min(rev.len())]
        )
    });
    let explore = explore_data(cfg, tree_name, &refs, Some(changes), None)?;
    Ok(json!({
        "title": title,
        "note": note,
        "scopes": scopes,
        "explore": explore,
    }))
}

/// A history page (`/{tree}/history/{rev}/{path}`), with its `data` (see
/// `history_data`).  (Its files show their symbols' sparklines, of the
/// scope's changes, rather than their outline diffs, which have all of the
/// files' changes across their commits.)
pub fn render_history(
    tree_name: &str,
    data: serde_json::Value,
    writer: &mut dyn Write,
) -> Result<(), &'static str> {
    let extras = ExploreExtras {
        missing: vec![],
        truncated: false,
        other_branches: vec![],
        explore_url: String::new(),
        note: data["note"].as_str().map(str::to_string),
        scopes: data["scopes"].as_array().cloned().unwrap_or_default(),
        semantic_head: None,
        semantic_failure: None,
        landed: vec![],
        pull_request: None,
    };
    let title = data["title"].as_str().unwrap_or_default().to_string();
    render_explore(
        None,
        tree_name,
        &title,
        data["explore"].clone(),
        extras,
        writer,
    )
}

/// The patch stack strip (see `hyperblame::stack_strip`) of the file at `path`
/// in `rev` for the stack of commits of an explore page or a try push's page,
/// `stack` being the page's URL in the tree (ex: "explore/bug/123" or
/// "try/hg/abcdef012345", which its files' links pass to their views), as JSON
/// for the views' script (`/{tree}/{stack}/stack-strip/{rev}/{path}`), with
/// the stack's `base`, the outline's default baseline in its context (see
/// outline.js): where it left the trunk, its first commit's parent, or for a
/// try push's stack rebased onto the indexed revision (its delta's head; see
/// `try_delta`), that revision.
pub fn stack_strip_json(
    cfg: &Config,
    tree_name: &str,
    stack: &str,
    rev: &str,
    path: &str,
) -> Result<String, &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let git = tree_config.get_git()?;
    let history = git.history.as_ref().ok_or("This tree has no history")?;
    let commits: Vec<Oid> = match stack.split('/').collect::<Vec<_>>().as_slice() {
        ["try", _, _] => {
            let push = crate::try_import::stack_push(stack).ok_or("Not a try push")?;
            crate::try_import::push_commits(cfg, tree_name, &push).0
        }
        ["explore", kind, keys] => explore_refs(cfg, tree_name, kind, keys)?
            .refs
            .iter()
            .filter_map(|commit_ref| Oid::from_str(&commit_ref.rev).ok())
            .collect(),
        _ => return Err("Unknown stack"),
    };
    let commit = git
        .repo
        .revparse_single(rev)
        .and_then(|object| object.peel_to_commit())
        .map_err(|_| "Bad revision")?;
    let source = read_commit_file(&git.repo, &commit, path).ok_or("No such file")?;
    // (On a try push's rebased stack's pages, its rebased commits stand for
    // its commits, whose tokens the history blames on them.)
    let commits = rebased_commits(&git.repo, commit.id(), commits);
    let strip = crate::hyperblame::stack_strip::stack_strip(
        history,
        &git.repo,
        &commits,
        commit.id(),
        path,
        &source,
    )
    .map_err(|e| {
        log::warn!("No stack strip for {} in {}: {}", path, rev, e);
        "No stack strip for the file"
    })?;
    let rebased_base = std::fs::read_to_string(
        crate::try_delta::delta_dir(&tree_config.paths.index_path, &commit.id().to_string())
            .join("meta.json"),
    )
    .ok()
    .and_then(|meta| serde_json::from_str::<crate::try_delta::TryDeltaMeta>(&meta).ok())
    .map(|meta| meta.base);
    let base = rebased_base.or_else(|| {
        let first = git.repo.find_commit(*commits.first()?).ok()?;
        Some(first.parent_id(0).ok()?.to_string())
    });
    let mut value = serde_json::to_value(&strip).map_err(|_| "Stack strip problems")?;
    value["base"] = json!(base);
    serde_json::to_string(&value).map_err(|_| "Stack strip problems")
}

/// Whether the history has the revision `rev` (ex: a try push's rebased
/// head; see import-try-rebased.sh).
fn history_has(history: &crate::hyperblame::token_blame::TreeHistory, rev: &str) -> bool {
    Oid::from_str(rev).is_ok_and(|oid| history.timeline_commit(oid).is_some())
}

/// The commits of a try push's stack (`commits`) as the commits of its stack
/// rebased onto the indexed revision (see rebase-try-push.sh) which `rev` is
/// or descends from, if it's one of them: their notes
/// (`refs/notes/try-rebased`) name the commits they're of.  (Commits which
/// landed before the rebase have no rebased commits, and stay.)
fn rebased_commits(repo: &git2::Repository, rev: Oid, commits: Vec<Oid>) -> Vec<Oid> {
    let mut rebased: HashMap<Oid, Oid> = HashMap::new();
    let mut current = repo.find_commit(rev).ok();
    while let Some(commit) = current {
        let original = repo
            .find_note(Some("refs/notes/try-rebased"), commit.id())
            .ok()
            .and_then(|note| Oid::from_str(note.message().ok()?.trim()).ok());
        let Some(original) = original else {
            break;
        };
        rebased.insert(original, commit.id());
        current = commit.parent(0).ok();
    }
    commits
        .into_iter()
        .map(|oid| rebased.get(&oid).copied().unwrap_or(oid))
        .collect()
}

struct ExploreExtras {
    /// The keys no commit mentions.
    missing: Vec<String>,
    /// Whether there were more commits than it shows.
    truncated: bool,
    /// The keys' commits on other branches (see `explore_page_data`).
    other_branches: Vec<serde_json::Value>,
    /// Its URL in the tree, for the other branches' trees' explore pages.
    explore_url: String,
    /// A note under its title (ex: a try push's).
    note: Option<String>,
    /// For a history page (see `history_data`), the scopes it can show,
    /// broadest first, as `{label, url}` (the shown one without a URL).
    scopes: Vec<serde_json::Value>,
    /// For a try push with a semantic delta (see `try_delta`), the head of
    /// its stack rebased onto the indexed revision, whose pages have the
    /// push's analysis, and whose searches (and the page's) are of the push.
    semantic_head: Option<String>,
    /// For a try push without one, why, if its import's analysis failed.
    semantic_failure: Option<crate::try_import::SemanticFailure>,
    /// For a try push, its commits which have landed (see
    /// `try_import::LandedCommit`), as `{rev, summary, landed, landed_summary,
    /// same_changes}`.
    landed: Vec<serde_json::Value>,
    /// For a spec's pull request (see `try_import::is_pull_request`), its page
    /// upstream (ex: on GitHub), if any, and its page here, which imports its
    /// newer commits: `{link, path}`.
    pull_request: Option<serde_json::Value>,
}

/// The data of an explore page (see `render_explore`) of the commits `refs`,
/// oldest first, as explore.liquid takes it: its `commits`, and its files'
/// `facets` and `file_groups`, from what the commits changed (`changes`, if
/// not all of it), with the files' links with the stack strip of the page at
/// `stack_url`, if any (see `stack_strip_json`).  (For trees' servers' explore
/// pages; see `AbstractServer::explore`.)
pub fn explore_data(
    cfg: &Config,
    tree_name: &str,
    refs: &[CommitRef],
    changes: Option<Vec<explore::CommitChanges>>,
    stack_url: Option<&str>,
) -> Result<serde_json::Value, &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let git = tree_config.get_git()?;
    let history = git.history.as_ref();
    let mut commits: Vec<ExploreCommit> = refs
        .iter()
        .enumerate()
        .map(|(i, commit_ref)| explore_commit(tree_config, git, i + 1, commit_ref))
        .collect();
    let changes: Vec<_> = match changes {
        Some(changes) => changes,
        None => refs
            .iter()
            .map(|commit_ref| commit_changes(history, &git.repo, &commit_ref.rev))
            .collect(),
    };
    for (backout, backed_out, reland) in explore::relands(history, refs, &changes) {
        let revs = |indices: &[usize]| indices.iter().map(|&i| refs[i].rev.as_str()).join(",");
        commits[backout].interdiff = Some(format!(
            "/{}/interdiff/{}/{}",
            tree_name,
            revs(&backed_out),
            revs(&reland)
        ));
    }
    // (A try push's files are its stack rebased onto the indexed revision's,
    // whose head's pages have the push's semantic analysis, its blame and its
    // stack strip (see `try_delta`, `FileExtras::stack`), if the history has
    // it (see import-try-rebased.sh; not, ex, a spec's pull request's): its
    // files link to the head's files, if it has them, and their outline diffs
    // are from the indexed revision to the head.)
    let rebased = stack_url
        .and_then(crate::try_import::stack_push)
        .and_then(|push| crate::try_import::semantic_head(cfg, tree_name, &push))
        .filter(|head| history.is_some_and(|history| history_has(history, head)))
        .and_then(|head| {
            let tree = git
                .repo
                .find_commit(Oid::from_str(&head).ok()?)
                .ok()?
                .tree()
                .ok()?;
            let base = crate::try_delta::delta_meta(&tree_config.paths.index_path, &head)?.base;
            let base_tree = git
                .repo
                .find_commit(Oid::from_str(&base).ok()?)
                .ok()?
                .tree()
                .ok()?;
            Some((head, tree, base, base_tree))
        });
    if let Some((head, _, _, _)) = &rebased
        && let Ok(head) = Oid::from_str(head)
    {
        let originals: Vec<Oid> = refs
            .iter()
            .filter_map(|r| Oid::from_str(&r.rev).ok())
            .collect();
        let rebased_revs = rebased_commits(&git.repo, head, originals.clone());
        for (original, rebased_rev) in originals.iter().zip(rebased_revs) {
            if *original != rebased_rev
                && let Some(commit) = commits.iter_mut().find(|c| c.rev == original.to_string())
            {
                commit.rebased = Some(rebased_rev.to_string());
            }
        }
    }
    // (Files link to their views with the page's stack strip; see
    // `stack_strip_json`.)
    let mut files = blot_files(
        refs,
        &changes,
        |path, rev| {
            let rev = match &rebased {
                Some((head, tree, _, _)) if tree.get_path(Path::new(path)).is_ok() => head.as_str(),
                _ => rev,
            };
            let url = format!("/{}/rev/{}/{}", tree_name, rev, url_encode_path(path));
            match stack_url {
                Some(stack_url) => format!("{}?stack={}", url, stack_url),
                None => url,
            }
        },
        |_, _| false,
    );
    // (The symbols the commits moved to other files, on both files' rows.)
    if let Some(history) = history {
        let moves: Vec<(usize, explore::SymbolMove)> = refs
            .iter()
            .enumerate()
            .flat_map(|(i, commit_ref)| {
                explore::commit_symbol_moves(history, &commit_ref.rev)
                    .into_iter()
                    .map(move |m| (i, m))
            })
            .collect();
        explore::annotate_moves(&mut files, &moves, |path, commit, before| {
            let rev = &refs[commit].rev;
            let rev = match before {
                true => Oid::from_str(rev)
                    .ok()
                    .and_then(|oid| git.repo.find_commit(oid).ok()?.parent_id(0).ok())
                    .map_or_else(|| rev.clone(), |parent| parent.to_string()),
                false => rev.clone(),
            };
            format!("/{}/rev/{}/{}", tree_name, rev, url_encode_path(path))
        });
    }
    // (In trees with history, each file's outline diff from before the first
    // of the commits changing it to after the last.)
    if history.is_some() {
        for file in &mut files {
            let mut changed = (0..changes.len()).filter(|&i| changes[i].contains_key(&file.path));
            let (Some(first), last) = (changed.next(), changed.last()) else {
                continue;
            };
            let parent = Oid::from_str(&refs[first].rev)
                .ok()
                .and_then(|oid| git.repo.find_commit(oid).ok()?.parent_id(0).ok());
            // (Not if the rebased stack doesn't change the file, ex: if the
            // commits changing it landed.)
            let entry =
                |tree: &git2::Tree| tree.get_path(Path::new(&file.path)).ok().map(|e| e.id());
            let range = match &rebased {
                Some((head, tree, base, base_tree))
                    if entry(tree).is_some() && entry(tree) != entry(base_tree) =>
                {
                    Some((base.clone(), head.clone()))
                }
                _ => parent
                    .map(|parent| (parent.to_string(), refs[last.unwrap_or(first)].rev.clone())),
            };
            if let Some((from, to)) = range {
                file.outline = format!(
                    "/{}/outline-diff/{}/{}/{}",
                    tree_name,
                    from,
                    to,
                    url_encode_path(&file.path)
                );
            }
        }
    }
    let (facets, file_groups) = explore_facets(cfg, tree_name, files);
    Ok(json!({
        "commits": commits,
        "facets": facets,
        "file_groups": file_groups,
    }))
}

/// The names of the symbols (history contexts) of explore data's files
/// (`ExploreFile`s, in `file_groups`' `files`, or a list of them) as they're
/// shown: without their escapes (see `config_tokenizer::unescape_name`).
fn unescape_explore_names(value: &mut serde_json::Value) {
    fn unescape_symbols(symbols: &mut serde_json::Value) {
        for symbol in symbols.as_array_mut().into_iter().flatten() {
            for key in ["name", "pretty"] {
                if let Some(name) = symbol[key].as_str() {
                    symbol[key] = unescape_name(name).into();
                }
            }
            for change in symbol["moves"].as_array_mut().into_iter().flatten() {
                if let Some(title) = change["title"].as_str() {
                    change["title"] = unescape_name(title).into();
                }
            }
            unescape_symbols(&mut symbol["children"]);
        }
    }
    for item in value.as_array_mut().into_iter().flatten() {
        match item.get_mut("files") {
            Some(files) => unescape_explore_names(files),
            None => unescape_symbols(&mut item["symbols"]),
        }
    }
}

/// Explore data's files (see `unescape_explore_names`) in the order they're
/// shown (see `explore::file_order_key`; ex: headers first), without outline
/// diffs yet (see `explore_outlines`).
fn order_explore_files(file_groups: &mut serde_json::Value) {
    for group in file_groups.as_array_mut().into_iter().flatten() {
        if let Some(files) = group["files"].as_array_mut() {
            files.sort_by(|a, b| {
                let path =
                    |file: &serde_json::Value| file["path"].as_str().unwrap_or("").to_string();
                let (a, b) = (path(a), path(b));
                explore::file_order_key(&a).cmp(&explore::file_order_key(&b))
            });
            for file in files {
                file["outline_rows"] = serde_json::Value::Null;
                file["change_size"] = serde_json::Value::Null;
                file["excerpt_url"] = serde_json::Value::Null;
            }
        }
    }
}

/// The most files of an explore page whose outline diffs it shows (see
/// `explore_outlines`); the others show their symbols.
const MAX_EXPLORE_OUTLINES: usize = 200;

/// How many outline diffs an explore page asks its tree's server for at once.
const EXPLORE_OUTLINE_CONCURRENCY: usize = 8;

/// Under each of explore data's files (in `file_groups`; see `explore_data`)
/// with an outline diff (`outline`, its page's URL), the diff's rows (see the
/// function `outline_rows`) of the definitions which changed and of the ones
/// they're in (`outline_rows`), after a row of the changes outside of them,
/// if any, with their commits linking to the page's (`commits`), and how
/// many tokens the diff added and removed (`change_size`, for facets.js's
/// sorting by it), and the URL of the data of its rows' excerpts, if the page
/// is a stack's (`stack`, its URL in the tree, ex: "try/hg/abcdef012345"; see
/// `outline_excerpts_json`).  The diffs are `server`'s.
fn explore_outlines(
    server: &(dyn AbstractServer + Send + Sync),
    tree_name: &str,
    stack: &str,
    commits: &serde_json::Value,
    file_groups: &mut serde_json::Value,
) {
    // (Their revisions, from their pages' URLs:
    // `/{tree}/outline-diff/{from}/{to}/{path}`.)
    let mut requests = vec![];
    for (g, group) in file_groups.as_array().into_iter().flatten().enumerate() {
        for (f, file) in group["files"].as_array().into_iter().flatten().enumerate() {
            let outline = file["outline"].as_str().unwrap_or("");
            let segments: Vec<&str> = outline.splitn(6, '/').collect();
            if let (["", _, "outline-diff", from, to, _], Some(path)) =
                (segments.as_slice(), file["path"].as_str())
            {
                requests.push(((g, f), from.to_string(), to.to_string(), path.to_string()));
            }
        }
    }
    requests.truncate(MAX_EXPLORE_OUTLINES);
    let server_ = server.clonify();
    let diffs = run_on_runtime(async move {
        use futures_util::stream::{self, StreamExt as FuturesStreamExt};
        // (Each on a task of its own, as a local server's are synchronous.)
        let fetches = FuturesStreamExt::map(stream::iter(requests), |(at, from, to, path)| {
            let server = server_.clonify();
            async move {
                let diff = tokio::spawn(async move {
                    let diff = server.outline_diff(&from, &to, &path, false).await;
                    (path, diff)
                })
                .await;
                (at, diff)
            }
        });
        FuturesStreamExt::collect::<Vec<_>>(fetches.buffer_unordered(EXPLORE_OUTLINE_CONCURRENCY))
            .await
    });

    // (By their revisions, and their rebased commits', which the rebased
    // head's outline diffs name.)
    let numbers: HashMap<&str, u64> = commits
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|commit| {
            let number = commit["number"].as_u64();
            [commit["rev"].as_str(), commit["rebased"].as_str()]
                .into_iter()
                .filter_map(move |rev| Some((rev?, number?)))
        })
        .collect();
    for ((g, f), fetched) in diffs {
        let (path, diff) = match fetched {
            Ok((path, Ok(value))) => {
                match serde_json::from_value::<outline_diff::OutlineDiff>(value) {
                    Ok(diff) => (path, diff),
                    Err(err) => {
                        log::warn!("Bad outline diff of {}: {}", path, err);
                        continue;
                    }
                }
            }
            Ok((path, Err(err))) => {
                log::warn!("No outline diff of {}: {:?}", path, err);
                continue;
            }
            Err(_) => continue,
        };
        // (Commits other than the page's, ex: between its commits, link to
        // theirs.)
        let (mut rows, _) = outline_rows(tree_name, &path, &diff, |_, commit| {
            match numbers.get(commit.rev.as_str()) {
                Some(n) => (format!("#commit-{}", n), n.to_string()),
                None => (
                    format!("/{}/commit/{}", tree_name, commit.rev),
                    commit.rev[..8.min(commit.rev.len())].to_string(),
                ),
            }
        });
        for row in &mut rows {
            // (Oldest first, as the page's commits are.)
            if let Some(commits) = row["commits"].as_array_mut() {
                commits.reverse();
            }
        }
        let changed = |tokens: &outline_diff::OutlineTokens| tokens.added + tokens.removed;
        let mut shown = vec![];
        if changed(&diff.toplevel) > 0 {
            shown.push(json!({
                "status": "changed",
                "depth": 0,
                "context": false,
                "title": "The changes outside of any definition",
                "name": explore::TOP_LEVEL,
                "range": EXCERPT_TOP_LEVEL,
                "link": null,
                "note": null,
                "badge": outline_delta(&diff.toplevel),
                "commits": [],
            }));
        }
        shown.extend(
            rows.into_iter()
                .filter(|row| row["status"] != "unchanged" || row["context"] == true),
        );
        let size: u32 = changed(&diff.toplevel)
            + diff
                .entries
                .iter()
                .map(|entry| changed(&entry.tokens))
                .sum::<u32>();
        // (Files whose outline diffs have no changes, ex: as the history
        // doesn't have their syntax, show their symbols.)
        let file = &mut file_groups[g]["files"][f];
        if !shown.is_empty() {
            file["outline_rows"] = json!(shown);
            if !stack.is_empty() {
                file["excerpt_url"] = json!(format!(
                    "/{}/{}/excerpts/{}/{}/{}",
                    tree_name,
                    stack,
                    diff.from.rev,
                    diff.to.rev,
                    url_encode_path(&path)
                ));
            }
        }
        file["change_size"] = json!(size);
    }
}

/// The key of an outline diff's entry's excerpt in its excerpts' data (see
/// `outline_excerpts_json`), by its lines in the "to" file, or for removed
/// ones, in the "from" file: "12-40" or "from:12-40".
fn excerpt_range_key((first, last): (u32, u32), from: bool) -> String {
    format!("{}{}-{}", if from { "from:" } else { "" }, first, last)
}

/// The key of the excerpt of the changes outside of any definition (see
/// `excerpt_range_key`).
const EXCERPT_TOP_LEVEL: &str = "top";

/// The most lines of a definition an excerpt's data has all of (see
/// `outline_excerpts_json`); bigger ones' have the lines around their changes
/// (`EXCERPT_WINDOW`) and their structure's.
const EXCERPT_WHOLE_LINES: u32 = 300;

/// How many lines before and after a change an excerpt's data has, for
/// definitions bigger than `EXCERPT_WHOLE_LINES` and outside of definitions.
const EXCERPT_WINDOW: u32 = 12;

/// The most lines of a control flow or definition's header that count as its
/// structure (ex: a long condition's), from its first line (see
/// `excerpt_structure`).
const EXCERPT_MAX_HEADER_LINES: usize = 4;

/// How much the structure of each level out of the control flow enclosing a
/// change matters, relatively (see `excerpt_structure`).
const EXCERPT_STRUCTURE_DECAY: f32 = 0.8;

/// How much a syntax node's header matters as the structure around changes
/// in it (see `excerpt_structure`), if it does, and whether it's control
/// flow (ex: an `if` or its `else`, a loop, a `case`), whose nesting decays
/// its outer levels', rather than a definition (ex: a function's signature).
/// (The kinds of the grammars of `cst_tokenizer::parse_source_for_analysis`.)
fn excerpt_node_weight(kind: &str) -> Option<(f32, bool)> {
    match kind {
        "if_statement"
        | "if_expression"
        | "if_let_expression"
        | "else_clause"
        | "elif_clause"
        | "for_statement"
        | "for_range_loop"
        | "for_in_statement"
        | "enhanced_for_statement"
        | "for_expression"
        | "while_statement"
        | "while_expression"
        | "do_statement"
        | "do_while_statement"
        | "loop_expression"
        | "switch_statement"
        | "switch_expression"
        | "case_statement"
        | "switch_case"
        | "switch_default"
        | "switch_block_statement_group"
        | "match_expression"
        | "match_arm"
        | "when_expression"
        | "when_entry"
        | "try_statement"
        | "try_expression"
        | "catch_clause"
        | "except_clause"
        | "finally_clause"
        | "with_statement" => Some((1.0, true)),
        "function_definition"
        | "function_declaration"
        | "function_item"
        | "method_definition"
        | "method_declaration"
        | "constructor_declaration"
        | "function_expression"
        | "arrow_function"
        | "lambda_expression"
        | "template_declaration" => Some((0.9, false)),
        "class_specifier" | "struct_specifier" | "union_specifier" | "enum_specifier"
        | "class_declaration" | "class_definition" | "class" | "impl_item" | "trait_item"
        | "object_declaration" => Some((0.6, false)),
        _ => None,
    }
}

/// The kinds of the bodies whose starts end their parents' headers (see
/// `excerpt_structure`).
fn excerpt_is_body(kind: &str) -> bool {
    matches!(
        kind,
        "compound_statement"
            | "block"
            | "statement_block"
            | "declaration_list"
            | "field_declaration_list"
            | "enumerator_list"
            | "class_body"
            | "enum_body"
            | "function_body"
            | "control_structure_body"
            | "switch_body"
            | "switch_block"
            | "match_block"
    )
}

/// How much each line of `source` (1-based) matters as the structure around
/// its lines `changed`, if `path`'s language has a grammar for code (see
/// `cst_tokenizer::parse_source_for_analysis`): the headers (their lines up
/// to where their bodies start) of the control flow enclosing each changed
/// line, most for the innermost (ex: the condition of the `if` it's in, and
/// the `else` of the branch it's in), and of the definitions enclosing them
/// (ex: a method's signature, its class's).  (Explore pages' excerpts bias
/// what they show toward these; see `outline_excerpts_json`.)
fn excerpt_structure(
    parse: &crate::tree_sitter_support::cst_tokenizer::SourceParse,
    changed: &BTreeSet<u32>,
) -> BTreeMap<u32, f32> {
    let mut scores: BTreeMap<u32, f32> = BTreeMap::new();
    let mut seen = HashSet::new();
    for &line in changed {
        let Some(node) = excerpt_line_node(parse, line) else {
            continue;
        };
        let mut level = 0;
        let mut node = Some(node);
        while let Some(current) = node {
            if let Some((weight, control)) = excerpt_node_weight(current.kind())
                && seen.insert((current.id(), level))
            {
                let first = current.start_position().row;
                let mut cursor = current.walk();
                let body = current
                    .named_children(&mut cursor)
                    .find(|child| excerpt_is_body(child.kind()))
                    .map_or(first, |body| body.start_position().row);
                let score = weight * EXCERPT_STRUCTURE_DECAY.powi(level);
                for header in first..=body.min(first + EXCERPT_MAX_HEADER_LINES - 1) {
                    let entry = scores.entry(header as u32 + 1).or_default();
                    *entry = entry.max(score);
                }
                if control {
                    level += 1;
                }
            }
            node = current.parent();
        }
    }
    scores
}

/// The smallest syntax node of `parse` spanning the line `line` (1-based)
/// from its first non-blank character.
fn excerpt_line_node(
    parse: &crate::tree_sitter_support::cst_tokenizer::SourceParse,
    line: u32,
) -> Option<tree_sitter::Node<'_>> {
    let text = parse.text.lines().nth(line.checked_sub(1)? as usize)?;
    let start = text.len() - text.trim_start().len();
    let row = line as usize - 1;
    parse.tree.root_node().descendant_for_point_range(
        tree_sitter::Point::new(row, start),
        tree_sitter::Point::new(row, text.len().max(start)),
    )
}

/// The most lines of a statement whose lines explore pages' excerpts show
/// together (see `excerpt_statements`).
const EXCERPT_MAX_STATEMENT_LINES: usize = 8;

/// For each of the lines `lines` (1-based) of `parse` which continues a
/// statement (or a declaration) started on an earlier line, of at most
/// `EXCERPT_MAX_STATEMENT_LINES` lines, the statement's first line, so that
/// explore pages' excerpts show its lines together rather than a fragment of
/// it (ex: a call's last argument's line).
fn excerpt_statements(
    parse: &crate::tree_sitter_support::cst_tokenizer::SourceParse,
    lines: &BTreeSet<u32>,
) -> BTreeMap<u32, u32> {
    let mut starts = BTreeMap::new();
    for &line in lines {
        let row = line as usize - 1;
        let mut node = excerpt_line_node(parse, line);
        while let Some(current) = node {
            let kind = current.kind();
            let first = current.start_position().row;
            if excerpt_node_weight(kind).is_some() || excerpt_is_body(kind) {
                break;
            }
            if (kind.ends_with("statement")
                || kind.ends_with("declaration")
                || kind == "expression_statement"
                || kind == "lexical_declaration")
                && first < row
            {
                if current.end_position().row - first < EXCERPT_MAX_STATEMENT_LINES {
                    starts.insert(line, first as u32 + 1);
                }
                break;
            }
            node = current.parent();
        }
    }
    starts
}

/// The data of the excerpts of the changes of the file at `path` from the
/// revision `from` to `to`, as the outline diff between them (see
/// `outline_diff_data`) has them, as JSON for explore pages' panel (see
/// outline-excerpts.js; `/{tree}/{stack}/excerpts/{from}/{to}/{path}`,
/// `stack` being the page's URL in the tree, ex: "try/hg/abcdef012345", for its
/// stack strip; see `stack_strip_json`).  The changed definitions' excerpts'
/// lines (`entries`, by `excerpt_range_key`): the lines they added (`own`, in
/// the "to" file, and not in the definitions nested in them) and removed
/// (`removed`, indices in the file's), and the changes outside of any
/// definition's (`EXCERPT_TOP_LEVEL`).  And the lines of the "to" file which
/// the excerpts could show (`lines`, by number: their HTML, `h`, whether they
/// changed, `c`, how much they matter as the structure around changes, `s`,
/// see `excerpt_structure`, and the first lines of the statements they
/// continue, `u`, see `excerpt_statements`): all of the changed definitions'
/// lines, or for big ones the lines around their changes, and the removed
/// lines (`removed`: after which line of the "to" file, their lines in the
/// "from" file, and their HTML), and the file's stack strip (`strip`).  And
/// for a try push with a semantic analysis, the URL of its slicing's lines on
/// its changes' dataflow (`dataflow`; see `dataflow::impact`), which are its
/// rebased head's, and the head's lines of the lines, if they're not the
/// same (`head_lines`).  The page's panel chooses which lines to show by how
/// much room it has.
pub fn outline_excerpts_json(
    server: &(dyn AbstractServer + Send + Sync),
    tree_name: &str,
    stack: &str,
    from: &str,
    to: &str,
    path: &str,
) -> Result<String, &'static str> {
    let (server_, from_, to_, path_) = (
        server.clonify(),
        from.to_string(),
        to.to_string(),
        path.to_string(),
    );
    let value =
        run_on_runtime(async move { server_.outline_diff(&from_, &to_, &path_, true).await })
            .map_err(|_| "No outline diff of the file")?;
    let diff: outline_diff::OutlineDiff =
        serde_json::from_value(value).map_err(|_| "Bad outline diff")?;
    let from_path = diff.from_path.clone().unwrap_or_else(|| path.to_string());
    let text_at = |rev: &str, path: &str| -> Result<String, &'static str> {
        let (server, rev, path) = (server.clonify(), rev.to_string(), path.to_string());
        Ok(
            run_on_runtime(async move { server.rev_file(&rev, &path).await })
                .map_err(|_| "No such revision")?
                .map(|file| file.text)
                .unwrap_or_default(),
        )
    };
    let to_text = text_at(&diff.to.rev, path)?;
    let from_text = text_at(&diff.from.rev, &from_path)?;
    let format = languages::select_formatting(path);
    if let FormatAs::Binary = format {
        return Err("Binary file");
    }

    // The lines the range added (in the "to" file) and removed (in the
    // "from" file, after which line of the "to" file, and for replaced
    // lines, the first line replacing them).
    let mut added: BTreeSet<u32> = BTreeSet::new();
    let mut removed: Vec<(u32, u32)> = vec![];
    let mut replaced_by: Vec<Option<u32>> = vec![];
    for op in similar::TextDiff::from_lines(&from_text, &to_text).ops() {
        let (tag, old, new) = op.as_tag_tuple();
        if tag == similar::DiffTag::Equal {
            continue;
        }
        let after = new.start as u32;
        let replacing = (!new.is_empty()).then_some(after + 1);
        added.extend(new.map(|line| line as u32 + 1));
        for line in old {
            removed.push((after, line as u32 + 1));
            replaced_by.push(replacing);
        }
    }

    // Each change's innermost definition, by its lines (or for a removed
    // definition's lines, by its lines in the "from" file).
    let innermost = |line: u32, from: bool| -> Option<String> {
        diff.entries
            .iter()
            .filter_map(|entry| {
                let lines = match (from, entry.status) {
                    (true, outline_diff::OutlineStatus::Removed) => entry.from_lines?,
                    (false, outline_diff::OutlineStatus::Removed) => return None,
                    (false, _) => entry.lines?,
                    (true, _) => return None,
                };
                (lines.0 <= line && line <= lines.1).then_some((lines.1 - lines.0, lines, from))
            })
            .min()
            .map(|(_, lines, from)| excerpt_range_key(lines, from))
    };
    let mut entries: BTreeMap<String, (Vec<u32>, Vec<usize>)> = BTreeMap::new();
    for &line in &added {
        let key = innermost(line, false).unwrap_or_else(|| EXCERPT_TOP_LEVEL.to_string());
        entries.entry(key).or_default().0.push(line);
    }
    for (index, &(after, line)) in removed.iter().enumerate() {
        // (Replaced lines are their replacements' definition's, and removed
        // lines between two of a definition's lines are its.)
        let key = innermost(line, true)
            .or_else(|| replaced_by[index].and_then(|first| innermost(first, false)))
            .or_else(|| {
                innermost(after, false)
                    .filter(|key| innermost(after + 1, false).as_ref() == Some(key))
            })
            .unwrap_or_else(|| EXCERPT_TOP_LEVEL.to_string());
        entries.entry(key).or_default().1.push(index);
    }

    // The lines the excerpts could show.
    let positions: BTreeSet<u32> = added
        .iter()
        .copied()
        .chain(removed.iter().map(|&(after, _)| after.max(1)))
        .collect();
    let parse =
        crate::tree_sitter_support::cst_tokenizer::parse_source_for_analysis(path, &to_text).ok();
    let structure = parse
        .as_ref()
        .map(|parse| excerpt_structure(parse, &positions))
        .unwrap_or_default();
    let line_count = to_text.lines().count() as u32;
    let mut wanted: BTreeSet<u32> = BTreeSet::new();
    let window = |wanted: &mut BTreeSet<u32>, line: u32, (first, last): (u32, u32)| {
        let start = line.saturating_sub(EXCERPT_WINDOW).max(first);
        wanted.extend(start..=(line + EXCERPT_WINDOW).min(last));
    };
    for (key, (own, removals)) in &entries {
        let range = key
            .split_once('-')
            .filter(|_| !key.starts_with("from:"))
            .and_then(|(first, last)| Some((first.parse().ok()?, last.parse().ok()?)));
        let changes = own
            .iter()
            .copied()
            .chain(removals.iter().map(|&index| removed[index].0.max(1)));
        match range {
            Some((first, last)) if last - first < EXCERPT_WHOLE_LINES => {
                wanted.extend(first..=last);
            }
            Some((first, last)) => {
                wanted.extend([first, last]);
                wanted.extend(structure.range(first..=last).map(|(line, _)| *line));
                for line in changes {
                    window(&mut wanted, line, (first, last));
                }
            }
            None => {
                for line in changes {
                    window(&mut wanted, line, (1, line_count));
                }
            }
        }
    }

    let statements = parse
        .as_ref()
        .map(|parse| excerpt_statements(parse, &wanted))
        .unwrap_or_default();
    let (to_lines, _) = format_code(None, &TreeJumprefs::default(), format, path, &to_text, &[]);
    let lines: serde_json::Map<String, serde_json::Value> = wanted
        .iter()
        .filter_map(|&line| {
            let formatted = to_lines.get(line as usize - 1)?;
            let mut value = json!({ "h": formatted.line });
            if added.contains(&line) {
                value["c"] = json!(true);
            }
            if let Some(score) = structure.get(&line) {
                value["s"] = json!((*score as f64 * 100.0).round() / 100.0);
            }
            if let Some(start) = statements.get(&line) {
                value["u"] = json!(start);
            }
            Some((line.to_string(), value))
        })
        .collect();
    let removed_rows: Vec<serde_json::Value> = if removed.is_empty() {
        vec![]
    } else {
        let format = languages::select_formatting(&from_path);
        let (from_lines, _) = match format {
            FormatAs::Binary => (vec![], String::new()),
            format => format_code(
                None,
                &TreeJumprefs::default(),
                format,
                &from_path,
                &from_text,
                &[],
            ),
        };
        removed
            .iter()
            .map(|&(after, line)| {
                json!({
                    "after": after,
                    "line": line,
                    "h": from_lines.get(line as usize - 1).map_or("", |formatted| &formatted.line),
                })
            })
            .collect()
    };
    let strip = {
        let (server, stack, rev, path) = (
            server.clonify(),
            stack.to_string(),
            diff.to.rev.clone(),
            path.to_string(),
        );
        run_on_runtime(async move { server.stack_strip_json(&stack, &rev, &path).await })
            .ok()
            .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
    };

    // (For a try push with a semantic analysis, its slicing's lines on its
    // changes' dataflow, by its rebased head's lines, which may not be the
    // file's at `to` if the push was rebased past changes to it.)
    let (dataflow, head_lines) = match excerpt_try_slicing(server, stack) {
        Some((base, head)) => {
            let head_text = text_at(&head, path)?;
            let head_lines: Option<BTreeMap<u32, u32>> = (head_text != to_text).then(|| {
                let mut map = BTreeMap::new();
                for op in similar::TextDiff::from_lines(&to_text, &head_text).ops() {
                    let (tag, old, new) = op.as_tag_tuple();
                    if tag == similar::DiffTag::Equal {
                        for (to_line, head_line) in old.zip(new) {
                            if wanted.contains(&(to_line as u32 + 1)) {
                                map.insert(to_line as u32 + 1, head_line as u32 + 1);
                            }
                        }
                    }
                }
                map
            });
            let query = format!("slice-rev:{}..{} slice-format:lines", base, head);
            (
                Some(format!(
                    "/{}/query/default?q={}&rev={}",
                    tree_name,
                    urlencoding::encode(&query),
                    head
                )),
                head_lines,
            )
        }
        None => (None, None),
    };
    serde_json::to_string(&json!({
        "path": path,
        "from_path": diff.from_path,
        "from": diff.from.rev,
        "to": diff.to.rev,
        "link": format!("/{}/rev/{}/{}", tree_name, diff.to.rev, url_encode_path(path)),
        "from_link": format!("/{}/rev/{}/{}", tree_name, diff.from.rev, url_encode_path(&from_path)),
        "entries": entries
            .into_iter()
            .map(|(key, (own, removals))| (key, json!({ "own": own, "removed": removals })))
            .collect::<serde_json::Map<_, _>>(),
        "lines": lines,
        "removed": removed_rows,
        "strip": strip,
        "dataflow": dataflow,
        "head_lines": head_lines,
    }))
    .map_err(|_| "Excerpt problems")
}

/// The base and rebased head of the try push whose stack's URL in the tree is
/// `stack` (ex: "try/hg/abcdef012345"), if it has a semantic analysis (see
/// `try_delta`), whose slicing's lines on its changes' dataflow its files'
/// excerpts prefer (see `outline_excerpts_json`).
fn excerpt_try_slicing(
    server: &(dyn AbstractServer + Send + Sync),
    stack: &str,
) -> Option<(String, String)> {
    let push = crate::try_import::stack_push(stack)?;
    let server_ = server.clonify();
    let head = run_on_runtime(async move { server_.try_push(&push).await })
        .ok()??
        .semantic_head?;
    let base = server.try_overlay(&head)?.try_delta_meta()?.base;
    Some((base, head))
}

/// An explore page with its `data` (see `explore_data`), and its files'
/// outline diffs, from `server`, if given (see `explore_outlines`).
fn render_explore(
    server: Option<&(dyn AbstractServer + Send + Sync)>,
    tree_name: &str,
    title: &str,
    data: serde_json::Value,
    extras: ExploreExtras,
    writer: &mut dyn Write,
) -> Result<(), &'static str> {
    let commit_count = data["commits"].as_array().map_or(0, Vec::len);
    let page_title = format!("{} - mozsearch", title);
    let opt = Options {
        title: &page_title,
        tree_name,
        include_date: env::var("MOZSEARCH_DIFFABLE").is_err(),
        revision: None,
        breadcrumbs_links_to: BreadcrumbsLinksTo::Latest,
        extra_content_classes: "explore",
        search_rev: extras.semantic_head.as_deref(),
    };
    // (explore.liquid has the navigation panel, with the facets.)
    output::generate_header(&opt, writer)?;
    let mut file_groups = data["file_groups"].clone();
    unescape_explore_names(&mut file_groups);
    order_explore_files(&mut file_groups);
    if let Some(server) = server {
        explore_outlines(
            server,
            tree_name,
            &extras.explore_url,
            &data["commits"],
            &mut file_groups,
        );
    }
    let globals = liquid::to_object(&json!({
        "tree": tree_name,
        "title": title,
        "commits": data["commits"],
        // (Arranged here, if another server's data; see `rearranged_facets`.)
        "facets": crate::cmd_pipeline::facets::rearranged_facets(
            &data["facets"],
            &server.map_or_else(Vec::new, |server| server.expanded_subsystems()),
        ),
        "file_groups": file_groups,
        "outlines": server.is_some(),
        "slot": explore::slot_width(commit_count),
        "missing": extras.missing,
        "truncated": extras.truncated,
        "other_branches": extras.other_branches,
        "explore_url": extras.explore_url,
        "note": extras.note,
        "scopes": extras.scopes,
        "semantic_head": extras.semantic_head,
        "semantic_failure": extras.semantic_failure,
        "landed": extras.landed,
        "pull_request": extras.pull_request,
    }))
    .map_err(|_| "Template problems")?;
    build_and_parse_explore()
        .render_to(writer, &globals)
        .map_err(|_| "Template problems")?;
    output::generate_footer(&opt, tree_name, "", writer).unwrap();
    Ok(())
}

/// The page of a try push (`/{tree}/try/hg/{rev}`; see `try_import`): once the
/// tree has imported it, the explore page of its commits, and before, its
/// import's status (with `notice`, ex: why it couldn't be asked for), and a
/// button asking for it (see `request_try_import`), and while it waits or
/// runs, polling its status (see `format_try_status`).  Returns whether it's
/// the imported push's page, which doesn't change (so it can be cached),
/// rather than its import's status's.
pub fn format_try(
    cfg: &Config,
    server: &(dyn AbstractServer + Send + Sync),
    tree_name: &str,
    rev: &str,
    notice: Option<&str>,
    writer: &mut dyn Write,
) -> Result<bool, &'static str> {
    use crate::try_import::{ImportStatus, is_pull_request, normalize_push, push_title};

    let rev = normalize_push(rev).ok_or("Not a try push")?;
    let (push_name, push_link) = push_title(cfg, tree_name, &rev);
    let status = try_import_status(server, &rev)?;
    match status {
        ImportStatus::Unavailable => return Err("This tree can't import try pushes"),
        ImportStatus::Imported { .. } => {
            let (server_, rev_) = (server.clonify(), rev.clone());
            let push = run_on_runtime(async move { server_.try_push(&rev_).await })
                .ok()
                .flatten()
                .ok_or("The try push's import isn't there")?;
            let explore_url = crate::try_import::push_url_path(&rev);
            let (server_, revs, stack_url) = (
                server.clonify(),
                push.stack
                    .iter()
                    .map(|commit| commit.rev.clone())
                    .collect::<Vec<_>>(),
                explore_url.clone(),
            );
            let data =
                run_on_runtime(async move { server_.explore(&revs, Some(&stack_url)).await })
                    .map_err(|_| "The try push's commits aren't there")?;
            let extras = ExploreExtras {
                missing: vec![],
                truncated: push.truncated,
                other_branches: vec![],
                explore_url,
                // (And if it has none, why, if its import's analysis failed.)
                semantic_failure: push.semantic_failure,
                landed: push
                    .landed
                    .iter()
                    .map(|landed| {
                        json!({
                            "rev": landed.rev,
                            "summary": landed.rev_summary,
                            "landed": landed.landed,
                            "landed_summary": landed.summary,
                            "same_changes": landed.same_changes,
                        })
                    })
                    .collect(),
                pull_request: is_pull_request(&rev).then(|| {
                    json!({
                        "link": push_link,
                        "path": crate::try_import::push_url_path(&rev),
                    })
                }),
                note: Some(format!(
                    "Its stack (its commits since it diverged from the trunk{}), imported into \
                     this tree's history, for its commits' blame and what they changed{}.",
                    match push.task_selections {
                        0 => String::new(),
                        1 => ", but the one selecting try's tasks".to_string(),
                        n => format!(", but the {} selecting try's tasks", n),
                    },
                    match push.semantic_head {
                        Some(_) => "",
                        None => ", but not indexed, so its revisions have no jumps or references",
                    }
                )),
                scopes: vec![],
                semantic_head: push.semantic_head,
            };
            render_explore(Some(server), tree_name, &push_name, data, extras, writer)?;
            return Ok(true);
        }
        _ => {}
    }

    let title = format!("{} - mozsearch", push_name);
    let opt = Options {
        title: &title,
        tree_name,
        include_date: env::var("MOZSEARCH_DIFFABLE").is_err(),
        revision: None,
        breadcrumbs_links_to: BreadcrumbsLinksTo::Latest,
        extra_content_classes: "try-import-page",
        search_rev: None,
    };
    output::generate_header(&opt, writer)?;
    output::generate_panel(&opt, writer, &[], true)?;
    let globals = liquid::to_object(&json!({
        "tree": tree_name,
        "rev": rev,
        "name": push_name,
        "link": push_link,
        "push_path": crate::try_import::push_url_path(&rev),
        "status_path": format!(
            "try-status/{}",
            crate::try_import::push_url_path(&rev).trim_start_matches("try/")
        ),
        "status": status,
        "notice": notice,
    }))
    .map_err(|_| "Template problems")?;
    crate::templating::builder::build_and_parse_try_import()
        .render_to(writer, &globals)
        .map_err(|_| "Template problems")?;
    output::generate_footer(&opt, tree_name, "", writer).unwrap();
    Ok(false)
}

/// How many recently landed bugs a tree's landing page lists (see
/// `recent_bugs`), from at most how many commits, of how many days before the
/// newest.
const LANDING_BUGS: usize = 40;
const LANDING_MAX_COMMITS: usize = 5000;
const LANDING_DAYS: i64 = 14;

/// A bug whose commits recently landed (see `recent_bugs`).
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct RecentBug {
    pub bug: String,
    /// Its newest commit's summary line, without its "Bug N - " and its
    /// reviewers.
    pub summary: String,
    /// Its newest commit's date, in UTC.
    #[serde(rename = "isoDate")]
    pub iso_date: String,
    pub commits: usize,
}

/// The bugs whose commits landed most recently on `head`, newest first, at
/// most `limit` of them: the bugs of its commits (but merges) of the
/// `LANDING_DAYS` before the newest (at most `LANDING_MAX_COMMITS` of them),
/// by their summary lines' leading "Bug N" (not other bugs they mention, ex:
/// "(follow-up to bug M)"), but those whose newest commit there is a backout
/// (by any bug it mentions: a landing backed out, and not relanded since).
pub fn recent_bugs(repo: &Repository, head: Oid, limit: usize) -> Vec<RecentBug> {
    let Ok(mut walk) = repo.revwalk() else {
        return vec![];
    };
    // (In libgit2's default order, newest first, as `git log`'s, which it
    // walks as it goes: `Sort::TIME` sorts the whole history first, 7.5 s
    // rather than 54 ms for firefox-main's last 2 weeks' 2723 commits.)
    if walk.push(head).is_err() {
        return vec![];
    }
    let mut bugs: Vec<RecentBug> = vec![];
    // (Each bug's index in `bugs`, or None if it was backed out.)
    let mut seen: HashMap<String, Option<usize>> = HashMap::new();
    let mut newest: Option<i64> = None;
    for oid in walk.take(LANDING_MAX_COMMITS).flatten() {
        let Ok(commit) = repo.find_commit(oid) else {
            continue;
        };
        let time = commit.time().seconds();
        if time < *newest.get_or_insert(time) - LANDING_DAYS * 24 * 60 * 60 {
            break;
        }
        if commit.parent_count() > 1 {
            continue;
        }
        let message = String::from_utf8_lossy(commit.message_bytes());
        let commit_ref = crate::commit_index::commit_ref(&commit);
        let summary = message.lines().next().unwrap_or("");
        let bugs_of_commit = match commit_ref.backout {
            true => crate::commit_index::message_bugs(&message),
            false => leading_bug(summary).into_iter().collect(),
        };
        for bug in bugs_of_commit {
            match seen.get(&bug) {
                Some(Some(i)) if !commit_ref.backout => bugs[*i].commits += 1,
                Some(_) => {}
                None if commit_ref.backout => {
                    seen.insert(bug, None);
                }
                None => {
                    seen.insert(bug.clone(), Some(bugs.len()));
                    bugs.push(RecentBug {
                        bug,
                        summary: bug_summary(summary),
                        iso_date: commit_ref.iso_date.clone(),
                        commits: 1,
                    });
                }
            }
        }
    }
    bugs.truncate(limit);
    bugs
}

/// A commit summary line's leading bug's number (ex: "2083426" for "Bug
/// 2083426 - Defer throwing X").
fn leading_bug(summary: &str) -> Option<String> {
    static LEADING: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^(?i)\s*bug\s*([1-9][0-9]{2,6})\b").unwrap()
    });
    Some(LEADING.captures(summary)?[1].to_string())
}

/// How many characters of a bug's summary its landing page shows.
const LANDING_SUMMARY_CHARS: usize = 200;

/// A commit's summary line without its "Bug N - " and its reviewers (ex:
/// "Defer throwing X." for "Bug 2083426 - Defer throwing X.
/// r=gfx-reviewers,ahale"), at most `LANDING_SUMMARY_CHARS` of it.
fn bug_summary(summary: &str) -> String {
    static PREFIX: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^(?i)\s*bug\s*[0-9]+\s*[-:,.]*\s*").unwrap()
    });
    static REVIEWERS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?:\s+(?:r|sr|a|ir)[=?]\S*)+\s*$").unwrap()
    });
    let summary = PREFIX.replace(summary, "");
    let summary = REVIEWERS.replace(&summary, "").trim().to_string();
    match summary.char_indices().nth(LANDING_SUMMARY_CHARS) {
        Some((at, _)) => format!("{}\u{2026}", summary[..at].trim_end()),
        None => summary,
    }
}

/// A query page's URL in the tree, for `query`.
fn landing_query_url(tree_name: &str, query: &str) -> String {
    let mut url = url::form_urlencoded::Serializer::new(String::new());
    url.append_pair("q", query);
    format!("/{}/query/default?{}", tree_name, url.finish())
}

/// The data of a tree's landing page (see `format_landing`): its indexed
/// revision (`head`); its system diagrams, if it has call roll-ups (see
/// `cmd_block_diagram`): the whole tree's subsystems by calls (and by data, if
/// it has data roll-ups), and each subsystem's classes, from the whole tree's
/// call roll-up's members (`subsystems`, most calls first: in, out and
/// within; see `root_rollup`); and its recently landed bugs (see
/// `recent_bugs`), if it has explore pages for them (a commit index).  For
/// local indexes; see `AbstractServer::landing`.
pub fn landing_data(
    cfg: &Config,
    tree_name: &str,
    root_rollup: Option<crate::file_format::rollup::Rollup>,
    data_rollups: bool,
) -> Result<serde_json::Value, &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let quote = |name: &str| match name.contains('\'') {
        true => format!("\"{}\"", name),
        false => format!("'{}'", name),
    };
    let diagrams = root_rollup.as_ref().map(|rollup| {
        json!({
            "calls": landing_query_url(tree_name, "block-diagram:*"),
            "data": data_rollups.then(|| landing_query_url(tree_name, "block-diagram:* block-edges:data")),
            "arcs": landing_query_url(tree_name, "block-diagram:* block-view:arcs"),
            "subsystems": rollup
                .members
                .iter()
                .map(|member| json!({
                    "name": member.pretty,
                    "count": member.count,
                    "url": landing_query_url(
                        tree_name,
                        &format!("block-diagram:{}", quote(member.pretty.as_str())),
                    ),
                }))
                .collect::<Vec<_>>(),
        })
    });
    let git = tree_config.get_git().ok();
    let head = git.and_then(|git| git.repo.head().ok()?.peel_to_commit().ok());
    let explore = git.is_some_and(|git| {
        git.history.as_ref().is_some_and(|history| {
            let branch = tree_config.paths.git_branch.as_deref().unwrap_or("HEAD");
            CommitIndex::open_branch(&Path::new(&history.path).join("commit-index"), branch)
                .is_some()
        })
    });
    let bugs = match (git, &head) {
        (Some(git), Some(head)) if explore => recent_bugs(&git.repo, head.id(), LANDING_BUGS),
        _ => vec![],
    };
    Ok(json!({
        "head": head.as_ref().map(|head| json!({
            "rev": head.id().to_string(),
            "isoDate": crate::commit_index::commit_ref(head).iso_date,
        })),
        "diagrams": diagrams,
        "bugs": bugs,
        "bugDays": LANDING_DAYS,
    }))
}

/// A tree's landing page (`/{tree}/`), from its `landing_data`: a link to
/// browse its source, links to its system diagrams, and its recently landed
/// bugs, linking to their explore pages.  It's rendered once, at index time,
/// as the tree's state doesn't change (`searchfox-tool "render landing"`, to
/// `pages/landing.html`, which nginx serves).
pub fn render_landing(
    tree_name: &str,
    data: serde_json::Value,
    writer: &mut dyn Write,
) -> Result<(), &'static str> {
    let title = format!("{} - mozsearch", tree_name);
    let opt = Options {
        title: &title,
        tree_name,
        include_date: env::var("MOZSEARCH_DIFFABLE").is_err(),
        revision: None,
        breadcrumbs_links_to: BreadcrumbsLinksTo::Latest,
        extra_content_classes: "landing-page",
        search_rev: None,
    };
    output::generate_header(&opt, writer)?;
    output::generate_panel(&opt, writer, &[], true)?;
    let globals = liquid::to_object(&json!({
        "tree": tree_name,
        "data": data,
    }))
    .map_err(|_| "Template problems")?;
    crate::templating::builder::build_and_parse_landing()
        .render_to(writer, &globals)
        .map_err(|_| "Template problems")?;
    output::generate_footer(&opt, tree_name, "", writer).unwrap();
    Ok(())
}

/// The status of the import of the try push `rev` into `server`'s tree.
fn try_import_status(
    server: &(dyn AbstractServer + Send + Sync),
    rev: &str,
) -> Result<crate::try_import::ImportStatus, &'static str> {
    let (server, rev) = (server.clonify(), rev.to_string());
    run_on_runtime(async move { server.try_import_status(&rev).await })
        .map_err(|_| "This tree can't import try pushes")
}

/// The status of the import of a try push into the tree (see `try_import`),
/// as JSON (`/{tree}/try-status/hg/{rev}`), for its page's polling.
pub fn format_try_status(
    server: &(dyn AbstractServer + Send + Sync),
    rev: &str,
) -> Result<String, &'static str> {
    let rev = crate::try_import::normalize_push(rev).ok_or("Not a try push")?;
    let status = try_import_status(server, &rev)?;
    serde_json::to_string(&status).map_err(|_| "Status problems")
}

/// Ask for a try push to be imported into the tree (see `try_import`).
pub fn request_try_import(
    server: &(dyn AbstractServer + Send + Sync),
    rev: &str,
) -> Result<(), String> {
    let rev = crate::try_import::normalize_push(rev).ok_or("Not a try push")?;
    let server = server.clonify();
    run_on_runtime(async move { server.request_try_import(&rev).await }).map_err(|err| match err {
        // (The import machinery's own reasons, ex: too many waiting.)
        ServerError::StickyProblem(details) | ServerError::TransientProblem(details)
            if matches!(details.layer, crate::abstract_server::ErrorLayer::BadInput) =>
        {
            details.message
        }
        _ => "The try push couldn't be asked for".to_string(),
    })
}

/// An ISO 8601 UTC time as the pages show it (ex: "2026-10-26 14:03 UTC" for
/// "2026-10-26T14:03:07Z").
fn utc_minute(iso: &str) -> String {
    match iso.get(..16) {
        Some(minute) if iso.ends_with('Z') => format!("{} UTC", minute.replace('T', " ")),
        _ => iso.to_string(),
    }
}

/// The pages of the summaries of the open pull requests of a tree of specs'
/// spec repositories (see `spec_prs`): `/{tree}/spec-prs` (`spec` being None),
/// its repositories, with how many pull requests each has, by review decision
/// and author; and `/{tree}/spec-prs/{spec}`, a repository's pull requests,
/// newest first, with facets filtering them (see `spec_prs::pr_facets`), each
/// with its page as a try push (see `try_import`), which imports it, its
/// GitHub page, what it changes (files and their sections, linking to them in
/// the trunk's renderings), and its import's status (see
/// `spec_prs::pr_imports`), with a button asking for the imports of those
/// which need them, if there are at most `try_import::MAX_BATCH`, and
/// while some wait or run, polling their status (see
/// `format_spec_pr_imports`).  Both have their syncs' status (with `notice`,
/// ex: why one couldn't be asked for), a button asking for one, and while one
/// waits or runs, poll its status (see `format_spec_prs_sync_status`),
/// reloading once it's done.
pub fn format_spec_prs(
    server: &(dyn AbstractServer + Send + Sync),
    tree_name: &str,
    spec: Option<&str>,
    notice: Option<&str>,
    writer: &mut dyn Write,
) -> Result<(), &'static str> {
    use crate::spec_prs::{can_review, pr_facet_values, pr_facets, review_name};

    let status = spec_prs_sync_status(server, spec)?;
    if status == crate::spec_prs::SyncStatus::Unavailable {
        return Err("This tree has no pull request summaries");
    }
    let (server_, spec_) = (server.clonify(), spec.map(str::to_string));
    let summaries = run_on_runtime(async move { server_.spec_prs(spec_.as_deref()).await })
        .map_err(|_| "The pull request summaries aren't there")?;
    let query = spec.map_or_else(String::new, |spec| format!("?spec={}", spec));
    let mut globals = json!({
        "tree": tree_name,
        "spec": spec,
        "status": status,
        "notice": notice,
        "status_url": format!("/{}/spec-prs/sync-status{}", tree_name, query),
        "sync_url": format!("/{}/spec-prs/sync{}", tree_name, query),
        // (Each page's, set below.)
        "repos": [],
        "summary": null,
        "prs": [],
        "facets": [],
        // (A repository's page's; see below.)
        "imports": {
            "available": false,
            "count": 0,
            "needed": 0,
            "pending": 0,
            "current": 0,
            "batch": true,
            "max_batch": crate::try_import::MAX_BATCH,
            "import_url": "",
            "status_url": "",
        },
    });
    let title = match spec {
        None => {
            let repos: Vec<serde_json::Value> = summaries
                .iter()
                .map(|summary| {
                    let count = |f: &dyn Fn(&crate::spec_prs::PrSummary) -> bool| {
                        summary.prs.iter().filter(|pr| f(pr)).count()
                    };
                    json!({
                        "spec": summary.spec,
                        "repo": summary.repo,
                        "url": summary.url,
                        "synced": utc_minute(&summary.synced),
                        "count": summary.prs.len(),
                        "approved": count(&|pr| pr.review.as_deref() == Some("APPROVED")),
                        "changes_requested": count(&|pr| pr.review.as_deref() == Some("CHANGES_REQUESTED")),
                        "by_reviewers": count(&|pr| can_review(pr.association.as_deref())),
                        "drafts": count(&|pr| pr.draft),
                    })
                })
                .collect();
            globals["repos"] = json!(repos);
            "Pull requests of the specs".to_string()
        }
        Some(spec) => {
            // (A spec repository not synced yet has a page with the button.)
            let Some(summary) = summaries.first() else {
                globals["title"] = json!(format!("Pull requests of {}", spec));
                return render_spec_prs_page(tree_name, globals, writer);
            };
            // (None if the tree can't import them.)
            let imports = spec_pr_imports(server, spec).unwrap_or_default();
            let import_of = |number: u32| imports.iter().find(|import| import.number == number);
            let needed = imports.iter().filter(|import| import.needed()).count();
            let pending = imports
                .iter()
                .filter(|import| {
                    matches!(
                        import.status,
                        crate::try_import::ImportStatus::Queued { .. }
                            | crate::try_import::ImportStatus::Running { .. }
                    )
                })
                .count();
            globals["imports"] = json!({
                // (Not for trees which can't import them.)
                "available": imports.iter().any(|import| {
                    import.status != crate::try_import::ImportStatus::Unavailable
                }),
                "needed": needed,
                "pending": pending,
                "count": imports.len(),
                "current": imports.iter().filter(|import| import.current).count(),
                "batch": needed <= crate::try_import::MAX_BATCH,
                "max_batch": crate::try_import::MAX_BATCH,
                "import_url": format!("/{}/spec-prs/import?spec={}", tree_name, spec),
                "status_url": format!("/{}/spec-prs/import-status?spec={}", tree_name, spec),
            });
            let prs: Vec<serde_json::Value> = summary
                .prs
                .iter()
                .map(|pr| {
                    let files: Vec<serde_json::Value> = pr
                        .files
                        .iter()
                        .map(|file| {
                            let rendered = file
                                .spec
                                .as_ref()
                                .map(|_| format!("/{}/rendered/{}", tree_name, file.path));
                            json!({
                                "path": file.path,
                                "added": file.added,
                                "removed": file.removed,
                                "link": rendered.clone().unwrap_or_else(|| {
                                    format!("/{}/source/{}", tree_name, file.path)
                                }),
                                // (The view with all of the pull requests'
                                // changes of it; see `format_rendered`.)
                                "with_prs": file.spec.as_ref().map(|_| {
                                    format!("/{}/rendered-prs/{}", tree_name, file.path)
                                }),
                                "sections": file.sections.iter().map(|section| json!({
                                    "title": section.title,
                                    "link": match (&rendered, &section.id) {
                                        (Some(rendered), Some(id)) => Some(format!("{}#{}", rendered, id)),
                                        _ => None,
                                    },
                                })).collect::<Vec<_>>(),
                            })
                        })
                        .collect();
                    json!({
                        "number": pr.number,
                        "title": pr.title,
                        "url": pr.url,
                        "author": pr.author,
                        "can_review": can_review(pr.association.as_deref()),
                        "draft": pr.draft,
                        "labels": pr.labels,
                        "review": pr.review,
                        "review_name": review_name(pr.review.as_deref()),
                        "review_class": pr
                            .review
                            .as_deref()
                            .unwrap_or("none")
                            .to_lowercase()
                            .replace('_', "-"),
                        "updated": pr.updated.as_deref().map(utc_minute),
                        "try_path": format!("try/{}/{}", spec, pr.number),
                        "facets": serde_json::to_string(&pr_facet_values(pr, chrono::Utc::now())).unwrap_or_default(),
                        "added": pr.files.iter().map(|file| file.added).sum::<u32>(),
                        "removed": pr.files.iter().map(|file| file.removed).sum::<u32>(),
                        "files": files,
                        "error": pr.error,
                        "import": import_of(pr.number).map_or_else(
                            || json!({ "number": pr.number, "state": "none", "text": "", "needed": false }),
                            spec_pr_import_view,
                        ),
                    })
                })
                .collect();
            globals["summary"] = json!({
                "repo": summary.repo,
                "url": summary.url,
                "synced": utc_minute(&summary.synced),
                "snapshot": summary.snapshot.as_deref().map(utc_minute),
            });
            globals["prs"] = json!(prs);
            globals["facets"] = json!(pr_facets(&summary.prs, chrono::Utc::now()));
            format!("Pull requests of {}", summary.repo)
        }
    };
    globals["title"] = json!(title);
    render_spec_prs_page(tree_name, globals, writer)
}

/// An open pull request's import (see `spec_prs::PrImport`) as its summary
/// shows it: its `state` ("current", "older" for one imported at an older
/// head, "queued", "running", "failed", or "none"), its text, and whether it
/// `needed` importing.
fn spec_pr_import_view(import: &crate::spec_prs::PrImport) -> serde_json::Value {
    use crate::try_import::ImportStatus;
    let (state, text) = match &import.status {
        ImportStatus::Imported { .. } if import.current => ("current", "Imported".to_string()),
        ImportStatus::Imported { .. } => ("older", "Imported at an older head".to_string()),
        ImportStatus::Queued { .. } => ("queued", "Waiting to be imported".to_string()),
        ImportStatus::Running { seconds, .. } => ("running", format!("Importing ({}s)", seconds)),
        ImportStatus::Failed { .. } => ("failed", "Its import failed".to_string()),
        _ => ("none", String::new()),
    };
    json!({ "number": import.number, "state": state, "text": text, "needed": import.needed() })
}

/// The imports of the open pull requests of `server`'s tree's spec repository
/// `spec` (see `spec_prs::pr_imports`).
fn spec_pr_imports(
    server: &(dyn AbstractServer + Send + Sync),
    spec: &str,
) -> Result<Vec<crate::spec_prs::PrImport>, &'static str> {
    let (server, spec) = (server.clonify(), spec.to_string());
    run_on_runtime(async move { server.spec_pr_imports(&spec).await })
        .map_err(|_| "This tree doesn't import pull requests")
}

/// The imports of the open pull requests of the tree's spec repository
/// `spec`, as their summaries show them (see `spec_pr_import_view`), as JSON
/// (`/{tree}/spec-prs/import-status?spec=`), for their page's polling.
pub fn format_spec_pr_imports(
    server: &(dyn AbstractServer + Send + Sync),
    spec: &str,
) -> Result<String, &'static str> {
    let imports = spec_pr_imports(server, spec)?;
    let views: Vec<serde_json::Value> = imports.iter().map(spec_pr_import_view).collect();
    serde_json::to_string(&views).map_err(|_| "Status problems")
}

/// Ask for the imports of the open pull requests of the tree's spec
/// repository `spec` which need them, or of those of `numbers` which do (see
/// `spec_prs::request_pr_imports`); how many it asked for.
pub fn request_spec_pr_imports(
    server: &(dyn AbstractServer + Send + Sync),
    spec: &str,
    numbers: Option<Vec<u32>>,
) -> Result<usize, String> {
    let (server, spec) = (server.clonify(), spec.to_string());
    run_on_runtime(async move {
        server
            .request_spec_pr_imports(&spec, numbers.as_deref())
            .await
    })
    .map_err(|err| match err {
        ServerError::StickyProblem(details) | ServerError::TransientProblem(details)
            if matches!(details.layer, crate::abstract_server::ErrorLayer::BadInput) =>
        {
            details.message
        }
        _ => "The imports couldn't be asked for".to_string(),
    })
}

/// What `/{tree}/source/{path}` is when the tree has no file or directory
/// there (see `format_missing_source`).
pub enum MissingSource {
    /// Where it is now: the file or directory it moved to (with where it
    /// moved from and when, for the note it shows; see redirect-note.js).
    Redirect(String),
    /// A 404 page saying what became of it.
    Page(String),
}

/// What `/{tree}/source/{path}`, which the tree doesn't have, is, by what
/// became of it (see `hyperblame::file_fate`): if it moved to a file the tree
/// has (or for a directory, most of its files moved to one), a redirect
/// there; else a page saying when it was deleted, with its last version and
/// where its content went, or what became of a directory's files, or if its
/// history knows nothing of it, the corrections of its name's typos (see
/// `typos`); with the nearest directory the tree has.
pub fn format_missing_source(
    cfg: &Config,
    tree_name: &str,
    path: &str,
) -> Result<MissingSource, &'static str> {
    use crate::hyperblame::file_fate::Fate;

    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let git = tree_config.get_git()?;
    let tip = rendered_commit(git, None)?;
    let path = path.trim_matches('/');
    let fate = crate::hyperblame::file_fate::fate(git, tip.id(), path);
    let redirect = |to: &str, from: &str, rev: &str| {
        MissingSource::Redirect(format!(
            "/{}/source/{}?moved-from={}&moved-in={}",
            tree_name,
            url_encode_path(to),
            urlencoding::encode(from),
            rev
        ))
    };
    let pairs = |pairs: &[(String, usize)]| -> Vec<serde_json::Value> {
        pairs
            .iter()
            .map(|(path, count)| json!({ "path": path, "count": count }))
            .collect()
    };
    let mut globals = json!({
        "tree": tree_name,
        "path": path,
        "deleted": null,
        "moves": [],
        "summary": "",
        "parent": "",
        "content": [],
        "directory": null,
        "corrections": [],
        "ancestor": "",
    });
    match &fate {
        Some(Fate::Moved { path: to, moves }) => {
            let rev = moves.last().map_or("", |event| event.rev.as_str());
            return Ok(redirect(to, path, rev));
        }
        Some(Fate::Deleted {
            moves,
            deleted,
            parent,
            content,
        }) => {
            let summary = Oid::from_str(&deleted.rev)
                .ok()
                .and_then(|rev| git.repo.find_commit(rev).ok())
                .and_then(|commit| commit.summary().ok().flatten().map(str::to_string))
                .unwrap_or_default();
            globals["deleted"] = json!(deleted);
            globals["moves"] = json!(moves);
            globals["summary"] = json!(summary);
            globals["parent"] = json!(parent.clone().unwrap_or_default());
            globals["content"] = json!(pairs(content));
        }
        Some(Fate::Directory {
            files,
            deleted,
            moved_to,
        }) => {
            // (Most of its files which weren't deleted moved there.)
            if let Some((to, count)) = moved_to.first()
                && !to.is_empty()
                && count * 2 > files - deleted
            {
                return Ok(redirect(to, path, ""));
            }
            globals["directory"] = json!({
                "files": files,
                "deleted": deleted,
                "moved_to": pairs(moved_to),
            });
        }
        None => {
            let name = path.rsplit('/').next().unwrap_or(path);
            if let Some(words) = crate::typos::Words::open(&tree_config.paths.index_path) {
                let corrections: Vec<String> = words
                    .corrections(name)
                    .into_iter()
                    .map(|correction| correction.word)
                    .collect();
                globals["corrections"] = json!(corrections);
            }
        }
    }
    // (The nearest directory the tree has.)
    let tree = tip.tree().map_err(|_| "Git commit with no tree")?;
    let ancestor = std::iter::successors(path.rsplit_once('/').map(|(dir, _)| dir), |dir| {
        dir.rsplit_once('/').map(|(dir, _)| dir)
    })
    .find(|dir| {
        tree.get_path(Path::new(dir))
            .is_ok_and(|entry| entry.kind() == Some(git2::ObjectType::Tree))
    })
    .unwrap_or_default();
    globals["ancestor"] = json!(ancestor);

    let mut writer = Vec::new();
    let page_title = format!("{} - mozsearch", path);
    let opt = Options {
        title: &page_title,
        tree_name,
        include_date: env::var("MOZSEARCH_DIFFABLE").is_err(),
        revision: None,
        breadcrumbs_links_to: BreadcrumbsLinksTo::Latest,
        extra_content_classes: "missing-source-page",
        search_rev: None,
    };
    output::generate_header(&opt, &mut writer)?;
    output::generate_panel(&opt, &mut writer, &[], true)?;
    let globals = liquid::to_object(&globals).map_err(|_| "Template problems")?;
    crate::templating::builder::build_and_parse_missing_source()
        .render_to(&mut writer, &globals)
        .map_err(|_| "Template problems")?;
    output::generate_footer(&opt, tree_name, "", &mut writer).map_err(|_| "Footer problems")?;
    Ok(MissingSource::Page(String::from_utf8(writer).unwrap()))
}

/// A page of pull request summaries, from `format_spec_prs`'s globals.
fn render_spec_prs_page(
    tree_name: &str,
    globals: serde_json::Value,
    writer: &mut dyn Write,
) -> Result<(), &'static str> {
    let page_title = format!(
        "{} - mozsearch",
        globals["title"].as_str().unwrap_or_default()
    );
    let opt = Options {
        title: &page_title,
        tree_name,
        include_date: env::var("MOZSEARCH_DIFFABLE").is_err(),
        revision: None,
        breadcrumbs_links_to: BreadcrumbsLinksTo::Latest,
        extra_content_classes: "spec-prs-page",
        search_rev: None,
    };
    output::generate_header(&opt, writer)?;
    output::generate_panel(&opt, writer, &[], true)?;
    let globals = liquid::to_object(&globals).map_err(|_| "Template problems")?;
    crate::templating::builder::build_and_parse_spec_prs()
        .render_to(writer, &globals)
        .map_err(|_| "Template problems")?;
    output::generate_footer(&opt, tree_name, "", writer).unwrap();
    Ok(())
}

/// The status of the syncs of `server`'s tree's pull request summaries (or of
/// `spec`'s).
fn spec_prs_sync_status(
    server: &(dyn AbstractServer + Send + Sync),
    spec: Option<&str>,
) -> Result<crate::spec_prs::SyncStatus, &'static str> {
    let (server, spec) = (server.clonify(), spec.map(str::to_string));
    run_on_runtime(async move { server.spec_prs_sync_status(spec.as_deref()).await })
        .map_err(|_| "This tree has no pull request summaries")
}

/// The status of the syncs of the tree's pull request summaries (or of
/// `spec`'s; see `spec_prs`), as JSON (`/{tree}/spec-prs/sync-status`), for
/// their pages' polling.
pub fn format_spec_prs_sync_status(
    server: &(dyn AbstractServer + Send + Sync),
    spec: Option<&str>,
) -> Result<String, &'static str> {
    let status = spec_prs_sync_status(server, spec)?;
    serde_json::to_string(&status).map_err(|_| "Status problems")
}

/// Ask for a sync of the tree's pull request summaries (or of `spec`'s; see
/// `spec_prs`).
pub fn request_spec_prs_sync(
    server: &(dyn AbstractServer + Send + Sync),
    spec: Option<&str>,
) -> Result<(), String> {
    let (server, spec) = (server.clonify(), spec.map(str::to_string));
    run_on_runtime(async move { server.request_spec_prs_sync(spec.as_deref()).await }).map_err(
        |err| match err {
            ServerError::StickyProblem(details) | ServerError::TransientProblem(details)
                if matches!(details.layer, crate::abstract_server::ErrorLayer::BadInput) =>
            {
                details.message
            }
            _ => "The sync couldn't be asked for".to_string(),
        },
    )
}

/// The data of a commit's page (see `render_commit`): its `date` (RFC 3339)
/// and its `content` (its message, revisions, people and changed files, as
/// HTML).  (For trees' servers; see `AbstractServer::commit_page`.)
pub fn commit_page_data(
    cfg: &Config,
    tree_name: &str,
    rev: &str,
) -> Result<serde_json::Value, &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let git = tree_config.get_git()?;
    let commit_obj = git.repo.revparse_single(rev).map_err(|_| "Bad revision")?;
    let commit = commit_obj.as_commit().ok_or("Bad revision")?;
    let mut content = Vec::new();
    generate_commit_info(tree_name, tree_config, &mut content, commit)?;
    Ok(json!({
        "date": git_time_to_chrono(commit.time()).to_rfc3339(),
        "content": String::from_utf8_lossy(&content),
    }))
}

/// A commit's page (`/{tree}/commit/{rev}`), with its `data` (see
/// `commit_page_data`).
pub fn render_commit(
    tree_name: &str,
    rev: &str,
    data: serde_json::Value,
    writer: &mut dyn Write,
) -> Result<(), &'static str> {
    let date = chrono::DateTime::parse_from_rfc3339(data["date"].as_str().unwrap_or_default())
        .map_err(|_| "Bad date")?;

    let title = format!("{} - mozsearch", rev);
    let opt = Options {
        title: &title,
        tree_name,
        include_date: true,
        revision: Some(RevisionData {
            rev,
            desc: "",
            date,
        }),
        breadcrumbs_links_to: BreadcrumbsLinksTo::Historical,
        extra_content_classes: "commit",
        search_rev: None,
    };

    output::generate_header(&opt, writer)?;

    output::generate_breadcrumbs(&opt, writer, "", &[], false)?;

    output::generate_panel(&opt, writer, &[], true)?;

    writer
        .write_all(data["content"].as_str().unwrap_or_default().as_bytes())
        .map_err(|_| "Write problems")?;

    output::generate_footer(&opt, tree_name, "", writer).unwrap();

    Ok(())
}

#[cfg(test)]
mod tests {
    /// The DWIM lookups without repositories (see `dwim_lookup`): a spec
    /// repository's pull request is its try push's page in the tree of specs
    /// with the spec (by its URL, ignoring case and ".git"), or its `SPEC_`
    /// linked tree; none for other repositories, and none of the lookups
    /// needing a history or try imports, for a tree without them.
    #[test]
    fn test_dwim_lookup() {
        use crate::file_format::config::{Config, TreeConfig, TreeConfigPaths};
        let dir = std::env::temp_dir().join(format!("dwim-lookup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("specs.toml"),
            "[[spec]]\nname = \"html\"\nurl = \"https://github.com/whatwg/html.git\"\n",
        )
        .unwrap();
        // (The code tree's index, without specs.)
        let code_dir = dir.join("code");
        std::fs::create_dir_all(&code_dir).unwrap();
        let tree = |dir: &std::path::Path, linked: Option<&str>| {
            let mut settings = serde_json::json!({
                "priority": 1, "on_error": "halt", "cache": "everything",
                "index_path": dir.to_string_lossy(), "files_path": dir.to_string_lossy(),
                "objdir_path": dir.to_string_lossy(),
                "codesearch_path": "", "codesearch_port": 0,
            });
            if let Some(linked) = linked {
                settings["linked_trees"] = serde_json::json!({ "SPEC_": linked });
            }
            TreeConfig {
                paths: serde_json::from_value::<TreeConfigPaths>(settings).unwrap(),
                git: None,
            }
        };
        let mut trees = std::collections::BTreeMap::new();
        trees.insert("dwim-specs".to_string(), tree(&dir, None));
        trees.insert("dwim-code".to_string(), tree(&code_dir, Some("dwim-specs")));
        let cfg = Config {
            trees,
            mozsearch_path: String::new(),
            config_repo_path: dir.to_string_lossy().into_owned(),
            url_map_path: None,
            doc_trees_path: None,
            tree_index_paths: Default::default(),
            config_file: String::new(),
            query_limits: Default::default(),
            crash_reports_url: None,
            bugzilla_url: None,
        };
        let lookup = |tree: &str, kind: &str, captures: &[(&str, &str)]| {
            let capture = |name: &str| {
                captures
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.to_string())
            };
            super::dwim_lookup(&cfg, tree, kind, &capture)
        };
        let pr = [("repo", "WHATWG/html"), ("number", "13007")];
        assert_eq!(
            lookup("dwim-specs", "spec-pr", &pr).as_deref(),
            Some("/dwim-specs/try/html/13007")
        );
        // (From a tree whose spec tree it is, ex: firefox-disco's wubspecs.)
        assert_eq!(
            lookup("dwim-code", "spec-pr", &pr).as_deref(),
            Some("/dwim-specs/try/html/13007")
        );
        assert_eq!(
            lookup(
                "dwim-specs",
                "spec-pr",
                &[("repo", "mozilla/glean"), ("number", "1")]
            ),
            None
        );
        assert_eq!(lookup("dwim-specs", "explore-bug", &[("bug", "1")]), None);
        assert_eq!(
            lookup("dwim-specs", "hg-commit", &[("rev", "0123456789ab")]),
            None
        );
        assert_eq!(
            lookup(
                "dwim-specs",
                "treeherder-try",
                &[("params", "repo=try&revision=0123456789ab")]
            ),
            None
        );
        assert_eq!(
            lookup("dwim-specs", "commit", &[("rev", "0123456789ab")]),
            None
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// rendered_prs.liquid's parts (see `render_rendered_prs`): the panel's
    /// facets and collapsible list, and the header's sentence, which a spec
    /// without pull requests changing the page, or without its pull requests
    /// synced, has, but not the panel's.
    #[test]
    fn test_rendered_prs_parts() {
        let render = |globals: &serde_json::Value, part: &str| {
            let mut out = vec![];
            super::render_rendered_prs(&liquid::to_object(globals).unwrap(), part, &mut out)
                .unwrap();
            String::from_utf8(out).unwrap()
        };
        let pr = |number: u32| {
            serde_json::json!({
                "number": number,
                "title": format!("Change {}", number),
                "try_path": format!("/specs/try/gizmo/{}", number),
                "facets": "{}",
                "data": "{}",
                "error": "",
                "loads": true,
            })
        };
        let globals = |prs: Vec<serde_json::Value>, synced: bool| {
            serde_json::json!({
                "tree": "specs",
                "spec_repo": "gizmo",
                "summary": synced.then(|| serde_json::json!({ "repo": "whatwg/gizmo", "synced": "2026-10-28 18:00 UTC" })),
                "prs": prs,
                "facets": [],
            })
        };

        let two = globals(vec![pr(1), pr(2)], true);
        let panel = render(&two, "panel");
        assert!(
            panel.contains(r#"<section class="facet-box" data-noun="pull request""#),
            "{}",
            panel
        );
        assert!(
            panel.contains(r#"<details class="sp-pr-list-box" open>"#),
            "{}",
            panel
        );
        assert!(
            panel.contains(r#"<span class="sp-pr-list-count">(2)</span>"#),
            "{}",
            panel
        );
        assert_eq!(panel.matches(r#"<li class="sp-pr-entry""#).count(), 2);
        let header = render(&two, "header");
        assert!(
            header.contains("the navigation panel lists them"),
            "{}",
            header
        );
        assert!(
            !header.contains("sp-pr-entry") && !header.contains("facet-box"),
            "{}",
            header
        );

        // (A long list is collapsed at first.)
        let many = globals((1..=21).map(pr).collect(), true);
        assert!(render(&many, "panel").contains(r#"<details class="sp-pr-list-box">"#));

        for (sparse, says) in [
            (
                globals(vec![], true),
                "None of whatwg/gizmo's open pull requests",
            ),
            (globals(vec![], false), "haven't been synced yet"),
        ] {
            assert_eq!(render(&sparse, "panel").trim(), "");
            assert!(render(&sparse, "header").contains(says));
        }

        // (A pull request whose first commits are another's, and the other's.)
        let mut stacked = pr(2);
        stacked["contains"] = serde_json::json!({
            "number": 1,
            "try_path": "/specs/try/gizmo/1",
            "before": "Its first commit is ",
            "after": "'s; only its own changes are shown here.",
        });
        let mut base = pr(1);
        base["contained_by"] = serde_json::json!({
            "prs": [{ "number": 2, "try_path": "/specs/try/gizmo/2" }, { "number": 3, "try_path": "/specs/try/gizmo/3" }],
            "before": "Its commits are in ",
            "after": " too, which show only their own changes.",
        });
        let panel = render(&globals(vec![stacked, base], true), "panel");
        assert!(
            panel.contains(r#"<span class="sp-pr-note sp-pr-contains">Its first commit is <a href="/specs/try/gizmo/1">#1</a>&#39;s; only its own changes are shown here.</span>"#),
            "{}",
            panel
        );
        assert!(
            panel.contains(r#"<span class="sp-pr-note sp-pr-contained">Its commits are in <a href="/specs/try/gizmo/2">#2</a> and <a href="/specs/try/gizmo/3">#3</a> too, which show only their own changes.</span>"#),
            "{}",
            panel
        );
    }

    /// What the entries say about pull requests whose first commits are
    /// others' (see `containment_note`), and others'.
    #[test]
    fn test_containment_notes() {
        use crate::spec_prs::{ContainedHow, Containment};
        let containment = |matched, of, own, how| Containment {
            number: 1940,
            matched,
            of,
            own,
            base: "a".repeat(40),
            how,
        };
        let note = |c: &Containment| {
            let (before, after) = super::containment_note(c);
            format!("{}#1940{}", before, after)
        };
        assert_eq!(
            note(&containment(1, 1, 2, ContainedHow::Commits)),
            "Its first commit is #1940's; only its own changes are shown here."
        );
        assert_eq!(
            note(&containment(2, 2, 2, ContainedHow::Commits)),
            "Its commits are #1940's; it has no changes of its own."
        );
        assert_eq!(
            note(&containment(2, 3, 4, ContainedHow::Changes)),
            "Its first 2 commits are versions of #1940's first 2 (of 3), with the same changes; only its own changes are shown here."
        );
        assert_eq!(
            note(&containment(1, 2, 2, ContainedHow::Messages)),
            "Its first commit is an earlier version of #1940's first (of 2), by its message; only its own changes are shown here."
        );
        let partial = containment(1, 2, 2, ContainedHow::Messages);
        let full = containment(1, 1, 3, ContainedHow::Commits);
        assert_eq!(
            super::contained_by_note(&[&full]),
            (
                "Its commits are in ".to_string(),
                " too, which shows only its own changes.".to_string()
            )
        );
        assert_eq!(
            super::contained_by_note(&[&full, &partial]).0,
            "Some of its commits (or versions of them) are in "
        );
    }

    #[test]
    fn test_annotate_spec_symbols() {
        let specs: crate::spec_analyze::SpecsConfig = toml::from_str(
            r#"
            [[spec]]
            name = "html"
            format = "wattsi"
            [[spec]]
            name = "infra"
            format = "bikeshed"
            source = "infra.bs"
            "#,
        )
        .unwrap();
        let html_spec = &specs.specs[0];
        // (The tree has the symbols but `nope`'s.)
        let has_sym = |sym: &str| !sym.ends_with("_nope");
        let mut syms = vec![];
        let annotated = annotate_spec_symbols(
            concat!(
                r##"<p><dfn id="policy-container">policy container</dfn> is a "##,
                r##"<a id="x:struct" href="/t/rendered/infra/infra.bs#struct">struct</a>, "##,
                r##"<a href="#policy-container">again</a>, "##,
                r##"<a href="/t/rendered/html/source/anchor/concept-fetch">fetch</a>, "##,
                r##"<a href="/t/rendered/html/source/dom.html">a page</a>, "##,
                r##"<a href="#nope">nope</a>.</p>"##,
                // (A heading's `<dfn>` without an id is the heading's.)
                r##"<h5 id="data-state"><span class="sp-secno">1</span> <dfn>Data state</dfn></h5>"##,
                r##"<p><dfn>no id</dfn></p>"##
            ),
            "t",
            &specs,
            html_spec,
            &has_sym,
            &mut syms,
        );
        assert_eq!(
            annotated,
            concat!(
                r##"<p><dfn data-symbols="SPEC_html_policy@2Dcontainer" id="policy-container">policy container</dfn> is a "##,
                r##"<a data-symbols="SPEC_infra_struct" data-rendered-def="/t/rendered/infra/infra.bs#struct" id="x:struct" href="/t/rendered/infra/infra.bs#struct">struct</a>, "##,
                r##"<a data-symbols="SPEC_html_policy@2Dcontainer" data-rendered-def="#policy-container" href="#policy-container">again</a>, "##,
                r##"<a data-symbols="SPEC_html_concept@2Dfetch" data-rendered-def="/t/rendered/html/source/anchor/concept-fetch" href="/t/rendered/html/source/anchor/concept-fetch">fetch</a>, "##,
                r##"<a href="/t/rendered/html/source/dom.html">a page</a>, "##,
                r##"<a href="#nope">nope</a>.</p>"##,
                r##"<h5 id="data-state"><span class="sp-secno">1</span> <dfn data-symbols="SPEC_html_data@2Dstate">Data state</dfn></h5>"##,
                r##"<p><dfn>no id</dfn></p>"##
            )
        );
        assert_eq!(syms.len(), 5);
    }

    #[test]
    fn test_spec_for_url() {
        let specs: crate::spec_analyze::SpecsConfig = toml::from_str(
            r#"
            [[spec]]
            name = "example"
            format = "wattsi"
            rendered = "https://example.spec.whatwg.org/"
            [[spec]]
            name = "example-2"
            rendered = "https://example.spec.whatwg.org/2/"
            [[spec]]
            name = "grid"
            format = "bikeshed"
            rendered = "https://drafts.csswg.org/css-grid-2/"
            aliases = ["https://drafts.csswg.org/css-grid/"]
            "#,
        )
        .unwrap();
        let name = |url: &str| spec_for_url(&specs, url).map(|spec| spec.name.as_str());
        assert_eq!(name("https://example.spec.whatwg.org/"), Some("example"));
        assert_eq!(name("http://Example.Spec.WHATWG.org"), Some("example"));
        // (The longest URL it's under: a later level's, not its series'.)
        assert_eq!(
            name("https://example.spec.whatwg.org/2/"),
            Some("example-2")
        );
        assert_eq!(name("https://drafts.csswg.org/css-grid/"), Some("grid"));
        assert_eq!(name("https://drafts.csswg.org/css-gridx/"), None);
        assert_eq!(name("https://example.org/"), None);
    }

    #[test]
    fn test_bug_summary() {
        assert_eq!(
            super::bug_summary(
                "Bug 2083426 - Defer throwing webglcontextcreationerror. r=gfx-reviewers,ahale"
            ),
            "Defer throwing webglcontextcreationerror."
        );
        assert_eq!(
            super::bug_summary("bug 123: Part 2: fix it r?foo a=bar"),
            "Part 2: fix it"
        );
        assert_eq!(super::bug_summary("Bug 5"), "");
        let long = format!("Bug 6 - {}", "word ".repeat(100));
        assert!(super::bug_summary(&long).ends_with("word\u{2026}"));
        assert_eq!(
            super::leading_bug("Bug 2083426 - X (bug 777)").as_deref(),
            Some("2083426")
        );
        assert_eq!(super::leading_bug("No bug - X (bug 777)"), None);
    }

    /// The bugs which landed most recently, newest first: a bug's commits
    /// counted, and its newest summary; merges skipped; a bug whose newest
    /// commit is a backout left out, unless it relanded since; and only the
    /// last `LANDING_DAYS`.
    #[test]
    fn test_recent_bugs() {
        use git2::{Oid, Repository};
        let dir = std::env::temp_dir().join(format!("recent-bugs-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let repo = Repository::init(&dir).unwrap();
        let tree_oid = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let day = 24 * 60 * 60;
        let start = 1_700_000_000;
        let mut head: Option<Oid> = None;
        let mut commit = |message: &str, days: i64, extra_parent: Option<Oid>| -> Oid {
            let sig = git2::Signature::new(
                "A",
                "a@example.com",
                &git2::Time::new(start + days * day, 0),
            )
            .unwrap();
            let parents: Vec<git2::Commit> = head
                .iter()
                .chain(extra_parent.iter())
                .map(|p| repo.find_commit(*p).unwrap())
                .collect();
            let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
            let oid = repo
                .commit(None, &sig, &sig, message, &tree, &parent_refs)
                .unwrap();
            head = Some(oid);
            oid
        };
        commit("Bug 1001 - Too old. r=x", 0, None);
        commit("Bug 1002 - Part 1: first. r=x", 20, None);
        commit("Bug 1002 - Part 2: second. r=x", 21, None);
        commit("Bug 1003 - Landed. r=x", 22, None);
        commit("Bug 1005 - Follow-up to bug 1006. r=x", 22, None);
        let side = commit("Bug 1004 - Relanded later. r=x", 23, None);
        commit("Backed out changeset abc (Bug 1003) for failures", 24, None);
        commit("Merge autoland to mozilla-central a=merge", 25, Some(side));
        let head = commit("Bug 1004 - Relanded. r=x", 26, None);
        let bugs = super::recent_bugs(&repo, head, 10);
        let got: Vec<(&str, &str, usize)> = bugs
            .iter()
            .map(|b| (b.bug.as_str(), b.summary.as_str(), b.commits))
            .collect();
        assert_eq!(
            got,
            vec![
                ("1004", "Relanded.", 2),
                ("1005", "Follow-up to bug 1006.", 1),
                ("1002", "Part 2: second.", 2)
            ]
        );
        assert_eq!(super::recent_bugs(&repo, head, 1).len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_citation_extent() {
        let extent = |source: &str, first: usize, next: Option<usize>| -> Vec<String> {
            let lines: Vec<String> = source.lines().map(str::to_string).collect();
            let len = super::citation_extent(&lines, first, next);
            lines[first..first + len].to_vec()
        };
        // (TransformStream::CreateGeneric's, as of 2026-10.)
        let create = r#"already_AddRefed<TransformStream> TransformStream::CreateGeneric(
    const GlobalObject& aGlobal, ErrorResult& aRv) {
  // Step 4. Let readableSizeAlgorithm be an algorithm that returns 1.
  // Note: Callers should recognize nullptr as a callback that returns 1.
  RefPtr<QueuingStrategySize> readableSizeAlgorithm;

  // Step 5. Let transformAlgorithmWrapper be an algorithm that runs these steps
  // given a value chunk:
  // Step 6. Let flushAlgorithmWrapper be an algorithm that runs these steps:
  // (Done by TransformerAlgorithmsWrapper)

  // Step 8. Perform ! InitializeTransformStream(stream, startPromise,
  // writableHighWaterMark, writableSizeAlgorithm, readableHighWaterMark,
  // readableSizeAlgorithm).
  RefPtr<TransformStream> stream =
      new TransformStream(global, nullptr, nullptr);
  stream->Initialize(aGlobal.Context(), startPromise, writableHighWaterMark,
                     writableSizeAlgorithm, readableHighWaterMark,
                     readableSizeAlgorithm, aRv);
  if (aRv.Failed()) {
    return nullptr;
  }

  return stream.forget();
}

void Other() {}
"#;
        // A step's comment, and code, up to the next step's.
        assert_eq!(extent(create, 2, Some(5)).len(), 3);
        // (Its comment, if the next step's follows it.)
        assert_eq!(extent(create, 6, Some(8)).len(), 2);
        assert_eq!(extent(create, 8, Some(11)).len(), 2);
        // Comments and blank lines don't end it, nor 8 lines: the rest of the
        // block does.
        let step8 = extent(create, 11, None);
        assert_eq!(
            step8.last().map(String::as_str),
            Some("  return stream.forget();")
        );
        // A citation before a definition is of it, to its end, or its first
        // step's comment.
        let whole = "// https://example.org/#count\nint Count(\n    int a) {\n  int b = a;\n\n  return b;\n}\n\nvoid Other() {}\n";
        assert_eq!(extent(whole, 0, None).len(), 7);
        let lines: Vec<String> = create.lines().map(str::to_string).collect();
        let stepped = ["// https://streams.spec.whatwg.org/#transformstream-set-up".to_string()]
            .into_iter()
            .chain(lines)
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(extent(&stepped, 0, None).len(), 3);
        // A block comment's citation, before a definition.
        let block = "/**\n * Steps 1 and 2 are handled by LookupCustomElementDefinition.\n */\nFoo* Lookup(int a) {\n  return nullptr;\n}\n\nint b;\n";
        assert_eq!(extent(block, 1, None).len(), 5);
        // A step inside a block ends with it.
        let nested = "  if (a) {\n    // Step 3. Throw.\n    return Err();\n  }\n  Next();\n";
        assert_eq!(extent(nested, 1, None).len(), 2);
        // A step of a statement goes on after it (to the next step's).
        let statement = "  // Step 2. For each x, frob it, then wibble.\n  for (auto x : xs) {\n    Frob(x);\n  }\n  Wibble();\n  // Step 3.\n";
        assert_eq!(extent(statement, 0, None).len(), 5);
    }

    #[test]
    fn test_redirect_target() {
        use super::RevKnowledge::*;
        let names: Vec<String> = ["beta", "release"].iter().map(|n| n.to_string()).collect();
        let others = |knowledge: [super::RevKnowledge; 2]| names.iter().zip(knowledge);
        // A revision this tree's repo has but its blame and history don't
        // goes to the first tree whose do.
        assert_eq!(
            super::redirect_target(Unknown, others([Unknown, Known])),
            Some("release".to_string())
        );
        assert_eq!(
            super::redirect_target(Unknown, others([Unknown, Missing])),
            None
        );
        // Not if this tree has it, can't tell, or its repo doesn't have it.
        for this in [Known, CantTell, Missing] {
            assert_eq!(super::redirect_target(this, others([Known, Known])), None);
        }
    }

    use super::*;
    use crate::cmd_pipeline::facets::FacetValueView;

    #[test]
    fn test_split_strings_at_records() {
        use crate::file_format::analysis::Location;
        let input = "s = \"<a data-l10n-id='x-y'>\";\nt = \"bc\";\nu = \"bc\";\nv = \"\u{e9}a\";\n";
        let FormatAs::FormatCLike(spec) = languages::select_formatting("a.js") else {
            panic!("JS isn't C-like");
        };
        let record = |lineno, col_start, col_end| WithLocation {
            loc: Location {
                lineno,
                col_start,
                col_end,
            },
            data: vec![],
        };
        // Records inside a string, at the start of one, covering one (which
        // stays whole, as does the overlapping record inside it), and inside
        // an "é" (which doesn't split it).
        let analysis = [
            record(1, 22, 25),
            record(2, 4, 6),
            record(3, 4, 8),
            record(3, 5, 6),
            record(4, 6, 7),
        ];
        let tokens =
            split_strings_at_records(input, tokenize::tokenize_c_like(input, spec), &analysis);
        let strings: Vec<&str> = tokens
            .iter()
            .filter(|t| t.kind == tokenize::TokenKind::StringLiteral)
            .map(|t| &input[t.start..t.end])
            .collect();
        assert_eq!(
            strings,
            [
                "\"<a data-l10n-id='",
                "x-y",
                "'>\"",
                "\"b",
                "c\"",
                "\"bc\"",
                "\"\u{e9}a\""
            ]
        );
    }

    #[test]
    fn test_split_plain_text_at_records() {
        use crate::file_format::analysis::Location;
        let input = "See {ref}`a-label` and [b](b.md).\nNothing here.\n";
        let record = |lineno, col_start, col_end| WithLocation {
            loc: Location {
                lineno,
                col_start,
                col_end,
            },
            data: vec![],
        };
        let analysis = [record(1, 10, 17), record(1, 27, 31)];
        let tokens = split_strings_at_records(input, tokenize::tokenize_plain(input), &analysis);
        let pieces: Vec<(&str, bool)> = tokens
            .iter()
            .filter(|t| t.kind != tokenize::TokenKind::Newline)
            .map(|t| {
                (
                    &input[t.start..t.end],
                    t.kind == tokenize::TokenKind::Identifier(None),
                )
            })
            .collect();
        assert_eq!(
            pieces,
            [
                ("See {ref}`", false),
                ("a-label", true),
                ("` and [b](", false),
                ("b.md", true),
                (").", false),
                ("Nothing here.", false)
            ]
        );
    }

    #[test]
    fn test_split_comments_at_records() {
        use crate::file_format::analysis::Location;
        let input = "int x;  // See [SMDOC] X Y here.\n";
        let FormatAs::FormatCLike(spec) = languages::select_formatting("a.cpp") else {
            panic!("C++ isn't C-like");
        };
        let analysis = [WithLocation {
            loc: Location {
                lineno: 1,
                col_start: 23,
                col_end: 26,
            },
            data: vec![],
        }];
        let tokens =
            split_strings_at_records(input, tokenize::tokenize_c_like(input, spec), &analysis);
        let comments: Vec<&str> = tokens
            .iter()
            .filter(|t| t.kind == tokenize::TokenKind::Comment)
            .map(|t| &input[t.start..t.end])
            .collect();
        assert_eq!(comments, ["// See [SMDOC] ", "X Y", " here."]);
    }

    #[test]
    fn test_message_diff_html() {
        assert_eq!(
            message_diff_html("Bug 1 - Fix. r=a", "Bug 1 - Fix. r=a"),
            ""
        );
        assert_eq!(
            message_diff_html("Bug 1 - Fix <x>. r=a,b", "Bug 1 - Fix <x>. r=a,c,d"),
            r#"Bug 1 - Fix &lt;x>. r=a,<del class="idiff-only-a">b</del><ins class="idiff-new">c,d</ins>"#
        );
        // Unchanged runs of lines away from the changes are elided.
        let a = "Bug 1 - One.\n\n1\n2\n3\n4\n5\n6\n7\n8";
        let b = "Bug 1 - One.\n\n1\n2\n3\n4\n5\n6\n7\n8\n\nBug 1 - Two.";
        assert_eq!(
            message_diff_html(a, b),
            "<span class=\"interdiff-messages-gap\">⋯ 8 unchanged lines</span>\n7\n8\n\n<ins class=\"idiff-new\">Bug 1 - Two.</ins>"
        );
    }

    fn explore_file(path: &str) -> ExploreFile {
        ExploreFile {
            path: path.to_string(),
            link: String::new(),
            sparkline: String::new(),
            symbols: vec![],
            same: false,
            note: String::new(),
            excerpts: String::new(),
            facets: String::new(),
            groups: String::new(),
            title: String::new(),
            outline: String::new(),
        }
    }

    #[test]
    fn test_explore_facets() {
        let toml = r#"
[pathkind.normal]
name = "Core code"
decision_order = 0
sort_order = 0

[pathkind.test]
name = "Test files"
decision_order = 1
sort_order = 1

[pathkind.test.heuristics]
dir_names = ["tests"]
"#;
        let json = br#"{
            "dom/base/a.cpp": {"path_kind": "normal", "is_dir": false, "subsystem": "Core/DOM"},
            "dom/base/b.cpp": {"path_kind": "normal", "is_dir": false, "subsystem": "Core/DOM"},
            "dom/base/c.cpp": {"path_kind": "normal", "is_dir": false, "subsystem": "Core/Layout"},
            "dom/tests/t.html": {"path_kind": "test", "is_dir": false, "subsystem": "Core/DOM"}
        }"#;
        let map = PathFacetMap::parse(json, toml).unwrap();
        // (A file the indexed revision doesn't have, in a test directory.)
        let files = [
            "dom/tests/gone.html",
            "dom/base/c.cpp",
            "dom/base/a.cpp",
            "dom/tests/t.html",
            "dom/base/b.cpp",
        ]
        .into_iter()
        .map(explore_file)
        .collect();
        let (facets, groups) = explore_facets_with(Some(&map), files);

        // Grouped by path kind, in the kinds' order, and then by path.
        let grouped: Vec<(&str, Vec<&str>)> = groups
            .iter()
            .map(|group| {
                (
                    group.name.as_str(),
                    group.files.iter().map(|f| f.path.as_str()).collect(),
                )
            })
            .collect();
        assert_eq!(
            grouped,
            vec![
                (
                    "Core code",
                    vec!["dom/base/a.cpp", "dom/base/b.cpp", "dom/base/c.cpp"]
                ),
                (
                    "Test files",
                    vec!["dom/tests/gone.html", "dom/tests/t.html"]
                ),
            ]
        );
        let names = |key: &str| -> Vec<(String, u32)> {
            fn walk(values: &[FacetValueView], out: &mut Vec<(String, u32)>) {
                for value in values {
                    out.push((value.name.clone(), value.count));
                    walk(&value.values, out);
                }
            }
            let mut out = vec![];
            walk(
                &facets.iter().find(|f| f.key == key).unwrap().values,
                &mut out,
            );
            out
        };
        assert_eq!(
            names("kind"),
            vec![("Core code".to_string(), 3), ("Test files".to_string(), 2)]
        );
        // Products, their components, and the unknown subsystem last.
        assert_eq!(
            names("subsystem"),
            vec![
                ("Core".to_string(), 4),
                ("DOM".to_string(), 3),
                ("Layout".to_string(), 1),
                ("Unknown".to_string(), 1),
            ]
        );
        // The directories share "dom/", which is collapsed, and those with
        // fewer than 3 files are "dom/ other".
        assert_eq!(
            names("dir"),
            vec![("dom/base/".to_string(), 3), ("dom/ other".to_string(), 2)]
        );

        // facets.js's data: the values each file is in (with their
        // ancestors), and its group for each way of grouping.
        let gone = &groups[1].files[0];
        let facets: serde_json::Value = serde_json::from_str(&gone.facets).unwrap();
        assert_eq!(
            facets,
            json!({"kind": ["test"], "subsystem": ["?"], "dir": ["dom/*"]})
        );
        let file_groups: serde_json::Value = serde_json::from_str(&gone.groups).unwrap();
        assert_eq!(file_groups["kind"], json!([1, "Test files"]));
        assert_eq!(file_groups["subsystem"][1], "Unknown subsystem");
        assert!(gone.title.contains("guessed"));
        let a: serde_json::Value = serde_json::from_str(&groups[0].files[0].facets).unwrap();
        assert_eq!(a["subsystem"], json!(["Core/", "Core/DOM"]));
        assert_eq!(groups[0].files[0].title, "Subsystem: Core :: DOM");
    }
}
