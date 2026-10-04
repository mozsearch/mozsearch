use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fmt::Write as _;
use std::io::Write;
use std::ops::{Deref, Range};
use std::path::Path;
use std::process::Command;
use std::rc::Rc;
use std::time::Instant;

use crate::abstract_server::FileMatch;
use crate::blame;
use crate::commit_index::{CommitIndex, CommitRef};
use crate::file_format::analysis_manglings::make_file_sym_from_path;
use crate::file_format::bisectable_mmap::BisectableMmap;
use crate::file_format::code_coverage_report;
use crate::file_format::coverage::InterpolatedCoverage;
use crate::file_format::jumpref::{
    JumprefData, JumprefTraversals, determine_desired_extra_syms_from_jumpref,
    extra_syms_next_step_lookups,
};
use crate::file_format::repo_data_ingestion::ConcisePerFileInfo;
use crate::git_ops::{self, coverage_history, coverage_summary, git_time_to_chrono};
use crate::hyperblame::explore::{self, ExploreCommit, blot_files, commit_changes};
use crate::hyperblame::future;
use crate::hyperblame::interdiff::{
    self, IdToken, Mark as InterdiffMark, Side as InterdiffSideTokens, TokenId,
};
use crate::hyperblame::page_blame::{CommitMeta, PageBlame, page_blame};
use crate::hyperblame::page_data_cache::{PAGE_DATA_CACHE, PageData};
use crate::hyperblame::peephole::{self, Cursor, peephole_page};
use crate::hyperblame::token_blame::{TreeHistory, blame_tokens};
use crate::languages;
use crate::languages::FormatAs;
use crate::links;
use crate::templating::builder::{
    build_and_parse_coverage_history, build_and_parse_dir_listing, build_and_parse_explore,
};
use crate::tokenize;
use crate::utils::OwnedOrBorrowed;

use crate::file_format::analysis::{
    AnalysisSource, ExpansionInfo, WithLocation, collect_file_syms_from_source,
};
use crate::file_format::config::{Config, GitData, TreeConfig};
use crate::output::{
    self, BreadcrumbsLinksTo, F, Options, PanelItem, PanelItemLabel, PanelSection, RevisionData,
};
use crate::url_encode_path::url_encode_path;

use git2::{Oid, Repository, Tree, TreeEntry};
use itertools::Itertools;
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

