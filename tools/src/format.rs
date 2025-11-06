use std::collections::{BTreeMap, HashMap};
use std::env;
use std::io::Write;
use std::ops::Deref;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use crate::blame;
use crate::file_format::analysis_manglings::make_file_sym_from_path;
use crate::file_format::crossref_converter::{
    determine_desired_extra_syms_from_jumpref, extra_syms_next_step_lookups, JumprefTraversals,
};
use crate::file_format::crossref_lookup::CrossrefLookupMap;
use crate::git_ops;
use crate::languages;
use crate::languages::FormatAs;
use crate::links;
use crate::tokenize;

use crate::file_format::analysis::{AnalysisSource, ExpansionInfo, WithLocation};
use crate::file_format::config::{extract_info_from_blame_commit, Config, GitData, TreeConfig};
use crate::output::{self, Options, PanelItem, PanelSection, F};
use crate::url_encode_path::url_encode_path;

use chrono::datetime::DateTime;
use chrono::naive::datetime::NaiveDateTime;
use chrono::offset::fixed::FixedOffset;
use itertools::Itertools;
use serde_json::{json, to_string, to_string_pretty, Map};
use ustr::{ustr, Ustr, UstrMap};

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
    jumpref_lookup: &Option<CrossrefLookupMap>,
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

                        // Pass-through local symbol information that won't be available from the
                        // cross-reference database because it was marked no_crossref.  This is only
                        // intended to cover type information about the locals; other info like srcsym
                        // and targetsym doesn't make sense for locals.
                        if a.no_crossref {
                            if let Some(type_pretty) = a.type_pretty {
                                let mut obj = Map::new();
                                if let Some(syntax_kind) = a.get_syntax_kind() {
                                    obj.insert(
                                        "syntax".to_string(),
                                        json!(syntax_kind.to_string()),
                                    );
                                }
                                obj.insert("type".to_string(), json!(type_pretty.to_string()));
                                if let Some(type_sym) = &a.type_sym {
                                    obj.insert("typesym".to_string(), json!(type_sym.to_string()));
                                }
                                generated_sym_info.insert(*sym, json!(obj));
                            }
                        } else if let Some(lookup) = jumpref_lookup {
                            if let Ok(jumpref) = lookup.lookup(sym) {
                                // See if there are any binding slot symbols that we should also
                                // include.  This allows us to do things like, when presenting a
                                // context menu for a synthetic XPIDL symbol, we can also provide an
                                // option to go directly to the C++ binding definition.
                                let mut extra_syms =
                                    determine_desired_extra_syms_from_jumpref(&jumpref);
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
                                    if let Some(extra_traversed) =
                                        jumpref_traversed.get_mut(&extra_sym)
                                    {
                                        // The jumpref should already be in generated_sym_info, it's
                                        // just a question if we need to run an extra traversal for it.
                                        if extra_traversed.contains(next_step) {
                                            continue;
                                        }
                                        *extra_traversed |= next_step;
                                        if let Some(extra_jumpref) =
                                            generated_sym_info.get(&extra_sym)
                                        {
                                            for (next_sym, next_traversals) in
                                                extra_syms_next_step_lookups(
                                                    extra_jumpref,
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
                                                    &extra_jumpref,
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
                                syms,
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
                            _ => vec![],
                        })
                    });
                    let classes = classes.collect::<Vec<_>>();
                    if !classes.is_empty() {
                        format!("class=\"{}\" ", classes.join(" "))
                    } else if has_datum {
                        // If the token has analysis record, do not apply keyword.
                        "".to_owned()
                    } else if let Some(ref style) = maybe_style {
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
            expansions.sort_unstable_by(|a, b| Ord::cmp(&(&a.0, &a.1 .1), &(&b.0, &b.1 .1)));

            // Format expansions into html
            let expansions = expansions.into_iter().map(|(key, (platform, expansion))| {
                let html = expansion_to_html(&key, &platform, &expansion);
                (key, (platform, html))
            });

            // Group by key again
            let expansions = expansions.group_by(|(key, _)| key.clone());

            // For each key: merge platforms that yielded the same expansion together
            expansions
                .into_iter()
                .map(|(key, expansions)| {
                    // First into a Vec<(platform: String, expansion: String)>
                    let expansions = expansions.fold(
                        Vec::<(String, String)>::new(),
                        |mut expansions, (_symbol, (platform, expansion))| {
                            if let Some((last_platform, last_expansion)) = expansions.last_mut() {
                                if *last_expansion == expansion {
                                    last_platform.push(' ');
                                    last_platform.push_str(&platform);
                                    return expansions;
                                }
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

/// Renders source code with blame annotations and semantic analysis data (if provided).
/// The caller provides the panel sections.  Currently used by `output-file.rs` to statically
/// generate the tip of whatever branch it's on with semantic analysis data, and `format_path` to
/// dynamically generate the contents of a file without semantic analysis data.
#[allow(clippy::too_many_arguments)]
pub fn format_file_data(
    cfg: &Config,
    tree_name: &str,
    panel: &[PanelSection],
    info_boxes: String,
    commit: &Option<git2::Commit>,
    blame_commit: &Option<git2::Commit>,
    path: &str,
    data: String,
    crossref_lookup_map: &Option<CrossrefLookupMap>,
    analysis: &[WithLocation<Vec<AnalysisSource>>],
    coverage: &Option<Vec<i64>>,
    writer: &mut dyn Write,
) -> Result<FormatPerfInfo, &'static str> {
    let tree_config = cfg.trees.get(tree_name).ok_or("Invalid tree")?;

    let mut format_perf = FormatPerfInfo::default();

    let format = languages::select_formatting(path);
    if let FormatAs::Binary = format {
        write!(writer, "Binary file").unwrap();
        return Ok(format_perf);
    };

    let slug = format_to_slug_attribute(&format);
    let pre_format_code = Instant::now();
    let (output_lines, sym_json) = format_code(
        Some(cfg),
        crossref_lookup_map,
        format,
        path,
        &data,
        analysis,
    );
    format_perf.format_code_duration_us = pre_format_code.elapsed().as_micros() as u64;

    let pre_blame_lines = Instant::now();
    let blame_lines = git_ops::get_blame_lines(tree_config.git.as_ref(), blame_commit, path);
    format_perf.blame_lines_duration_us = pre_blame_lines.elapsed().as_micros() as u64;

    let pre_commit = Instant::now();
    let revision_owned = match *commit {
        Some(ref commit) => {
            let rev = commit.id().to_string();
            let (header, _) = blame::commit_header(commit)?;
            Some((rev, header))
        }
        None => None,
    };
    let revision = match revision_owned {
        Some((ref rev, ref header)) => Some((rev.as_str(), header.as_str())),
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
        extra_content_classes: "source-listing not-diff",
    };

    output::generate_header(&opt, writer)?;

    output::generate_breadcrumbs(&opt, writer, path, !analysis.is_empty())?;

    output::generate_panel(&opt, writer, panel, false)?;

    let info_boxes_container = F::Seq(vec![
        F::S(r#"<section class="info-boxes" id="info-boxes-container">"#),
        F::Indent(vec![F::T(info_boxes)]),
        F::S("</section>"),
    ]);
    output::generate_formatted(writer, &info_boxes_container, 0)?;

    if let Some(ext) = path_wrapper.extension() {
        if ext.to_str().unwrap() == "svg" {
            if let Some(url) = tree_config.paths.make_raw_resource_branch_url(path) {
                output::generate_svg_preview(writer, &url)?
            }
        }
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
            //    than the source code.  We map this to -4.
            //
            // We also have -3 and -2 from interpolate_coverage, and -1
            // which is directly part of the coverage data we receive (that
            // interpolation converts to -2 and -3.)
            match coverage.get(i).unwrap_or(&-4) {
                -4 => r#" class="cov-strip cov-uncovered cov-unknown" role="button" aria-label="missing data""#.to_owned(),
                -3 => r#" class="cov-strip cov-miss cov-interpolated" role="button" aria-label="uncovered""#.to_owned(),
                -2 => r#" class="cov-strip cov-hit cov-interpolated" role="button" aria-label="uncovered""#.to_owned(),
                -1 => r#" class="cov-strip cov-uncovered cov-known" role="button" aria-label="uncovered""#.to_owned(),
                 0 => r#" class="cov-strip cov-miss cov-known" role="button" aria-label="miss" data-coverage="0""#.to_owned(),
                // Should this directly be a CSS variable?
                 x => format!(
                    r#" class="cov-strip cov-hit cov-known cov-log10-{}" role="button" aria-label="hit {}{}" data-coverage="{}""#,
                    (*x as f64).log10().floor() as u32,
                    if *x < 1000 { *x } else { *x / 1000 },
                    if *x < 1000 { "" } else { "k" },
                    *x)
            }
        } else {
            " class=\"cov-strip cov-no-data\"".to_owned()
        };

        // Compute the blame data for this line (if any)
        let blame_data = if let Some(ref lines) = blame_lines {
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

            let same_rev_as_last = last_revs.map_or(false, |last| last == revs);
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
                    "<div role=\"cell\"><div{}></div></div>",
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

    output::generate_footer(&opt, tree_name, path, writer).unwrap();

    format_perf.format_mixing_duration_us = pre_format_mixing.elapsed().as_micros() as u64;

    Ok(format_perf)
}

fn format_to_slug_attribute(format: &FormatAs) -> String {
    let slug = match format {
        FormatAs::FormatTagLike(spec) => spec.markdown_slug,
        FormatAs::FormatCLike(spec) => spec.markdown_slug,
        _ => "",
    };

    if slug.is_empty() {
        return String::new();
    }

    format!(r#" data-markdown-slug="{}""#, slug)
}

fn entry_to_blob(repo: &git2::Repository, entry: &git2::TreeEntry) -> Result<String, &'static str> {
    match entry.kind() {
        Some(git2::ObjectType::Blob) => {}
        _ => return Err("Invalid path; expected file"),
    }

    if entry.filemode() == 120000 {
        return Err("Path is to a symlink");
    }

    Ok(git_ops::read_blob_entry(repo, entry))
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
    let commit = commit_obj.into_commit().map_err(|_| "Bad revision")?;
    let commit_tree = commit.tree().map_err(|_| "Bad revision")?;
    let path_obj = Path::new(path);
    let data = match commit_tree.get_path(path_obj) {
        Ok(entry) => entry_to_blob(&git.repo, &entry)?,
        Err(_) => {
            // Check to see if this path is inside a submodule
            let mut test_path = path_obj.parent();
            loop {
                let subrepo_path = match test_path {
                    Some(path) => path,
                    None => return Err("File not found"),
                };
                let entry = match commit_tree.get_path(subrepo_path) {
                    Ok(e) => e,
                    Err(_) => {
                        test_path = subrepo_path.parent();
                        continue;
                    }
                };
                if entry.kind() != Some(git2::ObjectType::Commit) {
                    return Err("File not found");
                }

                // If we get here, the path is inside a submodule
                let subrepo_path = subrepo_path.to_str().ok_or("UTF-8 error")?;
                let subrepo = git
                    .repo
                    .find_submodule(subrepo_path)
                    .map_err(|_| "Can't find submodule")?;
                let subrepo = subrepo.open().map_err(|_| "Can't open submodule")?;
                let path_in_subrepo = path_obj
                    .strip_prefix(subrepo_path)
                    .map_err(|_| "Submodule path error")?;
                let subentry = subrepo
                    .find_commit(entry.id())
                    .and_then(|commit| commit.tree())
                    .and_then(|tree| tree.get_path(path_in_subrepo))
                    .map_err(|_| "File not found in submodule")?;
                break entry_to_blob(&subrepo, &subentry)?;
            }
        }
    };

    // Get blame.
    let blame_commit = if let Some(ref blame_repo) = git.blame_repo {
        let blame_oid = git
            .blame_map
            .get(&commit.id())
            .ok_or("Unable to find blame for revision")?;
        Some(
            blame_repo
                .find_commit(*blame_oid)
                .map_err(|_| "Blame is not a blob")?,
        )
    } else {
        None
    };

    let analysis = Vec::new();

    let hg_rev: &str = tree_config
        .git
        .as_ref()
        .and_then(|git| git.hg_map.get(&commit.id()))
        .map(|rev| rev.as_ref()) // &String to &str conversion
        .unwrap_or("default");

    let encoded_path = url_encode_path(path);

    let mut vcs_panel_items = vec![];
    vcs_panel_items.push(PanelItem {
        title: "Go to latest version".to_owned(),
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
            title: "Git log".to_owned(),
            link,
            update_link_lineno: "",
            accel_key: hg_log_link.is_none().then_some('L'),
            copyable: true,
        });
    }
    if let Some(link) = hg_log_link {
        vcs_panel_items.push(PanelItem {
            title: "Mercurial log".to_owned(),
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
            title: "Raw".to_owned(),
            link,
            update_link_lineno: "",
            accel_key: Some('R'),
            copyable: true,
        });
    }

    if tree_config.paths.git_blame_path.is_some() {
        vcs_panel_items.push(PanelItem {
            title: "Blame".to_owned(),
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

    format_file_data(
        cfg,
        tree_name,
        &panel,
        "".to_string(),
        &Some(commit),
        &blame_commit,
        path,
        data,
        &None,
        &analysis,
        &None,
        writer,
    )
    .map(|_| ())
}

pub fn create_markdown_panel_section(add_symbol_link: bool) -> PanelSection {
    let mut markdown_panel_items = vec![];
    markdown_panel_items.push(PanelItem {
        title: "Filename Link".to_owned(),
        link: String::new(),
        update_link_lineno: "",
        accel_key: Some('F'),
        copyable: true,
    });
    if add_symbol_link {
        markdown_panel_items.push(PanelItem {
            title: "Symbol Link".to_owned(),
            link: String::new(),
            update_link_lineno: "",
            accel_key: Some('S'),
            copyable: true,
        });
    }
    markdown_panel_items.push(PanelItem {
        title: "Code Block".to_owned(),
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

    let mut blames = Vec::new();

    for parent_oid in commit.parent_ids() {
        let blame_repo = match git.blame_repo {
            Some(ref r) => r,
            None => {
                blames.push(None);
                continue;
            }
        };

        let blame_oid = git
            .blame_map
            .get(&parent_oid)
            .ok_or("Unable to find blame")?;
        let blame_commit = blame_repo
            .find_commit(*blame_oid)
            .map_err(|_| "Blame is not a blob")?;
        let blame_tree = blame_commit.tree().map_err(|_| "Bad revision")?;
        match blame_tree.get_path(Path::new(path)) {
            Ok(blame_entry) => {
                let blame = git_ops::read_blob_entry(blame_repo, &blame_entry);
                let blame_lines = blame.lines().map(|s| s.to_owned()).collect::<Vec<_>>();
                blames.push(Some(blame_lines));
            }
            Err(_) => {
                blames.push(None);
            }
        }
    }

    let mut new_lineno = 1;
    let mut old_lineno = commit.parent_ids().map(|_| 1).collect::<Vec<_>>();

    let mut lines = split_lines(&difftxt);
    for i in 0..lines.len() {
        if lines[i].starts_with('@') && i + 1 < lines.len() {
            lines = lines.split_off(i + 1);
            break;
        }
    }

    let mut new_lines = String::new();

    let mut output = Vec::new();
    for line in lines {
        if line.is_empty() || line.starts_with('\\') {
            continue;
        }

        let num_parents = commit.parents().count();
        let (origin, content) = line.split_at(num_parents);
        let origin = origin.chars().collect::<Vec<_>>();
        let mut cur_blame = None;
        for i in 0..num_parents {
            let has_minus = origin.contains(&'-');
            if (has_minus && origin[i] == '-') || (!has_minus && origin[i] != '+') {
                cur_blame = match blames[i] {
                    Some(ref lines) => Some(&lines[old_lineno[i] - 1]),
                    None => return Err("expected blame for '-' line, none found"),
                };
                old_lineno[i] += 1;
            }
        }

        let mut lno = -1;
        if !origin.contains(&'-') {
            new_lines.push_str(content);
            new_lines.push('\n');

            lno = new_lineno;
            new_lineno += 1;
        }

        output.push((lno, cur_blame, origin, content));
    }

    let format = languages::select_formatting(path);
    if let FormatAs::Binary = format {
        return Err("Cannot diff binary file");
    };
    let analysis = Vec::new();
    let slug = format_to_slug_attribute(&format);
    let (formatted_lines, _) = format_code(Some(cfg), &None, format, path, &new_lines, &analysis);

    let (header, _) = blame::commit_header(commit)?;

    let filename = Path::new(path).file_name().unwrap().to_str().unwrap();
    let title = format!("{} - mozsearch", filename);
    let opt = Options {
        title: &title,
        tree_name,
        include_date: true,
        revision: Some((rev, &header)),
        extra_content_classes: "source-listing diff",
    };

    output::generate_header(&opt, writer)?;

    output::generate_breadcrumbs(&opt, writer, path, false)?;

    let encoded_path = url_encode_path(path);

    let mut vcs_panel_items = vec![
        PanelItem {
            title: "Show changeset".to_owned(),
            link: format!("/{}/commit/{}", tree_name, rev),
            update_link_lineno: "",
            accel_key: None,
            copyable: true,
        },
        PanelItem {
            title: "Show file without diff".to_owned(),
            link: format!("/{}/rev/{}/{}", tree_name, rev, encoded_path),
            update_link_lineno: "#{}",
            accel_key: None,
            copyable: true,
        },
        PanelItem {
            title: "Go to latest version".to_owned(),
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
            title: "Git log".to_owned(),
            link,
            update_link_lineno: "",
            accel_key: hg_log_link.is_none().then_some('L'),
            copyable: true,
        });
    }
    if let Some(link) = hg_log_link {
        vcs_panel_items.push(PanelItem {
            title: "Mercurial log".to_owned(),
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

    let f = F::Seq(vec![F::T(format!(
        "<div id=\"file\" class=\"file\" role=\"table\"{}>",
        slug
    ))]);

    output::generate_formatted(writer, &f, 0).unwrap();

    fn entity_replace(s: String) -> String {
        s.replace("&", "&amp;").replace("<", "&lt;")
    }

    let mut last_rev = String::new();
    let mut last_color = false;
    for &(lineno, blame, ref origin, content) in &output {
        let blame_data = match blame {
            Some(blame) => {
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
            None => " class=\"blame-strip\"".to_owned(),
        };

        let content = entity_replace(content.to_owned());
        let content = if lineno > 0 && (lineno as usize) < formatted_lines.len() + 1 {
            &formatted_lines[(lineno as usize) - 1].line
        } else {
            &content
        };

        let origin = origin.iter().cloned().collect::<String>();

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
                // Coverage info.
                F::T(format!(
                    "<div role=\"cell\" class=\"blame-container\"><div{}></div></div>",
                    blame_data
                )),
                // Blame info.
                F::T(format!(
                    "<div role=\"cell\" class=\"blame-container\"><div{}></div></div>",
                    blame_data
                )),
                // The line number.
                F::T(format!(
                    "<div role=\"cell\" class=\"line-number\" data-line-number=\"{}\"></div>",
                    if lineno > 0 {
                        format!("{}", lineno)
                    } else {
                        "".to_owned()
                    },
                )),
                // The source line.
                F::T(format!(
                    "<code role=\"cell\" class=\"source-line{}\">{} {}\n</code>",
                    class, origin, content
                )),
            ]),
            F::S("</div>"),
        ]);

        output::generate_formatted(writer, &f, 0).unwrap();
    }

    let f = F::Seq(vec![F::S("</div>")]);
    output::generate_formatted(writer, &f, 0).unwrap();

    output::generate_footer(&opt, tree_name, path, writer).unwrap();

    Ok(())
}

fn generate_commit_info(
    tree_name: &str,
    tree_config: &TreeConfig,
    writer: &mut dyn Write,
    commit: &git2::Commit,
    blame_commit: Option<&git2::Commit>,
) -> Result<(), &'static str> {
    let (header, remainder) = blame::commit_header(commit)?;

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
    let oldgit = if let Some(blame_commit) = blame_commit {
        let blame_info = extract_info_from_blame_commit(blame_commit);
        if let Some(oldrevs) = blame_info.oldrevs {
            vec![F::T(format!("<tr><td>old {} git revs:</td><td>{}</td></tr>",
                              tree_config.paths.oldtree_name.clone().unwrap_or_default(), oldrevs))]
        } else {
            vec![]
        }
    } else {
        vec![]
    };

    let hg = match git.hg_map.get(&commit.id()) {
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

    let naive_t = NaiveDateTime::from_timestamp(commit.time().seconds(), 0);
    let tz = FixedOffset::east(commit.time().offset_minutes() * 60);
    let t: DateTime<FixedOffset> = DateTime::from_utc(naive_t, tz);
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

    let blame_commit = match (&git.blame_repo, git.blame_map.get(&commit.id())) {
        (Some(blame_repo), Some(blame_oid)) => {
            blame_repo.find_commit(*blame_oid).ok()
        }
        _ => {
            None
        }
    };

    let title = format!("{} - mozsearch", rev);
    let opt = Options {
        title: &title,
        tree_name,
        include_date: true,
        revision: None,
        extra_content_classes: "commit",
    };

    output::generate_header(&opt, writer)?;

    output::generate_breadcrumbs(&opt, writer, "", false)?;

    output::generate_panel(&opt, writer, &[], true)?;

    generate_commit_info(tree_name, tree_config, writer, commit, blame_commit.as_ref())?;

    output::generate_footer(&opt, tree_name, "", writer).unwrap();

    Ok(())
}
