use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::fs::create_dir_all;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;

extern crate env_logger;
#[macro_use]
extern crate log;

extern crate clap;
use clap::Parser;
use serde_json::{json, Map};
extern crate tools;
use tools::file_format::config;
use tools::file_format::crossref_converter::convert_crossref_value_to_sym_info_rep;
use tools::file_format::repo_data_ingestion::RepoIngestion;
use tools::logging::LoggedSpan;
use tools::logging::init_logging;
use tools::templating::builder::build_and_parse_repo_ingestion_explainer;
use tools::{
    file_format::analysis::{
        read_analysis, read_structured, read_target, AnalysisKind, SearchResult,
        StructuredBindingSlotInfo,
    },
};
use ustr::Ustr;
use ustr::UstrSet;
use ustr::ustr;

/// The size for a payload line (inclusive of leading indicating character and
/// newline) at which we store it externally in `crossref-extra` instead of
/// inline in the `crossref` file itself.
const EXTERNAL_STORAGE_THRESHOLD: usize = 1024 * 3;

/// Splits "pretty" identifiers into their scope components based on C++ style
/// `::` delimiters, ignoring anything that looks like a template param inside
/// (potentially nested) `<` and `>` pairs.
///
/// Note that although searchfox effectively understands JS-style "Foo.bar"
/// hierarchy, this is currently accomplished via `js-analyze.js` emitting 2
/// records: `{ pretty: "Foo", sym: "#Foo", ...}` and `{ pretty: "Foo.bar", sym:
/// "Foo#bar", ...}`.  This approach will likely be revisited when we move to
/// using LSIF/similar indexing, in which case this method will likely want to
/// become language aware and we would start only emitting a single record for
/// a single symbol.
fn split_scopes(id: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut argument_nesting = 0;
    for (index, m) in id.match_indices(|c| c == ':' || c == '<' || c == '>') {
        if m == ":" && argument_nesting == 0 {
            if start != index {
                result.push(id[start..index].to_owned());
                start = index + 1;
            } else {
                start = index + 1;
            }
        } else if m == "<" {
            argument_nesting += 1;
        } else if m == ">" {
            argument_nesting -= 1;
        }
    }
    result.push(id[start..].to_owned());
    return result;
}

#[derive(Parser)]
struct CrossrefCli {
    /// Path to the variable-expanded config file
    #[clap(value_parser)]
    config_file: String,

    /// The tree in the config file we're cross-referencing
    #[clap(value_parser)]
    tree_name: String,

    /// Path to the file containing a list of all of the known analysis files to
    /// ingest.  This is expected to be a subset of the contents of
    /// INDEX_ROOT/all-files which will be located using the config_file and
    /// tree_name.
    #[clap(value_parser)]
    analysis_files_list_path: String,
}

/// Process all analysis files, deriving the `crossref`, `jumps`, and `identifiers` output files.
/// See https://github.com/mozsearch/mozsearch/blob/master/docs/crossref.md for high-level
/// documentation on how this works (locally, `docs/crossref.md`).
///
/// ## Implementation
/// There are 3 phases of processing:
/// 1. Repo data ingestion aggregates any per-file information (bugzilla component
///    mappings, test information) and performs file-level classifications like
///    pre-computing a path_kind for every file.
/// 2. The analysis files are read, populating `table`, `pretty_table`, `id_table`, and
///    `meta_table` incrementally.  Primary cross-reference information comes from target records,
///    but the file is also processed for source records in order to populate `meta_table` with
///    meta-information about the symbol.
/// 2. The table is consumed with jumps generated as a byproduct.
///
/// ### Memory Management
/// Memory usage grows continually throughout phase 1.  Because we load many identical strings,
/// we use string interning so that all long-lived strings are reference-counted interned strings.

