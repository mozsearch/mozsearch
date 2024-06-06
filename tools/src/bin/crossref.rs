use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::fs;
use std::fs::create_dir_all;
use std::fs::File;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;

#[macro_use]
extern crate tracing;

extern crate clap;
use clap::Parser;
use itertools::Itertools;
use serde_json::{json, Map};
extern crate tools;
use tools::file_format::analysis::AnalysisStructured;
use tools::file_format::analysis::OntologySlotInfo;
use tools::file_format::analysis::OntologySlotKind;
use tools::file_format::analysis::StructuredPointerInfo;
use tools::file_format::analysis::StructuredTag;
use tools::file_format::analysis::{
    read_analysis, read_structured, read_target, AnalysisKind, SearchResult,
    StructuredBindingSlotInfo, AnalysisTarget, Location, BindingSlotProps,
    TargetTag, LineRange,
};
use tools::file_format::analysis_manglings::make_file_sym_from_path;
use tools::file_format::analysis_manglings::split_pretty;
use tools::file_format::config;
use tools::file_format::crossref_converter::convert_crossref_value_to_sym_info_rep;
use tools::file_format::ontology_mapping::{
    OntologyLabelOwningClass, OntologyMappingIngestion, OntologyPointerKind,
};
use tools::file_format::repo_data_ingestion::RepoIngestion;
use tools::logging::init_logging;
use tools::logging::LoggedSpan;
use tools::templating::builder::build_and_parse_ontology_ingestion_explainer;
use tools::templating::builder::build_and_parse_repo_ingestion_explainer;
use ustr::ustr;
use ustr::Ustr;
use ustr::UstrMap;
use ustr::UstrSet;

/// The size for a payload line (inclusive of leading indicating character and
/// newline) at which we store it externally in `crossref-extra` instead of
/// inline in the `crossref` file itself.
const EXTERNAL_STORAGE_THRESHOLD: usize = 1024 * 3;

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

    /// Path to the file containing a list of all the files which doesn't have
    /// analysis but still needs FILE_* target.
    #[clap(value_parser)]
    other_resources_list_path: String,
}

type SearchResultTable = BTreeMap<Ustr, BTreeMap<AnalysisKind, BTreeMap<Ustr, Vec<SearchResult>>>>;
type PrettyTable = HashMap<Ustr, Ustr>;
type IdTable = UstrMap<UstrSet>;
type MetaTable = BTreeMap<Ustr, AnalysisStructured>;
type CalleesTable = BTreeMap<Ustr, BTreeMap<Ustr, (Ustr, BTreeSet<u32>)>>;
type FieldMemberUseTable = BTreeMap<Ustr, BTreeMap<Ustr, Vec<(Ustr, OntologyPointerKind)>>>;
type XrefLinkSubclass = Vec<(Ustr, Ustr)>;
type XrefLinkOverride = Vec<(Ustr, Ustr)>;
type XrefLinkSlots = BTreeMap<(Ustr, Ustr), (BindingSlotProps, Option<Ustr>)>;

