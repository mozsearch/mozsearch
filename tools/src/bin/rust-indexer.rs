extern crate clap;
extern crate env_logger;
#[macro_use]
extern crate log;
extern crate protobuf;
extern crate rls_analysis;
extern crate rls_data as data;
extern crate scip;
extern crate tools;

use crate::data::GlobalCrateId;
use crate::data::{DefKind, ImplKind};
use clap::Parser;
use rls_analysis::{AnalysisHost, AnalysisLoader, SearchDirectory};
use serde_json::to_string;
use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap};
use std::fs::{self, File};
use std::hash::Hash;
use std::io;
use std::io::{BufRead, BufReader, Read, Seek};
use std::path::{Path, PathBuf};
use tools::file_format::analysis::{
    AnalysisKind, AnalysisSource, AnalysisTarget, LineRange, Location, SourceRange, SourceTag,
    TargetTag, WithLocation,
};
use ustr::ustr;

// Note: This file has been forked into scip-indexer.rs for generic SCIP
// ingestion purposes.  (And this comment is being added so git's copy
// detection has an easier time of realizing that!)

/// A global definition id in a crate.
///
/// FIXME(emilio): This key is kind of slow, because GlobalCrateId contains a
/// String. There's a "disambiguator" field which may be more than enough for
/// our purposes.
#[derive(Clone, Hash, Debug, Eq, PartialEq)]
pub struct DefId(GlobalCrateId, u32);

/// A map from global definition ids to the actual definition.
pub struct Defs {
    map: HashMap<DefId, data::Def>,
}

/// Local filesystem path mappings and metadata which exist for the following
/// purposes:
/// 1. Know where to output the analysis files.
///   - There is only ever one analysis output directory.
/// 2. Know how to locate rust source files in order to hackily extract strings
///    that should have been in the save-analysis files.
///    - After config scripts run and normalize things there are 2 source
///      directories: revision controlled source (cross-platform) and the
///      (per-platform) generated files directory.
#[derive(Debug)]
struct TreeInfo<'a> {
    /// Local filesystem path root for the analysis dir where rust-indexer.rs
    /// should write its output.
    out_analysis_dir: &'a Path,
    /// Local filesystem path root for the source tree.  In the searchfox path
    /// space presented to users, this means all paths not prefixed with
    /// `__GENERATED__`.
    srcdir: &'a Path,
    /// Local filesystem path root for the per-platform generated source tree.
    /// In the searchfox path space presented to users, this means paths
    /// prefixed with `__GENERATED__`.
    generated: &'a Path,
    /// The searchfox path space prefix for generated.
    generated_friendly: &'a Path,
}

fn construct_qualname(scope: &str, name: &str) -> String {
    // Some of the names don't start with ::, for example:
    //   __self_0_0$282
    //   <Loader>::new
    // Since we're gluing it to the "scope" (which might be a crate name)
    // we'll insert the :: to make it more readable
    let glue = if name.starts_with("::") { "" } else { "::" };
    format!("{}{}{}", scope, glue, name)
}

fn sanitize_symbol(sym: &str) -> String {
    // Downstream processing of the symbol doesn't deal well with
    // these characters, so replace them with underscores.
    fn is_special_char(c: char) -> bool {
        matches!(c, ',' | '.' | '(' | ')' | '-')
    }
    fn is_separator(c: char) -> bool {
        c.is_ascii_whitespace() || matches!(c, '#' | '/')
    }
    sym.replace(is_special_char, "_")
        .trim_matches('_')
        .replace(is_separator, "::")
        .trim_matches(':')
        .into()
}

fn pretty_symbol(sym: &str) -> Cow<str> {
    use scip::symbol::SymbolFormatOptions;
    if let Ok(sym) = scip::symbol::parse_symbol(&sym) {
        return Cow::Owned(scip::symbol::format_symbol_with(
            sym,
            SymbolFormatOptions {
                include_scheme: false,
                include_package_manager: false,
                include_package_name: true,
                include_package_version: false,
                include_descriptor: true,
            },
        ));
    }
    Cow::Borrowed(sym)
}