#[tokio::main]
async fn main() {
    // This will honor RUST_LOG, but more importantly enables our LoggedSpan
    // mechanism.
    //
    // Note that this marks us transitioning to an async multi-threaded runtime
    // for crossref, but as of the time of writing this, the logging
    // infrastructure is the only async/multi-threaded thing going on, but this
    // will hopefully open the door to more.  (In particular, the semantic
    // linkage mechanism discussed in https://bugzilla.mozilla.org/show_bug.cgi?id=1727789
    // and adjacent bugs would potentially like to see us re-processing the
    // analysis files in parallel after the initial crossref-building phase.)
    init_logging();

    let cli = CrossrefCli::parse();

    let tree_name = &cli.tree_name;
    let cfg = config::load(&cli.config_file, false, Some(&tree_name));

    let tree_config = cfg.trees.get(tree_name).unwrap();

    let analysis_filenames_file = &cli.analysis_files_list_path;

    // This is just the list of analysis files.
    let analysis_relative_paths: Vec<Ustr> = BufReader::new(File::open(analysis_filenames_file).unwrap())
        .lines()
        .map(|x| ustr(&x.unwrap()))
        .collect();

    let all_files_list_path = format!("{}/all-files", tree_config.paths.index_path);
    let all_files_paths: Vec<Ustr> = fs::read_to_string(all_files_list_path)
        .unwrap()
        .lines()
        .map(|x| ustr(&x))
        .collect();

    let all_dirs_list_path = format!("{}/all-dirs", tree_config.paths.index_path);
    let all_dirs_paths: Vec<Ustr> = fs::read_to_string(all_dirs_list_path)
        .unwrap()
        .lines()
        .map(|x| ustr(&x))
        .collect();

    // ## Ingest Repo-Wide Information
    // This will buffer ALL of the tracing logging in our crate between now
    // and when we retrieve it to emit diagnostics.  To this end, we want
    // verbose logging to be conditioned on our "probe" mechanism, which means
    // that we only enable logs for specific values that match our probe, which
    // is currently controlled by environment variables like `PROBE_PATH` (but
    // where we could imagine that our trees might always designate a default
    // probe so that we could have a few instructive data points for people to
    // learn from rather than an excessive wall of text with no curation).
    let logged_ingestion_span = LoggedSpan::new_logged_span("repo_ingestion");

    let per_file_info_toml_str = cfg.read_tree_config_file_with_default("per-file-info.toml").unwrap();
    let mut ingestion = RepoIngestion::new(&per_file_info_toml_str).expect("Your per-file-info.toml file has issues");
    ingestion.ingest_file_list_and_apply_heuristics(&all_files_paths, tree_config);
    ingestion.ingest_dir_list(&all_dirs_paths);

    ingestion.ingest_files(|root: &str, file: &str| {
        cfg.maybe_read_file_from_given_root(&cli.tree_name, root, file)
    }).unwrap();

    // After this point we will only have the concise information populated.
    // We're doing this to minimize our peak memory usage here, but if we find
    // that we actually want to add more data to the per-file detailed
    // information, we should probably evaluate what's happening in practice.
    // While we can always load the detailed information back in as we iterate
    // through the analysis files we consume, for now we're only storing the
    // coverage info and it might be reasonable to not bother writing out the
    // detailed files until the end when we write out the concise file.
    ingestion.state.write_out_and_drop_detailed_file_info(&tree_config.paths.index_path);

    // Consume the ingestion logged span, pass it through our repo-ingestion
    // explainer template, and write it do sik.
    {
        let ingestion_json = logged_ingestion_span.retrieve_serde_json().await;
        let crossref_diag_dir = format!("{}/diags/crossref", tree_config.paths.index_path);
        let ingestion_diag_path = format!("{}/repo_ingestion.md", crossref_diag_dir);
        create_dir_all(crossref_diag_dir).unwrap();

        let globals = liquid::object!({
            "logs": vec![ingestion_json],
        });
        let explain_template = build_and_parse_repo_ingestion_explainer();
        let output = explain_template.render(&globals).unwrap();
        std::fs::write(ingestion_diag_path, output).unwrap();
    }

    // ## Process all the analysis files
    let xref_file = format!("{}/crossref", tree_config.paths.index_path);
    let xref_ext_file = format!("{}/crossref-extra", tree_config.paths.index_path);
    let jumpref_file = format!("{}/jumpref", tree_config.paths.index_path);
    let jumpref_ext_file = format!("{}/jumpref-extra", tree_config.paths.index_path);
    let id_file = format!("{}/identifiers", tree_config.paths.index_path);

    // Nested table hierarchy keyed by: [symbol, kind, path] with Vec<SearchResult> as the leaf
    // values.
    let mut table = BTreeMap::new();
    // Maps (raw) symbol to interned-pretty symbol string.  Each raw symbol is unique, but there
    // may be many raw symbols that map to the same pretty symbol string.
    let mut pretty_table = HashMap::new();
    // Reverse of pretty_table.  The key is the pretty symbol, and the value is a BTreeSet of all
    // of the raw symbols that map to the pretty symbol.  Pretty symbols that start with numbers or
    // include whitespace are considered illegal and not included in the map.
    let mut id_table = BTreeMap::new();
    // Maps (raw) symbol to `SymbolMeta` info for this symbol.  This information is currently
    // extracted from the source records during an additional pass of the analysis file, looking
    // only at defs.  However, in the future, this will likely come from a new type of record.
    let mut meta_table = BTreeMap::new();
    // Maps (raw) symbol to a BTreeSet of the (raw) symbols it "calls".  (The
    // term makes most sense when dealing with functions/similar.  This was
    // formerly dubbed "consumes" in prototyping, but that was even more
    // confusing.  This may want to get renamed again.)
    let mut callees_table = BTreeMap::new();

    // As we process the source entries and build the SourceMeta, we keep a running list of what
    // cross-SourceMeta links need to be established.  We then process this after all of the files
    // have been processed and we know all symbols are known.

    // Pairs of [parent class sym, subclass sym] to add subclass to parent.
    let mut xref_link_subclass = Vec::new();
    // Pairs of [parent method sym, overridden by sym] to add the override to the parent.
    let mut xref_link_override = Vec::new();

    let mut xref_link_slots = Vec::new();

    for path in &analysis_relative_paths {
        print!("File {}\n", path);

        let analysis_fname = format!("{}/analysis/{}", tree_config.paths.index_path, path);
        let analysis = read_analysis(&analysis_fname, &mut read_target);

        // Load the source file and chop it up into `lines` so that we extract
        // the `line` for each result.  In the future this could move to
        // dynamic extraction that uses the `peek_range` if available and this
        // line if it's not.
        let source_fname = tree_config.find_source_file(path);
        let source_file = match File::open(source_fname.clone()) {
            Ok(f) => f,
            Err(_) => {
                println!("Unable to open source file {}", source_fname);
                continue;
            }
        };
        let reader = BufReader::new(&source_file);
        // We operate in String space here on a per-file basis, but these will be
        // flattened to ustrs when converted into a SearchResult.  The intent here
        // is that because Ustr instances permanently retain all provided strings
        // that we don't tell it about Strings until we're sure they'll be retained
        // be a SearchResult.
        let lines: Vec<_> = reader
            .lines()
            .map(|l| match l {
                Ok(line) => {
                    let line_cut = line.trim_end();
                    let len = line_cut.len();
                    let line_cut = line_cut.trim_start();
                    let offset = (len - line_cut.len()) as u32;
                    let buf: String = line_cut.chars().take(100).collect();
                    (buf, offset)
                }
                Err(_) => (String::from(""), 0),
            })
            .collect();

        for datum in analysis {
            // pieces are all `AnalysisTarget` instances.
            for piece in datum.data {
                let t1 = table.entry(piece.sym).or_insert(BTreeMap::new());
                let t2 = t1.entry(piece.kind).or_insert(BTreeMap::new());
                let t3 = t2.entry(path.clone()).or_insert(Vec::new());
                let lineno = (datum.loc.lineno - 1) as usize;
                if lineno >= lines.len() {
                    print!("Bad line number in file {} (line {})\n", path, lineno);
                    continue;
                }

                let (line, offset) = lines[lineno].clone();

                // Idempotently insert the symbol -> pretty symbol mapping into `pretty_table`.
                pretty_table.insert(piece.sym, piece.pretty);

                // If this is a use and there's a contextsym, we want to create a "callees"
                // entry under the contextsym.  We also want to invert the use of "context"
                // to be the symbol in question; it's not useful to name the context symbol
                // redundantly when it's the symbol we're attaching data to.
                if piece.kind == AnalysisKind::Use && !piece.contextsym.is_empty() {
                    let callees = callees_table
                        .entry(piece.contextsym)
                        .or_insert(BTreeSet::new());
                    callees.insert(piece.sym);
                }

                t3.push(SearchResult {
                    lineno: datum.loc.lineno,
                    bounds: (datum.loc.col_start - offset, datum.loc.col_end - offset),
                    line,
                    context: piece.context,
                    contextsym: piece.contextsym,
                    peek_range: piece.peek_range,
                });

                // Idempotently insert the pretty symbol -> symbol mapping as long as the pretty
                // symbol looks sane.  (Whitespace breaks the `identifiers` file's text format, so
                // we can't include them.)
                let ch = piece.sym.chars().nth(0).unwrap();
                if !(ch >= '0' && ch <= '9') && !piece.sym.contains(' ') {
                    let t1 = id_table.entry(piece.pretty).or_insert(BTreeSet::new());
                    t1.insert(piece.sym);
                }
            }
        }

        let structured_analysis = read_analysis(&analysis_fname, &mut read_structured);
        for datum in structured_analysis {
            // pieces are all `AnalysisStructured` instances that were generated alongside source
            // definition records.
            for piece in datum.data {
                meta_table.entry(piece.sym).or_insert_with(|| {
                    for super_info in &piece.supers {
                        xref_link_subclass.push((super_info.sym, piece.sym));
                    }

                    for override_info in &piece.overrides {
                        xref_link_override.push((override_info.sym, piece.sym));
                    }

                    for slot_info in &piece.binding_slots {
                        xref_link_slots.push((
                            slot_info.sym,
                            StructuredBindingSlotInfo {
                                slot_kind: slot_info.slot_kind,
                                slot_lang: slot_info.slot_lang,
                                sym: piece.sym,
                            }));
                    }

                    piece
                });
            }
        }
    }

    // ## Process deferred meta cross-referencing
    for (super_sym, sub_sym) in xref_link_subclass {
        if let Some(super_meta) = meta_table.get_mut(&super_sym) {
            super_meta.subclass_syms.push(sub_sym);
        }
    }

    for (method_sym, override_sym) in xref_link_override {
        if let Some(method_meta) = meta_table.get_mut(&method_sym) {
            method_meta.overridden_by_syms.push(override_sym);
        }
    }

    for (slotted_sym, slot_owner) in xref_link_slots {
        if let Some(slotted) = meta_table.get_mut(&slotted_sym) {
            slotted.slot_owner = Some(slot_owner);
        }
    }

    // ## Write out the crossref and jumpref databases.
    let mut xref_out = File::create(xref_file).unwrap();
    let mut xref_ext_out = File::create(xref_ext_file).unwrap();

    let mut jumpref_out = File::create(jumpref_file).unwrap();
    let mut jumpref_ext_out = File::create(jumpref_ext_file).unwrap();

    // We need to know offset positions in the `-extra` file.  File::tell is a
    // nightly-only experimental API as documented at
    // https://github.com/rust-lang/rust/issues/71213 which makes it preferable
    // to avoid (although I think we may already be dependent on use of nightly
    // for save-analysis purposes?).  Seek::seek with a relative offset of 0
    // seems to be the standard fallback but there are suggestions that can
    // trigger flushes in buffered writers, etc.  So for now we're just keeping
    // track of offsets ourselves and relying on our tests to make sure we don't
    // mess up.
    let mut xref_ext_offset: usize = 0;
    let mut jumpref_ext_offset: usize = 0;

    // Let's only report missing concise info at most once, as for those cases
    // where we have them (ex: NSS), there's usually a lot of symbols in the
    // file and we'd end up reporting the missing info a lot.
    let mut reported_missing_concise = UstrSet::default();

    for (id, id_data) in table {
        let mut kindmap = Map::new();
        for (kind, kind_data) in &id_data {
            let mut result = Vec::new();
            for (path, results) in kind_data {
                if let Some(concise_info) = ingestion.state.concise_per_file.get(path) {
                    result.push(json!({
                        "path": path,
                        "path_kind": concise_info.path_kind,
                        "lines": results,
                    }));
                } else {
                    // NSS seems to have an issue with auto-generated files we
                    // don't know about, so this can't be a warning because it's
                    // too spammy.
                    if reported_missing_concise.insert(path.clone()) {
                        info!("Missing concise info for path '{}'", path);
                    }
                }
            }
            let kindstr = match *kind {
                AnalysisKind::Use => "uses",
                AnalysisKind::Def => "defs",
                AnalysisKind::Assign => "assignments",
                AnalysisKind::Decl => "decls",
                AnalysisKind::Forward => "forwards",
                AnalysisKind::Idl => "idl",
            };
            kindmap.insert(kindstr.to_string(), json!(result));
        }
        if let Some(callee_syms) = callees_table.get(&id) {
            let mut callees = Vec::new();
            for callee_sym in callee_syms {
                if let Some(meta) = meta_table.get(callee_sym) {
                    let mut obj = BTreeMap::new();
                    obj.insert("sym".to_string(), callee_sym);
                    if let Some(pretty) = pretty_table.get(callee_sym) {
                        obj.insert("pretty".to_string(), pretty);
                    }
                    obj.insert("kind".to_string(), &meta.kind);
                    callees.push(json!(obj));
                }
            }
            kindmap.insert("callees".to_string(), json!(callees));
        }
        // Put the metadata in there too.
        let mut fallback_pretty = None;
        if let Some(meta) = meta_table.get(&id) {
            kindmap.insert("meta".to_string(), json!(meta));
        } else {
            fallback_pretty = pretty_table.get(&id);
        }

        let kindmap = json!(kindmap);
        {
            let id_line = format!("!{}\n", id);
            let inline_line = format!(":{}\n", kindmap.to_string());
            if inline_line.len() >= EXTERNAL_STORAGE_THRESHOLD {
                // ### External storage.
                xref_out.write_all(id_line.as_bytes()).unwrap();
                // We write out the identifier in the extra file as well so that it
                // can be interpreted in the same fashion.
                xref_ext_out.write_all(id_line.as_bytes()).unwrap();
                xref_ext_offset += id_line.len();

                let ext_offset_line = format!(
                    "@{:x} {:x}\n",
                    // Skip the leading ":"
                    xref_ext_offset + 1,
                    // Subtract off the leading ":" but keep the newline.
                    inline_line.len() - 1
                );
                xref_out.write_all(ext_offset_line.as_bytes()).unwrap();

                xref_ext_out.write_all(inline_line.as_bytes()).unwrap();
                xref_ext_offset += inline_line.len();
            } else {
                // ### Inline storage.
                xref_out.write_all(id_line.as_bytes()).unwrap();
                xref_out.write_all(inline_line.as_bytes()).unwrap();
            }
        }

        // Also write out/update the jumpref.
        let jumpref_info = convert_crossref_value_to_sym_info_rep(kindmap, &id, fallback_pretty);
        {
            let id_line = format!("!{}\n", id);
            let inline_line = format!(":{}\n", jumpref_info.to_string());
            if inline_line.len() >= EXTERNAL_STORAGE_THRESHOLD {
                // ### External storage.
                jumpref_out.write_all(id_line.as_bytes()).unwrap();
                // We write out the identifier in the extra file as well so that it
                // can be interpreted in the same fashion.
                jumpref_ext_out.write_all(id_line.as_bytes()).unwrap();
                jumpref_ext_offset += id_line.len();

                let ext_offset_line = format!(
                    "@{:x} {:x}\n",
                    // Skip the leading ":"
                    jumpref_ext_offset + 1,
                    // Subtract off the leading ":" but keep the newline.
                    inline_line.len() - 1
                );
                jumpref_out.write_all(ext_offset_line.as_bytes()).unwrap();

                jumpref_ext_out.write_all(inline_line.as_bytes()).unwrap();
                jumpref_ext_offset += inline_line.len();
            } else {
                // ### Inline storage.
                jumpref_out.write_all(id_line.as_bytes()).unwrap();
                jumpref_out.write_all(inline_line.as_bytes()).unwrap();
            }
        }
    }

    let mut idf = File::create(id_file).unwrap();
    for (id, syms) in id_table {
        for sym in syms {
            let components = split_scopes(&id.as_str());
            for i in 0..components.len() {
                let sub = &components[i..components.len()];
                let sub = sub.join("::");

                if !sub.is_empty() {
                    let line = format!("{} {}\n", sub, sym);
                    let _ = idf.write_all(line.as_bytes());
                }
            }
        }
    }

    ingestion.state.write_out_concise_file_info(&tree_config.paths.index_path);
}