/// Renders source code into a Vec of HTML-formatted lines wrapped in `FormattedLine` objects that
/// provide the metadata for the position:sticky post-processing step.  Caller is responsible
/// for generating line numbers and any blame information.
pub fn format_code(
    cfg: Option<&Config>,
    jumpref_lookup: &Option<BisectableMmap<JumprefData>>,
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
        FormatAs::FormatCLike(spec) => tokenize::tokenize_c_like(input, spec),
        FormatAs::FormatXPIDL(spec, cdata_spec) => {
            tokenize::tokenize_xpidl(input, spec, cdata_spec)
        }
        FormatAs::FormatTagLike(script_spec) => tokenize::tokenize_tag_like(input, script_spec),
    };

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
    if let Some(lookup) = jumpref_lookup {
        let file_sym = make_file_sym_from_path(path);
        if let Ok(jumpref) = lookup.lookup(&file_sym) {
            generated_sym_info.insert(ustr(&file_sym), jumpref);
            jumpref_traversed.insert(ustr(&file_sym), JumprefTraversals::empty());
        }
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
            | (&tokenize::TokenKind::StringLiteral, Some(d)) => {
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

                        if !a.no_crossref
                            && let Some(lookup) = jumpref_lookup
                            && let Ok(jumpref) = lookup.lookup(sym)
                        {
                            // See if there are any binding slot symbols that we should also
                            // include.  This allows us to do things like, when presenting a
                            // context menu for a synthetic XPIDL symbol, we can also provide an
                            // option to go directly to the C++ binding definition.
                            let mut extra_syms =
                                determine_desired_extra_syms_from_jumpref(jumpref.as_ref());
                            jumpref_traversed
                                .entry(*sym)
                                .and_modify(|t| *t |= JumprefTraversals::NormalExtra)
                                .or_insert(JumprefTraversals::NormalExtra);
                            while let Some((extra_sym, next_step)) = extra_syms.pop() {
                                // No need to lookup and add what we already know if there is
                                // no next step.  But if there is a next step, we potentially
                                // need to look-up a third symbol which may not already have
                                // been loaded.)
                                let extra_sym = ustr(&extra_sym);
                                if let Some(extra_traversed) = jumpref_traversed.get_mut(&extra_sym)
                                {
                                    // The jumpref should already be in generated_sym_info, it's
                                    // just a question if we need to run an extra traversal for it.
                                    if extra_traversed.contains(next_step) {
                                        continue;
                                    }
                                    *extra_traversed |= next_step;
                                    if let Some(extra_jumpref) = generated_sym_info.get(&extra_sym)
                                    {
                                        for (next_sym, next_traversals) in
                                            extra_syms_next_step_lookups(
                                                extra_jumpref.as_ref(),
                                                next_step,
                                            )
                                        {
                                            extra_syms.push((next_sym, next_traversals));
                                        }
                                    }
                                } else if let Ok(extra_jumpref) = lookup.lookup(&extra_sym) {
                                    // If there is a next step, process the info for what to contribute
                                    // to extra_syms before we consume the value by storing it.
                                    if !next_step.is_empty() {
                                        for (next_sym, next_traversals) in
                                            extra_syms_next_step_lookups(
                                                extra_jumpref.as_ref(),
                                                next_step,
                                            )
                                        {
                                            extra_syms.push((next_sym, next_traversals));
                                        }
                                    }
                                    jumpref_traversed.insert(extra_sym, next_step);
                                    generated_sym_info.insert(extra_sym, extra_jumpref);
                                }
                            }
                            generated_sym_info.insert(*sym, jumpref);
                        }
                    }
                }
            }
            _ => {}
        }

        let get_symbols =
            |token: &tokenize::Token, datum: &mut dyn Iterator<Item = &AnalysisSource>| {
                match &token.kind {
                    &tokenize::TokenKind::Identifier(_) | &tokenize::TokenKind::StringLiteral => {
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
            match token.kind {
                tokenize::TokenKind::Identifier(ref maybe_style) => {
                    let mut has_datum = false;
                    let classes = datum.flat_map(|a| {
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
            }
        };

        // Only get the symbols and style of the symbols that appear directly in the source code, not in expansions
        let datum_outside_expansions = datum.iter().flat_map(|d| d.iter());
        let has_expansion = |data: &AnalysisSource| {
            matches!(data.expansion_info, Some(ExpansionInfo::ExpandsTo(_)))
        };
        let (symbols, style) = if datum_outside_expansions.clone().any(has_expansion) {
            let symbols = get_symbols(
                &token,
                &mut datum_outside_expansions
                    .clone()
                    .filter(|&a| has_expansion(a)),
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
pub fn format_file_data(
    cfg: &Config,
    tree_name: &str,
    mut panel: Vec<PanelSection>,
    info_boxes: String,
    commit: &Option<git2::Commit>,
    breadcrumbs_links_to: BreadcrumbsLinksTo,
    blame_commit: &Option<git2::Commit>,
    coverage_commit: Option<&git2::Commit>,
    path: &str,
    data: String,
    jumpref_lookup: &Option<BisectableMmap<JumprefData>>,
    analysis: &[WithLocation<Vec<AnalysisSource>>],
    writer: &mut dyn Write,
) -> Result<FormattedFile, &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;

    let coverage = git_ops::get_coverage(tree_config.git.as_ref(), coverage_commit, path);

    let mut format_perf = FormatPerfInfo::default();

    let format = languages::select_formatting(path);
    if let FormatAs::Binary = format {
        write!(writer, "Binary file").unwrap();
        return Ok(FormattedFile {
            perf: format_perf,
            token_blame: None,
        });
    };

    let slug = format_to_slug_attribute(&format);
    let pre_format_code = Instant::now();
    let (output_lines, sym_json) =
        format_code(Some(cfg), jumpref_lookup, format, path, &data, analysis);
    format_perf.format_code_duration_us = pre_format_code.elapsed().as_micros() as u64;

    let pre_blame_lines = Instant::now();
    // We use the token-centric history if the tree has it (and it has the
    // file), and otherwise the classic line blame; trees only have one or the
    // other (see `config::git_data`).
    let mut token_blame = tree_config
        .git
        .as_ref()
        .zip(commit.as_ref())
        .and_then(|(git, commit)| page_token_blame(git, commit, path, &data))
        .filter(|page| {
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
    if let (Some(page), Some(commit)) = (&mut token_blame, commit) {
        set_token_blame_urls(page, tree_name, commit, path, breadcrumbs_links_to);
    }
    let blame_lines = match token_blame {
        Some(_) => None,
        None => git_ops::get_blame_lines(tree_config.git.as_ref(), blame_commit, path),
    };
    format_perf.blame_lines_duration_us = pre_blame_lines.elapsed().as_micros() as u64;

    let pre_commit = Instant::now();
    let revision_owned = match *commit {
        Some(ref commit) => {
            let rev = commit.id().to_string();
            let header = blame::commit_header(commit)?;
            let date = git_time_to_chrono(commit.time());
            Some((rev, header, date))
        }
        None => None,
    };
    let revision = match revision_owned {
        Some((ref rev, ref header, date)) => Some(RevisionData {
            rev: rev.as_str(),
            desc: header.as_str(),
            date,
        }),
        None => None,
    };
    format_perf.commit_info_duration_us = pre_commit.elapsed().as_micros() as u64;

    let pre_format_mixing = Instant::now();

    let path_wrapper = Path::new(path);
    let filename = path_wrapper.file_name().unwrap().to_str().unwrap();

    let title = format!("{} - mozsearch", filename);
    let opt = Options {
        title: &title,
        tree_name,
        include_date: env::var("MOZSEARCH_DIFFABLE").is_err(),
        revision,
        breadcrumbs_links_to,
        extra_content_classes: "source-listing not-diff",
    };

    output::generate_header(&opt, writer)?;

    let file_syms = collect_file_syms_from_source(path, analysis);

    output::generate_breadcrumbs(&opt, writer, path, &file_syms, !analysis.is_empty())?;

    let coverage_history = coverage_history(cfg.trees[tree_name].git.as_ref(), path);
    if let Some(coverage_history) = coverage_history {
        let liquid_globals = liquid::object!({
            "tree": tree_name,
            "path": path,
            "coverage_history": coverage_history,
        });

        let template = build_and_parse_coverage_history();
        template
            .render_to(writer, &liquid_globals)
            .or(Err("Template problems"))?;

        let coverage_summary = coverage_commit.as_ref().and_then(|commit| {
            let coverage_rev = commit.id().to_string();
            git_ops::coverage_summary(tree_config.git.as_ref(), &coverage_rev, path)
        });
        add_coverage_panel_item(&mut panel, coverage_summary.as_ref());
    }
    output::generate_panel(&opt, writer, &panel, false)?;

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
        "<div id=\"file\" class=\"file\" role=\"table\"{}>",
        slug
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
                nest_depth, nest_sym
            )
            .unwrap();
            nest_depth += 1;
        }

        // Emit the actual source line here.
        let f = F::Seq(vec![
            F::T(format!(
                "<div role=\"row\" id=\"line-{}\" class=\"source-line-with-number{}\">",
                lineno,
                if line.sym_starts_nest.is_some() {
                    " nesting-sticky-line"
                } else {
                    ""
                }
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
        hyperblame_commits_json(tree_config, git, revs),
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
) -> String {
    let infos: Vec<serde_json::Value> = revs
        .map(|rev| {
            Oid::from_str(rev)
                .and_then(|oid| git.repo.find_commit(oid))
                .ok()
                .and_then(|commit| blame::commit_info_json(tree_config, git, &commit).ok())
                .unwrap_or(serde_json::Value::Null)
        })
        .collect();
    to_string(&infos).unwrap()
}

/// One of the hyperblame data files (see `hyperblame_files`) for the file at
/// `path` in revision `rev`, for pages of revisions other than the tip.  We
/// use the data rendering the page left in `PAGE_DATA_CACHE`, if it's there.
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
            let page = page_token_blame(git, &commit, path, &source)?;
            Some(PageData::new(&page))
        })
        .ok_or("No token blame for the file")?;

    if file_name == "commits.json" {
        let revs = || data.revs.iter().map(String::as_str);
        return Ok(data
            .commits_json
            .get_or_init(|| hyperblame_commits_json(tree_config, git, revs()))
            .clone());
    }
    file_name
        .strip_prefix("lines-")
        .and_then(|name| name.strip_suffix(".json"))
        .and_then(|k| k.parse::<usize>().ok())
        .and_then(|k| data.chunks.get(k).cloned())
        .ok_or("No such hyperblame file")
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
    PAGE_DATA_CACHE.insert(
        (
            tree_name.to_string(),
            commit.id().to_string(),
            path.to_string(),
        ),
        PageData::new(&page),
    );
    Some(page)
}

/// The token-centric blame of the file at `path` in `commit` whose contents are
/// `source`, if the tree's history has it.  See `hyperblame::token_blame`.
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
                CommitMeta {
                    time: commit.time().seconds(),
                    author: name.to_string(),
                }
            }
            None => CommitMeta {
                time: 0,
                author: String::new(),
            },
        }
    }))
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
pub fn format_path(
    cfg: &Config,
    tree_name: &str,
    rev: &str,
    path: &str,
    writer: &mut dyn Write,
) -> Result<(), &'static str> {
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
            let blob = git_ops::read_blob_object(&object);
            format_blob(
                cfg,
                tree_name,
                path.to_str().expect("This Path was built from a str"),
                writer,
                tree_config,
                git,
                commit,
                blob,
            )
        }
        Some(git2::ObjectType::Tree) => {
            let tree = object
                .into_tree()
                .expect("Should really be a tree, we just checked the object kind.");

            format_tree(tree_name, rev, path, writer, git, commit, &repo, tree)
        }
        _ => Err("Invalid path"),
    }
}