// Given a definition, and the global crate id where that definition is found,
// return a qualified name that identifies the definition unambiguously.
fn crate_independent_qualname(def: &data::Def, crate_id: &data::GlobalCrateId) -> String {
    // For stuff with "no_mangle" functions or statics, or extern declarations,
    // we just use the name.
    //
    // TODO(emilio): Maybe there's a way to get the #[link_name] attribute from
    // here and make C++ agree with that? Though we don't use it so it may not
    // be worth the churn.
    fn use_unmangled_name(def: &data::Def) -> bool {
        match def.kind {
            DefKind::ForeignStatic | DefKind::ForeignFunction => true,
            DefKind::Static | DefKind::Function => {
                def.attributes.iter().any(|attr| attr.value == "no_mangle")
            }
            _ => false,
        }
    }

    if use_unmangled_name(def) {
        return def.name.clone();
    }

    construct_qualname(&crate_id.name, &def.qualname)
}

impl Defs {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    fn insert(&mut self, analysis: &data::Analysis, def: &data::Def) {
        let crate_id = analysis.prelude.as_ref().unwrap().crate_id.clone();
        let mut definition = def.clone();
        definition.qualname = crate_independent_qualname(&def, &crate_id);

        let index = definition.id.index;
        let defid = DefId(crate_id, index);
        debug!("Indexing def: {:?} -> {:?}", defid, definition);
        let previous = self.map.insert(defid, definition);
        if let Some(previous) = previous {
            // This shouldn't happen, but as of right now it can happen with
            // some builtin definitions when highly generic types are involved.
            // This is probably a rust bug, just ignore it for now.
            debug!(
                "Found a definition with the same ID twice? {:?}, {:?}",
                previous, def,
            );
        }
    }

    /// Getter for a given local id, which takes care of converting to a global
    /// ID and returning the definition if present.
    fn get(&self, analysis: &data::Analysis, id: data::Id) -> Option<data::Def> {
        let prelude = analysis.prelude.as_ref().unwrap();
        let krate_id = if id.krate == 0 {
            prelude.crate_id.clone()
        } else {
            // TODO(emilio): This escales with the number of crates in this
            // particular crate, but it's probably not too bad, since it should
            // be a pretty fast linear search.
            let krate = prelude
                .external_crates
                .iter()
                .find(|krate| krate.num == id.krate);

            let krate = match krate {
                Some(k) => k,
                None => {
                    debug!("Crate not found: {:?}", id);
                    return None;
                }
            };

            krate.id.clone()
        };

        let id = DefId(krate_id, id.index);
        let result = self.map.get(&id).cloned();
        if result.is_none() {
            debug!("Def not found: {:?}", id);
        }
        result
    }
}

#[derive(Clone)]
pub struct Loader {
    deps_dirs: Vec<PathBuf>,
}

impl Loader {
    pub fn new(deps_dirs: Vec<PathBuf>) -> Self {
        Self { deps_dirs }
    }
}

impl AnalysisLoader for Loader {
    fn needs_hard_reload(&self, _: &Path) -> bool {
        true
    }

    fn fresh_host(&self) -> AnalysisHost<Self> {
        AnalysisHost::new_with_loader(self.clone())
    }

    fn set_path_prefix(&mut self, _: &Path) {}

    fn abs_path_prefix(&self) -> Option<PathBuf> {
        None
    }
    fn search_directories(&self) -> Vec<SearchDirectory> {
        self.deps_dirs
            .iter()
            .map(|pb| SearchDirectory {
                path: pb.clone(),
                prefix_rewrite: None,
            })
            .collect()
    }
}

fn def_kind_to_human(kind: DefKind) -> &'static str {
    match kind {
        DefKind::Enum => "enum",
        DefKind::Local => "local",
        DefKind::ExternType => "extern type",
        DefKind::Const => "constant",
        DefKind::Field => "field",
        DefKind::Function | DefKind::ForeignFunction => "function",
        DefKind::Macro => "macro",
        DefKind::Method => "method",
        DefKind::Mod => "module",
        DefKind::Static | DefKind::ForeignStatic => "static",
        DefKind::Struct => "struct",
        DefKind::Tuple => "tuple",
        DefKind::TupleVariant => "tuple variant",
        DefKind::Union => "union",
        DefKind::Type => "type",
        DefKind::Trait => "trait",
        DefKind::StructVariant => "struct variant",
    }
}

