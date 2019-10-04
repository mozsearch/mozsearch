#[macro_use]
extern crate clap;
extern crate env_logger;
#[macro_use]
extern crate log;
extern crate rls_analysis;
extern crate rls_data as data;
extern crate tools;

use crate::data::GlobalCrateId;
use crate::data::{DefKind, ImplKind};
use rls_analysis::{AnalysisHost, AnalysisLoader, SearchDirectory};
use std::collections::{BTreeSet, HashMap};
use std::fs::{self, File};
use std::io;
use std::io::{BufRead, BufReader, Read, Seek};
use std::path::{Path, PathBuf};
use tools::file_format::analysis::{
    AnalysisKind, AnalysisSource, AnalysisTarget, LineRange, Location, SourceRange, WithLocation,
};

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

#[derive(Debug)]
struct TreeInfo<'a> {
    src_dir: &'a Path,
    output_dir: &'a Path,
    objdir: &'a Path,
    libstd: &'a Path,
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
    // these characters, so replace them with underscores
    sym.replace(",", "_").replace(" ", "_").replace("\n", "_")
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
            kind,
            pretty: sanitized.clone(),
            sym: sanitized.clone(),
            context: String::from(context.unwrap_or("")),
            contextsym: String::from(context.unwrap_or("")),
            peek_range: LineRange {
                start_lineno: 0,
                end_lineno: 0,
            },
        },
        loc: loc.clone(),
    };
    out_data.insert(format!("{}", target_data));

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
            syntax: vec![],
            pretty: pretty.to_string(),
            sym: vec![sanitized],
            no_crossref: false,
            nesting_range,
        },
        loc,
    };
    out_data.insert(format!("{}", source_data));
}

/// Normalizes an absolute file path to be either a src_dir-relative path or an obj_dir relative
/// path with __GENERATED__ prefixed onto it.
fn find_generated_or_src_file(file_name: &Path, tree_info: &TreeInfo) -> Option<PathBuf> {
    if let Ok(generated_path) = file_name.strip_prefix(tree_info.objdir) {
        return Some(Path::new("__GENERATED__").join(generated_path));
    }
    if let Ok(std_path) = file_name.strip_prefix(tree_info.libstd) {
        return Some(Path::new("__GENERATED__").join("__RUST__").join(std_path));
    }
    file_name
        .strip_prefix(tree_info.src_dir)
        .ok()
        .map(From::from)
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
        Err(e) => {
            error!("ERROR: Unable to read file: {:?}", e);
            None
        }
    }
}

fn analyze_file(
    file_name: &PathBuf,
    defs: &Defs,
    file_analysis: &data::Analysis,
    tree_info: &TreeInfo,
) {
    use std::io::Write;

    debug!("Running analyze_file for {}", file_name.display());

    let file = match find_generated_or_src_file(file_name, tree_info) {
        Some(f) => f,
        None => {
            error!(
                "File not in the source directory or objdir: {}",
                file_name.display()
            );
            return;
        }
    };

    // Attempt to open the source file to extract information not currently available from the
    // analysis data.  Some analysis information may not be emitted if we are unable to access the
    let maybe_source_file = match File::open(&file_name) {
        Ok(f) => Some(f),
        Err(_) => None,
    };

    let output_file = tree_info.output_dir.join(file);
    let mut dataset = BTreeSet::new();
    read_existing_contents(&mut dataset, &output_file);
    let mut output_dir = output_file.clone();
    output_dir.pop();
    if let Err(err) = fs::create_dir_all(output_dir) {
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

/// Uses heuristics to determine if the path is a Windows path,
/// and if so, returns a Linux "equivalent". Not really equivalent,
/// because there is no such thing, but equivalent enough for our
/// purposes - that is, is_relative() returns a correct thing, and
/// strip_prefix will work on relative paths.
/// Note that this is a "linuxzed" path and not a "unixed" path
/// this code should ever only really run inside the linux vagrant
/// machine.
/// There are real-life cases in which this code is too simple, see
/// https://github.com/nrc/rls-data/issues/20 for an example.
fn linuxized_path(path: &PathBuf) -> PathBuf {
    // Windows paths have backslashes and so on a Linux host will
    // just have a single long component
    if path.components().count() == 1 {
        if let Some(pathstr) = path.to_str() {
            if pathstr.find('\\').is_some() {
                // Has a backslash, so probably a Windows path. Let's lean
                // those slashes forward
                let converted = pathstr.replace('\\', "/");
                if converted.find(":/") == Some(1) {
                    // Starts with a drive letter, so let's turn this into
                    // an absolute path
                    let abs = "/".to_string() + &converted;
                    return PathBuf::from(abs);
                }
                // Turn it into a relative path
                return PathBuf::from(converted);
            }
        }
    }
    // Probably a Linux path, or a Windows path that's some sort of
    // weird edge case. In that case just return it as-is
    path.clone()
}

fn analyze_crate(analysis: &data::Analysis, defs: &Defs, tree_info: &TreeInfo) {
    // Create and populate per-file Analysis instances from the provided per-crate Analysis file.
    let mut per_file = HashMap::new();

    info!("Analyzing crate: {:?}", analysis.prelude);

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

    let crate_name = &*analysis.prelude.as_ref().unwrap().crate_id.name;

    // TODO(emilio): This is good enough, for now, but I guess we may want
    // something better...
    let is_std = match crate_name {
        "std" | "alloc" | "jemalloc" | "dlmalloc" | "compiler_builtins" | "unwind" | "libc"
        | "panic_abort" | "panic_unwind" | "core" | "rustc" | "backtrace" => true,
        name => name.starts_with("rustc_") || name.starts_with("alloc_"),
    };

    for (mut name, analysis) in per_file.drain() {
        if name.is_relative() {
            if is_std {
                name = tree_info.libstd.join(name)
            } else {
                name = tree_info.src_dir.join(name);
            }
        }
        analyze_file(&name, defs, &analysis, tree_info);
    }
}

fn main() {
    use clap::Arg;
    env_logger::init();
    let matches = app_from_crate!()
        .args_from_usage(
            "<src>      'Points to the source root'
             <output>   'Points to the directory where searchfox metadata should go'
             <objdir>   'Points to the objdir generated files may come from'
             <libstd>   'Points to the directory with the rust source'",
        )
        .arg(
            Arg::with_name("input")
                .required(false)
                .multiple(true)
                .help("rustc analysis directories"),
        )
        .get_matches();

    let src_dir = Path::new(matches.value_of("src").unwrap());
    let output_dir = Path::new(matches.value_of("output").unwrap());
    let objdir = Path::new(matches.value_of("objdir").unwrap());
    let libstd = Path::new(matches.value_of("libstd").unwrap());

    let tree_info = TreeInfo {
        src_dir,
        output_dir,
        objdir,
        libstd,
    };

    info!("Tree info: {:?}", tree_info);

    let input_dirs = match matches.values_of("input") {
        Some(inputs) => inputs.map(PathBuf::from).collect(),
        None => vec![],
    };
    let loader = Loader::new(input_dirs);

    let crates = rls_analysis::read_analysis_from_files(&loader, Default::default(), &[]);

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