fn format_tree(
    tree_name: &str,
    rev: &str,
    path: &Path,
    writer: &mut dyn Write,
    git: &GitData,
    commit: git2::Commit<'_>,
    repo: &Repository,
    tree: Tree<'_>,
) -> Result<(), &'static str> {
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
                    .to_object(repo)
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
        }],
        raw_items: vec![],
    }];

    let coverage_history = coverage_history(Some(git), path);
    let coverage = git_ops::coverage_summary(Some(git), &coverage_rev, path);
    if coverage_history.is_some() {
        let coverage_commit = git.coverage_repo.as_ref().and_then(|repo| {
            repo.revparse_single(&format!("refs/tags/reverse/all/all/{}", commit.id()))
                .ok()
                .and_then(|object| object.peel_to_commit().ok())
        });
        let coverage_summary = coverage_commit.as_ref().and_then(|commit| {
            let coverage_rev = commit.id().to_string();
            git_ops::coverage_summary(Some(git), &coverage_rev, path)
        });
        add_coverage_panel_item(&mut panel, coverage_summary.as_ref());
    }

    let commit_hash = commit.id().to_string();
    let date = git_time_to_chrono(commit.time());
    let date = date.format("%F %T %z").to_string();
    let date = liquid::model::DateTime::from_str(&date).unwrap();

    let liquid_globals = liquid::object!({
        "tree": tree_name,
        // the header always needs this
        "query": "",
        "path": path,
        "files": files,
        "rev": rev,
        "rev_box": {
            "long": commit_hash,
            "short": &commit_hash[..8],
            "desc_html": desc_html,
            "date": date,
        },
        "coverage": coverage,
        "coverage_history": coverage_history,
        "panel": panel,
    });

    let template = build_and_parse_dir_listing();
    template
        .render_to(writer, &liquid_globals)
        .or(Err("Template problems"))
}

