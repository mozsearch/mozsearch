use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::fs::File;
use std::fs::{create_dir_all, remove_file};
use std::io::BufRead;
use std::io::BufReader;
use std::io::BufWriter;
use std::io::Read;
use std::io::Write;
use std::time::Instant;

#[macro_use]
extern crate tracing;

use chrono::Local;
use rayon::prelude::*;
extern crate clap;
use clap::Parser;
use itertools::Itertools;
use serde::Serialize;
use serde_json::Map;
extern crate tools;
use tools::cmd_pipeline::symbol_graph::is_implementable;
use tools::dataflow::summary::{
    Category, FieldEdge, Summary, field_class, hint_summary, is_field_sym, is_value_type_class,
    parse_hint_flow, untemplated_pretty,
};
use tools::file_format::analysis::AnalysisStructured;
use tools::file_format::analysis::BindingSlotLang;
use tools::file_format::analysis::OntologySlotInfo;
use tools::file_format::analysis::OntologySlotKind;
use tools::file_format::analysis::PathSearchResult;
use tools::file_format::analysis::StructuredPointerInfo;
use tools::file_format::analysis::StructuredTag;
use tools::file_format::analysis::{
    AnalysisKind, AnalysisTarget, BindingSlotProps, Location, SearchResult,
    StructuredBindingSlotInfo, WithLocation, collect_file_syms_from_target, read_analysis,
    read_structured, read_target,
};
use tools::file_format::analysis::{BindingOwnerLang, BindingSlotKind, DocLink};
use tools::file_format::analysis_manglings::make_file_sym_from_path;
use tools::file_format::analysis_manglings::split_pretty;
use tools::file_format::bisectable_mmap::BisectableMmap;
use tools::file_format::config;
use tools::file_format::config::{Config, FindSourceFile, TreeConfig};
use tools::file_format::crossref::Callee;
use tools::file_format::crossref::CrossrefData;
use tools::file_format::crossref::FieldInfos;
use tools::file_format::crossref::FieldMemberUse;
use tools::file_format::jumpref::{TreeJumprefs, convert_crossref_value_to_sym_info_rep};
use tools::file_format::ontology_mapping::OntologyRunnableMode;
use tools::file_format::ontology_mapping::{
    OntologyLabelOwningClass, OntologyMappingIngestion, OntologyPointerKind,
};
use tools::file_format::recency::{FileRecency, Recency, RecencyFrom};
use tools::file_format::repo_data_ingestion::{
    ConcisePerFileInfo, RepoIngestion, heuristic_path_kind,
};
use tools::file_format::rollup::{
    ROLLUP_EDGES, ROLLUP_EXAMPLE_EDGES, ROLLUP_EXAMPLES, ROLLUP_MEMBERS, ROLLUP_TREE_EDGES, Rollup,
    RollupEdge, RollupExample, RollupMember, RollupPair, directory_subsystems, is_boring_callee,
    root_subsystem_sym, subsystem_sym,
};
use tools::hyperblame::recency::{CONTEXT_KINDS, FileLinkage, HistoryDigests, link_definitions};
use tools::logging::LoggedSpan;
use tools::logging::init_logging;
use tools::rendered_docs::is_doc_path;
use tools::templating::builder::build_and_parse_ontology_ingestion_explainer;
use tools::templating::builder::build_and_parse_repo_ingestion_explainer;
use tools::tree_sitter_support::preprocessor::{ConditionalStacks, is_c_family};
use tools::utils::case_semisensitive_str_cmp;
use ustr::Ustr;
use ustr::UstrMap;
use ustr::UstrSet;
use ustr::ustr;

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

    #[clap(value_parser)]
    thread_count: usize,

    /// Cross-reference only the listed analysis files, for a try push's
    /// semantic delta (see `bin/build-try-delta.rs`): with the per-file
    /// information of the tree's index at this path (rather than ingesting
    /// the whole tree), without history digests, writing only the crossref,
    /// jumpref, and identifiers files, into `--index-path`.
    #[clap(long)]
    subset_of: Option<String>,

    /// Instead of the config's index path (with the analysis files in its
    /// `analysis/`).
    #[clap(long)]
    index_path: Option<String>,

    /// Instead of the config's files path (ex: a try push's versions of the
    /// files, for `--subset-of`).
    #[clap(long)]
    files_path: Option<String>,

    /// Instead of the config's objdir path.
    #[clap(long)]
    objdir_path: Option<String>,
}

/// The repo ingestion of a subset of the tree's files (see
/// `CrossrefCli::subset_of`): the per-file information of the tree's index at
/// `base_index`, and for files it doesn't have (ex: a try push's new files),
/// their heuristic path kinds.
fn subset_ingestion(cfg: &Config, base_index: &str, paths: &[Ustr]) -> RepoIngestion {
    let per_file_info_toml_str = cfg
        .read_tree_config_file_with_default("per-file-info.toml")
        .unwrap();
    let mut ingestion = RepoIngestion::new(&per_file_info_toml_str)
        .expect("Your per-file-info.toml file has issues");
    let concise_path = format!("{}/concise-per-file-info.json", base_index);
    let concise: BTreeMap<Ustr, ConcisePerFileInfo<Ustr>> =
        serde_json::from_reader(BufReader::new(File::open(&concise_path).unwrap())).unwrap();
    let wanted: UstrSet = paths.iter().copied().collect();
    ingestion.state.concise_per_file.extend(
        concise
            .into_iter()
            .filter(|(path, _)| wanted.contains(path)),
    );
    for path in paths {
        if !ingestion.state.concise_per_file.contains_key(path) {
            let path_kind = heuristic_path_kind(&ingestion.config.pathkind, path);
            ingestion
                .state
                .with_file_info(path, false, |info, _| info.path_kind = path_kind);
        }
    }
    ingestion
}

// Nested table hierarchy keyed by: [symbol, kind, path] with Vec<SearchResult>
// as the leaf values.
type SearchResultTable = BTreeMap<Ustr, BTreeMap<AnalysisKind, BTreeMap<Ustr, Vec<SearchResult>>>>;

// The linearlied SearchResultTable to feed to the writer threads.
type SearchResultList = Vec<(
    Ustr,
    BTreeMap<AnalysisKind, BTreeMap<Ustr, Vec<SearchResult>>>,
)>;

// The output from the reader threads for SearchResultTable.
type SearchResultItems = Vec<(Ustr, AnalysisKind, Ustr, SearchResult)>;
type TopLevelSearchResultItems = Vec<Ustr>;

// Maps (raw) symbol to interned-pretty symbol string.  Each raw symbol is
// unique, but there may be many raw symbols that map to the same pretty symbol
// string.
type PrettyTable = HashMap<Ustr, Ustr>;

// The output from the reader threads for PrettyTable.
type PrettyItems = Vec<(Ustr, Ustr)>;

// Reverse of pretty_table.  The key is the pretty symbol, and the value is a
// UstrSet of all of the raw symbols that map to the pretty symbol.  Pretty
// symbols that start with numbers or include whitespace are considered illegal
// and not included in the map.
//
// This table has been modified so that it is populated with the suffix
// variations immediately.  So for the symbol "foo::bar::Baz" we will add
// entries for "Baz", "bar::Baz", and "foo::bar::Baz".  Previously we would
// only add the full variation and compute the suffixes when writing its
// contents out, but we now need/want this for processing field type strings
// because we do not currently have the fully qualified symbols available.
// In the future we hopefully will have better type representations for fields.
//
// An alternate approach would be for us to write the identifier table out
// earlier and just memory map that for subsequent processing.  Not doing that
// right now because the ustr rep potentially could end up comparable in memory
// usage if the identifer file is fully paged in, and for performance we would
// want it fully paged in, so might as well use the memory so we fail faster if
// we don't have the memory available.
type IdTable = UstrMap<UstrSet>;

// The output from the reader threads for IdTable.
type IdItems = Vec<(Ustr, Ustr)>;

// Maps (raw) symbol to `SymbolMeta` info for this symbol.  Currently, we
// require that the language analyzer created a "structured" record and we
// use that, but it could make sense for us to automatically generate a stub
// meta for symbols for which we didn't find a structured record.  A minor
// awkwardness here is that we would really want to use the "source" records
// for this (as we did prior to the introduction of the structured record
// type), but we currently don't retain those.  (But we do currently read
// the file 2x; maybe it would be better to read it once and have the
// records grouped by type so we can improve that).
type MetaTable = BTreeMap<Ustr, AnalysisStructured>;

// The output from the reader threads for MetaTable.
type StructuredItems = Vec<AnalysisStructured>;

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
//
// The tuple's last member is whether the symbol only cites the "callee" (only
// doc-uses it, ex: a Gecko function's comment citing a web spec's algorithm),
// which if it's callable, the symbol implements rather than calls (see
// `Callee::implements`).
type CalleesTable = BTreeMap<Ustr, BTreeMap<Ustr, (Ustr, BTreeSet<u32>, bool)>>;

// The output from the reader threads for CalleesTable: (symbol, callee, path,
// line, whether it's a citation).
type CalleesItems = Vec<(Ustr, Ustr, Ustr, u32, bool)>;

// Uses' contexts which fill binding slots of the uses' symbols (see
// `AnalysisTarget::slot`): (symbol, its pretty name, context symbol, slot
// kind, the context's language).
type UseSlotItems = Vec<(Ustr, Ustr, Ustr, BindingSlotKind, BindingSlotLang)>;

/// The binding slot language of a file's symbols, by its path.
fn path_slot_lang(path: &str) -> Option<BindingSlotLang> {
    let ext = path.rsplit_once('.')?.1;
    match ext {
        "c" | "cc" | "cpp" | "cxx" | "h" | "hh" | "hpp" | "hxx" | "mm" | "m" => {
            Some(BindingSlotLang::Cpp)
        }
        "js" | "jsm" | "mjs" | "cjs" | "sjs" | "ts" | "mts" => Some(BindingSlotLang::JS),
        "rs" => Some(BindingSlotLang::Rust),
        "java" | "kt" => Some(BindingSlotLang::Jvm),
        _ => None,
    }
}

/// The structured record of a symbol whose uses' contexts fill its binding
/// slots, if its analysis has none (see `BindingOwnerLang::Event`).
fn event_structured(sym: Ustr, pretty: Ustr) -> AnalysisStructured {
    AnalysisStructured {
        structured: StructuredTag::Structured,
        pretty,
        sym,
        js_sym: None,
        type_pretty: None,
        kind: ustr("event"),
        subsystem: None,
        parent_sym: None,
        slot_owner: None,
        impl_kind: ustr("event"),
        size_bytes: None,
        alignment_bytes: None,
        own_vf_ptr_bytes: None,
        binding_slots: vec![],
        ontology_slots: vec![],
        supers: vec![],
        methods: vec![],
        fields: vec![],
        overrides: vec![],
        props: vec![],
        labels: BTreeSet::default(),
        subclass_syms: vec![],
        overridden_by_syms: vec![],
        variants: vec![],
        supplement: false,
        docs: vec![],
        extra: Map::default(),
        can_gc: None,
        gc_path: None,
    }
}

// Maps the (raw) symbol corresponding to a type to a BTreeMap whose key
// is the class referencing the type and whose values are a vec of tuples of
// the form (field pretty, pointer kind).
type FieldMemberUseTable = BTreeMap<Ustr, BTreeMap<Ustr, Vec<(Ustr, OntologyPointerKind)>>>;

// As we process the source entries and build the SourceMeta, we keep a running
// list of what cross-SourceMeta links need to be established.  We then process
// this after all of the files have been processed and we know all symbols are
// known.

// Pairs of [parent class sym, subclass sym] to add subclass to parent.
type XrefLinkSubclass = Vec<(Ustr, Ustr)>;

// Pairs of [parent method sym, overridden by sym] to add the override to the
// parent.
type XrefLinkOverride = Vec<(Ustr, Ustr)>;

// (owner symbol, slotted symbol) -> slot props
// This is a BTreeMap and not a HashMap to force a stable ordering and avoid
// flaky tests.
type XrefLinkSlots = BTreeMap<(Ustr, Ustr), (BindingSlotProps, Option<Ustr>)>;

// The output from the reader threads for XrefLinkSlots, with whether the
// slotted symbol is the pretty name of the symbols to slot (for slots whose
// `sym` is empty, which `resolve_pretty_slots` resolves).
type XrefLinkSlotsItems = Vec<((Ustr, Ustr), (BindingSlotProps, Option<Ustr>, bool))>;

// Maps JS symbol to possible IDL/Glean symbols.
type JSIDLTable = HashMap<Ustr, Vec<Ustr>>;
const MAX_JS_IDL_SYMS: usize = 4;

// Maps JavaScript symbols to possible TypeScript symbols.
type JsTsTable = HashMap<Ustr, Vec<Ustr>>;