/// Potentially non-helpful mapping of impl kind.
fn impl_kind_to_human(kind: &ImplKind) -> &'static str {
    match kind {
        ImplKind::Inherent => "impl",
        ImplKind::Direct => "impl for",
        ImplKind::Indirect => "impl for ref",
        ImplKind::Blanket => "impl for where",
        _ => "impl for where deref",
    }
}

/// Given two spans, create a new super-span that encloses them both if the files match.  If the
/// files don't match, just return the first span as-is.
fn union_spans(a: &data::SpanData, b: &data::SpanData) -> data::SpanData {
    if a.file_name != b.file_name {
        return a.clone();
    }

    let (byte_start, line_start, column_start) = if a.byte_start < b.byte_start {
        (a.byte_start, a.line_start, a.column_start)
    } else {
        (b.byte_start, b.line_start, b.column_start)
    };

    let (byte_end, line_end, column_end) = if a.byte_end > b.byte_end {
        (a.byte_end, a.line_end, a.column_end)
    } else {
        (b.byte_end, b.line_end, b.column_end)
    };

    data::SpanData {
        file_name: a.file_name.clone(),
        byte_start,
        byte_end,
        line_start,
        line_end,
        column_start,
        column_end,
    }
}

/// For the purposes of trying to figure out the actual effective nesting range of some type of
/// definition, union its span (which just really covers the symbol name) plus the spans of all of
/// its descendants.  This should end up with a sufficiently reasonable line value.  This is a hack.
fn recursive_union_spans_of_def(
    def: &data::Def,
    file_analysis: &data::Analysis,
    defs: &Defs,
) -> data::SpanData {
    let mut span = def.span.clone();
    for id in &def.children {
        // It should already be the case that the children are in the same krate, but better safe
        // than sorry.
        if id.krate != def.id.krate {
            continue;
        }
        let kid = defs.get(file_analysis, *id);

        if let Some(ref kid) = kid {
            let rec_span = recursive_union_spans_of_def(kid, file_analysis, defs);
            span = union_spans(&span, &rec_span);
        }
    }

    span
}

/// Given a list of ids of defs, run recursive_union_spans_of_def on all of them and union up the
/// result.  Necessary for when dealing with impls.
fn union_spans_of_defs(
    initial_span: &data::SpanData,
    ids: &[data::Id],
    file_analysis: &data::Analysis,
    defs: &Defs,
) -> data::SpanData {
    let mut span = initial_span.clone();
    for id in ids {
        let kid = defs.get(file_analysis, *id);

        if let Some(ref kid) = kid {
            let rec_span = recursive_union_spans_of_def(kid, file_analysis, defs);
            span = union_spans(&span, &rec_span);
        }
    }

    span
}

/// If we unioned together a span that only covers 1 or 2 lines, normalize it to None because
/// nothing interesting will happen from a presentation perspective.  (If we had proper AST info
/// about the span, it would be appropriate to keep it and expose it, but this is all derived from
/// shoddy inference.)
fn ignore_boring_spans(span: &data::SpanData) -> Option<&data::SpanData> {
    match span {
        span if span.line_end.0 > span.line_start.0 + 1 => Some(span),
        _ => None,
    }
}

fn pretty_for_impl(imp: &data::Impl, qualname: &str) -> String {
    let mut pretty = impl_kind_to_human(&imp.kind).to_owned();
    pretty.push_str(" ");
    pretty.push_str(qualname);

    pretty
}

fn pretty_for_def(def: &data::Def, qualname: &str) -> String {
    let mut pretty = def_kind_to_human(def.kind).to_owned();
    pretty.push_str(" ");
    // We use the unsanitized qualname here because it's more human-readable
    // and the source-analysis pretty name is allowed to have commas and such
    pretty.push_str(qualname);

    pretty
}

fn visit_def(
    out_data: &mut BTreeSet<String>,
    kind: AnalysisKind,
    location: &data::SpanData,
    qualname: &str,
    def: &data::Def,
    context: Option<&str>,
    nesting: Option<&data::SpanData>,
) {
    let pretty = pretty_for_def(&def, &qualname);
    visit_common(
        out_data, kind, location, qualname, &pretty, context, nesting,
    );
}