fn format_blob(
    cfg: &Config,
    tree_name: &str,
    path: &str,
    writer: &mut dyn Write,
    tree_config: &TreeConfig,
    git: &GitData,
    commit: git2::Commit<'_>,
    data: String,
) -> Result<(), &'static str> {
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

    let analysis = Vec::new();

    let hg_rev = tree_config
        .git
        .as_ref()
        .and_then(|git| git.hg_rev(commit.id()))
        .unwrap_or_else(|| "default".to_string());
    let hg_rev: &str = &hg_rev;

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
    });

    let gh_log_link = tree_config
        .paths
        .github_repo
        .as_ref()
        .map(|gh_root| format!("{}/commits/{}/{}", gh_root, commit.id(), encoded_path));
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
        });
    }

    if let Some(link) =
        tree_config
            .paths
            .make_raw_resource_rev_url(&commit.id().to_string(), hg_rev, path)
    {
        vcs_panel_items.push(PanelItem {
            label: PanelItemLabel::Plaintext("Raw".to_owned()),
            tooltip: "Open a raw file of the current file".to_owned(),
            id: "panel-raw",
            link,
            update_link_lineno: "",
            accel_key: Some('R'),
            copyable: true,
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
        });
    }
    let panel = vec![
        PanelSection {
            name: "Revision control".to_owned(),
            items: vcs_panel_items,
            raw_items: vec![],
        },
        create_markdown_panel_section(false),
    ];

    let rev = commit.id().to_string();
    let formatted = format_file_data(
        cfg,
        tree_name,
        panel,
        "".to_string(),
        &Some(commit),
        BreadcrumbsLinksTo::Historical,
        &blame_commit,
        coverage_commit.as_ref(),
        path,
        data,
        &None,
        &analysis,
        writer,
    )?;
    // The page will request its hyperblame data right away; see
    // `hyperblame_file`.
    if let Some(page) = formatted.token_blame {
        PAGE_DATA_CACHE.insert(
            (tree_name.to_string(), rev, path.to_string()),
            PageData::new(&page),
        );
    }
    Ok(())
}