fn process_analysis_target(
    mut piece: AnalysisTarget,
    path: &Ustr, file_sym: &Ustr, lineno: usize, loc: &Location,
    table: &mut SearchResultTable, pretty_table: &mut PrettyTable,
    id_table: &mut IdTable, callees_table: &mut CalleesTable,
    lines: &Vec<(String, u32)>
) {
    if piece.pretty.is_empty() {
        info!("Skipping empty pretty for symbol {}", piece.sym);
        return;
    }

    // XXX temporary include hack; we should fix this in the C++ indexer, but I want to
    // see how it works out.
    if piece.sym.starts_with("FILE_") && piece.contextsym.is_empty() {
        piece.context = path.clone();
        piece.contextsym = file_sym.clone();
    }

    let t1 = table.entry(piece.sym).or_insert_with(|| BTreeMap::new());
    let t2 = t1.entry(piece.kind).or_insert_with(|| BTreeMap::new());
    let t3 = t2.entry(path.clone()).or_insert_with(|| Vec::new());

    let (line, offset) = lines[lineno].clone();

    // Idempotently insert the symbol -> pretty symbol mapping into `pretty_table`.
    pretty_table.insert(piece.sym, piece.pretty);

    // If this is a use and there's a contextsym, we want to create a "callees"
    // entry under the contextsym.  We also want to invert the use of "context"
    // to be the symbol in question; it's not useful to name the context symbol
    // redundantly when it's the symbol we're attaching data to.
    if piece.kind == AnalysisKind::Use && !piece.contextsym.is_empty() {
        let callee_syms = callees_table
            .entry(piece.contextsym)
            .or_insert_with(|| BTreeMap::new());
        let (from_path, callee_jump_lines) = callee_syms
            .entry(piece.sym)
            .or_insert_with(|| (path.clone(), BTreeSet::new()));
        if from_path == path {
            callee_jump_lines.insert(loc.lineno);
        }
        // XXX otherwise weird things are happening, but I'm not
        // sure we need to warn on this.
    }

    t3.push(SearchResult {
        lineno: loc.lineno,
        bounds: (loc.col_start - offset, loc.col_end - offset),
        line,
        context: piece.context,
        contextsym: piece.contextsym,
        peek_range: piece.peek_range,
    });

    // Idempotently insert the pretty identifier -> symbol mapping as long as the pretty
    // symbol looks sane.  (Whitespace breaks the `identifiers` file's text format, so
    // we can't include them.)
    let ch = piece.sym.chars().nth(0).unwrap();
    if !(ch >= '0' && ch <= '9') && !piece.sym.contains(' ') {
        // Split the pretty identifier into parts so for "foo::bar::Baz"
        // we can emit ["foo::bar::Baz", "bar::Baz", "Baz"] into our
        // identifiers table so people don't have to always type out
        // the full identifier.
        //
        // NOTE: We are passing "" as the symbol here in order to
        // avoid splitting paths (which detects a "FILE_" prefix),
        // but we may want to support multiple pretty delimiters
        // beyond "::" here in the future.  (Although there's
        // something to be said for normalizing on use of "::" for
        // everything but paths, and we sorta do this for scip-indexer
        // already.)
        let (components, delim) = split_pretty(&piece.pretty.as_str(), "");
        for i in 0..components.len() {
            let sub = &components[i..components.len()];
            let sub = sub.join(delim);

            if !sub.is_empty() {
                let t1 = id_table.entry(ustr(&sub)).or_insert(UstrSet::default());
                t1.insert(piece.sym);
            }
        }
    }
}

fn process_analysis_structured(
    mut piece: AnalysisStructured, subsystem: Option<Ustr>,
    meta_table: &mut MetaTable,
    xref_link_subclass: &mut XrefLinkSubclass,
    xref_link_override: &mut XrefLinkOverride,
    xref_link_slots: &mut XrefLinkSlots,
) {
    meta_table.entry(piece.sym).or_insert_with(|| {
        for super_info in &piece.supers {
            xref_link_subclass.push((super_info.sym, piece.sym));
        }

        for override_info in &piece.overrides {
            xref_link_override.push((override_info.sym, piece.sym));
        }

        // We remove all bindings infos from AnalysisStructured instances here
        // but add them back both ways when we iterate over xref_link_slots.
        for slot_info in piece.binding_slots.drain(..) {
            xref_link_slots.insert((piece.sym, slot_info.sym), (slot_info.props, subsystem.clone()));
        }
        if let Some(slot_info) = piece.slot_owner.take() {
            xref_link_slots.insert((slot_info.sym, piece.sym), (slot_info.props, subsystem.clone()));
        }

        piece.subsystem = subsystem.clone();

        piece
    });
}