fn visit_common(
    out_data: &mut BTreeSet<String>,
    kind: AnalysisKind,
    location: &data::SpanData,
    qualname: &str,
    pretty: &str,
    context: Option<&str>,
    nesting: Option<&data::SpanData>,
) {
    // Searchfox uses 1-indexed lines, 0-indexed columns.
    let col_end = if location.line_start != location.line_end {
        // Rust spans are multi-line... So we just use the start column as
        // the end column if it spans multiple rows, searchfox has fallback
        // code to handle this.
        location.column_start.zero_indexed().0
    } else {
        location.column_end.zero_indexed().0
    };
    let loc = Location {
        lineno: location.line_start.0,
        col_start: location.column_start.zero_indexed().0,
        col_end,
    };

    let sanitized = sanitize_symbol(qualname);
    let target_data = WithLocation {
        data: AnalysisTarget {
            target: TargetTag::Target,
            kind,
            pretty: ustr(&sanitized),
            sym: ustr(&sanitized),
            context: ustr(context.unwrap_or("")),
            contextsym: ustr(context.unwrap_or("")),
            peek_range: LineRange {
                start_lineno: 0,
                end_lineno: 0,
            },
            arg_ranges: vec![],
        },
        loc: loc.clone(),
    };
    out_data.insert(to_string(&target_data).unwrap());

    let nesting_range = match nesting {
        Some(span) => SourceRange {
            // Hack note: These positions would ideally be those of braces.  But they're not, so
            // while the position:sticky UI stuff should work-ish, other things will not.
            start_lineno: span.line_start.0,
            start_col: span.column_start.zero_indexed().0,
            end_lineno: span.line_end.0,
            end_col: span.column_end.zero_indexed().0,
        },
        None => SourceRange {
            start_lineno: 0,
            start_col: 0,
            end_lineno: 0,
            end_col: 0,
        },
    };

    let source_data = WithLocation {
        data: AnalysisSource {
            source: SourceTag::Source,
            syntax: vec![],
            pretty: ustr(&pretty),
            sym: vec![ustr(&sanitized)],
            no_crossref: false,
            nesting_range,
            // TODO: Expose type information for fields/etc.
            type_pretty: None,
            type_sym: None,
            arg_ranges: vec![],
        },
        loc,
    };
    out_data.insert(to_string(&source_data).unwrap());
}

/// Normalizes a searchfox user-visible relative file path to be an absolute
/// local filesystem path.  No attempt is made to validate the existence of the
/// path.  That's up to the caller.
fn searchfox_path_to_local_path(searchfox_path: &Path, tree_info: &TreeInfo) -> PathBuf {
    if let Ok(objdir_path) = searchfox_path.strip_prefix(tree_info.generated_friendly) {
        return tree_info.generated.join(objdir_path);
    }
    tree_info.srcdir.join(searchfox_path)
}

fn read_existing_contents(map: &mut BTreeSet<String>, file: &Path) {
    if let Ok(f) = File::open(file) {
        let reader = BufReader::new(f);
        for line in reader.lines() {
            map.insert(line.unwrap());
        }
    }
}

fn extract_span_from_source_as_buffer(
    reader: &mut File,
    span: &data::SpanData,
) -> io::Result<Box<[u8]>> {
    reader.seek(std::io::SeekFrom::Start(span.byte_start.into()))?;
    let len = (span.byte_end - span.byte_start) as usize;
    let mut buffer: Box<[u8]> = vec![0; len].into_boxed_slice();
    reader.read_exact(&mut buffer)?;
    Ok(buffer)
}

/// Given a reader and a span from that file, extract the text contained by the span.  If the span
/// covers multiple lines, then whatever newline delimiters the file has will be included.
///
/// In the event of a file read error or the contents not being valid UTF-8, None is returned.
/// We will log to log::Error in the event of a file read problem because this can be indicative
/// of lower level problems (ex: in vagrant), but not for utf-8 errors which are more expected
/// from sketchy source-files.
fn extract_span_from_source_as_string(
    mut reader: &mut File,
    span: &data::SpanData,
) -> Option<String> {
    match extract_span_from_source_as_buffer(&mut reader, &span) {
        Ok(buffer) => match String::from_utf8(buffer.into_vec()) {
            Ok(s) => Some(s),
            Err(_) => None,
        },
        // This used to error! but the error payload was always just
        // `Unable to read file: Custom { kind: UnexpectedEof, error: "failed to fill whole buffer" }`
        // which was not useful or informative and may be due to invalid spans
        // being told to us by save-analysis.
        Err(_) => None,
    }
}