pub fn create_markdown_panel_section(add_symbol_link: bool) -> PanelSection {
    let mut markdown_panel_items = vec![];
    markdown_panel_items.push(PanelItem {
        label: PanelItemLabel::Plaintext("Filename Link".to_owned()),
        tooltip: "Copy a Markdown link to clipboard, with the filename".to_owned(),
        id: "panel-copy-filename-link",
        link: String::new(),
        update_link_lineno: "",
        accel_key: Some('F'),
        copyable: true,
    });
    if add_symbol_link {
        markdown_panel_items.push(PanelItem {
            label: PanelItemLabel::Plaintext("Symbol Link".to_owned()),
            tooltip: "Copy a Markdown link to clipboard, with the selected symbol".to_owned(),
            id: "panel-copy-symbol-link",
            link: String::new(),
            update_link_lineno: "",
            accel_key: Some('S'),
            copyable: true,
        });
    }
    markdown_panel_items.push(PanelItem {
        label: PanelItemLabel::Plaintext("Code Block".to_owned()),
        tooltip: "Copy a Markdown code block of the selected code to clipboard".to_owned(),
        id: "panel-copy-code-block",
        link: String::new(),
        update_link_lineno: "",
        accel_key: Some('C'),
        copyable: true,
    });
    PanelSection {
        name: "Copy as Markdown".to_owned(),
        items: markdown_panel_items,
        raw_items: vec![],
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
pub fn format_diff(
    cfg: &Config,
    tree_name: &str,
    rev: &str,
    path: &str,
    writer: &mut dyn Write,
) -> Result<(), &'static str> {
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
        return format_path(cfg, tree_name, rev, path, writer);
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
    let mut token_blames: Vec<Option<PageBlame>> = match &git.history {
        Some(_) => std::iter::once(commit.clone())
            .chain(commit.parents())
            .map(|c| revision_token_blame(tree_name, git, &c, path))
            .collect(),
        None => vec![],
    };

    let line_blames = git.blame_repo.as_ref().map(|_| parent_blames.as_slice());
    let (rows, new_lines, num_lines) = parse_diff_rows(
        &difftxt,
        commit.parents().count(),
        self_blame.as_ref(),
        line_blames,
    )?;
    drop_mismatched_token_blames(path, &mut token_blames, &num_lines);

    if let FormatAs::Binary = languages::select_formatting(path) {
        return Err("Cannot diff binary file");
    };

    let header = blame::commit_header(commit)?;
    let date = git_time_to_chrono(commit.time());

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
        },
        PanelItem {
            label: PanelItemLabel::Plaintext("Show file without diff".to_owned()),
            tooltip: "Open the last revision without the current diff".to_owned(),
            id: "panel-vcs-without-diff",
            link: format!("/{}/rev/{}/{}", tree_name, rev, encoded_path),
            update_link_lineno: "#{}",
            accel_key: None,
            copyable: true,
        },
        PanelItem {
            label: PanelItemLabel::Plaintext("Go to latest version".to_owned()),
            tooltip: "Open the latest revision-agnostic link of the current file".to_owned(),
            id: "panel-vcs-latest",
            link: format!("/{}/source/{}", tree_name, encoded_path),
            update_link_lineno: "#{}",
            accel_key: None,
            copyable: false,
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
    /// `format_diff`'s `token_blames`).
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

/// What interdiffs add to the rows of a diff page (see `format_interdiff`):
/// attributes for the code of the rows of the lines of the file in the commit
/// and of the removed lines of the file in its (one) parent, by their
/// (1-based) lines, and rows to insert before the row of a line of the file in
/// the commit (or after the last row, for the number of lines + 1).
#[derive(Default)]
struct DiffRowExtras {
    line_attrs: HashMap<usize, String>,
    removed_line_attrs: HashMap<usize, String>,
    rows_before: BTreeMap<usize, Vec<String>>,
}

/// Write the rows of a diff page (see `parse_diff_rows`), with their blame
/// strips, and its `BLAME_INFOS` (see `format_diff`).  `new_lines` is the
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
    let (formatted_lines, _) = format_code(Some(cfg), &None, format, path, new_lines, &analysis);

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
    revs: HashSet<String>,
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
    explore::order_commits(repo, &mut refs);
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
    let revs = commits.iter().map(|c| c.id().to_string()).collect();
    Ok(InterdiffSide {
        commits,
        revs,
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
            blame
                .tokens
                .iter()
                .map(|t| IdToken {
                    id: (
                        t.data.introduced.source_rev.to_string(),
                        t.data.introduced.path.to_string(),
                        t.data.introduced.lineno,
                    ),
                    text: source[t.range.clone()].to_string(),
                    range: t.range.clone(),
                    line: t.line,
                })
                .collect()
        };
        Ok((source, tokens))
    }

    /// The tokens of the file at the side's base which the side's commits
    /// removed: those each commit which changed the file removed from it.
    fn removed(&mut self, side: &InterdiffSide) -> Result<HashSet<TokenId>, &'static str> {
        let entry = |commit: &git2::Commit| {
            commit
                .tree()
                .ok()
                .and_then(|tree| tree.get_path(Path::new(self.path)).ok())
                .map(|entry| entry.id())
        };
        let mut removed = HashSet::new();
        for commit in &side.commits {
            let parent = commit.parent(0).map_err(|_| "A commit has no parent")?;
            if entry(&parent) == entry(commit) {
                continue;
            }
            let (before, after) = (self.get(&parent)?, self.get(commit)?);
            removed.extend(interdiff::removed(&before.1, &after.1));
        }
        let base = self.get(&side.base)?;
        let base_ids: HashSet<&TokenId> = base.1.iter().map(|t| &t.id).collect();
        removed.retain(|id| base_ids.contains(id));
        Ok(removed)
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

/// The rows of A's lines with the tokens which only A added (see
/// `hyperblame::interdiff::Interdiff::only_a`), to go before the rows of B's
/// lines (see `DiffRowExtras::rows_before`): each goes after the row of B's
/// line with the nearest matched token before A's tokens.
fn interdiff_only_a_rows(
    a_source: &str,
    a_tokens: &[IdToken],
    only_a: &[(usize, Option<usize>)],
) -> BTreeMap<usize, Vec<String>> {
    fn entity_replace(s: &str) -> String {
        s.replace("&", "&amp;").replace("<", "&lt;")
    }
    let starts = line_starts(a_source);
    // A's lines with the tokens, with where they go and the tokens' byte ranges
    // in the line.
    let mut lines: BTreeMap<usize, (Option<usize>, Vec<Range<usize>>)> = BTreeMap::new();
    for &(i, anchor) in only_a {
        let token = &a_tokens[i];
        let start = starts[token.line];
        lines
            .entry(token.line)
            .or_insert((anchor, vec![]))
            .1
            .push(token.range.start - start..token.range.end - start);
    }
    let mut rows: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for (line, (anchor, ranges)) in lines {
        let end = starts.get(line + 1).map_or(a_source.len(), |next| next - 1);
        let text = &a_source[starts[line]..end];
        let mut content = String::new();
        let mut pos = 0;
        for range in ranges {
            content.push_str(&entity_replace(&text[pos..range.start]));
            write!(
                content,
                r#"<span class="idiff-only-a">{}</span>"#,
                entity_replace(&text[range.clone()])
            )
            .unwrap();
            pos = range.end;
        }
        content.push_str(&entity_replace(&text[pos..]));
        // After B's line `anchor` (0-based), so before line `anchor` + 2.
        rows.entry(anchor.map_or(1, |anchor| anchor + 2))
            .or_default()
            .push(format!(
                r#"<div role="row" class="source-line-with-number interdiff-only-a"><div class="line-strip"><div role="cell" class="blame-container"><div class="blame-strip"></div></div></div><div role="cell" class="line-number" data-line-number=""></div><code role="cell" class="source-line" title="Line {} of A's version, whose highlighted tokens B doesn't have">~ {}
</code></div>"#,
                line + 1,
                content
            ));
    }
    rows
}

/// The commits of a side of an interdiff as an HTML list.
fn interdiff_commits_html(tree_name: &str, side: &InterdiffSide) -> String {
    let mut html = String::from("<ol>");
    for commit in &side.commits {
        let header = blame::commit_header(commit).unwrap_or_default();
        write!(
            html,
            r#"<li><a href="/{}/commit/{}">{}</a> {}</li>"#,
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

/// The interdiff of the file at `path` between two versions of a patch (ex: a
/// landing and its reland), each a commit or a stack of them (comma-separated
/// revisions): B's diff of the file, from before its first commit to its last,
/// with its added and removed tokens marked by how they compare with A's (see
/// `hyperblame::interdiff`).
pub fn format_interdiff(
    cfg: &Config,
    tree_name: &str,
    a_revs: &str,
    b_revs: &str,
    path: &str,
    writer: &mut dyn Write,
) -> Result<(), &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let git = tree_config.get_git()?;
    let history = git
        .history
        .as_ref()
        .ok_or("Interdiffs need the token-centric history")?;
    let repo: &Repository = &git.repo;
    let a = interdiff_side(repo, a_revs)?;
    let b = interdiff_side(repo, b_revs)?;

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
    let (a_removed, b_removed) = (files.removed(&a)?, files.removed(&b)?);
    let result = interdiff::interdiff(
        &InterdiffSideTokens {
            revs: &a.revs,
            base: &a_base.1,
            post: &a_post.1,
            removed: &a_removed,
        },
        &InterdiffSideTokens {
            revs: &b.revs,
            base: &b_base.1,
            post: &b_post.1,
            removed: &b_removed,
        },
    );

    // B's diff of the file.  (The system git, with SHA-1 collision detection,
    // since this works on a source repo; see `fast_import_git` for our git.)
    let git_path = tree_config.get_git_path()?;
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
    let (rows, new_lines, num_lines) = parse_diff_rows(&difftxt, 1, None, None)?;
    let mut token_blames = vec![
        revision_token_blame(tree_name, git, b.post(), path),
        revision_token_blame(tree_name, git, &b.base, path),
    ];
    drop_mismatched_token_blames(path, &mut token_blames, &num_lines);
    if let FormatAs::Binary = languages::select_formatting(path) {
        return Err("Cannot diff binary file");
    };

    let extras = DiffRowExtras {
        line_attrs: interdiff_attrs(&b_post.0, &b_post.1, &result.post_marks),
        removed_line_attrs: interdiff_attrs(&b_base.0, &b_base.1, &result.base_marks),
        rows_before: interdiff_only_a_rows(&a_post.0, &a_post.1, &result.only_a),
    };

    let b_rev = b.post().id().to_string();
    let header = blame::commit_header(b.post())?;
    let filename = Path::new(path).file_name().unwrap().to_str().unwrap();
    let title = format!("{} interdiff - mozsearch", filename);
    let opt = Options {
        title: &title,
        tree_name,
        include_date: true,
        revision: Some(RevisionData {
            rev: &b_rev,
            desc: &header,
            date: git_time_to_chrono(b.post().time()),
        }),
        breadcrumbs_links_to: BreadcrumbsLinksTo::Historical,
        extra_content_classes: "source-listing diff interdiff",
    };
    output::generate_header(&opt, writer)?;
    let file_syms = vec![make_file_sym_from_path(path)];
    output::generate_breadcrumbs(&opt, writer, path, &file_syms, false)?;

    let encoded_path = url_encode_path(path);
    let (a_key, b_key) = (a.key(), b.key());
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
            },
            PanelItem {
                label: PanelItemLabel::Plaintext("All files".to_owned()),
                tooltip: "The files A or B changed".to_owned(),
                id: "panel-interdiff-files",
                link: format!("/{}/interdiff/{}/{}", tree_name, a_key, b_key),
                update_link_lineno: "",
                accel_key: None,
                copyable: true,
            },
            PanelItem {
                label: PanelItemLabel::Plaintext("Show B's version".to_owned()),
                tooltip: "Open the file as of B's last commit".to_owned(),
                id: "panel-interdiff-b",
                link: format!("/{}/rev/{}/{}", tree_name, b_rev, encoded_path),
                update_link_lineno: "#{}",
                accel_key: None,
                copyable: true,
            },
        ],
        raw_items: vec![],
    }];
    output::generate_panel(&opt, writer, &sections, false)?;

    let counts = &result.counts;
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    write!(
        writer,
        r#"<section class="interdiff-summary">
<div class="interdiff-sides"><div><h3>A</h3>{}</div><div><h3>B</h3>{}</div></div>
<p class="interdiff-legend">B's diff, compared with A's: <span class="idiff-new">{} token{} new in B</span>, <span class="idiff-same">{} the same as A's</span>, <span class="idiff-only-a">{} of A's which B doesn't have</span> (on rows of A's lines), <span class="idiff-rm-new">{} removed only by B</span>, <span class="idiff-rm-same">{} removed by both</span>, <span class="idiff-kept">{} removed by A but kept by B</span>, and <span class="idiff-base">{} from neither patch</span>.</p>
</section>
"#,
        interdiff_commits_html(tree_name, &a),
        interdiff_commits_html(tree_name, &b),
        counts.new,
        plural(counts.new),
        counts.same,
        counts.only_a,
        counts.removed_new,
        counts.removed_same,
        counts.kept,
        counts.base,
    )
    .unwrap();

    write_diff_rows(writer, cfg, path, &rows, &new_lines, &token_blames, &extras)?;

    output::generate_footer(&opt, tree_name, path, writer).unwrap();
    Ok(())
}