fn make_subsystem(path: &Ustr, file_sym: &Ustr,
                  ingestion: &mut RepoIngestion,
                  meta_table: &mut MetaTable, pretty_table: &mut PrettyTable,
                  id_table: &mut IdTable) -> Option<Ustr> {
    let concise_info = ingestion.state.concise_per_file.get(path);

    if let Some(concise) = concise_info {
        let file_structured = AnalysisStructured {
            structured: StructuredTag::Structured,
            pretty: path.clone(),
            sym: file_sym.clone(),
            type_pretty: None,
            kind: ustr("file"),
            subsystem: concise.subsystem.clone(),
            // For most analytical purposes, we want to think of files as atomic,
            // so I don't think there is any upside to modeling the containing
            // directory as a parent.  Especially since we don't yet have a
            // `DIR_blah` symbol type yet or a clear reason to want one.
            parent_sym: None,
            slot_owner: None,
            impl_kind: ustr("impl"),
            size_bytes: None,
            own_vf_ptr_bytes: None,
            binding_slots: vec![],
            ontology_slots: vec![],
            supers: vec![],
            methods: vec![],
            fields: vec![],
            overrides: vec![],
            props: vec![],
            labels: BTreeSet::default(),

            idl_sym: None,
            subclass_syms: vec![],
            overridden_by_syms: vec![],
            variants: vec![],
            extra: Map::default(),
        };
        meta_table.insert(file_structured.sym.clone(), file_structured);
        pretty_table.insert(file_sym.clone(), path.clone());
        let t1 = id_table
            .entry(path.clone())
            .or_insert_with(|| UstrSet::default());
        t1.insert(file_sym.clone());
        concise.subsystem.clone()
    } else {
        None
    }
}

fn line_to_buf_and_offset(line: String) -> (String, u32) {
    let line_cut = line.trim_end();
    let len = line_cut.len();
    let line_cut = line_cut.trim_start();
    let offset = (len - line_cut.len()) as u32;
    let buf: String = line_cut.chars().take(100).collect();
    (buf, offset)
}