fn create_output_dir(output_file: &Path) -> io::Result<()> {
    let mut output_dir = output_file.to_owned();
    output_dir.pop();
    fs::create_dir_all(output_dir)
}

fn analyze_file(
    searchfox_path: &PathBuf,
    defs: &Defs,
    file_analysis: &data::Analysis,
    tree_info: &TreeInfo,
) {
    use std::io::Write;

    debug!("Running analyze_file for {}", searchfox_path.display());

    let local_source_path = searchfox_path_to_local_path(searchfox_path, tree_info);

    if !local_source_path.exists() {
        info!(
            "Skipping nonexistent source file with searchfox path '{}' which mapped to local path '{}'",
            searchfox_path.display(),
            local_source_path.display()
        );
        return;
    };

    // Attempt to open the source file to extract information not currently available from the
    // analysis data.  Some analysis information may not be emitted if we are unable to access the
    // file.
    let maybe_source_file = match File::open(&local_source_path) {
        Ok(f) => Some(f),
        Err(_) => None,
    };

    let output_file = tree_info.out_analysis_dir.join(searchfox_path);
    let mut dataset = BTreeSet::new();
    read_existing_contents(&mut dataset, &output_file);
    if let Err(err) = create_output_dir(&output_file) {
        error!(
            "Couldn't create dir for: {}, {:?}",
            output_file.display(),
            err
        );
        return;
    }
    let mut file = match File::create(&output_file) {
        Ok(f) => f,
        Err(err) => {
            error!(
                "Couldn't open output file: {}, {:?}",
                output_file.display(),
                err
            );
            return;
        }
    };

    // Be chatty about the files we're outputting so that it's easier to follow
    // the path of rust analysis generation.
    info!(
        "Writing analysis for '{}' to '{}'",
        searchfox_path.display(),
        output_file.display()
    );

    for import in &file_analysis.imports {
        let id = match import.ref_id {
            Some(id) => id,
            None => {
                debug!(
                    "Dropping import {} ({:?}): {}, no ref",
                    import.name, import.kind, import.value
                );
                continue;
            }
        };

        let def = match defs.get(file_analysis, id) {
            Some(def) => def,
            None => {
                debug!(
                    "Dropping import {} ({:?}): {}, no def for ref {:?}",
                    import.name, import.kind, import.value, id
                );
                continue;
            }
        };

        visit_def(
            &mut dataset,
            AnalysisKind::Use,
            &import.span,
            &def.qualname,
            &def,
            None,
            None,
        )
    }

    for def in &file_analysis.defs {
        let parent = def
            .parent
            .and_then(|parent_id| defs.get(file_analysis, parent_id));

        if let Some(ref parent) = parent {
            if parent.kind == DefKind::Trait {
                let trait_dependent_name = construct_qualname(&parent.qualname, &def.name);
                visit_def(
                    &mut dataset,
                    AnalysisKind::Def,
                    &def.span,
                    &trait_dependent_name,
                    &def,
                    Some(&parent.qualname),
                    None,
                )
            }
        }

        let crate_id = &file_analysis.prelude.as_ref().unwrap().crate_id;
        let qualname = crate_independent_qualname(&def, crate_id);
        let nested_span = recursive_union_spans_of_def(def, &file_analysis, &defs);
        let maybe_nested = ignore_boring_spans(&nested_span);
        visit_def(
            &mut dataset,
            AnalysisKind::Def,
            &def.span,
            &qualname,
            &def,
            parent.as_ref().map(|p| &*p.qualname),
            maybe_nested,
        )
    }

    // We want to expose impls as "def,namespace" with an inferred nesting_range for their
    // contents.  I don't know if it's a bug or just a dubious design decision, but the impls all
    // have empty values and no names, so to get a useful string out of them, we need to extract
    // the contents of their span directly.
    //
    // Because the name needs to be extracted from the source file, we omit this step if we were
    // unable to open the file.
    if let Some(mut source_file) = maybe_source_file {
        for imp in &file_analysis.impls {
            // (for simple.rs at least, there is never a parent)

            let name = match extract_span_from_source_as_string(&mut source_file, &imp.span) {
                Some(s) => s,
                None => continue,
            };

            let crate_id = &file_analysis.prelude.as_ref().unwrap().crate_id;
            let qualname = construct_qualname(&crate_id.name, &name);
            let pretty = pretty_for_impl(&imp, &qualname);
            let nested_span = union_spans_of_defs(&imp.span, &imp.children, &file_analysis, &defs);
            let maybe_nested = ignore_boring_spans(&nested_span);
            // XXX visit_common currently never emits any syntax types; we want to pretend this is
            // a namespace once it does.
            visit_common(
                &mut dataset,
                AnalysisKind::Def,
                &imp.span,
                &qualname,
                &pretty,
                None,
                maybe_nested,
            )
        }
    }

    for ref_ in &file_analysis.refs {
        let def = match defs.get(file_analysis, ref_.ref_id) {
            Some(d) => d,
            None => {
                debug!(
                    "Dropping ref {:?}, kind {:?}, no def",
                    ref_.ref_id, ref_.kind
                );
                continue;
            }
        };
        visit_def(
            &mut dataset,
            AnalysisKind::Use,
            &ref_.span,
            &def.qualname,
            &def,
            /* context = */ None, // TODO
            /* nesting = */ None,
        )
    }

    for obj in &dataset {
        file.write_all(obj.as_bytes()).unwrap();
        write!(file, "\n").unwrap();
    }
}