/// The files which either side of an interdiff changed (see `format_interdiff`),
/// with links to their interdiffs.
pub fn format_interdiff_files(
    cfg: &Config,
    tree_name: &str,
    a_revs: &str,
    b_revs: &str,
    writer: &mut dyn Write,
) -> Result<(), &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let git = tree_config.get_git()?;
    let repo: &Repository = &git.repo;
    let a = interdiff_side(repo, a_revs)?;
    let b = interdiff_side(repo, b_revs)?;

    // The files each side's commits changed.
    let mut paths: BTreeMap<String, [bool; 2]> = BTreeMap::new();
    for (i, side) in [&a, &b].into_iter().enumerate() {
        for commit in &side.commits {
            let parent_tree = commit.parent(0).and_then(|p| p.tree()).ok();
            let tree = commit.tree().map_err(|_| "Bad revision")?;
            let diff = repo
                .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)
                .map_err(|_| "Diff failed")?;
            for delta in diff.deltas() {
                for file in [delta.old_file(), delta.new_file()] {
                    if let Some(path) = file.path().and_then(|p| p.to_str()) {
                        paths.entry(path.to_string()).or_default()[i] = true;
                    }
                }
            }
        }
    }

    let title = "Interdiff - mozsearch";
    let opt = Options {
        title,
        tree_name,
        include_date: true,
        revision: None,
        breadcrumbs_links_to: BreadcrumbsLinksTo::Historical,
        extra_content_classes: "interdiff-files",
    };
    output::generate_header(&opt, writer)?;
    output::generate_panel(&opt, writer, &[], true)?;
    let (a_key, b_key) = (a.key(), b.key());
    write!(
        writer,
        r#"<section class="interdiff-summary">
<div class="interdiff-sides"><div><h3>A</h3>{}</div><div><h3>B</h3>{}</div></div>
<p>The files A or B changed, each with B's diff compared with A's (<a href="/{}/interdiff/{}/{}">swap A and B</a>):</p>
<ul class="interdiff-file-list">
"#,
        interdiff_commits_html(tree_name, &a),
        interdiff_commits_html(tree_name, &b),
        tree_name,
        b_key,
        a_key,
    )
    .unwrap();
    for (path, [in_a, in_b]) in &paths {
        let sides = match (in_a, in_b) {
            (true, true) => "",
            (true, false) => r#" <span class="interdiff-side-note">(only A)</span>"#,
            _ => r#" <span class="interdiff-side-note">(only B)</span>"#,
        };
        writeln!(
            writer,
            r#"<li><a href="/{}/interdiff/{}/{}/{}">{}</a>{}</li>"#,
            tree_name,
            a_key,
            b_key,
            url_encode_path(path),
            path.replace("&", "&amp;").replace("<", "&lt;"),
            sides
        )
        .unwrap();
    }
    writeln!(writer, "</ul>\n</section>").unwrap();
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