#[allow(clippy::too_many_arguments)]
fn process_analysis_target(
    mut piece: AnalysisTarget,
    path: &Ustr,
    fallback_file_sym: &Ustr,
    lineno: usize,
    loc: &Location,
    search_result_items: &mut SearchResultItems,
    pretty_items: &mut PrettyItems,
    id_items: &mut IdItems,
    callees_items: &mut CalleesItems,
    use_slot_items: &mut UseSlotItems,
    lines: &[(String, u32)],
    pp: Ustr,
) {
    if piece.pretty.is_empty() {
        info!("Skipping empty pretty for symbol {}", piece.sym);
        return;
    }

    // The C++ indexer now should properly emit context for all FILE symbols
    // of the file that they are found in, but we're retaining this for older
    // C++ code and because it might be useful for other indexers that emit FILE symbols.
    if piece.sym.starts_with("FILE_") && piece.contextsym.is_empty() {
        piece.context = *path;
        piece.contextsym = *fallback_file_sym;
    }

    let (line, offset) = lines[lineno].clone();

    // There cam be multiple identical items for the the symbol -> pretty symbol
    // mapping, which will be de-duplicated when merging.
    // The mapping from sym to pretty should be unique.
    pretty_items.push((piece.sym, piece.pretty));

    // If this is a use and there's a contextsym, we want to create a "callees"
    // entry under the contextsym.  We also want to invert the use of "context"
    // to be the symbol in question; it's not useful to name the context symbol
    // redundantly when it's the symbol we're attaching data to.
    //
    // (But a use whose context receives its symbol, ex: an observer's
    // comparison of its topic with an observer topic, doesn't call it, and
    // nor does a use of a C++ local or parameter; see `local_symbols`.)
    let is_local = piece.sym.starts_with("V_");
    if piece.kind == AnalysisKind::Use
        && !piece.contextsym.is_empty()
        && piece.slot != Some(BindingSlotKind::Recv)
        && !is_local
    {
        callees_items.push((piece.contextsym, piece.sym, *path, loc.lineno, false));
    }
    // (A citation of a callable symbol, ex: a web spec's algorithm, is an
    // implementation of it; see `CalleesTable`.)
    if piece.kind == AnalysisKind::DocUse && !piece.contextsym.is_empty() {
        callees_items.push((piece.contextsym, piece.sym, *path, loc.lineno, true));
    }
    if let Some(slot) = piece.slot
        && !piece.contextsym.is_empty()
        && let Some(lang) = path_slot_lang(path)
    {
        use_slot_items.push((piece.sym, piece.pretty, piece.contextsym, slot, lang));
    }

    search_result_items.push((
        piece.sym,
        piece.kind,
        *path,
        SearchResult {
            lineno: loc.lineno,
            bounds: (loc.col_start - offset, loc.col_end - offset),
            line,
            context: piece.context,
            contextsym: piece.contextsym,
            peek_range: piece.peek_range,
            pp,
            ctxtra: ustr(
                &piece
                    .ctxtra
                    .iter()
                    .map(|key| key.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
        },
    ));

    // Idempotently insert the pretty identifier -> symbol mapping as long as the
    // symbol looks sane.  (The `identifiers` file's lines are identifiers and
    // their symbols, separated by the line's last space, so symbols can't have
    // whitespace, and identifiers' other whitespace, ex: a web spec's term's
    // newline, is spaces.)
    let ch = piece.sym.chars().next().unwrap();
    if !ch.is_ascii_digit() && !piece.sym.contains(char::is_whitespace) {
        let pretty = if piece
            .pretty
            .contains(|c: char| c.is_whitespace() && c != ' ')
        {
            piece
                .pretty
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            piece.pretty.to_string()
        };
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
        // already.)  But web specs' terms are split by their spec and
        // "for"s, ex: "html Navigation/navigate(url, options)" is also
        // "Navigation/navigate(url, options)" and "navigate(url, options)".
        let split_sym = if piece.sym.starts_with("SPEC_") {
            piece.sym.as_str()
        } else {
            ""
        };
        let (components, delim) = split_pretty(&pretty, split_sym);
        // (A C++ local's or parameter's name alone, ex: "aCx" or "i", would
        // be one of thousands, so it's only an identifier with its
        // function's, ex: "Scale::aFactor".)
        let last = if is_local {
            components.len().saturating_sub(1)
        } else {
            components.len()
        };
        for i in 0..last {
            let sub = &components[i..components.len()];
            let sub = sub.join(delim);

            if sub.is_empty() {
                continue;
            }

            id_items.push((ustr(&sub), piece.sym));
        }
    }
}

/// Binding slots by pretty name (owner symbol, pretty name, props, subsystem)
/// become slots of the symbols whose structured records have that pretty name,
/// once all of the structured records are in the `meta_table`.  (Pretty names
/// that no symbol has don't become slots.)
fn resolve_pretty_slots(
    pretty_slots: Vec<(Ustr, Ustr, BindingSlotProps, Option<Ustr>)>,
    meta_table: &MetaTable,
    xref_link_slots: &mut XrefLinkSlots,
) {
    if pretty_slots.is_empty() {
        return;
    }
    let wanted: HashSet<Ustr> = pretty_slots.iter().map(|slot| slot.1).collect();
    let mut syms_by_pretty: HashMap<Ustr, Vec<Ustr>> = HashMap::new();
    for (sym, meta) in meta_table {
        if wanted.contains(&meta.pretty) {
            syms_by_pretty.entry(meta.pretty).or_default().push(*sym);
        }
    }
    for (owner_sym, pretty, props, subsystem) in pretty_slots {
        for slotted_sym in syms_by_pretty.get(&pretty).into_iter().flatten() {
            xref_link_slots.insert((owner_sym, *slotted_sym), (props, subsystem));
        }
    }
}

fn process_analysis_structured(
    mut piece: AnalysisStructured,
    subsystem: Option<Ustr>,
    structured_items: &mut Vec<AnalysisStructured>,
    xref_link_subclass: &mut XrefLinkSubclass,
    xref_link_override: &mut XrefLinkOverride,
    xref_link_slots_items: &mut XrefLinkSlotsItems,
) {
    for super_info in &piece.supers {
        xref_link_subclass.push((super_info.sym, piece.sym));
    }

    for override_info in &piece.overrides {
        xref_link_override.push((override_info.sym, piece.sym));
    }

    // We remove all bindings infos from AnalysisStructured instances here
    // but add them back both ways when we iterate over xref_link_slots_items.
    for slot_info in piece.binding_slots.drain(..) {
        match slot_info.pretty {
            Some(pretty) if slot_info.sym.is_empty() => xref_link_slots_items
                .push(((piece.sym, pretty), (slot_info.props, subsystem, true))),
            _ => xref_link_slots_items.push((
                (piece.sym, slot_info.sym),
                (slot_info.props, subsystem, false),
            )),
        }
    }
    if let Some(slot_info) = piece.slot_owner.take() {
        xref_link_slots_items.push((
            (slot_info.sym, piece.sym),
            (slot_info.props, subsystem, false),
        ));
    }

    piece.subsystem = subsystem;

    structured_items.push(piece)
}

/// Directories' subsystems for the files (see `directory_subsystems`), if
/// the tree's per-file info gives none of its files a subsystem.
fn fallback_subsystems(ingestion: &RepoIngestion, paths: &[Ustr]) -> HashMap<Ustr, Ustr> {
    let has_subsystems = ingestion
        .state
        .concise_per_file
        .values()
        .any(|info| info.subsystem.is_some());
    if has_subsystems {
        return HashMap::new();
    }
    directory_subsystems(paths)
}

/// A file's subsystem: its per-file info's, else its directory's (see
/// `fallback_subsystems`); and its file symbols' structured records.
fn make_subsystem(
    path: &Ustr,
    file_syms: &Vec<String>,
    ingestion: &RepoIngestion,
    dir_subsystems: &HashMap<Ustr, Ustr>,
    structured_items: &mut StructuredItems,
    pretty_items: &mut PrettyItems,
    id_items: &mut IdItems,
) -> Option<Ustr> {
    let concise_info = ingestion.state.concise_per_file.get(path);

    let subsystem = concise_info
        .and_then(|info| info.subsystem)
        .or_else(|| dir_subsystems.get(path).copied());

    for file_sym_str in file_syms {
        let file_sym = ustr(file_sym_str);
        let file_structured = AnalysisStructured {
            structured: StructuredTag::Structured,
            pretty: *path,
            sym: file_sym,
            js_sym: None,
            type_pretty: None,
            kind: ustr("file"),
            subsystem,
            // For most analytical purposes, we want to think of files as atomic,
            // so I don't think there is any upside to modeling the containing
            // directory as a parent.  Especially since we don't yet have a
            // `DIR_blah` symbol type yet or a clear reason to want one.
            parent_sym: None,
            slot_owner: None,
            impl_kind: ustr("impl"),
            size_bytes: None,
            alignment_bytes: None,
            own_vf_ptr_bytes: None,
            binding_slots: vec![],
            ontology_slots: vec![],
            supers: vec![],
            methods: vec![],
            fields: vec![],
            overrides: vec![],
            props: vec![],
            labels: BTreeSet::default(),

            subclass_syms: vec![],
            overridden_by_syms: vec![],
            variants: vec![],
            supplement: false,
            docs: vec![],
            extra: Map::default(),

            can_gc: None,
            gc_path: None,
        };
        structured_items.push(file_structured);
        pretty_items.push((file_sym, *path));
        id_items.push((*path, file_sym));
    }
    subsystem
}

fn line_to_buf_and_offset(line: String) -> (String, u32) {
    let line_cut = line.trim_end();
    let len = line_cut.len();
    let line_cut = line_cut.trim_start();
    let offset = (len - line_cut.len()) as u32;
    let buf: String = line_cut.chars().take(100).collect();
    (buf, offset)
}

struct AllFilesAndDirs {
    analysis_relative_paths: Vec<Ustr>,
    all_files_paths: Vec<Ustr>,
    all_dirs_paths: Vec<Ustr>,
}

/// The analysis files a list names, sorted, so that what crossref makes of
/// them doesn't depend on the list's order: they're merged in its order, and
/// the first of a symbol's structured records wins (as do other firsts, ex: a
/// call's lines are its first file's), and the list was in the order `find`
/// listed the files, which their file system gives (ex: the tests tree's
/// `T_Foo`, `T_Base` and `_Z4funcv`, defined in several test files, some in
/// Core/Templates and some in no subsystem, made that subsystem's internal
/// calls 7, 9 or 12 depending on the order, so its roll-up's snapshot failed
/// GitHub's CI now and then).
fn read_analysis_list(path: &str) -> Vec<Ustr> {
    let mut paths: Vec<Ustr> = BufReader::new(File::open(path).unwrap())
        .lines()
        .map(|x| ustr(&x.unwrap()))
        .collect();
    paths.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    paths
}

fn load_all_files_and_dirs(
    analysis_files_list_path: &String,
    index_path: &String,
) -> AllFilesAndDirs {
    // This is just the list of analysis files.
    let analysis_relative_paths = read_analysis_list(analysis_files_list_path);

    let all_files_list_path = format!("{}/all-files", index_path);

    let all_files_paths: Vec<Ustr> = fs::read_to_string(all_files_list_path)
        .unwrap()
        .lines()
        .map(ustr)
        .collect();

    let all_dirs_list_path = format!("{}/all-dirs", index_path);

    let all_dirs_paths: Vec<Ustr> = fs::read_to_string(all_dirs_list_path)
        .unwrap()
        .lines()
        .map(ustr)
        .collect();

    AllFilesAndDirs {
        analysis_relative_paths,
        all_files_paths,
        all_dirs_paths,
    }
}

fn ingest_files(
    cfg: &Config,
    tree_config: &TreeConfig,
    tree_name: &String,
    all_files_paths: Vec<Ustr>,
    all_dirs_paths: Vec<Ustr>,
) -> RepoIngestion {
    let per_file_info_toml_str = cfg
        .read_tree_config_file_with_default("per-file-info.toml")
        .unwrap();
    let mut ingestion = RepoIngestion::new(&per_file_info_toml_str)
        .expect("Your per-file-info.toml file has issues");
    ingestion.ingest_file_list_and_apply_heuristics(&all_files_paths, tree_config);
    ingestion.ingest_dir_list(&all_dirs_paths, tree_config);

    ingestion
        .ingest_files(|root: &str, file: &str| {
            cfg.maybe_read_file_from_given_root(tree_name, root, file)
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

    ingestion
}

async fn write_repo_ingestion_diag(tree_config: &TreeConfig, logged_ingestion_span: LoggedSpan) {
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

fn load_ontology(cfg: &Config) -> OntologyMappingIngestion {
    let ontology_toml_str = cfg
        .read_tree_config_file_with_default("ontology-mapping.toml")
        .unwrap();
    OntologyMappingIngestion::new(&ontology_toml_str).expect("ontology-mapping.toml has issues")
}

/// (sym, whether it's a definition rather than a declaration, path, its
/// history digest there, and the history context it's from)
type RecencyItems = Vec<(Ustr, bool, Ustr, Recency, String)>;

struct PerThreadAnalysisData {
    search_result_items: SearchResultItems,
    top_level_search_result_items: TopLevelSearchResultItems,
    pretty_items: PrettyItems,
    id_items: IdItems,
    structured_items: StructuredItems,
    callees_items: CalleesItems,
    use_slot_items: UseSlotItems,
    xref_link_subclass: XrefLinkSubclass,
    xref_link_override: XrefLinkOverride,
    xref_link_slots_items: XrefLinkSlotsItems,
    recency_items: RecencyItems,
    /// The files' digests for what symbols' don't cover.
    file_recency_items: Vec<(Ustr, FileRecency)>,
    /// How many files had history digests, how many the history had but
    /// didn't line up with, and how many (of the first) the history has no
    /// grammar for.
    recency_files: (usize, usize, usize),
    /// How many definitions linked to their own history contexts, and the
    /// files with definitions that didn't but should have.
    recency_linked: u32,
    recency_linkages: Vec<(Ustr, FileLinkage)>,
}

/// The history digests of the symbols defined (or declared) in a file: the
/// changes of the history contexts their definitions are in, if those are
/// their own (see `hyperblame::recency`), and the file's for what theirs don't
/// cover.  Returns whether the history lined up with the file, if it has the
/// file, and how its definitions linked.
fn file_recency_items(
    history: &HistoryDigests,
    path: &Ustr,
    source_fname: &str,
    analysis: &[WithLocation<Vec<AnalysisTarget>>],
    context_syms: &UstrMap<bool>,
    items: &mut RecencyItems,
    file_items: &mut Vec<(Ustr, FileRecency)>,
) -> Option<(bool, FileLinkage)> {
    let source = fs::read_to_string(source_fname).ok()?;
    if !history.has_file(path) {
        return None;
    }
    let Some(digests) = history.file(path, &source) else {
        return Some((false, FileLinkage::default()));
    };
    if !digests.has_grammar {
        // (No contexts to link to, so just the whole file's digest; see
        // `FileDigests::file_recency`.)
        file_items.push((*path, digests.file_recency()));
        return Some((
            true,
            FileLinkage {
                no_grammar: true,
                ..Default::default()
            },
        ));
    }
    let linkage = link_definitions(
        &digests,
        &source,
        analysis,
        context_syms,
        |sym, is_def, recency, context| {
            if !recency.is_empty() {
                items.push((sym, is_def, *path, recency, context.to_string()));
            }
        },
    );
    // (Misparses also put code in the top level's or namespaces' own changes,
    // so a file with them only has the whole file's.)
    let mut file_recency = digests.file_recency();
    if linkage.unlinked_total() > 0 {
        file_recency.scopes.clear();
    }
    // (Documents' analysis is their labels and references, and specs' sources'
    // is their terms and references to them (`SPEC_` symbols, see
    // `spec_analyze`), which don't give their lines contexts, so they're as if
    // they had no analysis.)
    let is_spec_source = analysis.iter().any(|datum| {
        datum
            .data
            .iter()
            .any(|target| target.kind == AnalysisKind::Def && target.sym.starts_with("SPEC_"))
    });
    if is_doc_path(path) || is_spec_source {
        file_recency.scopes.clear();
        file_recency.analyzed = false;
    }
    if !file_recency.file.is_empty() {
        file_items.push((*path, file_recency));
    }
    Some((true, linkage))
}

#[allow(clippy::too_many_arguments)]
/// An analysis file's target and structured records, by their locations, as
/// `read_analysis` with `read_target` and with `read_structured` would read
/// them, from one read (`read_analysis` groups a location's records in their
/// order, so each kind's groups are the combined groups' records of that
/// kind).
#[allow(clippy::type_complexity)]
fn read_targets_and_structured(
    analysis_fname: &str,
) -> (
    Vec<WithLocation<Vec<AnalysisTarget>>>,
    Vec<WithLocation<Vec<AnalysisStructured>>>,
) {
    enum Record {
        Target(AnalysisTarget),
        Structured(AnalysisStructured),
    }
    let records = read_analysis(analysis_fname, &mut |obj, loc, size| match obj
        .get("target")
        .is_some()
    {
        true => read_target(obj, loc, size).map(Record::Target),
        false => read_structured(obj, loc, size).map(Record::Structured),
    });
    let (mut targets, mut structured) = (vec![], vec![]);
    for WithLocation { loc, data } in records {
        let (mut these_targets, mut these_structured) = (vec![], vec![]);
        for record in data {
            match record {
                Record::Target(target) => these_targets.push(target),
                Record::Structured(record) => these_structured.push(record),
            }
        }
        if !these_targets.is_empty() {
            targets.push(WithLocation {
                loc,
                data: these_targets,
            });
        }
        if !these_structured.is_empty() {
            structured.push(WithLocation {
                loc,
                data: these_structured,
            });
        }
    }
    (targets, structured)
}

/// The records of a chunk of the analysis files (see `analysis_chunks`).
fn read_analysis_files_thread(
    paths: &[Ustr],
    index_path: &str,
    find_source_file: &FindSourceFile,
    ingestion: &RepoIngestion,
    dir_subsystems: &HashMap<Ustr, Ustr>,
    history: Option<&HistoryDigests>,
) -> PerThreadAnalysisData {
    let mut search_result_items = SearchResultItems::new();
    let mut top_level_search_result_items = TopLevelSearchResultItems::new();
    let mut pretty_items = PrettyItems::new();
    let mut id_items = IdItems::default();
    let mut structured_items = StructuredItems::new();
    let mut callees_items = CalleesItems::new();
    let mut use_slot_items = UseSlotItems::new();
    let mut xref_link_subclass = XrefLinkSubclass::new();
    let mut xref_link_override = XrefLinkOverride::new();
    let mut xref_link_slots_items = XrefLinkSlotsItems::new();
    let mut recency_items = RecencyItems::new();
    let mut file_recencies = vec![];
    let mut recency_files = (0, 0, 0);
    let mut recency_linked = 0;
    let mut recency_linkages = vec![];

    for path in paths {
        println!("File {}", path);

        let analysis_fname = format!("{}/analysis/{}", index_path, path);
        let fallback_file_sym: Ustr = ustr(&make_file_sym_from_path(path));

        // (Its target and structured records, from one read of it, rather
        // than one each, which parsed every record twice.)
        let (analysis, structured_analysis) = read_targets_and_structured(&analysis_fname);

        let file_syms = collect_file_syms_from_target(path, &analysis);

        let subsystem = make_subsystem(
            path,
            &file_syms,
            ingestion,
            dir_subsystems,
            &mut structured_items,
            &mut pretty_items,
            &mut id_items,
        );

        // We process the structured records before checking for the source file
        // to allow us to ingest the structured records from SCIP indexing that
        // do not actually correspond to a source file.  This is the case for
        // Java imports from the JDK/Kotlin/Android runtimes.
        // (The definitions that should have their own history contexts, and
        // whether they're functions; see `file_recency_items`.)
        let mut context_syms: UstrMap<bool> = UstrMap::default();
        for datum in structured_analysis {
            for piece in datum.data {
                if CONTEXT_KINDS.contains(&piece.kind.as_str()) {
                    context_syms.insert(piece.sym, matches!(&*piece.kind, "function" | "method"));
                }
                // If we don't have a location for the structured record then
                // this is the SCIP external structured record case mentioned
                // above and we need to insert the pretty and id mappings
                // since there won't be a target record for the definition.
                if datum.loc.lineno == 0 && !piece.supplement {
                    pretty_items.push((piece.sym, piece.pretty));
                    // TODO: extract out the logic from process_analysis_target
                    // so we can generate all the suffix variations here.  But
                    // for our current needs, just the exact pretty identifier
                    // is sufficient.
                    // (The ontology rule is always on the fully qualified
                    //  pretty.)
                    id_items.push((piece.pretty, piece.sym));
                    // We also need to make sure there's a top-level entry in
                    // the search_result_table, even if it's empty, so that
                    // when we're building the crossref, the structured record
                    // gets emitted.
                    top_level_search_result_items.push(piece.sym);
                }
                process_analysis_structured(
                    piece,
                    subsystem,
                    &mut structured_items,
                    &mut xref_link_subclass,
                    &mut xref_link_override,
                    &mut xref_link_slots_items,
                );
            }
        }

        // Load the source file and chop it up into `lines` so that we extract
        // the `line` for each result.  In the future this could move to
        // dynamic extraction that uses the `peek_range` if available and this
        // line if it's not.
        let source_fname = find_source_file.find(path);
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

        if lines.is_empty() {
            // Nothing to do with empty files.
            continue;
        }

        // C-family lines' preprocessor conditionals (see `SearchResult::pp`),
        // as each distinct stack's interned string and the lines' stacks.
        let conditionals = is_c_family(path)
            .then(|| fs::read_to_string(&source_fname).ok())
            .flatten()
            .map(|text| ConditionalStacks::new(&text))
            .filter(|conditionals| !conditionals.is_empty());
        let stack_pps: Vec<Ustr> = conditionals.as_ref().map_or(vec![], |conditionals| {
            conditionals
                .stacks()
                .iter()
                .map(|stack| ustr(&stack.join("\n")))
                .collect()
        });
        let line_pp = |lineno: usize| match &conditionals {
            Some(conditionals) => stack_pps[conditionals.stack_index(lineno)],
            None => Ustr::default(),
        };

        if let Some(history) = history {
            match file_recency_items(
                history,
                path,
                &source_fname,
                &analysis,
                &context_syms,
                &mut recency_items,
                &mut file_recencies,
            ) {
                Some((true, linkage)) => {
                    recency_files.0 += 1;
                    recency_files.2 += usize::from(linkage.no_grammar);
                    recency_linked += linkage.linked;
                    if linkage.unlinked_total() > 0 {
                        println!(
                            "History contexts aren't {} definitions' own in {}",
                            linkage.unlinked_total(),
                            path
                        );
                        recency_linkages.push((*path, linkage));
                    }
                }
                Some((false, _)) => {
                    recency_files.1 += 1;
                    println!("History doesn't line up with {}", path);
                }
                None => {}
            }
        }

        for datum in analysis {
            // If we're going to experience a bad line, skip out before
            // creating any structure.
            let lineno = (datum.loc.lineno - 1) as usize;
            if lineno >= lines.len() {
                println!("Bad line number in file {} (line {})", path, lineno);
                continue;
            }

            for piece in datum.data {
                process_analysis_target(
                    piece,
                    path,
                    &fallback_file_sym,
                    lineno,
                    &datum.loc,
                    &mut search_result_items,
                    &mut pretty_items,
                    &mut id_items,
                    &mut callees_items,
                    &mut use_slot_items,
                    &lines,
                    line_pp(lineno),
                );
            }
        }
    }

    PerThreadAnalysisData {
        search_result_items,
        top_level_search_result_items,
        pretty_items,
        id_items,
        structured_items,
        callees_items,
        use_slot_items,
        xref_link_subclass,
        xref_link_override,
        xref_link_slots_items,
        recency_items,
        file_recency_items: file_recencies,
        recency_files,
        recency_linked,
        recency_linkages,
    }
}

/// Give the memory freed in all of the threads' heaps back to the system:
/// glibc keeps it in each thread's arena otherwise, so that each thread's
/// heap stays the size of the biggest chunk it read (and the merge's tables
/// come on top).  read-analysis does after the chunks of files bigger than a
/// chunk: on minidisco it then peaks at 0.79 GB rather than 1.30 (1.07 before
/// chunks, when each thread read a 16th of the files).  config8's crossref
/// peaked at 85 GiB rather than 47 with chunks, and at 59 GiB trimming after
/// every chunk, which made read-analysis 5.7 minutes rather than 2.7 (each
/// trim walks every arena, which 16 threads then wait for).  (See
/// dataflow-summarize's, which does the same.)
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn release_freed_memory() {
    unsafe extern "C" {
        fn malloc_trim(pad: usize) -> std::os::raw::c_int;
    }
    // SAFETY: malloc_trim only releases free memory.
    unsafe {
        malloc_trim(0);
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn release_freed_memory() {}

/// The end of the chunk starting at `start` (see `analysis_chunks`).
fn chunk_end(chunks: &[std::ops::Range<usize>], start: usize) -> usize {
    chunks
        .iter()
        .find(|range| range.start == start)
        .map_or(start, |range| range.end)
}

/// An analysis file's size, for scheduling (see `analysis_chunks`): its own,
/// or its gzipped one's, as gzipped analysis is 10 to 15 times smaller.
fn analysis_bytes(index_path: &str, path: &str) -> u64 {
    let plain = format!("{}/analysis/{}", index_path, path);
    fs::metadata(&plain)
        .map(|meta| meta.len())
        .unwrap_or_else(|_| fs::metadata(format!("{}.gz", plain)).map_or(0, |meta| meta.len() * 10))
}

/// The analysis files' contiguous chunks (ranges of `paths`), each with about
/// a (32 x threads)th of their analysis's bytes (or a file bigger than that),
/// in the order to read them: the files bigger than that first, biggest
/// first, then the others in order, so that each is merged soon after it's
/// read (their records are merged in the files' order, so that the tables
/// are as one thread's reading would make them; see `read_analysis_files`).
/// (Each thread reading a 16th of the files, in order, left one reading a
/// chunk of big files for the last 8 of read-analysis's 13 minutes on
/// firefox-disco, while the others idled.)
/// Also returns which chunks are the files bigger than that.
fn analysis_chunks(
    index_path: &str,
    paths: &[Ustr],
    thread_count: usize,
) -> (Vec<std::ops::Range<usize>>, Vec<bool>) {
    let sizes: Vec<u64> = paths
        .iter()
        .map(|path| analysis_bytes(index_path, path))
        .collect();
    let target = (sizes.iter().sum::<u64>() / (thread_count as u64 * 32)).max(1);
    let mut chunks: Vec<(std::ops::Range<usize>, u64)> = vec![];
    let (mut start, mut bytes) = (0, 0);
    for (i, size) in sizes.iter().enumerate() {
        if i > start && bytes + size > target {
            chunks.push((start..i, bytes));
            (start, bytes) = (i, 0);
        }
        bytes += size;
    }
    if start < paths.len() {
        chunks.push((start..paths.len(), bytes));
    }
    chunks.sort_by_key(|(range, bytes)| match *bytes > target {
        true => (0, std::cmp::Reverse(*bytes), range.start),
        false => (1, std::cmp::Reverse(0), range.start),
    });
    let giants = chunks.iter().map(|(_, bytes)| *bytes > target).collect();
    (chunks.into_iter().map(|(range, _)| range).collect(), giants)
}

struct AnalysisData {
    search_result_table: SearchResultTable,
    pretty_table: PrettyTable,
    id_table: IdTable,
    meta_table: MetaTable,
    callees_table: CalleesTable,
    recency_table: UstrMap<(Recency, RecencyFrom)>,
    file_recency_table: BTreeMap<String, FileRecency>,
    recency_linked: u32,
    recency_linkages: Vec<(Ustr, FileLinkage)>,
}

fn read_analysis_files(
    analysis_relative_paths: Vec<Ustr>,
    tree_name: &String,
    tree_config: &TreeConfig,
    ingestion: &RepoIngestion,
    dir_subsystems: &HashMap<Ustr, Ustr>,
    history: Option<&HistoryDigests>,
    thread_count: usize,
) -> AnalysisData {
    let total = analysis_relative_paths.len();
    let thread_count = if total > 100 { thread_count } else { 1 };
    let index_path = tree_config.paths.index_path.as_str();
    let (chunks, giants) = analysis_chunks(index_path, &analysis_relative_paths, thread_count);

    info!(
        "{} analysis files found. Processing with {} threads ({} chunks of about equal analysis, biggest first)",
        total,
        thread_count,
        chunks.len()
    );

    let mut search_result_table = SearchResultTable::new();
    let mut pretty_table = PrettyTable::new();
    let mut id_table = IdTable::default();
    let mut meta_table = MetaTable::new();
    // (Supplements' documentation, for the symbols' metas once they're all in;
    // see `AnalysisStructured::supplement`.)
    let mut supplement_docs: Vec<(Ustr, Vec<DocLink>)> = vec![];
    let mut callees_table = CalleesTable::new();
    // A symbol's digest is the sum of its definitions' (in their files), or
    // of its declarations' if it has none.
    let mut def_recency: UstrMap<(Recency, RecencyFrom)> = UstrMap::default();
    let mut decl_recency: UstrMap<(Recency, RecencyFrom)> = UstrMap::default();
    let mut recency_seen: HashSet<(Ustr, bool, Ustr)> = HashSet::new();
    let mut recency_files = (0, 0, 0);
    let mut file_recency_table: BTreeMap<String, FileRecency> = BTreeMap::new();
    let mut all_recency_linked = 0;
    let mut all_recency_linkages = vec![];

    let mut xrefs = vec![];
    let mut all_use_slots = vec![];

    // (Each chunk's records, merged in the files' order, as they come.)
    let mut merge = |out: PerThreadAnalysisData| {
        let PerThreadAnalysisData {
            search_result_items,
            top_level_search_result_items,
            pretty_items,
            id_items,
            structured_items,
            callees_items,
            use_slot_items,
            xref_link_subclass,
            xref_link_override,
            xref_link_slots_items,
            recency_items,
            file_recency_items,
            recency_files: (digested, unaligned, no_grammar),
            recency_linked,
            recency_linkages,
        } = out;
        all_recency_linked += recency_linked;
        all_recency_linkages.extend(recency_linkages);
        for (path, file_recency) in file_recency_items {
            file_recency_table.insert(path.to_string(), file_recency);
        }
        recency_files.0 += digested;
        recency_files.1 += unaligned;
        recency_files.2 += no_grammar;

        for (sym, is_def, path, recency, context) in recency_items {
            if recency_seen.insert((sym, is_def, path)) {
                let table = if is_def {
                    &mut def_recency
                } else {
                    &mut decl_recency
                };
                // (Where it's from is the first definition, with a count.)
                let (total, from) = table.entry(sym).or_insert_with(|| {
                    (
                        Recency::default(),
                        RecencyFrom {
                            path: path.to_string(),
                            context,
                            sites: 0,
                        },
                    )
                });
                total.accumulate(&recency);
                from.sites += 1;
            }
        }

        xrefs.push((
            xref_link_subclass,
            xref_link_override,
            xref_link_slots_items,
        ));
        all_use_slots.extend(use_slot_items);

        for (id, kind, path, result) in search_result_items {
            let t1 = search_result_table.entry(id).or_default();
            let t2 = t1.entry(kind).or_default();
            let t3 = t2.entry(path).or_default();
            t3.push(result);
        }

        for id in top_level_search_result_items {
            search_result_table.entry(id).or_default();
        }

        for (sym, pretty) in pretty_items {
            pretty_table.insert(sym, pretty);
        }

        for (pretty, sym) in id_items {
            let t1 = id_table.entry(pretty).or_default();
            t1.insert(sym);
        }

        for structured in structured_items {
            // A supplement (see `AnalysisStructured::supplement`) is never the
            // meta, but adds its documentation to it (its binding slots were
            // taken, as every record's are).
            if structured.supplement {
                supplement_docs.push((structured.sym, structured.docs));
                continue;
            }
            // Producers just put the item without modifying the item, and thus
            // we don't have to merge the structured record.
            meta_table.entry(structured.sym).or_insert(structured);
        }

        for (sym, callee_sym, path, lineno, cites) in callees_items {
            let callee_syms = callees_table.entry(sym).or_default();
            let (from_path, callee_jump_lines, only_cites) = callee_syms
                .entry(callee_sym)
                .or_insert_with(|| (path, BTreeSet::new(), cites));
            *only_cites &= cites;
            if *from_path == path {
                callee_jump_lines.insert(lineno);
            }
            // XXX otherwise weird things are happening, but I'm not
            // sure we need to warn on this.
        }
    };

    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|s| {
        let (sender, receiver) = std::sync::mpsc::channel::<(usize, PerThreadAnalysisData)>();
        for _ in 0..thread_count {
            let sender = sender.clone();
            let (chunks, giants, next, paths) = (&chunks, &giants, &next, &analysis_relative_paths);
            let find_source_file = tree_config.get_find_source_file();
            s.spawn(move || {
                loop {
                    let taken = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(range) = chunks.get(taken) else {
                        break;
                    };
                    let data = read_analysis_files_thread(
                        &paths[range.clone()],
                        index_path,
                        &find_source_file,
                        ingestion,
                        dir_subsystems,
                        history,
                    );
                    if sender.send((range.start, data)).is_err() {
                        break;
                    }
                    // (What reading a file bigger than a chunk allocated is
                    // freed; give it back.)
                    let trim = giants[taken];
                    if trim {
                        release_freed_memory();
                    }
                }
            });
        }
        drop(sender);
        let mut pending: BTreeMap<usize, PerThreadAnalysisData> = BTreeMap::new();
        let mut merged = 0;
        for (start, data) in receiver {
            pending.insert(start, data);
            while let Some(data) = pending.remove(&merged) {
                merged = chunk_end(&chunks, merged);
                merge(data);
            }
        }
        assert!(pending.is_empty(), "Unmerged analysis chunks");
    });
    // (And what the reading threads freed: their arenas keep it otherwise,
    // and the roll-ups and writes after this are on this thread, so they
    // can't reuse it.  config8's crossref kept 65 GiB through them, and the
    // kernel swapped some out as their writes filled the page cache.)
    release_freed_memory();

    println!(
        "Performing crossref::read-analysis-merge step for {} : {}",
        tree_name,
        Local::now().format("%Y-%m-%dT%H:%M:%S%z")
    );

    println!(
        "Performing crossref::read-analysis-process step for {} : {}",
        tree_name,
        Local::now().format("%Y-%m-%dT%H:%M:%S%z")
    );

    // Process deferred meta cross-referencing once the meta_table is
    // fully populated.
    //
    // xref_link_subclass and xref_link_override are simple vector,
    // and we don't care about the duplication.
    //
    // xref_link_slots is a map, and we need to de-duplicate before reflecting
    // to the meta_table.
    let mut xref_link_slots = XrefLinkSlots::new();
    let mut pretty_slots = vec![];
    for item in xrefs {
        let (xref_link_subclass, xref_link_override, xref_link_slots_items) = item;

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

        for ((owner_sym, slotted), (props, subsystem, by_pretty)) in xref_link_slots_items {
            if by_pretty {
                pretty_slots.push((owner_sym, slotted, props, subsystem));
            } else {
                xref_link_slots.insert((owner_sym, slotted), (props, subsystem));
            }
        }
    }
    for (sym, docs) in supplement_docs {
        if let Some(meta) = meta_table.get_mut(&sym) {
            for doc in docs {
                if !meta.docs.contains(&doc) {
                    meta.docs.push(doc);
                }
            }
        }
    }
    resolve_pretty_slots(pretty_slots, &meta_table, &mut xref_link_slots);

    for ((owner_sym, slotted_sym), (props, subsystem)) in xref_link_slots {
        if let Some(owner) = meta_table.get_mut(&owner_sym) {
            owner.binding_slots.push(StructuredBindingSlotInfo {
                sym: slotted_sym,
                props,
                pretty: None,
            });
            if owner.subsystem.is_none() {
                owner.subsystem = subsystem;
            }
        }
        // (A spec's definition isn't its IDL's slot, even in the same tree, ex:
        // a spec tree's own WebIDL files', but what it reflects.)
        if props.slot_lang == BindingSlotLang::Spec {
            continue;
        }
        if let Some(slotted) = meta_table.get_mut(&slotted_sym) {
            slotted.slot_owner = Some(StructuredBindingSlotInfo {
                sym: owner_sym,
                props,
                pretty: None,
            });
            slotted.subsystem = subsystem;
        }
    }

    // Uses' contexts which fill their symbols' binding slots are slots of the
    // symbols (which get structured records if they have none), but the
    // symbols aren't their slot owners, since a context can fill slots of
    // many symbols (ex: a function notifying observers of several topics).
    all_use_slots.sort();
    all_use_slots.dedup();
    for (sym, pretty, slotted_sym, slot_kind, slot_lang) in all_use_slots {
        let owner = meta_table
            .entry(sym)
            .or_insert_with(|| event_structured(sym, pretty));
        owner.binding_slots.push(StructuredBindingSlotInfo {
            sym: slotted_sym,
            props: BindingSlotProps {
                slot_kind,
                slot_lang,
                impl_kind: None,
                owner_lang: BindingOwnerLang::Event,
            },
            pretty: None,
        });
    }

    for (sym, recency) in decl_recency {
        def_recency.entry(sym).or_insert(recency);
    }
    if history.is_some() {
        println!(
            "History digests: {} files ({} in languages the history has no grammar for, so just the files' digests), {} not lining up; {} symbols",
            recency_files.0,
            recency_files.2,
            recency_files.1,
            def_recency.len()
        );
    }

    AnalysisData {
        search_result_table,
        pretty_table,
        id_table,
        meta_table,
        callees_table,
        recency_table: def_recency,
        file_recency_table,
        recency_linked: all_recency_linked,
        recency_linkages: all_recency_linkages,
    }
}

/// Report the definitions that didn't link to their own history contexts but
/// should have (see `FileLinkage`), which mean the history's contexts are wrong
/// there (ex: tokenizer misparses), for targeted validation of fixes: a
/// summary, and `diags/crossref/recency-linkage.tsv`, by file, worst first.
fn write_recency_linkage(index_path: &str, linked: u32, mut linkages: Vec<(Ustr, FileLinkage)>) {
    linkages.sort_by(|a, b| {
        b.1.unlinked_total()
            .cmp(&a.1.unlinked_total())
            .then_with(|| a.0.cmp(&b.0))
    });
    let mut unlinked = [0u32; 3];
    for (_, linkage) in &linkages {
        for (total, n) in unlinked.iter_mut().zip(linkage.unlinked) {
            *total += n;
        }
    }
    println!(
        "History digest linkage: {} definitions linked to their own contexts; {} of functions, classes, and the like weren't, in {} files (at the top level {}, in namespaces {}, in other contexts {}); see diags/crossref/recency-linkage.tsv",
        linked,
        unlinked.iter().sum::<u32>(),
        linkages.len(),
        unlinked[0],
        unlinked[1],
        unlinked[2]
    );
    for (path, linkage) in linkages.iter().take(10) {
        println!("  {} {}", linkage.unlinked_total(), path);
    }

    let dir = format!("{}/diags/crossref", index_path);
    create_dir_all(&dir).unwrap();
    let mut out = BufWriter::new(File::create(format!("{}/recency-linkage.tsv", dir)).unwrap());
    writeln!(
        out,
        "# Definitions of functions, classes, and the like whose history contexts aren't their own (see hyperblame::recency), so they get no history digests, by file\n# path\tunlinked\tlinked\ttop level\tnamespaces\tother contexts\texamples (pretty in context at line)"
    )
    .unwrap();
    for (path, linkage) in &linkages {
        let examples: Vec<String> = linkage
            .examples
            .iter()
            .map(|(pretty, context, line)| format!("{} in {} at {}", pretty, context, line))
            .collect();
        writeln!(
            out,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            path,
            linkage.unlinked_total(),
            linkage.linked,
            linkage.unlinked[0],
            linkage.unlinked[1],
            linkage.unlinked[2],
            examples.join("; ")
        )
        .unwrap();
    }
    out.flush().unwrap();
}

/// The whole-file history digests of files without analysis (ex: docs), which
/// is all their lines can have (see `FileRecency`), several files at a time.
/// (On rayon's pool, whose threads are crossref's `thread_count`, in the
/// paths' order.)
fn unanalyzed_file_recency(history: &HistoryDigests, paths: &[Ustr]) -> Vec<(Ustr, FileRecency)> {
    paths
        .par_iter()
        .filter_map(|path| Some((*path, history.unanalyzed_file(path)?)))
        .collect()
}

/// Write the files' history digests for what symbols' don't cover (see
/// `FileRecency`) to `file-recency` (and `file-recency-extra`), keyed by path,
/// like the crossref.
fn write_file_recency(index_path: &str, table: BTreeMap<String, FileRecency>) {
    let mut out = BufWriter::new(File::create(format!("{}/file-recency", index_path)).unwrap());
    let mut ext_out =
        BufWriter::new(File::create(format!("{}/file-recency-extra", index_path)).unwrap());
    let mut ext_offset = 0;
    for (path, file_recency) in table {
        write_inline_and_ext(
            &mut out,
            &mut ext_out,
            &mut ext_offset,
            &ustr(&path),
            &file_recency,
            true,
        );
    }
    out.flush().unwrap();
    ext_out.flush().unwrap();
}

type FieldOwningClassRules = UstrMap<OntologyLabelOwningClass>;

fn read_field_owning_class_rules(ontology: &OntologyMappingIngestion) -> FieldOwningClassRules {
    // ### Extract field-processing rules to run over every class.
    let mut field_owning_class_rules: UstrMap<OntologyLabelOwningClass> = UstrMap::default();

    for (pretty_id, rule) in ontology.config.pretty.iter() {
        let Some(label_owning_class) = &rule.label_owning_class else {
            continue;
        };
        // We lookup by the type_pretty which currently will have "class " or "struct ""
        // prefixes.  In the interest of not having to mangle every type field, create
        // "class "-prefixed variants.  I'm not creating "struct "-prefixed variants
        // right now because most things should be classes.
        let type_prettied = format!("class {}", pretty_id);
        field_owning_class_rules.insert(ustr(&type_prettied), label_owning_class.clone());
    }

    field_owning_class_rules
}

fn process_class_fields(
    ontology: &OntologyMappingIngestion,
    id_table: &IdTable,
    // read-write
    meta_table: &mut MetaTable,
    field_owning_class_rules: FieldOwningClassRules,
) -> FieldMemberUseTable {
    // ### Process class/fields using ontology type information
    let mut field_member_use_table = FieldMemberUseTable::new();
    for meta in meta_table.values_mut() {
        if meta.kind.as_str() != "class" && meta.kind.as_str() != "struct" {
            continue;
        }

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
                    meta.labels.insert(label_rule.label);
                }
            }

            let (ptr_infos, type_labels) = ontology
                .config
                .maybe_parse_type_as_pointer(&field.type_pretty);
            for label in type_labels {
                meta.labels.insert(label);
            }
            for (ptr_kind, pointee_pretty) in ptr_infos {
                let Some(pointee_syms) = id_table.get(&pointee_pretty) else {
                    info!(
                        pretty = pointee_pretty.as_str(),
                        "Unable to map pretty identifier to symbols."
                    );
                    continue;
                };
                // We need to find the first symbol that's referring to a type.
                // Conveniently, for C++, these will always start with `T_`,
                // which is nice because we can't do a lookup in meta right now.
                // TODO: Generalize to better understand what's a type, especially
                // in JS.  It might be easiest to sidestep this problem by having
                // the analyzer be emitting structured information for the field
                // so that we're just working in symbol space in the first place.
                let best_sym = pointee_syms.iter().find(|s| s.starts_with("T_"));
                let Some(sym) = best_sym else {
                    continue;
                };
                field.pointer_info.push(StructuredPointerInfo {
                    kind: ptr_kind.clone(),
                    sym: *sym,
                });

                let member_uses = field_member_use_table.entry(*sym).or_default();
                let use_details = member_uses.entry(meta.sym).or_default();
                use_details.push((field.pretty, ptr_kind));
            }
        }
    }

    field_member_use_table
}

fn process_ontology_rules(
    ontology: &OntologyMappingIngestion,
    search_result_table: &SearchResultTable,
    id_table: &IdTable,
    // read-write
    meta_table: &mut MetaTable,
) {
    // ### Process Ontology Rules
    for (pretty_id, rule) in ontology.config.pretty.iter() {
        // #### Labels we just slap on
        if !rule.labels.is_empty()
            && let Some(root_syms) = id_table.get(pretty_id)
        {
            for sym in root_syms {
                let Some(sym_meta) = meta_table.get_mut(sym) else {
                    continue;
                };
                for label in &rule.labels {
                    sym_meta.labels.insert(*label);
                }
            }
        }

        // #### Runnables
        if let Some(runnable_mode) = &rule.runnable {
            info!(" Processing pretty runnable rule for: {}", pretty_id);
            if let Some(root_method_syms) = id_table.get(pretty_id) {
                // The list of symbols to process for the runnable relationship.
                // We process the root syms to find their descendants, but we
                // don't actually process the root symbols.  These pending syms
                // will both be directly processed and have their children
                // appended as well.
                let mut pending_method_syms = vec![];
                let mut is_jvm = false;
                for sym in root_method_syms {
                    // XXX We should really have an easy way to figure out the
                    // implementation language from the structured record.  Right
                    // now we only really have that for binding slots.
                    if sym.starts_with("S_jvm_") {
                        is_jvm = true;
                    }
                    let Some(sym_meta) = meta_table.get(sym) else {
                        continue;
                    };
                    for over in &sym_meta.overridden_by_syms {
                        pending_method_syms.push(*over);
                    }
                }

                info!("  found {} initial method syms", pending_method_syms.len());

                // (this is LIFO traversal, which is fine for us)
                while let Some(method_sym) = pending_method_syms.pop() {
                    info!("  processing method sym: {}", method_sym);

                    // use the method to find its owning class
                    let class_sym = if let Some(method_meta) = meta_table.get(&method_sym) {
                        for over in &method_meta.overridden_by_syms {
                            pending_method_syms.push(*over);
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
                    let linkage_syms = match runnable_mode {
                        OntologyRunnableMode::Constructor => {
                            if let Some(class_meta) = meta_table.get(&class_sym) {
                                let mut syms = vec![];
                                // For C++ we expect the constructors to have the same name as the class;
                                // currently for C++ we don't actually emit a special "props" "constructor"
                                // value.
                                //
                                // For the JVM we expect constructors to have a pretty name of "<init>".
                                let constructor_name: &str = if is_jvm {
                                    "<init>"
                                } else {
                                    class_meta.pretty.rsplit("::").next().unwrap()
                                };

                                let constructor_pretty =
                                    ustr(&format!("{}::{}", class_meta.pretty, constructor_name));
                                for method in &class_meta.methods {
                                    // Skip constructors that aren't known; this can happen for the copy
                                    // constructor/etc.
                                    if method.pretty == constructor_pretty
                                        && search_result_table.contains_key(&method.sym)
                                    {
                                        syms.push(method.sym);
                                    }
                                }
                                syms
                            } else {
                                continue;
                            }
                        }
                        OntologyRunnableMode::Class => {
                            vec![class_sym]
                        }
                    };

                    info!("  found linkage syms: {:?}", linkage_syms);

                    // ### mutate each of the constructors to have the ontology slot
                    for con_sym in &linkage_syms {
                        let Some(con_meta) = meta_table.get_mut(con_sym) else {
                            continue;
                        };
                        // XXX we could track precedence for runnable rules so that
                        // we could remove lower precedence relationships here.  This
                        // would be relevant for WorkerRunnable.

                        con_meta.ontology_slots.push(OntologySlotInfo {
                            slot_kind: OntologySlotKind::RunnableMethod,
                            syms: vec![method_sym],
                        });
                    }

                    // ### mutate our method_sym to have the ontology slot to the constructors
                    let Some(method_meta) = meta_table.get_mut(&method_sym) else {
                        continue;
                    };
                    method_meta.ontology_slots.push(OntologySlotInfo {
                        slot_kind: OntologySlotKind::RunnableConstructor,
                        syms: linkage_syms,
                    })
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
            if let Some(root_class_syms) = id_table.get(pretty_id) {
                let mut investigate_class_syms = vec![];
                // We don't care about the root itself, just its subclasses.
                for sym in root_class_syms {
                    let Some(sym_meta) = meta_table.get(sym) else {
                        continue;
                    };
                    for sub in &sym_meta.subclass_syms {
                        investigate_class_syms.push(*sub);
                    }
                }

                while let Some(class_sym) = investigate_class_syms.pop() {
                    let Some(sym_meta) = meta_table.get(&class_sym) else {
                        continue;
                    };

                    for sub in &sym_meta.subclass_syms {
                        investigate_class_syms.push(*sub);
                    }

                    // The structured record currently doesn't have a reference
                    // to its containing symbol; we need to pop the last pretty
                    // segment and perform a lookup.
                    let (pieces, delim) = split_pretty(&sym_meta.pretty, &sym_meta.sym);
                    let Some((_, containing_pieces)) = pieces.split_last() else {
                        continue;
                    };
                    let containing_pretty = containing_pieces.join(delim);
                    let containing_pretty_ustr = ustr(&containing_pretty);
                    let Some(containing_syms) = id_table.get(&containing_pretty_ustr) else {
                        continue;
                    };
                    for sym in containing_syms {
                        let Some(containing_meta) = meta_table.get_mut(sym) else {
                            continue;
                        };
                        for rule in &label_rule.labels {
                            containing_meta.labels.insert(rule.label);
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
        let Some(label_rule) = &rule.label_containing_class_field_uses else {
            continue;
        };
        info!(
            " Processing pretty label_containing_class_field_uses rule for: {}",
            pretty_id
        );
        let Some(root_class_syms) = id_table.get(pretty_id) else {
            continue;
        };
        let mut investigate_class_syms = vec![];
        // We don't care about the root itself, just its subclasses.
        for sym in root_class_syms {
            let Some(sym_meta) = meta_table.get(sym) else {
                continue;
            };
            for sub in &sym_meta.subclass_syms {
                investigate_class_syms.push(*sub);
            }
        }

        while let Some(class_sym) = investigate_class_syms.pop() {
            let Some(sym_meta) = meta_table.get(&class_sym) else {
                continue;
            };

            for sub in &sym_meta.subclass_syms {
                investigate_class_syms.push(*sub);
            }

            // The structured record currently doesn't have a reference
            // to its containing symbol; we need to pop the last pretty
            // segment and perform a lookup.
            let (pieces, delim) = split_pretty(&sym_meta.pretty, &sym_meta.sym);
            let Some((_, containing_pieces)) = pieces.split_last() else {
                continue;
            };
            let containing_pretty = containing_pieces.join(delim);
            let containing_pretty_ustr = ustr(&containing_pretty);
            let Some(containing_syms) = id_table.get(&containing_pretty_ustr) else {
                continue;
            };
            for sym in containing_syms {
                let Some(containing_meta) = meta_table.get_mut(sym) else {
                    continue;
                };
                for field in &mut containing_meta.fields {
                    let Some(kind_map) = search_result_table.get(&field.sym) else {
                        continue;
                    };
                    let Some(path_hits) = kind_map.get(&AnalysisKind::Use) else {
                        continue;
                    };
                    for hits in path_hits.values() {
                        for hit in hits {
                            for rule in &label_rule.labels {
                                if hit.context.ends_with(rule.context_sym_suffix.as_str()) {
                                    field.labels.insert(rule.label);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

async fn write_ontology_ingestion_diag(tree_config: &TreeConfig, logged_ontology_span: LoggedSpan) {
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

fn create_js_tables(meta_table: &MetaTable) -> (JSIDLTable, JsTsTable) {
    let mut js_idl_table = JSIDLTable::new();
    let mut js_ts_table = JsTsTable::new();
    for meta in meta_table.values() {
        for slot in &meta.binding_slots {
            if slot.props.slot_lang != BindingSlotLang::JS {
                continue;
            }
            let js_sym = slot.sym;
            let has_concrete_slot_owner = meta_table
                .get(&js_sym)
                .and_then(|js_meta| js_meta.slot_owner.as_ref())
                .is_some();

            if has_concrete_slot_owner {
                continue;
            }

            let idl_sym = meta.sym;

            let idl_syms = js_idl_table.entry(js_sym).or_default();
            if idl_syms.len() < MAX_JS_IDL_SYMS {
                idl_syms.push(idl_sym);
            }
        }

        if let Some(js_sym) = meta.js_sym {
            let ts_sym = meta.sym;
            js_ts_table.entry(js_sym).or_default().push(ts_sym);

            let ts_slot_owner = meta_table
                .get(&ts_sym)
                .and_then(|ts_meta| ts_meta.slot_owner.as_ref())
                .map(|idl_meta| idl_meta.sym);

            if let Some(ts_slot_owner) = ts_slot_owner {
                let idl_syms = js_idl_table.entry(js_sym).or_default();
                if idl_syms.len() < MAX_JS_IDL_SYMS {
                    idl_syms.push(ts_slot_owner);
                }
            }
        }
    }

    (js_idl_table, js_ts_table)
}

/// (`out` and `ext_out` should be buffered: this writes a few small pieces for
/// each symbol.)
fn write_inline_and_ext(
    out: &mut impl Write,
    ext_out: &mut impl Write,
    ext_offset: &mut usize,
    id: &Ustr,
    data: &impl Serialize,
    is_first_chunk: bool,
) {
    let id_line = format!("!{}\n", id);
    let inline_line = format!(":{}\n", serde_json::to_string(data).unwrap());
    if inline_line.len() >= EXTERNAL_STORAGE_THRESHOLD {
        // ### External storage.
        out.write_all(id_line.as_bytes()).unwrap();
        // We write out the identifier in the extra file as well so that it
        // can be interpreted in the same fashion.
        ext_out.write_all(id_line.as_bytes()).unwrap();
        *ext_offset += id_line.len();

        // Skip the leading ":"
        let marker_offset = *ext_offset + 1;
        // Subtract off the leading ":" but keep the newline.
        let marker_length = inline_line.len() - 1;

        let ext_offset_line = if is_first_chunk {
            format!("@{:x} {:x}\n", marker_offset, marker_length)
        } else {
            // The offset will be adjusted later.
            // Use decimal to make it easier to parse.
            format!("@!{} {:x}\n", marker_offset, marker_length)
        };
        out.write_all(ext_offset_line.as_bytes()).unwrap();

        ext_out.write_all(inline_line.as_bytes()).unwrap();
        *ext_offset += inline_line.len();
    } else {
        // ### Inline storage.
        out.write_all(id_line.as_bytes()).unwrap();
        out.write_all(inline_line.as_bytes()).unwrap();
    }
}

/// Calls between two classes or subsystems: how many, and by which pairs.
#[derive(Default)]
struct RollupAccum {
    count: u32,
    accessors: u32,
    ipc: u32,
    pairs: UstrMap<UstrMap<u32>>,
}

impl RollupAccum {
    fn add(&mut self, from: Ustr, to: Ustr, count: u32, accessor: bool, ipc: bool) {
        self.count += count;
        if accessor {
            self.accessors += count;
        }
        if ipc {
            self.ipc += count;
        }
        *self.pairs.entry(from).or_default().entry(to).or_default() += count;
    }

    fn pair_count(&self) -> u32 {
        self.pairs.values().map(|tos| tos.len() as u32).sum()
    }

    fn edge(&self, sym: Ustr, pretty: Ustr, pretty_of: &dyn Fn(&Ustr) -> Ustr) -> RollupEdge {
        let mut examples: Vec<(Ustr, Ustr, u32)> = self
            .pairs
            .iter()
            .flat_map(|(from, tos)| tos.iter().map(move |(to, count)| (*from, *to, *count)))
            .collect();
        examples.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));
        examples.truncate(ROLLUP_EXAMPLES);
        RollupEdge {
            sym,
            pretty,
            count: self.count,
            accessors: self.accessors,
            ipc: self.ipc,
            via: vec![],
            pairs: self.pair_count(),
            examples: examples
                .into_iter()
                .map(|(from, to, count)| RollupExample {
                    from,
                    from_pretty: pretty_of(&from),
                    to,
                    to_pretty: pretty_of(&to),
                    count,
                })
                .collect(),
        }
    }
}

/// Whether a structured record is of something that's called (a function or
/// method), for roll-ups.
fn is_rollup_callable(meta: &AnalysisStructured) -> bool {
    matches!(meta.kind.as_str(), "function" | "method")
}

/// An edge between two functions (a call) or between functions and fields
/// (data; see `tools::dataflow::summary::FieldEdge`), for roll-ups: its
/// ends, their classes, how much it stands for, and whether it's a call of
/// an accessor (a getter, setter or trivial method, by its dataflow
/// summary).
struct UnitEdge {
    from: Ustr,
    to: Ustr,
    from_class: Option<Ustr>,
    to_class: Option<Ustr>,
    count: u32,
    accessor: bool,
    /// Whether it's an IPDL message, from its generated `Send` method to the
    /// actor's `Recv` implementation.
    ipc: bool,
}

/// A method's or field's class, for roll-ups: its `parentsym`, if that's a
/// symbol with a structured record that isn't a namespace (else, for a C++
/// field, the class its symbol names); an IPDL struct's field's is the
/// struct, and a message's parameter's the message (see
/// `tools::dataflow::summary::IpdlSlots`).
fn rollup_class(meta_table: &MetaTable, sym: &Ustr) -> Option<Ustr> {
    match meta_table.get(sym) {
        Some(meta) => {
            let parent = meta.parent_sym?;
            let is_member = match meta.kind.as_str() {
                "method" | "field" => true,
                "parameter" => parent.starts_with("IPC_"),
                _ => false,
            };
            (is_member && !parent.starts_with("NS_") && meta_table.contains_key(&parent))
                .then_some(parent)
        }
        None => field_class(sym).filter(|class| meta_table.contains_key(class)),
    }
}

/// The XPIDL member whose generated TypeScript type a JS symbol (of
/// scip-typescript's) is of, ex: `XPIDL_nsIObserverService_notifyObservers`
/// for `S_js_typescript_generated/lib.gecko.xpcom.d.ts/global/nsIObserverService#notifyObservers().`
/// (the generated types are named as their IDL is).
fn xpidl_member_of_ts_sym(sym: &str) -> Option<Ustr> {
    let rest = sym.strip_prefix("S_js_typescript_generated/lib.gecko.xpcom.d.ts/global/")?;
    let (interface, member) = rest.split_once('#')?;
    let member = member
        .strip_suffix("().")
        .or_else(|| member.strip_suffix('.'))?;
    let is_name = |name: &str| {
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    };
    (is_name(interface) && is_name(member))
        .then(|| ustr(&format!("XPIDL_{}_{}", interface, member)))
}

/// Test code, for roll-ups, which leave out its calls (ex: the tests of many
/// subsystems calling GTest made one group of them, in the whole tree's block
/// diagram): the symbols whose definitions are all in test files (by their
/// per-file info's path kind; see per-file-info.toml's `pathkind.test`).
fn test_code(search_result_table: &SearchResultTable, ingestion: &RepoIngestion) -> UstrSet {
    let test = ustr("test");
    let is_test = |path: &Ustr| {
        ingestion
            .state
            .concise_per_file
            .get(path)
            .is_some_and(|info| info.path_kind == test)
    };
    search_result_table
        .iter()
        .filter(|(_, kinds)| {
            kinds
                .get(&AnalysisKind::Def)
                .is_some_and(|paths| !paths.is_empty() && paths.keys().all(is_test))
        })
        .map(|(sym, _)| *sym)
        .collect()
}

/// The calls between functions, for roll-ups, leaving out test code's (see
/// `test_code`), and callees labeled
/// `calls-diagram:stop` (assertions, logging, ...), as call diagrams do, and
/// other plumbing (`is_boring_callee`).  Some calls stand for others: an
/// IPDL message's `Send` method's for its actors' `Recv` implementations',
/// through IPC; an observer topic's (its notifications', its `send` slots')
/// for its observers' methods receiving it (its `recv` slots); and a JS
/// call of an IDL method's or attribute's JS binding (or, for XPIDL, of its
/// generated TypeScript type's member) for its C++ slot's (an XPIDL
/// interface's virtual method, as C++ callers through the interface call it,
/// or WebIDL's implementation).
fn call_edges(
    meta_table: &MetaTable,
    callees_table: &CalleesTable,
    categories: &UstrMap<Category>,
    test_code: &UstrSet,
) -> Vec<UnitEdge> {
    let stop = ustr("calls-diagram:stop");
    // (IPDL messages' generated `Send` methods, and their actors' `Recv`
    // implementations, which calls of the `Send` methods reach through IPC.)
    let mut ipc_targets: UstrMap<Vec<Ustr>> = UstrMap::default();
    for meta in meta_table.values() {
        if !meta.sym.starts_with("IPC_") {
            continue;
        }
        let slots = |kind: BindingSlotKind| -> Vec<Ustr> {
            meta.binding_slots
                .iter()
                .filter(|slot| slot.props.slot_kind == kind)
                .map(|slot| slot.sym)
                .collect()
        };
        let mut impls: Vec<Ustr> = vec![];
        for recv in slots(BindingSlotKind::Recv) {
            // (A generated base class's declaration: the actors' overrides.)
            match meta_table.get(&recv) {
                Some(m) if !m.overridden_by_syms.is_empty() => {
                    impls.extend(m.overridden_by_syms.iter().copied())
                }
                _ => impls.push(recv),
            }
        }
        for send in slots(BindingSlotKind::Send) {
            ipc_targets
                .entry(send)
                .or_default()
                .extend(impls.iter().copied());
        }
    }
    // (Observer topics' receivers, and JS bindings' C++ slots: of the JS
    // slots only one IDL owner has, as name-based JS symbols, ex:
    // `#init`, are many interfaces' slots, and calls of other objects'.)
    let mut stand_ins: UstrMap<Vec<Ustr>> = UstrMap::default();
    let mut js_slot_owners: UstrMap<Vec<Ustr>> = UstrMap::default();
    let mut owners_cpp: UstrMap<Vec<Ustr>> = UstrMap::default();
    for meta in meta_table.values() {
        let owned_by = |lang: BindingOwnerLang| {
            meta.binding_slots
                .iter()
                .any(|slot| slot.props.owner_lang == lang)
        };
        if owned_by(BindingOwnerLang::Event) {
            let receivers: Vec<Ustr> = meta
                .binding_slots
                .iter()
                .filter(|slot| slot.props.slot_kind == BindingSlotKind::Recv)
                .map(|slot| slot.sym)
                .collect();
            if !receivers.is_empty() {
                stand_ins.insert(meta.sym, receivers);
            }
        } else if owned_by(BindingOwnerLang::Idl) {
            let cpp: Vec<Ustr> = meta
                .binding_slots
                .iter()
                .filter(|slot| slot.props.slot_lang == BindingSlotLang::Cpp)
                .map(|slot| slot.sym)
                .collect();
            if cpp.is_empty() {
                continue;
            }
            for slot in &meta.binding_slots {
                if slot.props.slot_lang == BindingSlotLang::JS
                    && matches!(
                        slot.props.slot_kind,
                        BindingSlotKind::Method
                            | BindingSlotKind::Attribute
                            | BindingSlotKind::Getter
                            | BindingSlotKind::Setter
                    )
                {
                    js_slot_owners.entry(slot.sym).or_default().push(meta.sym);
                }
            }
            owners_cpp.insert(meta.sym, cpp);
        }
    }
    for (js, mut owners) in js_slot_owners {
        owners.sort();
        owners.dedup();
        if let [owner] = owners.as_slice() {
            stand_ins.insert(js, owners_cpp[owner].clone());
        }
    }
    let mut edges = vec![];
    for (caller, callees) in callees_table {
        let Some(caller_meta) = meta_table.get(caller) else {
            continue;
        };
        if !is_rollup_callable(caller_meta) || test_code.contains(caller) {
            continue;
        }
        let from_class = rollup_class(meta_table, caller);
        for (callee, (_, lines, only_cites)) in callees {
            if *only_cites || callee == caller {
                continue;
            }
            // (A call of an IPDL message's `Send` method is, through IPC, a
            // call of the actors' `Recv` implementations; the generated actor
            // class in between is plumbing.)
            if let Some(impls) = ipc_targets.get(callee) {
                for recv_impl in impls {
                    edges.push(UnitEdge {
                        from: *caller,
                        to: *recv_impl,
                        from_class,
                        to_class: rollup_class(meta_table, recv_impl),
                        count: lines.len().max(1) as u32,
                        accessor: false,
                        ipc: true,
                    });
                }
                continue;
            }
            let targets = stand_ins.get(callee).or_else(|| {
                xpidl_member_of_ts_sym(callee).and_then(|owner| owners_cpp.get(&owner))
            });
            if let Some(targets) = targets {
                for target in targets {
                    if target == caller || !meta_table.get(target).is_some_and(is_rollup_callable) {
                        continue;
                    }
                    edges.push(UnitEdge {
                        from: *caller,
                        to: *target,
                        from_class,
                        to_class: rollup_class(meta_table, target),
                        count: lines.len().max(1) as u32,
                        accessor: false,
                        ipc: false,
                    });
                }
                continue;
            }
            let Some(callee_meta) = meta_table.get(callee) else {
                continue;
            };
            if !is_rollup_callable(callee_meta)
                || callee_meta.labels.contains(&stop)
                || is_boring_callee(&callee_meta.pretty)
            {
                continue;
            }
            edges.push(UnitEdge {
                from: *caller,
                to: *callee,
                from_class,
                to_class: rollup_class(meta_table, callee),
                count: lines.len().max(1) as u32,
                accessor: matches!(
                    categories.get(callee),
                    Some(Category::Getter | Category::Setter | Category::Trivial)
                ),
                ipc: false,
            });
        }
    }
    edges
}

/// The classes of a class's members held by value (not through pointers),
/// and of the values in its containers of values (ex: `nsTArray<Foo>`),
/// whose pointer infos are all `contains`.
fn by_value_member_classes(meta: &AnalysisStructured) -> impl Iterator<Item = Ustr> + '_ {
    meta.fields
        .iter()
        .filter(|f| {
            f.pointer_info
                .iter()
                .all(|p| p.kind == OntologyPointerKind::Contains)
                && f.type_sym.starts_with("T_")
                && !f.type_pretty.contains(['*', '&'])
        })
        .flat_map(|f| std::iter::once(f.type_sym).chain(f.pointer_info.iter().map(|p| p.sym)))
}

/// The classes held by value (as members, or their supers) in exactly one
/// other class, and that class (ex: a `Stats` only a `Store` holds), so data
/// roll-ups count their data as their owners'.  (Value types held by many
/// classes, ex: IPDL structs, stay classes of their own.)
fn unique_owners(meta_table: &MetaTable) -> UstrMap<Ustr> {
    let mut owners: UstrMap<UstrSet> = UstrMap::default();
    for meta in meta_table.values() {
        if !matches!(meta.kind.as_str(), "class" | "struct") {
            continue;
        }
        let mut held: Vec<Ustr> = by_value_member_classes(meta).collect();
        // (A held class's supers are held too.)
        let mut i = 0;
        while i < held.len() && held.len() < 64 {
            if let Some(m) = meta_table.get(&held[i]) {
                for sup in &m.supers {
                    if !held.contains(&sup.sym) {
                        held.push(sup.sym);
                    }
                }
            }
            i += 1;
        }
        for class in held {
            if class != meta.sym && meta_table.contains_key(&class) {
                owners.entry(class).or_default().insert(meta.sym);
            }
        }
    }
    owners
        .into_iter()
        .filter(|(class, set)| set.len() == 1 && !is_value_type_class(class))
        .map(|(class, set)| (class, *set.iter().next().unwrap()))
        .collect()
}

/// A class's own state's classes: it, its supers, and the classes of its
/// members it holds by value (not through pointers), and theirs, a few
/// levels deep (ex: `WorkerPrivate`'s `mLoadInfo`'s `WorkerLoadInfo`, and
/// its super `WorkerLoadInfoData`), whose fields are the class's own.
struct InternalClasses<'a> {
    meta_table: &'a MetaTable,
    memo: UstrMap<UstrSet>,
}

impl<'a> InternalClasses<'a> {
    fn new(meta_table: &'a MetaTable) -> Self {
        InternalClasses {
            meta_table,
            memo: UstrMap::default(),
        }
    }

    fn of(&mut self, class: Ustr) -> &UstrSet {
        let meta_table = self.meta_table;
        self.memo.entry(class).or_insert_with(|| {
            let mut set = UstrSet::default();
            set.insert(class);
            let mut frontier = vec![class];
            for _ in 0..3 {
                let mut next = vec![];
                for x in frontier {
                    let Some(meta) = meta_table.get(&x) else {
                        continue;
                    };
                    let by_value = by_value_member_classes(meta);
                    for y in meta.supers.iter().map(|s| s.sym).chain(by_value) {
                        if set.len() < 64 && meta_table.contains_key(&y) && set.insert(y) {
                            next.push(y);
                        }
                    }
                }
                frontier = next;
            }
            set
        })
    }
}

/// The edges between fields, and between functions and other objects'
/// fields, from `dataflow-summarize`, for roll-ups; a field of the edge's
/// function's class's own state (see `InternalClasses`) is the class's, and
/// a class held by value in only one other is its owner's (see
/// `unique_owners`); but not test code's (see `test_code`).
fn data_edges(
    meta_table: &MetaTable,
    field_edges: &[FieldEdge],
    test_code: &UstrSet,
) -> Vec<UnitEdge> {
    let mut internal = InternalClasses::new(meta_table);
    let owners = unique_owners(meta_table);
    // (A class held by value in one other class is its owner's, a few
    // levels up.)
    let owner_of = |class: Ustr| -> Ustr {
        let mut class = class;
        for _ in 0..3 {
            match owners.get(&class) {
                Some(owner) => class = *owner,
                None => break,
            }
        }
        class
    };
    // (The function the data flows in: an end, or the first `via`.)
    let function_of = |edge: &FieldEdge| {
        [edge.from, edge.to]
            .into_iter()
            .find(|sym| !is_field_sym(sym))
            .or_else(|| edge.via.first().copied())
    };
    field_edges
        .iter()
        .filter(|edge| !function_of(edge).is_some_and(|f| test_code.contains(&f)))
        .map(|edge| {
            let mut from_class = rollup_class(meta_table, &edge.from);
            let mut to_class = rollup_class(meta_table, &edge.to);
            let function = function_of(edge);
            if let Some(context) = function.and_then(|f| rollup_class(meta_table, &f)) {
                let own = internal.of(context);
                for class in [&mut from_class, &mut to_class] {
                    if class.is_some_and(|c| own.contains(&c)) {
                        *class = Some(context);
                    }
                }
            }
            from_class = from_class.map(owner_of);
            to_class = to_class.map(owner_of);
            UnitEdge {
                from: edge.from,
                to: edge.to,
                from_class,
                to_class,
                count: edge.count,
                accessor: false,
                ipc: false,
            }
        })
        .collect()
}

/// Roll edges between functions and fields up into edges between classes
/// and between subsystems (see `tools::file_format::rollup`).  A unit (for
/// subsystems) is a class, or a free function or global; its subsystem its
/// structured record's.  Returns the roll-ups by symbol (classes',
/// subsystems' pseudo-symbols', and the whole tree's), and the subsystems.
fn rollup_edges(meta_table: &MetaTable, edges: &[UnitEdge]) -> (UstrMap<Rollup>, BTreeSet<Ustr>) {
    let subsystem_of = |unit: Ustr, sym: Ustr| -> Option<Ustr> {
        meta_table
            .get(&unit)
            .and_then(|m| m.subsystem)
            .or_else(|| meta_table.get(&sym).and_then(|m| m.subsystem))
    };

    let mut class_edges: HashMap<(Ustr, Ustr), RollupAccum> = HashMap::new();
    let mut class_internal: UstrMap<u32> = UstrMap::default();
    let mut subsystem_edges: HashMap<(Ustr, Ustr), RollupAccum> = HashMap::new();
    let mut subsystem_internal: UstrMap<u32> = UstrMap::default();
    let mut subsystem_members: UstrMap<UstrMap<u32>> = UstrMap::default();
    for edge in edges {
        let count = edge.count;
        if let (Some(from), Some(to)) = (edge.from_class, edge.to_class) {
            if from == to {
                *class_internal.entry(from).or_default() += count;
            } else {
                class_edges.entry((from, to)).or_default().add(
                    edge.from,
                    edge.to,
                    count,
                    edge.accessor,
                    edge.ipc,
                );
            }
        }
        let from_unit = edge.from_class.unwrap_or(edge.from);
        let to_unit = edge.to_class.unwrap_or(edge.to);
        if let (Some(from), Some(to)) = (
            subsystem_of(from_unit, edge.from),
            subsystem_of(to_unit, edge.to),
        ) {
            *subsystem_members
                .entry(from)
                .or_default()
                .entry(from_unit)
                .or_default() += count;
            if from == to {
                *subsystem_internal.entry(from).or_default() += count;
            } else {
                *subsystem_members
                    .entry(to)
                    .or_default()
                    .entry(to_unit)
                    .or_default() += count;
                subsystem_edges.entry((from, to)).or_default().add(
                    from_unit,
                    to_unit,
                    count,
                    edge.accessor,
                    edge.ipc,
                );
            }
        }
    }

    let pretty_of = |sym: &Ustr| -> Ustr { meta_table.get(sym).map_or(*sym, |m| m.pretty) };
    let mut rollups: UstrMap<Rollup> = UstrMap::default();
    // (Each edge, at both ends, most first.)
    let finish = |rollup: &mut Rollup| {
        let order = |a: &RollupEdge, b: &RollupEdge| b.count.cmp(&a.count).then(a.sym.cmp(&b.sym));
        rollup.out.sort_by(order);
        rollup.into.sort_by(order);
        rollup.out_total = rollup.out.len() as u32;
        rollup.in_total = rollup.into.len() as u32;
        rollup.out.truncate(ROLLUP_EDGES);
        rollup.into.truncate(ROLLUP_EDGES);
        for edges in [&mut rollup.out, &mut rollup.into] {
            for edge in edges.iter_mut().skip(ROLLUP_EXAMPLE_EDGES) {
                edge.examples.clear();
            }
        }
    };
    for ((from, to), accum) in &class_edges {
        rollups
            .entry(*from)
            .or_default()
            .out
            .push(accum.edge(*to, pretty_of(to), &pretty_of));
        rollups
            .entry(*to)
            .or_default()
            .into
            .push(accum.edge(*from, pretty_of(from), &pretty_of));
    }
    for (class, internal) in &class_internal {
        rollups.entry(*class).or_default().internal = *internal;
    }
    rollups.values_mut().for_each(finish);

    let mut subsystems: BTreeSet<Ustr> = subsystem_members.keys().copied().collect();
    subsystems.extend(subsystem_internal.keys().copied());
    for ((from, to), accum) in &subsystem_edges {
        rollups
            .entry(subsystem_sym(from))
            .or_default()
            .out
            .push(accum.edge(subsystem_sym(to), *to, &pretty_of));
        rollups
            .entry(subsystem_sym(to))
            .or_default()
            .into
            .push(accum.edge(subsystem_sym(from), *from, &pretty_of));
    }
    let mut tree = Rollup::default();
    for subsystem in &subsystems {
        let sym = subsystem_sym(subsystem);
        let rollup = rollups.entry(sym).or_default();
        rollup.internal = subsystem_internal.get(subsystem).copied().unwrap_or(0);
        let mut members: Vec<RollupMember> = subsystem_members
            .get(subsystem)
            .into_iter()
            .flatten()
            .map(|(member, count)| RollupMember {
                sym: *member,
                pretty: pretty_of(member),
                count: *count,
            })
            .collect();
        members.sort_by(|a, b| b.count.cmp(&a.count).then(a.sym.cmp(&b.sym)));
        members.truncate(ROLLUP_MEMBERS);
        rollup.members = members;
        finish(rollup);
        tree.members.push(RollupMember {
            sym,
            pretty: *subsystem,
            count: rollup.internal
                + rollup.out.iter().map(|e| e.count).sum::<u32>()
                + rollup.into.iter().map(|e| e.count).sum::<u32>(),
        });
    }
    if !subsystems.is_empty() {
        tree.members
            .sort_by(|a, b| b.count.cmp(&a.count).then(a.sym.cmp(&b.sym)));
        let mut pairs: Vec<RollupPair> = subsystem_edges
            .iter()
            .map(|((from, to), accum)| RollupPair {
                from: subsystem_sym(from),
                to: subsystem_sym(to),
                count: accum.count,
                pairs: accum.pair_count(),
            })
            .collect();
        pairs.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then(a.from.cmp(&b.from))
                .then(a.to.cmp(&b.to))
        });
        pairs.truncate(ROLLUP_TREE_EDGES);
        tree.edges = pairs;
        tree.internal = subsystem_internal.values().sum();
        rollups.insert(root_subsystem_sym(), tree);
    }
    (rollups, subsystems)
}

/// A subsystem pseudo-symbol's structured record.
fn subsystem_structured(sym: Ustr, subsystem: Ustr) -> AnalysisStructured {
    AnalysisStructured {
        structured: StructuredTag::Structured,
        pretty: subsystem,
        sym,
        js_sym: None,
        type_pretty: None,
        kind: ustr("subsystem"),
        subsystem: (!subsystem.is_empty()).then_some(subsystem),
        parent_sym: None,
        slot_owner: None,
        impl_kind: ustr("impl"),
        size_bytes: None,
        alignment_bytes: None,
        own_vf_ptr_bytes: None,
        binding_slots: vec![],
        ontology_slots: vec![],
        supers: vec![],
        methods: vec![],
        fields: vec![],
        overrides: vec![],
        props: vec![],
        labels: BTreeSet::default(),
        subclass_syms: vec![],
        overridden_by_syms: vec![],
        variants: vec![],
        supplement: false,
        docs: vec![],
        extra: Map::default(),
        can_gc: None,
        gc_path: None,
    }
}

/// The roll-ups of calls and of data, by symbol, and the accessors'
/// summaries, for crossref entries (see `add_rollups`).
#[derive(Default)]
struct RollupTables {
    calls: UstrMap<Rollup>,
    data: UstrMap<Rollup>,
    summaries: UstrMap<Summary>,
}

/// Roll up the calls, and the data edges from `dataflow-summarize` if the
/// tree has them (see `rollup_edges`), and add the subsystems'
/// pseudo-symbols to the tables, so they're written with the roll-ups.
/// Returns them, and the accessors' summaries (see `read_dataflow_outputs`).
fn add_rollups(
    index_path: &str,
    meta_table: &mut MetaTable,
    search_result_table: &mut SearchResultTable,
    callees_table: &CalleesTable,
    tree_metas: Option<MetaTable>,
    ingestion: &RepoIngestion,
) -> RollupTables {
    // (Each part's time is printed, as the step is long on one core and its
    // parts are only told apart this way.)
    let started = Instant::now();
    let (categories, field_edges, summaries) = read_dataflow_outputs(index_path);
    let read_secs = started.elapsed().as_secs_f64();
    let test_code = test_code(search_result_table, ingestion);
    let (call_rollups, data_rollups, subsystems) = {
        // (A subset's roll-ups attribute calls as the tree's do: by the
        // tree's structured records, else its own; see `tree_rollup_metas`.)
        let combined;
        let view: &MetaTable = match tree_metas {
            Some(tree_metas) => {
                let mut metas = meta_table.clone();
                metas.extend(tree_metas);
                combined = metas;
                &combined
            }
            None => meta_table,
        };
        let mark = Instant::now();
        let calls = call_edges(view, callees_table, &categories, &test_code);
        let call_edge_secs = mark.elapsed().as_secs_f64();
        let mark = Instant::now();
        let (call_rollups, mut subsystems) = rollup_edges(view, &calls);
        let call_rollup_secs = mark.elapsed().as_secs_f64();
        let call_count = calls.len();
        drop(calls);
        let mark = Instant::now();
        let data = data_edges(view, &field_edges, &test_code);
        let data_edge_secs = mark.elapsed().as_secs_f64();
        let mark = Instant::now();
        let (mut data_rollups, data_subsystems) = rollup_edges(view, &data);
        add_field_flows(view, &field_edges, &mut data_rollups);
        let data_rollup_secs = mark.elapsed().as_secs_f64();
        println!(
            "Roll-ups: read the dataflow outputs in {:.1} s; {} call edges in {:.1} s, rolled up in \
             {:.1} s; {} data edges in {:.1} s, rolled up with the fields' flows in {:.1} s; {:.1} s \
             in all",
            read_secs,
            call_count,
            call_edge_secs,
            call_rollup_secs,
            data.len(),
            data_edge_secs,
            data_rollup_secs,
            started.elapsed().as_secs_f64()
        );
        subsystems.extend(data_subsystems);
        (call_rollups, data_rollups, subsystems)
    };
    // (Every symbol with a roll-up gets an entry, ex: in a subset, a class
    // whose methods its files call without naming it.)
    for sym in call_rollups.keys().chain(data_rollups.keys()) {
        search_result_table.entry(*sym).or_default();
    }
    if !subsystems.is_empty() {
        let root = root_subsystem_sym();
        search_result_table.entry(root).or_default();
        meta_table.insert(root, subsystem_structured(root, ustr("")));
    }
    for subsystem in subsystems {
        let sym = subsystem_sym(&subsystem);
        search_result_table.entry(sym).or_default();
        meta_table.insert(sym, subsystem_structured(sym, subsystem));
    }
    RollupTables {
        calls: call_rollups,
        data: data_rollups,
        summaries,
    }
}

/// For a subset's call roll-ups (see `crossref_subset`): the tree's
/// structured records of the symbols its calls involve (callers, callees,
/// their classes, and IPDL messages' owners and `Recv` implementations),
/// from the tree's crossref, so that the subset's roll-ups attribute calls
/// as the tree's do (and calls of the tree's functions outside the subset
/// count), which a try push's roll-up deltas need (see `RollupDelta`).
/// Plumbing callees (by their pretty names) aren't looked up.
fn tree_rollup_metas(
    base_index: &str,
    callees_table: &CalleesTable,
    pretty_table: &PrettyTable,
) -> MetaTable {
    let mut metas = MetaTable::new();
    let Some(tree) = BisectableMmap::<CrossrefData>::new(
        &format!("{}/crossref", base_index),
        &format!("{}/crossref-extra", base_index),
    ) else {
        return metas;
    };
    let mut missing: UstrSet = UstrSet::default();
    let mut fetch = |sym: Ustr, metas: &mut MetaTable| -> Option<AnalysisStructured> {
        if let Some(meta) = metas.get(&sym) {
            return Some(meta.clone());
        }
        if missing.contains(&sym) {
            return None;
        }
        match tree.lookup(&sym).ok().flatten().and_then(|data| data.meta) {
            Some(meta) => {
                metas.insert(sym, meta.clone());
                Some(meta)
            }
            None => {
                missing.insert(sym);
                None
            }
        }
    };
    let mut syms: Vec<Ustr> = callees_table.keys().copied().collect();
    syms.extend(
        callees_table
            .values()
            .flat_map(|callees| callees.keys().copied())
            .filter(|sym| {
                !pretty_table
                    .get(sym)
                    .is_some_and(|pretty| is_boring_callee(pretty))
            }),
    );
    syms.sort();
    syms.dedup();
    for sym in syms {
        let Some(meta) = fetch(sym, &mut metas) else {
            continue;
        };
        if let Some(parent) = meta.parent_sym {
            fetch(parent, &mut metas);
        }
        // (A call of an IPDL message's `Send` method reaches its `Recv`
        // implementations; see `call_edges`.)
        let Some(owner) = meta.slot_owner.as_ref().map(|o| o.sym) else {
            continue;
        };
        if !owner.starts_with("IPC_") {
            continue;
        }
        let Some(owner_meta) = fetch(owner, &mut metas) else {
            continue;
        };
        for slot in &owner_meta.binding_slots {
            let Some(slot_meta) = fetch(slot.sym, &mut metas) else {
                continue;
            };
            for sym in std::iter::once(slot.sym).chain(slot_meta.overridden_by_syms.iter().copied())
            {
                if let Some(parent) = fetch(sym, &mut metas).and_then(|m| m.parent_sym) {
                    fetch(parent, &mut metas);
                }
            }
        }
    }
    metas
}

/// Give each field (and function) in the edges between fields a data
/// roll-up of its own edges, for diagrams of fields (see
/// `cmd_block_diagram`'s field level), leaving out functions' reads and
/// writes of their classes' own state (see `InternalClasses`).
fn add_field_flows(
    meta_table: &MetaTable,
    field_edges: &[FieldEdge],
    rollups: &mut UstrMap<Rollup>,
) {
    let pretty_of = |sym: &Ustr| -> Ustr { meta_table.get(sym).map_or(*sym, |m| m.pretty) };
    let mut internal = InternalClasses::new(meta_table);
    let mut nodes: UstrSet = UstrSet::default();
    for edge in field_edges {
        // (Not a function's reads and writes of its class's own state's
        // fields, ex: an embedded member's.)
        let ends = [edge.from, edge.to];
        if let Some(function) = ends.iter().find(|sym| !is_field_sym(sym))
            && let Some(field) = ends.iter().find(|sym| is_field_sym(sym))
            && let (Some(context), Some(class)) = (
                rollup_class(meta_table, function),
                rollup_class(meta_table, field),
            )
            && internal.of(context).contains(&class)
        {
            continue;
        }
        let flow = |sym: Ustr| RollupEdge {
            sym,
            pretty: pretty_of(&sym),
            count: edge.count,
            pairs: 1,
            via: edge.via.clone(),
            ..Default::default()
        };
        rollups
            .entry(edge.from)
            .or_default()
            .out
            .push(flow(edge.to));
        rollups
            .entry(edge.to)
            .or_default()
            .into
            .push(flow(edge.from));
        nodes.insert(edge.from);
        nodes.insert(edge.to);
    }
    for node in nodes {
        let rollup = rollups.get_mut(&node).unwrap();
        let order = |a: &RollupEdge, b: &RollupEdge| b.count.cmp(&a.count).then(a.sym.cmp(&b.sym));
        rollup.out.sort_by(order);
        rollup.into.sort_by(order);
        rollup.out_total = rollup.out.len() as u32;
        rollup.in_total = rollup.into.len() as u32;
        rollup.out.truncate(ROLLUP_EDGES);
        rollup.into.truncate(ROLLUP_EDGES);
    }
}

/// The summaries of the functions the ontology has dataflow hints for (see
/// `OntologyRule::dataflow`), by their pretty names without template
/// arguments (all of their overloads and instantiations), at their
/// definitions, in place of `dataflow-summarize`'s, for traces to step over
/// their calls by (see `CrossrefData::dataflow`).
fn add_dataflow_hints(
    ontology: &OntologyMappingIngestion,
    pretty_table: &PrettyTable,
    search_result_table: &SearchResultTable,
    summaries: &mut UstrMap<Summary>,
) {
    let hints: HashMap<String, (Vec<_>, &str)> = ontology
        .config
        .pretty
        .iter()
        .filter_map(|(pretty, rule)| {
            let hint = rule.dataflow.as_ref()?;
            let flows: Vec<_> = hint
                .flows
                .iter()
                .filter_map(|flow| {
                    let parsed = parse_hint_flow(flow);
                    if parsed.is_none() {
                        eprintln!(
                            "ontology-mapping.toml: {}'s flow {:?} isn't a flow",
                            pretty, flow
                        );
                    }
                    parsed
                })
                .collect();
            Some((untemplated_pretty(pretty), (flows, hint.why.as_str())))
        })
        .collect();
    if hints.is_empty() {
        return;
    }
    for (sym, pretty) in pretty_table {
        let Some((flows, why)) = hints.get(&untemplated_pretty(pretty)) else {
            continue;
        };
        let (path, line) = search_result_table
            .get(sym)
            .and_then(|kinds| kinds.get(&AnalysisKind::Def))
            .and_then(|paths| paths.iter().next())
            .and_then(|(path, results)| Some((path.as_str(), results.first()?.lineno)))
            .unwrap_or(("", 0));
        summaries.insert(
            *sym,
            hint_summary(*sym, pretty, path, line, flows.clone(), why),
        );
    }
}

/// The functions' categories from `dataflow-summarize`'s summaries, its
/// edges between fields, and the accessors' (getters', setters' and trivial
/// methods') summaries, for their crossref entries (see
/// `CrossrefData::dataflow`), if the tree has them.
fn read_dataflow_outputs(
    index_path: &str,
) -> (UstrMap<Category>, Vec<FieldEdge>, UstrMap<Summary>) {
    let mut categories = UstrMap::default();
    let mut field_edges = vec![];
    let mut accessors = UstrMap::default();
    if let Ok(file) = File::open(format!("{}/dataflow-summaries", index_path)) {
        for line in BufReader::new(file).lines().map_while(|l| l.ok()) {
            if let Ok(summary) = serde_json::from_str::<Summary>(&line) {
                categories.insert(summary.sym, summary.category);
                if matches!(
                    summary.category,
                    Category::Getter | Category::Setter | Category::Trivial
                ) {
                    accessors.insert(summary.sym, summary);
                }
            }
        }
    }
    if let Ok(file) = File::open(format!("{}/dataflow-fields", index_path)) {
        for line in BufReader::new(file).lines().map_while(|l| l.ok()) {
            if let Ok(edge) = serde_json::from_str::<FieldEdge>(&line) {
                field_edges.push(edge);
            }
        }
    }
    (categories, field_edges, accessors)
}

fn write_crossref_and_jumpref_thread(
    xref_file: String,
    xref_ext_file: String,
    jumpref_file: String,
    jumpref_ext_file: String,
    search_result_list: &SearchResultList,
    start: usize,
    end: usize,
    ingestion: &RepoIngestion,
    pretty_table: &PrettyTable,
    meta_table: &MetaTable,
    callees_table: &CalleesTable,
    field_member_use_table: &FieldMemberUseTable,
    js_idl_table: &JSIDLTable,
    js_ts_table: &JsTsTable,
    recency_table: &UstrMap<(Recency, RecencyFrom)>,
    rollups: &RollupTables,
    is_first_chunk: bool,
    xref_ext_size: &mut Option<usize>,
    jumpref_ext_size: &mut Option<usize>,
) {
    // ## Write out the crossref and jumpref databases.
    // (Buffered: unbuffered, each symbol's few small writes were a system call
    // each, much of crossref's time writing.)
    let mut xref_out = BufWriter::new(File::create(xref_file).unwrap());
    let mut xref_ext_out = BufWriter::new(File::create(xref_ext_file).unwrap());

    let mut jumpref_out = BufWriter::new(File::create(jumpref_file).unwrap());
    let mut jumpref_ext_out = BufWriter::new(File::create(jumpref_ext_file).unwrap());

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

    for (id, id_data) in &search_result_list[start..end] {
        let mut crossref_data = CrossrefData::default();
        for (kind, kind_data) in id_data {
            let mut result = Vec::new();
            for (path, results) in kind_data {
                if let Some(concise_info) = ingestion.state.concise_per_file.get(path) {
                    result.push(PathSearchResult {
                        path: *path,
                        path_kind: concise_info.path_kind,
                        lines: results.clone(),
                        tree: None,
                    });
                } else {
                    // NSS seems to have an issue with auto-generated files we
                    // don't know about, so this can't be a warning because it's
                    // too spammy.
                    if reported_missing_concise.insert(*path) {
                        info!("Missing concise info for path '{}'", path);
                    }
                }
            }
            match kind {
                AnalysisKind::Use => crossref_data.uses = Some(result),
                AnalysisKind::Def => crossref_data.definitions = Some(result),
                AnalysisKind::Assign => crossref_data.assignments = Some(result),
                AnalysisKind::Decl => crossref_data.declarations = Some(result),
                AnalysisKind::Forward => crossref_data.forwards = Some(result),
                AnalysisKind::Idl => crossref_data.idl = Some(result),
                AnalysisKind::Idlp => crossref_data.idl_partial = Some(result),
                AnalysisKind::Glean => crossref_data.glean = Some(result),
                AnalysisKind::Alias => crossref_data.aliases = Some(result),
                AnalysisKind::DocDef => crossref_data.doc_defs = Some(result),
                AnalysisKind::DocUse => crossref_data.doc_uses = Some(result),
            }
        }
        if let Some(callee_syms) = callees_table.get(id) {
            let mut callees = Vec::new();
            for (callee_sym, (call_path, call_lines, only_cites)) in callee_syms {
                let Some(meta) = meta_table.get(callee_sym) else {
                    continue;
                };
                // (Citations are only of web specs' callable symbols, which
                // the symbol implements.)
                if *only_cites && !is_implementable(meta) {
                    continue;
                }
                let callee = Callee {
                    sym: *callee_sym,
                    pretty: if *only_cites {
                        Some(meta.pretty)
                    } else {
                        pretty_table.get(callee_sym).copied()
                    },
                    kind: meta.kind,
                    jump: format!("{}#{}", call_path, call_lines.iter().join(",")),
                    tree: None,
                    implements: *only_cites,
                };
                callees.push(callee);
            }
            // (Not for a symbol whose only "callees" were citations of
            // symbols it doesn't implement, ex: a document's of files.)
            if !callees.is_empty() || callee_syms.values().any(|(_, _, only_cites)| !only_cites) {
                crossref_data.callees = Some(callees);
            }
        }
        if let Some(fmu_syms) = field_member_use_table.get(id) {
            let mut fmus = Vec::new();
            for (fmu_sym, fmu_field_infos) in fmu_syms {
                let Some(meta) = meta_table.get(fmu_sym) else {
                    continue;
                };
                let fields: Vec<_> = fmu_field_infos
                    .into_iter()
                    .map(|&(pretty, ptr)| FieldInfos { pretty, ptr })
                    .collect();
                fmus.push(FieldMemberUse {
                    sym: *fmu_sym,
                    pretty: meta.pretty,
                    fields,
                });
            }
            crossref_data.field_member_uses = Some(fmus);
        }
        // Put the metadata in there too.
        let mut fallback_pretty = None;
        if let Some(meta) = meta_table.get(id) {
            crossref_data.meta = Some(meta.clone());
        } else {
            fallback_pretty = pretty_table.get(id).copied();
        }

        if let Some(idl_syms) = js_idl_table.get(id) {
            // Put the symbols only if there's a few candidates.
            if idl_syms.len() < MAX_JS_IDL_SYMS {
                crossref_data.idl_syms = Some(idl_syms.clone());
            }
        }

        if let Some(ts_syms) = js_ts_table.get(id) {
            crossref_data.ts_syms = ts_syms.clone();
        }

        if let Some((recency, from)) = recency_table.get(id) {
            crossref_data.recency = Some(*recency);
            crossref_data.recency_from = Some(from.clone());
        }

        if let Some(rollup) = rollups.calls.get(id) {
            crossref_data.call_rollup = Some(rollup.clone());
        }
        if let Some(rollup) = rollups.data.get(id) {
            crossref_data.data_rollup = Some(rollup.clone());
        }
        if let Some(summary) = rollups.summaries.get(id) {
            crossref_data.dataflow = Some(summary.clone());
        }

        write_inline_and_ext(
            &mut xref_out,
            &mut xref_ext_out,
            &mut xref_ext_offset,
            id,
            &crossref_data,
            is_first_chunk,
        );

        let jumpref_info =
            convert_crossref_value_to_sym_info_rep(Some(crossref_data), id, fallback_pretty);
        write_inline_and_ext(
            &mut jumpref_out,
            &mut jumpref_ext_out,
            &mut jumpref_ext_offset,
            id,
            &jumpref_info,
            is_first_chunk,
        );
    }

    for out in [
        &mut xref_out,
        &mut xref_ext_out,
        &mut jumpref_out,
        &mut jumpref_ext_out,
    ] {
        out.flush().unwrap();
    }
    xref_ext_size.replace(xref_ext_offset);
    jumpref_ext_size.replace(jumpref_ext_offset);
}

/// How many chunks of the symbols each of `write_crossref_and_jumpref`'s
/// threads writes, on average.
const WRITE_CHUNKS_PER_THREAD: usize = 8;
/// The buffers' sizes when concatenating the chunks' files.
const MERGE_BUF_SIZE: usize = 1 << 20;

fn write_crossref_and_jumpref(
    tree_name: &String,
    index_path: &String,
    search_result_table: SearchResultTable,
    ingestion: &RepoIngestion,
    pretty_table: PrettyTable,
    meta_table: MetaTable,
    callees_table: CalleesTable,
    field_member_use_table: FieldMemberUseTable,
    js_idl_table: JSIDLTable,
    js_ts_table: JsTsTable,
    recency_table: UstrMap<(Recency, RecencyFrom)>,
    rollups: RollupTables,
    thread_count: usize,
) {
    let search_result_list: SearchResultList = search_result_table.into_iter().collect();

    let total = search_result_list.len();
    let thread_count = if total > 100 { thread_count } else { 1 };

    if thread_count == 1 {
        let xref_file = format!("{}/crossref", index_path);
        let xref_ext_file = format!("{}/crossref-extra", index_path);
        let jumpref_file = format!("{}/jumpref", index_path);
        let jumpref_ext_file = format!("{}/jumpref-extra", index_path);

        let mut xref_ext_size = None;
        let mut jumpref_ext_size = None;

        write_crossref_and_jumpref_thread(
            xref_file,
            xref_ext_file,
            jumpref_file,
            jumpref_ext_file,
            &search_result_list,
            0,
            total,
            ingestion,
            &pretty_table,
            &meta_table,
            &callees_table,
            &field_member_use_table,
            &js_idl_table,
            &js_ts_table,
            &recency_table,
            &rollups,
            true,
            &mut xref_ext_size,
            &mut jumpref_ext_size,
        );

        return;
    }

    // Several chunks per thread, which threads claim as they finish theirs:
    // symbols' results are very uneven, so a chunk per thread straggles.  Each
    // chunk is written to its own files, which are concatenated in order.
    let chunk_count = (thread_count * WRITE_CHUNKS_PER_THREAD)
        .min(total / 100)
        .max(thread_count);
    let chunk = total.div_ceil(chunk_count);
    let chunk_count = total.div_ceil(chunk);
    info!(
        "{} search results found. Processing with {} threads ({} chunks of {})",
        total, thread_count, chunk_count, chunk
    );

    // crossref and jumpref files contain the offsets into the *-extra files.
    // The offset need to be adjusted when concatenating.
    let ext_sizes: std::sync::Mutex<Vec<(Option<usize>, Option<usize>)>> =
        std::sync::Mutex::new(vec![(None, None); chunk_count]);
    let next_chunk = std::sync::atomic::AtomicUsize::new(0);

    std::thread::scope(|s| {
        for _ in 0..thread_count {
            let ingestion = &ingestion;
            let search_result_list = &search_result_list;
            let pretty_table = &pretty_table;
            let meta_table = &meta_table;
            let callees_table = &callees_table;
            let field_member_use_table = &field_member_use_table;
            let js_idl_table = &js_idl_table;
            let js_ts_table = &js_ts_table;
            let recency_table = &recency_table;
            let rollups = &rollups;
            let ext_sizes = &ext_sizes;
            let next_chunk = &next_chunk;

            s.spawn(move || {
                loop {
                    let index = next_chunk.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if index >= chunk_count {
                        break;
                    }
                    let start = chunk * index;
                    let end = (chunk * (index + 1)).min(total);
                    let (mut xref_ext_size, mut jumpref_ext_size) = (None, None);
                    write_crossref_and_jumpref_thread(
                        format!("{}/crossref-{}", index_path, index),
                        format!("{}/crossref-extra-{}", index_path, index),
                        format!("{}/jumpref-{}", index_path, index),
                        format!("{}/jumpref-extra-{}", index_path, index),
                        search_result_list,
                        start,
                        end,
                        ingestion,
                        pretty_table,
                        meta_table,
                        callees_table,
                        field_member_use_table,
                        js_idl_table,
                        js_ts_table,
                        recency_table,
                        rollups,
                        index == 0,
                        &mut xref_ext_size,
                        &mut jumpref_ext_size,
                    );
                    ext_sizes.lock().unwrap()[index] = (xref_ext_size, jumpref_ext_size);
                }
            });
        }
    });
    let (xref_ext_sizes, jumpref_ext_sizes): (Vec<_>, Vec<_>) =
        ext_sizes.into_inner().unwrap().into_iter().unzip();

    println!(
        "Performing crossref::write-crossref-merge step for {} : {}",
        tree_name,
        Local::now().format("%Y-%m-%dT%H:%M:%S%z")
    );

    std::thread::scope(|s| {
        for (name, ext_sizes) in [("crossref", xref_ext_sizes), ("jumpref", jumpref_ext_sizes)] {
            let mut ext_offsets = vec![];
            let mut total = 0;
            for x in ext_sizes {
                ext_offsets.push(total);
                total += x.unwrap();
            }

            let index_path = index_path.clone();

            // (Lines into one reused buffer, without checking them as UTF-8:
            // a String per line made this the phase's time, ~1.5 minutes for
            // firefox-disco.)
            s.spawn(move || {
                let out_file = format!("{}/{}", index_path, name);
                let mut out_f =
                    BufWriter::with_capacity(MERGE_BUF_SIZE, File::create(out_file).unwrap());

                let mut line: Vec<u8> = Vec::new();
                for (i, ext_offset) in ext_offsets.iter().enumerate() {
                    let in_file = format!("{}/{}-{}", index_path, name, i);
                    let in_f = File::open(in_file.clone()).unwrap();
                    let mut reader = BufReader::with_capacity(MERGE_BUF_SIZE, in_f);
                    loop {
                        line.clear();
                        let size = reader.read_until(b'\n', &mut line).unwrap();
                        if size == 0 {
                            break;
                        }
                        // Adjust the *-extra file offset.
                        if i > 0 && line.starts_with(b"@!") {
                            let end = line.iter().position(|&b| b == b' ').unwrap();
                            let offset = std::str::from_utf8(&line[2..end])
                                .unwrap()
                                .parse::<usize>()
                                .unwrap();

                            write!(out_f, "@{:x}", offset + ext_offset).unwrap();
                            out_f.write_all(&line[end..]).unwrap();
                        } else {
                            out_f.write_all(&line).unwrap();
                        }
                    }
                    remove_file(in_file).unwrap()
                }
                out_f.flush().unwrap();
            });
        }

        // *-extra files can be simply concatenated.
        for name in ["crossref-extra", "jumpref-extra"] {
            let index_path = index_path.clone();

            s.spawn(move || {
                const BUF_SIZE: usize = 0xffff;
                let mut buf: [u8; BUF_SIZE] = [0; BUF_SIZE];

                let out_file = format!("{}/{}", index_path, name);
                let mut out_f = File::create(out_file).unwrap();

                for i in 0..chunk_count {
                    let in_file = format!("{}/{}-{}", index_path, name, i);
                    let mut in_f = File::open(in_file.clone()).unwrap();
                    loop {
                        let size = in_f.read(&mut buf).unwrap();
                        if size == 0 {
                            break;
                        }
                        out_f.write_all(&buf[..size]).unwrap();
                    }
                    remove_file(in_file).unwrap()
                }
            });
        }
    });
}

/// Write the identifiers file: a line "IDENTIFIER SYMBOL\n" for each of the
/// table's identifiers' symbols, sorted case-semisensitively (see
/// `case_semisensitive_str_cmp`, which is total, and the lines are distinct, so
/// the order doesn't depend on the sort's).  The lines are written into one
/// buffer, sized first, and their ranges in it sorted in parallel (rayon):
/// at firefox's scale, a `String` per line spent most of the phase in malloc,
/// on crossref's big heap, and the sort was single-threaded (~1.5 minutes).
fn write_identifiers(tree_config: &TreeConfig, id_table: IdTable) {
    let id_file = format!("{}/identifiers", tree_config.paths.index_path);

    let mut idf = BufWriter::new(File::create(id_file).unwrap());

    let (size, count) = id_table.iter().fold((0, 0), |(size, count), (id, syms)| {
        let size = size
            + syms
                .iter()
                .map(|sym| id.len() + sym.len() + 2)
                .sum::<usize>();
        (size, count + syms.len())
    });
    let mut text = String::with_capacity(size);
    let mut lines: Vec<std::ops::Range<usize>> = Vec::with_capacity(count);
    for (id, syms) in id_table {
        for sym in syms {
            let start = text.len();
            text.push_str(&id);
            text.push(' ');
            text.push_str(&sym);
            text.push('\n');
            lines.push(start..text.len());
        }
    }
    lines.par_sort_unstable_by(|lhs, rhs| {
        case_semisensitive_str_cmp(&text[lhs.clone()], &text[rhs.clone()])
    });

    for line in lines {
        let _ = idf.write_all(&text.as_bytes()[line]);
    }
    let _ = idf.flush();
}

/// Cross-reference a subset of the tree's files (see `CrossrefCli::subset_of`),
/// as `main` does the whole tree, but without history digests (and so without
/// file recency), and writing only the crossref, jumpref, and identifiers
/// files.
async fn crossref_subset(
    cli: &CrossrefCli,
    cfg: &Config,
    tree_config: &TreeConfig,
    base_index: &str,
) {
    let tree_name = &cli.tree_name;
    let analysis_relative_paths = read_analysis_list(&cli.analysis_files_list_path);
    let ingestion = subset_ingestion(cfg, base_index, &analysis_relative_paths);
    // (Directories' subsystems as the tree's are, by its files.)
    let tree_paths: Vec<Ustr> = File::open(format!("{}/all-analysis-files", base_index))
        .map(|file| {
            BufReader::new(file)
                .lines()
                .map_while(|l| l.ok())
                .map(|l| ustr(&l))
                .collect()
        })
        .unwrap_or_else(|_| analysis_relative_paths.clone());
    let dir_subsystems = fallback_subsystems(&ingestion, &tree_paths);
    let ontology = load_ontology(cfg);
    let AnalysisData {
        mut search_result_table,
        pretty_table,
        id_table,
        mut meta_table,
        callees_table,
        recency_table,
        ..
    } = read_analysis_files(
        analysis_relative_paths,
        tree_name,
        tree_config,
        &ingestion,
        &dir_subsystems,
        None,
        cli.thread_count,
    );

    // (Like `main`'s.)
    let linked = TreeJumprefs::open(&tree_config.paths, &cfg.tree_index_paths);
    if !linked.linked.is_empty() {
        let cited: UstrSet = callees_table
            .values()
            .flat_map(|callees| callees.iter())
            .filter(|(sym, (_, _, only_cites))| *only_cites && !meta_table.contains_key(*sym))
            .map(|(sym, _)| *sym)
            .collect();
        for sym in cited {
            if let Some(meta) = linked.lookup_linked(&sym).and_then(|data| data.meta) {
                meta_table.insert(sym, meta);
            }
        }
    }

    let field_owning_class_rules = read_field_owning_class_rules(&ontology);
    let field_member_use_table = process_class_fields(
        &ontology,
        &id_table,
        &mut meta_table,
        field_owning_class_rules,
    );
    process_ontology_rules(&ontology, &search_result_table, &id_table, &mut meta_table);
    let (js_idl_table, js_ts_table) = create_js_tables(&meta_table);
    let tree_metas = tree_rollup_metas(base_index, &callees_table, &pretty_table);
    let mut rollups = add_rollups(
        &tree_config.paths.index_path,
        &mut meta_table,
        &mut search_result_table,
        &callees_table,
        Some(tree_metas),
        &ingestion,
    );
    add_dataflow_hints(
        &ontology,
        &pretty_table,
        &search_result_table,
        &mut rollups.summaries,
    );
    write_crossref_and_jumpref(
        tree_name,
        &tree_config.paths.index_path,
        search_result_table,
        &ingestion,
        pretty_table,
        meta_table,
        callees_table,
        field_member_use_table,
        js_idl_table,
        js_ts_table,
        recency_table,
        rollups,
        cli.thread_count,
    );
    write_identifiers(tree_config, id_table);
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
    // (rayon's work, ex: sorting the identifiers, on as many threads as the
    // rest.)
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(cli.thread_count.max(1))
        .build_global();

    let tree_name = &cli.tree_name;
    println!(
        "Performing crossref::load-config step for {} : {}",
        tree_name,
        Local::now().format("%Y-%m-%dT%H:%M:%S%z")
    );
    let mut cfg = config::load(&cli.config_file, false, Some(tree_name), None, None);
    if let Some(tree_config) = cfg.trees.get_mut(tree_name) {
        if let Some(index_path) = &cli.index_path {
            tree_config.paths.index_path = index_path.clone();
        }
        if let Some(files_path) = &cli.files_path {
            tree_config.paths.files_path = files_path.clone();
        }
        if let Some(objdir_path) = &cli.objdir_path {
            tree_config.paths.objdir_path = objdir_path.clone();
        }
    }

    let tree_config = cfg.trees.get(tree_name).unwrap();

    if let Some(base_index) = &cli.subset_of {
        crossref_subset(&cli, &cfg, tree_config, base_index).await;
        return;
    }

    println!(
        "Performing crossref::load-files-and-dirs step for {} : {}",
        tree_name,
        Local::now().format("%Y-%m-%dT%H:%M:%S%z")
    );
    let AllFilesAndDirs {
        analysis_relative_paths,
        all_files_paths,
        all_dirs_paths,
    } = load_all_files_and_dirs(&cli.analysis_files_list_path, &tree_config.paths.index_path);

    // ## Ingest Repo-Wide Information
    println!(
        "Performing crossref::ingest step for {} : {}",
        tree_name,
        Local::now().format("%Y-%m-%dT%H:%M:%S%z")
    );
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

    let ingestion = ingest_files(
        &cfg,
        tree_config,
        tree_name,
        all_files_paths,
        all_dirs_paths,
    );

    // Consume the ingestion logged span, pass it through our repo-ingestion
    // explainer template, and write it do sik.
    drop(ingestion_entered);
    write_repo_ingestion_diag(tree_config, logged_ingestion_span).await;

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
    let ontology = load_ontology(&cfg);
    drop(ontology_entered);

    // ## Process all the analysis files
    println!(
        "Performing crossref::read-analysis step for {} : {}",
        tree_name,
        Local::now().format("%Y-%m-%dT%H:%M:%S%z")
    );
    // The tree's history, if it has one, for the symbols' history digests (see
    // `hyperblame::recency`).
    let history = tree_config.git.as_ref().and_then(HistoryDigests::open);
    println!(
        "History digests for {}: {}",
        tree_name,
        if history.is_some() { "yes" } else { "no" }
    );
    let analyzed: UstrSet = analysis_relative_paths.iter().copied().collect();
    let dir_subsystems = fallback_subsystems(&ingestion, &analysis_relative_paths);
    let AnalysisData {
        mut search_result_table,
        pretty_table,
        id_table,
        mut meta_table,
        callees_table,
        recency_table,
        mut file_recency_table,
        recency_linked,
        recency_linkages,
    } = read_analysis_files(
        analysis_relative_paths,
        tree_name,
        tree_config,
        &ingestion,
        &dir_subsystems,
        history.as_ref(),
        cli.thread_count,
    );

    // The structured records of the symbols the tree cites which its linked
    // trees define (see `TreeConfigPaths::linked_trees`, ex: firefox-disco's
    // citations of wubspecs' spec algorithms), for their kinds and pretty
    // names, so that its symbols implementing them have them as callees.
    let linked = TreeJumprefs::open(&tree_config.paths, &cfg.tree_index_paths);
    if !linked.linked.is_empty() {
        let cited: UstrSet = callees_table
            .values()
            .flat_map(|callees| callees.iter())
            .filter(|(sym, (_, _, only_cites))| *only_cites && !meta_table.contains_key(*sym))
            .map(|(sym, _)| *sym)
            .collect();
        for sym in cited {
            if let Some(meta) = linked.lookup_linked(&sym).and_then(|data| data.meta) {
                meta_table.insert(sym, meta);
            }
        }
    }

    // ## Run Ontology Processing
    println!(
        "Performing crossref::ontology-processing step for {} : {}",
        tree_name,
        Local::now().format("%Y-%m-%dT%H:%M:%S%z")
    );
    let ontology_entered = logged_ontology_span.span.clone().entered();

    info!("Processing ontology now that all analysis files have been read in.");

    let field_owning_class_rules = read_field_owning_class_rules(&ontology);
    let field_member_use_table = process_class_fields(
        &ontology,
        &id_table,
        &mut meta_table,
        field_owning_class_rules,
    );

    process_ontology_rules(&ontology, &search_result_table, &id_table, &mut meta_table);

    // Consume the ontology logged span, pass it through our ontology-ingestion
    // explainer template, and write it to disk.
    drop(ontology_entered);
    write_ontology_ingestion_diag(tree_config, logged_ontology_span).await;

    println!(
        "Performing crossref::js-idl and js-ts step for {} : {}",
        tree_name,
        Local::now().format("%Y-%m-%dT%H:%M:%S%z")
    );
    let (js_idl_table, js_ts_table) = create_js_tables(&meta_table);

    println!(
        "Performing crossref::rollup step for {} : {}",
        tree_name,
        Local::now().format("%Y-%m-%dT%H:%M:%S%z")
    );
    let mut rollups = add_rollups(
        &tree_config.paths.index_path,
        &mut meta_table,
        &mut search_result_table,
        &callees_table,
        None,
        &ingestion,
    );
    add_dataflow_hints(
        &ontology,
        &pretty_table,
        &search_result_table,
        &mut rollups.summaries,
    );

    println!(
        "Performing crossref::write-crossref step for {} : {}",
        tree_name,
        Local::now().format("%Y-%m-%dT%H:%M:%S%z")
    );
    write_crossref_and_jumpref(
        tree_name,
        &tree_config.paths.index_path,
        search_result_table,
        &ingestion,
        pretty_table,
        meta_table,
        callees_table,
        field_member_use_table,
        js_idl_table,
        js_ts_table,
        recency_table,
        rollups,
        cli.thread_count,
    );

    if let Some(history) = &history {
        println!(
            "Performing crossref::file-recency step for {} : {}",
            tree_name,
            Local::now().format("%Y-%m-%dT%H:%M:%S%z")
        );
        let unanalyzed: Vec<Ustr> = ingestion
            .state
            .concise_per_file
            .iter()
            .filter(|(path, info)| !info.is_dir && !analyzed.contains(*path))
            .map(|(path, _)| *path)
            .collect();
        let started = Instant::now();
        let unanalyzed_recency = unanalyzed_file_recency(history, &unanalyzed);
        println!(
            "History digests of files without analysis: {} of {}, in {:.1} s",
            unanalyzed_recency.len(),
            unanalyzed.len(),
            started.elapsed().as_secs_f64()
        );
        for (path, file_recency) in unanalyzed_recency {
            file_recency_table.insert(path.to_string(), file_recency);
        }
        write_file_recency(&tree_config.paths.index_path, file_recency_table);
        write_recency_linkage(
            &tree_config.paths.index_path,
            recency_linked,
            recency_linkages,
        );
    }

    println!(
        "Performing crossref::write-typo-words step for {} : {}",
        tree_name,
        Local::now().format("%Y-%m-%dT%H:%M:%S%z")
    );
    // (The words which queries finding nothing are corrected to: the
    // identifiers' names, and the files' and directories'; see
    // `tools::typos`.)
    {
        let index_path = &tree_config.paths.index_path;
        let files = std::fs::read_to_string(format!("{}/all-files", index_path))
            .or_else(|_| std::fs::read_to_string(format!("{}/repo-files", index_path)))
            .unwrap_or_default();
        if let Err(err) = tools::typos::write_words(
            index_path,
            id_table.iter().map(|(id, syms)| (id.as_str(), syms.len())),
            files.lines(),
        ) {
            eprintln!("Couldn't write the typo words: {}", err);
        }
    }

    println!(
        "Performing crossref::write-identifiers step for {} : {}",
        tree_name,
        Local::now().format("%Y-%m-%dT%H:%M:%S%z")
    );
    write_identifiers(tree_config, id_table);

    println!(
        "Performing crossref::write-concise-file-info step for {} : {}",
        tree_name,
        Local::now().format("%Y-%m-%dT%H:%M:%S%z")
    );
    ingestion
        .state
        .write_out_concise_file_info(&tree_config.paths.index_path);

    // (Without freeing its tables: their outputs are written, and at
    // firefox's scale, with tens of GB of them partly swapped out, freeing
    // them took minutes.)
    let _ = std::io::stdout().flush();
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xpidl_member_of_ts_sym() {
        let sym = "S_js_typescript_generated/lib.gecko.xpcom.d.ts/global/nsIObserverService#";
        assert_eq!(
            xpidl_member_of_ts_sym(&format!("{}notifyObservers().", sym)).as_deref(),
            Some("XPIDL_nsIObserverService_notifyObservers")
        );
        assert_eq!(
            xpidl_member_of_ts_sym(
                "S_js_typescript_generated/lib.gecko.xpcom.d.ts/global/nsIFile#path."
            )
            .as_deref(),
            Some("XPIDL_nsIFile_path")
        );
        assert_eq!(xpidl_member_of_ts_sym(sym), None);
        assert_eq!(
            xpidl_member_of_ts_sym("S_js_typescript/lib.dom.d.ts/global/Document#title."),
            None
        );
    }
}