// Replace any backslashes in the path with forward slashes.  Paths can be a
// combination of backslashes and forward slashes for windows platform builds
// because the paths are normalized by a sed script that will match backslashes
// and output front-slashes.  The sed script could be made smarter.
fn linuxized_path(path: &PathBuf) -> PathBuf {
    if let Some(pathstr) = path.to_str() {
        if pathstr.find('\\').is_some() {
            // Pesky backslashes, get rid of them!
            let converted = pathstr.replace('\\', "/");
            // If we're seeing this, it means the paths weren't normalized and
            // now it's a question of minimizing fallout.
            if converted.find(":/") == Some(1) {
                // Starts with a drive letter, so let's turn this into
                // an absolute path
                let abs = "/".to_string() + converted.as_str();
                return PathBuf::from(abs);
            }
            // Turn it into a relative path
            return PathBuf::from(converted);
        }
    }
    // Already a valid path!
    path.clone()
}

fn analyze_crate(analysis: &data::Analysis, defs: &Defs, tree_info: &TreeInfo) {
    // Create and populate per-file Analysis instances from the provided per-crate Analysis file.
    let mut per_file = HashMap::new();

    let crate_name = &*analysis.prelude.as_ref().unwrap().crate_id.name;
    info!("Analyzing crate: '{}'", crate_name);
    debug!("Crate prelude: {:?}", analysis.prelude);

    macro_rules! flat_map_per_file {
        ($field:ident) => {
            for item in &analysis.$field {
                let file_analysis = per_file
                    .entry(linuxized_path(&item.span.file_name))
                    .or_insert_with(|| {
                        let prelude = analysis.prelude.clone();
                        let mut analysis = data::Analysis::new(analysis.config.clone());
                        analysis.prelude = prelude;
                        analysis
                    });
                file_analysis.$field.push(item.clone());
            }
        };
    }

    flat_map_per_file!(imports);
    flat_map_per_file!(defs);
    flat_map_per_file!(impls);
    flat_map_per_file!(refs);
    flat_map_per_file!(macro_refs);
    flat_map_per_file!(relations);

    for (searchfox_path, analysis) in per_file.drain() {
        // Absolute paths mean that the save-analysis data wasn't normalized
        // into the searchfox path convention, which means we can't generate
        // analysis data, so just skip.
        //
        // This will be the case for libraries built with cargo that have paths
        // that have prefixes that look like "/cargo/registry/src/github.com-".
        if searchfox_path.is_absolute() {
            info!(
                "Skipping absolute analysis path {}",
                searchfox_path.display()
            );
            continue;
        }
        analyze_file(&searchfox_path, defs, &analysis, tree_info);
    }
}

#[derive(Parser)]
struct RustIndexerCli {
    /// Points to the source root (FILES_ROOT)
    #[clap(value_parser)]
    src: PathBuf,

    /// Points to the directory where searchfox metadata should go (ANALYSIS_ROOT)
    #[clap(value_parser)]
    output: PathBuf,