/// The most bugs or Phabricator revisions an `/explore/` page shows.
const MAX_EXPLORE_KEYS: usize = 10;

/// The `/explore/bug/BUGS` and `/explore/phab/REVS` pages, where `kind` is
/// "bug" or "phab" and `keys` is comma-separated: the commits which mention the
/// bugs (or Phabricator revisions), oldest first, and what they changed (see
/// `hyperblame::explore`).
pub fn format_explore(
    cfg: &Config,
    tree_name: &str,
    kind: &str,
    keys: &str,
    writer: &mut dyn Write,
) -> Result<(), &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;
    let git = tree_config.get_git()?;
    let history = git.history.as_ref();
    let index = history
        .and_then(|history| CommitIndex::open(&Path::new(&history.path).join("commit-index")))
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
    explore::order_commits(&git.repo, &mut refs);
    let truncated = refs.len() > explore::MAX_COMMITS;
    refs.truncate(explore::MAX_COMMITS);

    let mut commits: Vec<ExploreCommit> = refs
        .iter()
        .enumerate()
        .map(|(i, commit_ref)| {
            let header = Oid::from_str(&commit_ref.rev)
                .and_then(|oid| git.repo.find_commit(oid))
                .ok()
                .and_then(|commit| blame::commit_info_json(tree_config, git, &commit).ok())
                .and_then(|info| info["header"].as_str().map(str::to_string))
                .unwrap_or_else(|| commit_ref.rev.clone());
            // See `blame::commit_info_json` for the header's format.
            let summary = header.split("\n<br>").next().unwrap_or("").to_string();
            ExploreCommit {
                number: i + 1,
                rev: commit_ref.rev.clone(),
                iso_date: commit_ref.iso_date.clone(),
                backout: commit_ref.backout,
                interdiff: None,
                header,
                summary,
            }
        })
        .collect();
    let changes: Vec<_> = refs
        .iter()
        .map(|commit_ref| commit_changes(history, &git.repo, &commit_ref.rev))
        .collect();
    for (backout, backed_out, reland) in explore::relands(history, &refs, &changes) {
        let revs = |indices: &[usize]| indices.iter().map(|&i| refs[i].rev.as_str()).join(",");
        commits[backout].interdiff = Some(format!(
            "/{}/interdiff/{}/{}",
            tree_name,
            revs(&backed_out),
            revs(&reland)
        ));
    }
    let files = blot_files(&refs, &changes, |path, rev| {
        format!("/{}/rev/{}/{}", tree_name, rev, url_encode_path(path))
    });

    let title = match kind {
        "bug" if keys.len() == 1 => format!("Bug {}", keys[0]),
        "bug" => format!("Bugs {}", keys.join(", ")),
        _ => keys.join(", "),
    };
    let page_title = format!("{} - mozsearch", title);
    let opt = Options {
        title: &page_title,
        tree_name,
        include_date: env::var("MOZSEARCH_DIFFABLE").is_err(),
        revision: None,
        breadcrumbs_links_to: BreadcrumbsLinksTo::Latest,
        extra_content_classes: "explore",
    };
    output::generate_header(&opt, writer)?;
    output::generate_panel(&opt, writer, &[], true)?;
    let globals = liquid::to_object(&json!({
        "tree": tree_name,
        "title": title,
        "commits": commits,
        "files": files,
        "slot": explore::slot_width(commits.len()),
        "missing": missing,
        "truncated": truncated,
    }))
    .map_err(|_| "Template problems")?;
    build_and_parse_explore()
        .render_to(writer, &globals)
        .map_err(|_| "Template problems")?;
    output::generate_footer(&opt, tree_name, "", writer).unwrap();
    Ok(())
}

pub fn format_commit(
    cfg: &Config,
    tree_name: &str,
    rev: &str,
    writer: &mut dyn Write,
) -> Result<(), &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;

    let git = tree_config.get_git()?;
    let commit_obj = git.repo.revparse_single(rev).map_err(|_| "Bad revision")?;
    let commit = commit_obj.as_commit().ok_or("Bad revision")?;
    let date = git_time_to_chrono(commit.time());

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
    };

    output::generate_header(&opt, writer)?;

    output::generate_breadcrumbs(&opt, writer, "", &[], false)?;

    output::generate_panel(&opt, writer, &[], true)?;

    generate_commit_info(tree_name, tree_config, writer, commit)?;

    output::generate_footer(&opt, tree_name, "", writer).unwrap();

    Ok(())
}