/// Process all analysis files, deriving the `crossref`, `jumpref`, and `identifiers` output files.
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
///    but the file is also processed for structured records in order to populate `meta_table` with
///    meta-information about the symbol.
/// 2. The table is consumed, generating both crossref and jumpref information.
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
    let analysis_relative_paths: Vec<Ustr> =
        BufReader::new(File::open(analysis_filenames_file).unwrap())
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
    let ingestion_entered = logged_ingestion_span.span.clone().entered();

    let per_file_info_toml_str = cfg
        .read_tree_config_file_with_default("per-file-info.toml")
        .unwrap();
    let mut ingestion = RepoIngestion::new(&per_file_info_toml_str)
        .expect("Your per-file-info.toml file has issues");
    ingestion.ingest_file_list_and_apply_heuristics(&all_files_paths, tree_config);
    ingestion.ingest_dir_list(&all_dirs_paths);

    ingestion
        .ingest_files(|root: &str, file: &str| {
            cfg.maybe_read_file_from_given_root(&cli.tree_name, root, file)
        })
        .unwrap();

    // After this point we will only have the concise information populated.
    // We're doing this to minimize our peak memory usage here, but if we find
    // that we actually want to add more data to the per-file detailed
    // information, we should probably evaluate what's happening in practice.
    // While we can always load the detailed information back in as we iterate
    // through the analysis files we consume, for now we're only storing the
    // coverage info and it might be reasonable to not bother writing out the
    // detailed files until the end when we write out the concise file.
    ingestion
        .state
        .write_out_and_drop_detailed_file_info(&tree_config.paths.index_path);

    // Consume the ingestion logged span, pass it through our repo-ingestion
    // explainer template, and write it do sik.
    drop(ingestion_entered);
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

    // ## Load Ontology Config
    //
    // I moved this before the analysis ingestion thinking we might process some
    // rules as we ingest data.  (Specifically for `label_owning_class`.)  But
    // now it seems like it's probably reasonable to process that at the normal
    // post-analysis-ingestion time to avoid limiting our options there.  But
    // I'm leaving this loading ahead of the analysis ingestion because it does
    // seem preferable that if we're going to throw a fatal error due to a
    // misconfiguration that it's much better for us to do it earlier.
    let logged_ontology_span = LoggedSpan::new_logged_span("ontology");
    let ontology_entered = logged_ontology_span.span.clone().entered();

    let ontology_toml_str = cfg
        .read_tree_config_file_with_default("ontology-mapping.toml")
        .unwrap();
    let ontology = OntologyMappingIngestion::new(&ontology_toml_str)
        .expect("ontology-mapping.toml has issues");
    drop(ontology_entered);

    // ## Process all the analysis files
    let xref_file = format!("{}/crossref", tree_config.paths.index_path);
    let xref_ext_file = format!("{}/crossref-extra", tree_config.paths.index_path);
    let jumpref_file = format!("{}/jumpref", tree_config.paths.index_path);
    let jumpref_ext_file = format!("{}/jumpref-extra", tree_config.paths.index_path);
    let id_file = format!("{}/identifiers", tree_config.paths.index_path);

    // Nested table hierarchy keyed by: [symbol, kind, path] with Vec<SearchResult> as the leaf
    // values.
    let mut table = SearchResultTable::new();
    // Maps (raw) symbol to interned-pretty symbol string.  Each raw symbol is unique, but there
    // may be many raw symbols that map to the same pretty symbol string.
    let mut pretty_table = PrettyTable::new();
    // Reverse of pretty_table.  The key is the pretty symbol, and the value is a UstrSet of all
    // of the raw symbols that map to the pretty symbol.  Pretty symbols that start with numbers or
    // include whitespace are considered illegal and not included in the map.
    //
    // This table has been modified so that it is populated with the suffix variations immediately.
    // So for the symbol "foo::bar::Baz" we will add entries for "Baz", "bar::Baz", and
    // "foo::bar::Baz".  Previously we would only add the full variation and compute the suffixes
    // when writing its contents out, but we now need/want this for processing field type strings
    // because we do not currently have the fully qualified symbols available.  In the future
    // we hopefully will have better type representations for fields.
    //
    // An alternate approach would be for us to write the identifier table out earlier and just
    // memory map that for subsequent processing.  Not doing that right now because the ustr rep
    // potentially could end up comparable in memory usage if the identifer file is fully paged
    // in, and for performance we would want it fully paged in, so might as well use the memory
    // so we fail faster if we don't have the memory available.
    let mut id_table = IdTable::default();
    // Maps (raw) symbol to `SymbolMeta` info for this symbol.  Currently, we
    // require that the language analyzer created a "structured" record and we
    // use that, but it could make sense for us to automatically generate a stub
    // meta for symbols for which we didn't find a structured record.  A minor
    // awkwardness here is that we would really want to use the "source" records
    // for this (as we did prior to the introduction of the structured record
    // type), but we currently don't retain those.  (But we do currently read
    // the file 2x; maybe it would be better to read it once and have the
    // records grouped by type so we can improve that).
    let mut meta_table = MetaTable::new();
    // Maps the (raw) symbol making the calls to a BTreeMap whose keys are the
    // symbols being called and whose values are a tuple of the path where the
    // calls are happening and a BTreeSet of the lines in the path where these
    // calls happen.  This is used so that on graphs we can have the edges have
    // a source link that highlights all of the lines where the calls are
    // happening.
    //
    // The term "callees" used here makes most sense when dealing with
    // functions/similar, but it's not just for those cases.  We also use it for
    // field accesses, etc.  This was formerly dubbed "consumes" in prototyping,
    // but that was even more confusing.  Another rename may be in order.
    let mut callees_table = CalleesTable::new();
    // Maps the (raw) symbol corresponding to a type to a BTreeMap whose key
    // is the class referencing the type and whose values are a vec of tuples of
    // the form (field pretty, pointer kind).
    let mut field_member_use_table = FieldMemberUseTable::new();

    // As we process the source entries and build the SourceMeta, we keep a running list of what
    // cross-SourceMeta links need to be established.  We then process this after all of the files
    // have been processed and we know all symbols are known.

    // Pairs of [parent class sym, subclass sym] to add subclass to parent.
    let mut xref_link_subclass = XrefLinkSubclass::new();
    // Pairs of [parent method sym, overridden by sym] to add the override to the parent.
    let mut xref_link_override = XrefLinkOverride::new();
    // (owner symbol, slotted symbol) -> slot props
    // This is a BTreeMap and not a HashMap to force a stable ordering and avoid flaky tests.
    let mut xref_link_slots = XrefLinkSlots::new();

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
                Ok(line) => line_to_buf_and_offset(line),
                Err(_) => (String::from(""), 0),
            })
            .collect();

        let file_sym = ustr(&make_file_sym_from_path(path));

        for datum in analysis {
            // If we're going to experience a bad line, skip out before
            // creating any structure.
            let lineno = (datum.loc.lineno - 1) as usize;
            if lineno >= lines.len() {
                print!("Bad line number in file {} (line {})\n", path, lineno);
                continue;
            }

            for piece in datum.data {
                process_analysis_target(
                    piece, &path, &file_sym, lineno, &datum.loc,
                    &mut table, &mut pretty_table, &mut id_table,
                    &mut callees_table, &lines);
            }
        }

        let subsystem = make_subsystem(
            &path, &file_sym,
            &mut ingestion, &mut meta_table, &mut pretty_table, &mut id_table);

        let structured_analysis = read_analysis(&analysis_fname, &mut read_structured);
        for datum in structured_analysis {
            for piece in datum.data {
                process_analysis_structured(
                    piece, subsystem,
                    &mut meta_table, &mut xref_link_subclass,
                    &mut xref_link_override, &mut xref_link_slots);
            }
        }
    }

    let other_resources_file = &cli.other_resources_list_path;

    let other_resources_relative_paths: Vec<Ustr> =
        BufReader::new(File::open(other_resources_file).unwrap())
            .lines()
            .map(|x| ustr(&x.unwrap()))
            .collect();

    for path in &other_resources_relative_paths {
        print!("File {}\n", path);

        let pretty = ustr(format!("file {}", path).as_str());
        let file_sym = ustr(&make_file_sym_from_path(path));

        let line_and_offset = {
            let source_fname = tree_config.find_source_file(path);
            let source_file = match File::open(source_fname.clone()) {
                Ok(f) => f,
                Err(_) => {
                    println!("Unable to open source file {}", source_fname);
                    continue;
                }
            };

            let mut reader = BufReader::new(&source_file);
            let mut line: String = String::default();
            match reader.read_line(&mut line) {
                Ok(_) => line_to_buf_and_offset(line),
                Err(_) => ("(binary file)".to_string(), 0 as u32),
            }
        };
        let lines = vec![line_and_offset];

        let loc = Location {
            lineno: 0,
            col_start: 0,
            col_end: 0,
        };
        let piece = AnalysisTarget {
            target: TargetTag::Target,
            kind: AnalysisKind::Def,
            pretty: pretty,
            sym: file_sym,
            context: ustr(""),
            contextsym: ustr(""),
            peek_range: LineRange {
                start_lineno: 0,
                end_lineno: 0,
            },
            arg_ranges: vec![],
        };

        process_analysis_target(
            piece, &path, &file_sym, 0, &loc,
            &mut table, &mut pretty_table, &mut id_table,
            &mut callees_table, &lines);

        let _ = make_subsystem(
            &path, &file_sym,
            &mut ingestion, &mut meta_table, &mut pretty_table, &mut id_table);
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

    for ((owner_sym, slotted_sym), (props, subsystem)) in xref_link_slots {
        if let Some(owner) = meta_table.get_mut(&owner_sym) {
            owner.binding_slots.push(StructuredBindingSlotInfo {
                sym: slotted_sym,
                props,
            });
            if owner.subsystem.is_none() {
                owner.subsystem = subsystem.clone();
            }
        }
        if let Some(slotted) = meta_table.get_mut(&slotted_sym) {
            slotted.slot_owner = Some(StructuredBindingSlotInfo {
                sym: owner_sym,
                props,
            });
            slotted.subsystem = subsystem;
        }
    }

    // ## Run Ontology Processing
    let ontology_entered = logged_ontology_span.span.clone().entered();

    info!("Processing ontology now that all analysis files have been read in.");

    // ### Extract field-processing rules to run over every class.
    let mut field_owning_class_rules: UstrMap<OntologyLabelOwningClass> = UstrMap::default();

    for (pretty_id, rule) in ontology.config.pretty.iter() {
        if let Some(label_owning_class) = &rule.label_owning_class {
            // We lookup by the type_pretty which currently will have "class " or "struct ""
            // prefixes.  In the interest of not having to mangle every type field, create
            // "class "-prefixed variants.  I'm not creating "struct "-prefixed variants
            // right now because most things should be classes.
            let type_prettied = format!("class {}", pretty_id);
            field_owning_class_rules.insert(ustr(&type_prettied), label_owning_class.clone());
        }
    }

    // ### Process class/fields using ontology type information
    for meta in meta_table.values_mut() {
        if meta.kind.as_str() == "class" || meta.kind.as_str() == "struct" {
            for field in &mut meta.fields {
                // In order to avoid getting confused by native types, require that we have some
                // typesym.  We won't have a typesym for native types.
                if field.type_sym.is_empty() {
                    continue;
                }

                // Note that the type_pretty will have a "class " prefix which is why we already
                // pre-transformed our rules when populating the rule map.
                if let Some(rule) = field_owning_class_rules.get(&field.type_pretty) {
                    for label_rule in &rule.labels {
                        meta.labels.insert(label_rule.label.clone());
                    }
                }

                let (ptr_infos, type_labels) = ontology
                .config
                .maybe_parse_type_as_pointer(&field.type_pretty);
                for label in type_labels {
                    meta.labels.insert(label);
                }
                for (ptr_kind, pointee_pretty) in ptr_infos
                {
                    if let Some(pointee_syms) = id_table.get(&pointee_pretty) {
                        // We need to find the first symbol that's referring to a type.
                        // Conveniently, for C++, these will always start with `T_`,
                        // which is nice because we can't do a lookup in meta right now.
                        // TODO: Generalize to better understand what's a type, especially
                        // in JS.  It might be easiest to sidestep this problem by having
                        // the analyzer be emitting structured information for the field
                        // so that we're just working in symbol space in the first place.
                        let best_sym = pointee_syms.iter().find(|s| s.starts_with("T_"));
                        if let Some(sym) = best_sym {
                            field.pointer_info.push(StructuredPointerInfo {
                                kind: ptr_kind.clone(),
                                sym: sym.clone(),
                            });

                            let member_uses = field_member_use_table
                                .entry(sym.clone())
                                .or_insert_with(|| BTreeMap::new());
                            let use_details = member_uses
                                .entry(meta.sym.clone())
                                .or_insert_with(|| Vec::new());
                            use_details.push((field.pretty.clone(), ptr_kind));
                        }
                    } else {
                        info!(
                            pretty = pointee_pretty.as_str(),
                            "Unable to map pretty identifier to symbols."
                        );
                    }
                }
            }
        }
    }

    // ### Process Ontology Rules
    for (pretty_id, rule) in ontology.config.pretty.iter() {
        // #### Labels we just slap on
        if rule.labels.len() > 0 {
            if let Some(root_syms) = id_table.get(&pretty_id) {
                for sym in root_syms {
                    if let Some(sym_meta) = meta_table.get_mut(&sym) {
                        for label in &rule.labels {
                            sym_meta.labels.insert(label.clone());
                        }
                    }
                }
            }
        }

        // #### Runnables
        if rule.runnable {
            info!(" Processing pretty runnable rule for: {}", pretty_id);
            if let Some(root_method_syms) = id_table.get(&pretty_id) {
                // The list of symbols to process for the runnable relationship.
                // We process the root syms to find their descendants, but we
                // don't actually process the root symbols.  These pending syms
                // will both be directly processed and have their children
                // appended as well.
                let mut pending_method_syms = vec![];
                for sym in root_method_syms {
                    if let Some(sym_meta) = meta_table.get(&sym) {
                        for over in &sym_meta.overridden_by_syms {
                            pending_method_syms.push(over.clone());
                        }
                    }
                }

                info!("  found {} initial method syms", pending_method_syms.len());

                // (this is LIFO traversal, which is fine for us)
                while let Some(method_sym) = pending_method_syms.pop() {
                    info!("  processing method sym: {}", method_sym);

                    // use the method to find its owning class
                    let class_sym = if let Some(method_meta) = meta_table.get(&method_sym) {
                        for over in &method_meta.overridden_by_syms {
                            pending_method_syms.push(over.clone());
                        }

                        match method_meta.parent_sym {
                            Some(p) => p,
                            _ => continue,
                        }
                    } else {
                        continue;
                    };

                    info!("  found class sym: {}", class_sym);

                    // ### use the class to find its constructors
                    let constructor_syms = if let Some(class_meta) = meta_table.get(&class_sym) {
                        let mut syms = vec![];
                        let class_name = class_meta.pretty.rsplit("::").next().unwrap();
                        // We expect the constructors to have the same name as the class; currently
                        // for C++ we don't actually emit a special "props" "constructor" value.
                        let constructor_pretty =
                            ustr(&format!("{}::{}", class_meta.pretty, class_name));
                        for method in &class_meta.methods {
                            // Skip constructors that aren't known; this can happen for the copy
                            // constructor/etc.
                            if method.pretty == constructor_pretty
                                && table.contains_key(&method.sym)
                            {
                                syms.push(method.sym);
                            }
                        }
                        syms
                    } else {
                        continue;
                    };

                    info!("  found constructor syms: {:?}", constructor_syms);

                    // ### mutate each of the constructors to have the ontology slot
                    for con_sym in &constructor_syms {
                        if let Some(con_meta) = meta_table.get_mut(&con_sym) {
                            // XXX we could track precedence for runnable rules so that
                            // we could remove lower precedence relationships here.  This
                            // would be relevant for WorkerRunnable.

                            con_meta.ontology_slots.push(OntologySlotInfo {
                                slot_kind: OntologySlotKind::RunnableMethod,
                                syms: vec![method_sym],
                            });
                        }
                    }

                    // ### mutate our method_sym to have the ontology slot to the constructors
                    if let Some(method_meta) = meta_table.get_mut(&method_sym) {
                        method_meta.ontology_slots.push(OntologySlotInfo {
                            slot_kind: OntologySlotKind::RunnableConstructor,
                            syms: constructor_syms,
                        })
                    }
                }
            }
        }

        // #### Class Labeling (Some)
        //
        // Some rules are processed as we process structured fields above.

        if let Some(label_rule) = &rule.label_containing_class {
            info!(
                " Processing pretty label_containing_class for: {}",
                pretty_id
            );
            if let Some(root_class_syms) = id_table.get(&pretty_id) {
                let mut investigate_class_syms = vec![];
                // We don't care about the root itself, just its subclasses.
                for sym in root_class_syms {
                    if let Some(sym_meta) = meta_table.get(&sym) {
                        for sub in &sym_meta.subclass_syms {
                            investigate_class_syms.push(sub.clone());
                        }
                    }
                }

                while let Some(class_sym) = investigate_class_syms.pop() {
                    let sym_meta = match meta_table.get(&class_sym) {
                        Some(m) => m,
                        None => continue,
                    };

                    for sub in &sym_meta.subclass_syms {
                        investigate_class_syms.push(sub.clone());
                    }

                    // The structured record currently doesn't have a reference
                    // to its containing symbol; we need to pop the last pretty
                    // segment and perform a lookup.
                    let (pieces, delim) = split_pretty(&sym_meta.pretty, &sym_meta.sym);
                    let containing_pieces = match pieces.split_last() {
                        Some((_, rest)) => rest,
                        None => continue,
                    };
                    let containing_pretty = containing_pieces.join(delim);
                    let containing_pretty_ustr = ustr(&containing_pretty);
                    if let Some(containing_syms) = id_table.get(&containing_pretty_ustr) {
                        for sym in containing_syms {
                            if let Some(containing_meta) = meta_table.get_mut(&sym) {
                                for rule in &label_rule.labels {
                                    containing_meta.labels.insert(rule.label.clone());
                                }
                            }
                        }
                    }
                }
            }
        }

        // #### Field Labeling
        //
        // We start from an ancestral class and find all of its subclasses and all of their fields.
        // For each field, we check its uses and see if they match the rules.  If so, we will plan
        // to add a label to the field on its class.  (Currently we do not do anythign to the
        // structured info for field symbol itself.)
        if let Some(label_rule) = &rule.label_containing_class_field_uses {
            info!(
                " Processing pretty label_containing_class_field_uses rule for: {}",
                pretty_id
            );
            if let Some(root_class_syms) = id_table.get(&pretty_id) {
                let mut investigate_class_syms = vec![];
                // We don't care about the root itself, just its subclasses.
                for sym in root_class_syms {
                    if let Some(sym_meta) = meta_table.get(&sym) {
                        for sub in &sym_meta.subclass_syms {
                            investigate_class_syms.push(sub.clone());
                        }
                    }
                }

                while let Some(class_sym) = investigate_class_syms.pop() {
                    let sym_meta = match meta_table.get(&class_sym) {
                        Some(m) => m,
                        None => continue,
                    };

                    for sub in &sym_meta.subclass_syms {
                        investigate_class_syms.push(sub.clone());
                    }

                    // The structured record currently doesn't have a reference
                    // to its containing symbol; we need to pop the last pretty
                    // segment and perform a lookup.
                    let (pieces, delim) = split_pretty(&sym_meta.pretty, &sym_meta.sym);
                    let containing_pieces = match pieces.split_last() {
                        Some((_, rest)) => rest,
                        None => continue,
                    };
                    let containing_pretty = containing_pieces.join(delim);
                    let containing_pretty_ustr = ustr(&containing_pretty);
                    if let Some(containing_syms) = id_table.get(&containing_pretty_ustr) {
                        for sym in containing_syms {
                            if let Some(containing_meta) = meta_table.get_mut(&sym) {
                                for field in &mut containing_meta.fields {
                                    if let Some(kind_map) = table.get(&field.sym) {
                                        if let Some(path_hits) = kind_map.get(&AnalysisKind::Use) {
                                            for hits in path_hits.values() {
                                                for hit in hits {
                                                    for rule in &label_rule.labels {
                                                        if hit.context.ends_with(
                                                            rule.context_sym_suffix.as_str(),
                                                        ) {
                                                            field.labels.insert(rule.label.clone());
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Consume the ontology logged span, pass it through our ontology-ingestion
    // explainer template, and write it to disk.
    drop(ontology_entered);
    {
        let ingestion_json = logged_ontology_span.retrieve_serde_json().await;
        let crossref_diag_dir = format!("{}/diags/crossref", tree_config.paths.index_path);
        let ingestion_diag_path = format!("{}/ontology_ingestion.md", crossref_diag_dir);
        create_dir_all(crossref_diag_dir).unwrap();

        let globals = liquid::object!({
            "logs": vec![ingestion_json],
        });
        let explain_template = build_and_parse_ontology_ingestion_explainer();
        let output = explain_template.render(&globals).unwrap();
        std::fs::write(ingestion_diag_path, output).unwrap();
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
                AnalysisKind::Alias => "aliases",
            };
            kindmap.insert(kindstr.to_string(), json!(result));
        }
        if let Some(callee_syms) = callees_table.get(&id) {
            let mut callees = Vec::new();
            for (callee_sym, (call_path, call_lines)) in callee_syms {
                if let Some(meta) = meta_table.get(callee_sym) {
                    let mut obj = BTreeMap::new();
                    obj.insert("sym".to_string(), callee_sym.to_string());
                    if let Some(pretty) = pretty_table.get(callee_sym) {
                        obj.insert("pretty".to_string(), pretty.to_string());
                    }
                    obj.insert("kind".to_string(), meta.kind.to_string());
                    obj.insert(
                        "jump".to_string(),
                        format!("{}#{}", call_path, call_lines.iter().join(",")),
                    );
                    callees.push(json!(obj));
                }
            }
            kindmap.insert("callees".to_string(), json!(callees));
        }
        if let Some(fmu_syms) = field_member_use_table.get(&id) {
            let mut fmus = Vec::new();
            for (fmu_sym, fmu_field_infos) in fmu_syms {
                if let Some(meta) = meta_table.get(fmu_sym) {
                    let mut fields = vec![];
                    for (field_pretty, ptr_kind) in fmu_field_infos {
                        fields.push(json!({
                            "pretty": field_pretty,
                            "ptr": ptr_kind,
                        }));
                    }
                    fmus.push(json!({
                        "sym": fmu_sym,
                        "pretty": meta.pretty,
                        "fields": fields,
                    }));
                }
            }
            kindmap.insert("field-member-uses".to_string(), json!(fmus));
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
            let line = format!("{} {}\n", id, sym);
            let _ = idf.write_all(line.as_bytes());
        }
    }

    ingestion
        .state
        .write_out_concise_file_info(&tree_config.paths.index_path);
}