    /// Points to the generated source files root (GENERATED)
    #[clap(value_parser)]
    generated: PathBuf,

    /// Whether the inputs are scip files or analysis directories.
    #[clap(short, long, value_parser)]
    scip: bool,

    /// Common prefix to the scip files. If given e.g., the objdir, we can infer
    /// that a given scip file in objdir/tools/rust.scip refers to tools/ rather
    /// than top-level srcdir locations.
    #[clap(long, value_parser)]
    scip_prefix: Option<PathBuf>,

    /// rustc analysis directories or scip inputs
    #[clap(value_parser)]
    inputs: Vec<PathBuf>,
}

fn analyze_using_rls(tree_info: &TreeInfo, inputs: Vec<PathBuf>) {
    let loader = Loader::new(inputs);

    let crates =
        rls_analysis::read_analysis_from_files(&loader, Default::default(), &[] as &[&str]);

    info!(
        "Crates: {:?}",
        crates.iter().map(|k| &k.id.name).collect::<Vec<_>>()
    );

    // Create and populate Defs, a map from Id to Def, across all crates before beginning analysis.
    // This is necessary because Def and Ref instances name Defs via Id.
    let mut defs = Defs::new();
    for krate in &crates {
        for def in &krate.analysis.defs {
            defs.insert(&krate.analysis, def);
        }
    }

    for krate in crates {
        analyze_crate(&krate.analysis, &defs, &tree_info);
    }
}

// https://docs.rs/scip/latest/scip/types/struct.Occurrence.html#structfield.range
fn scip_range_to_searchfox_location(range: &[i32]) -> Location {
    // Searchfox uses 1-indexed lines, 0-indexed columns.
    let line_start = range[0] as u32 + 1;
    let col_start = range[1] as u32;
    let line_end = if range.len() == 3 {
        line_start
    } else {
        range[2] as u32 + 1
    };
    let col_end = *range.last().unwrap() as u32;
    // Rust spans are multi-line... So we just use the start column as
    // the end column if it spans multiple rows, searchfox has fallback
    // code to handle this.
    let col_end = if line_start != line_end {
        col_start
    } else {
        col_end
    };
    Location {
        lineno: line_start,
        col_start,
        col_end,
    }
}

fn write_line(mut file: &mut File, data: &impl serde::Serialize) {
    use std::io::Write;
    serde_json::to_writer(&mut file, data).unwrap();
    file.write_all(b"\n").unwrap();
}

fn scip_roles_to_searchfox_tags(roles: i32) -> Vec<AnalysisKind> {
    let mut values = vec![];

    macro_rules! map_to_searchfox {
        ($scip:ident, $sfox:ident) => {
            if roles & scip::types::SymbolRole::$scip as i32 != 0 {
                if values.last() != Some(&AnalysisKind::$sfox) {
                    values.push(AnalysisKind::$sfox);
                }
            }
        };
    }

    map_to_searchfox!(Definition, Def);
    map_to_searchfox!(Import, Use);
    map_to_searchfox!(WriteAccess, Use);
    map_to_searchfox!(ReadAccess, Use);
    map_to_searchfox!(Generated, Use);
    map_to_searchfox!(Test, Use);
    map_to_searchfox!(Import, Use);

    values
}

fn analyze_using_scip(tree_info: &TreeInfo, scip_prefix: Option<&PathBuf>, scip_file: PathBuf) {
    use protobuf::Message;
    use scip::types::*;

    let file = File::open(&scip_file).expect("Can't open scip file");
    let byte_count = file.metadata().expect("Failed to get file metadata").len();
    let mut file = BufReader::new(file);
    let mut file = protobuf::CodedInputStream::from_buf_read(&mut file);
    let index = Index::parse_from(&mut file).expect("Failed to read scip index");

    for doc in &index.documents {
        let searchfox_path = Path::new(&doc.relative_path).to_owned();
        let searchfox_path =
            match scip_prefix.and_then(|prefix| scip_file.strip_prefix(prefix).ok()) {
                Some(p) => {
                    let mut p = p.to_owned();
                    p.pop();
                    p.join(&searchfox_path)
                }
                None => searchfox_path,
            };

        let output_file = tree_info.out_analysis_dir.join(&searchfox_path);
        if let Err(err) = create_output_dir(&output_file) {
            error!(
                "Couldn't create dir for: {}, {:?}",
                output_file.display(),
                err
            );
            continue;
        }
        let mut file = match File::create(&output_file) {
            Ok(f) => f,
            Err(err) => {
                error!(
                    "Couldn't open output file: {}, {:?}",
                    output_file.display(),
                    err
                );
                continue;
            }
        };
        // Be chatty about the files we're outputting so that it's easier to follow
        // the path of rust analysis generation.
        info!(
            "Writing analysis for '{}' to '{}'",
            searchfox_path.display(),
            output_file.display()
        );

        // A map from local symbol to the index in doc.symbols for this document.
        let mut doc_symbols_to_index = HashMap::new();
        for (index, symbol) in doc.symbols.iter().enumerate() {
            doc_symbols_to_index.insert(symbol.symbol.clone(), index);
        }

        let lookup_symbol = |s: &str| -> Cow<SymbolInformation> {
            match doc_symbols_to_index.get(s) {
                Some(i) => Cow::Borrowed(&doc.symbols[*i]),
                None => {
                    debug!("Didn't find symbol {:?} in local symbol table", s);
                    // Fake it till you make it? We have no info for this
                    // symbol, so...
                    Cow::Owned(SymbolInformation {
                        symbol: s.to_owned(),
                        ..Default::default()
                    })
                }
            }
        };

        for occurrence in &doc.occurrences {
            let loc = scip_range_to_searchfox_location(&occurrence.range);
            let symbol = lookup_symbol(&occurrence.symbol);
            if scip::symbol::is_local_symbol(&symbol.symbol) {
                continue;
            }
            {
                let global = sanitize_symbol(&symbol.symbol);
                let pretty = pretty_symbol(&symbol.symbol);
                let source_data = WithLocation {
                    data: AnalysisSource {
                        source: SourceTag::Source,
                        // TODO: Fill syntax.
                        syntax: vec![],
                        pretty: ustr(&pretty),
                        sym: vec![ustr(&global)],
                        no_crossref: false,
                        // TODO(bug 1796870): Nesting.
                        nesting_range: SourceRange::default(),
                        // TODO: Expose type information for fields/etc.
                        type_pretty: None,
                        type_sym: None,
                        arg_ranges: vec![],
                    },
                    loc,
                };
                write_line(&mut file, &source_data);
            }

            // TODO: Contextual info.
            let context = None;

            let get_target_data = |sym: &str, kind: AnalysisKind| -> WithLocation<AnalysisTarget> {
                let global = sanitize_symbol(sym);
                let pretty = pretty_symbol(sym);
                WithLocation {
                    data: AnalysisTarget {
                        target: TargetTag::Target,
                        kind,
                        pretty: ustr(&pretty),
                        sym: ustr(&global),
                        context: ustr(context.unwrap_or("")),
                        contextsym: ustr(context.unwrap_or("")),
                        peek_range: LineRange {
                            start_lineno: 0,
                            end_lineno: 0,
                        },
                        arg_ranges: vec![],
                    },
                    loc: loc.clone(),
                }
            };

            write_line(
                &mut file,
                &get_target_data(&symbol.symbol, AnalysisKind::Use),
            );

            for kind in scip_roles_to_searchfox_tags(occurrence.symbol_roles) {
                write_line(&mut file, &get_target_data(&symbol.symbol, kind));
            }

            for relationship in &symbol.relationships {
                let kind = if relationship.is_type_definition || relationship.is_implementation {
                    AnalysisKind::Def
                } else {
                    AnalysisKind::Use
                };
                write_line(&mut file, &get_target_data(&relationship.symbol, kind));
            }
        }

        println!("{}", doc.relative_path);
    }

    assert_eq!(file.pos(), byte_count, "Should've processed the whole file");
}

fn main() {
    env_logger::init();

    let cli = RustIndexerCli::parse();

    let tree_info = TreeInfo {
        srcdir: &cli.src,
        out_analysis_dir: &cli.output,
        generated: &cli.generated,
        generated_friendly: &PathBuf::from("__GENERATED__"),
    };

    info!("Tree info: {:?}", tree_info);

    if cli.scip {
        for file in cli.inputs {
            analyze_using_scip(&tree_info, cli.scip_prefix.as_ref(), file);
        }
    } else {
        analyze_using_rls(&tree_info, cli.inputs)
    }
}
