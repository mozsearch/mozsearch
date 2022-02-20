use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;

extern crate env_logger;

use serde::Serialize;
use serde_json::{json, Map};
extern crate tools;
use tools::config;
use tools::file_format::analysis::LineRange;
use tools::file_format::analysis::{read_analysis, read_structured, read_target, AnalysisKind};
use tools::find_source_file;
use ustr::{ustr, Ustr};

/// The size for a payload line (inclusive of leading indicating character and
/// newline) at which we store it externally in `crossref-extra` instead of
/// inline in the `crossref` file itself.
const EXTERNAL_STORAGE_THRESHOLD: usize = 1024 * 3;

#[derive(Clone, Debug, Serialize)]
struct SearchResult {
    #[serde(rename = "lno")]
    lineno: u32,
    bounds: (u32, u32),
    line: Ustr,
    context: Ustr,
    contextsym: Ustr,
    // We use to build up "peekLines" which we excerpted from the file here, but
    // this was never surfaced to users.  The plan at the time had been to try
    // and store specific file offsets that could be directly mapped/seeked, but
    // between effective caching of dynamic search results and good experiences
    // with lol_html, it seems like we will soon be able to just excerpt the
    // statically produced HTML efficiently enough through dynamic HTML
    // filtering.
    #[serde(
        rename = "peekRange",
        default,
        skip_serializing_if = "LineRange::is_empty"
    )]
    peek_range: LineRange,
}

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

/// Process all analysis files, deriving the `crossref`, `jumps`, and `identifiers` output files.
/// See https://github.com/mozsearch/mozsearch/blob/master/docs/crossref.md for high-level
/// documentation on how this works (locally, `docs/crossref.md`).
///
/// ## Implementation
/// There are 2 phases of processing:
/// 1. The analysis files are read, populating `table`, `pretty_table`, `id_table`, and
///    `meta_table` incrementally.  Primary cross-reference information comes from target records,
///    but the file is also processed for source records in order to populate `meta_table` with
///    meta-information about the symbol.
/// 2. The table is consumed with jumps generated as a byproduct.
///
/// ### Memory Management
/// Memory usage grows continually throughout phase 1.  Because we load many identical strings,
/// we use string interning so that all long-lived strings are reference-counted interned strings.
fn main() {
    env_logger::init();
    let args: Vec<_> = env::args().collect();

    let cfg = config::load(&args[1], false);

    let tree_name = &args[2];
    let tree_config = cfg.trees.get(tree_name).unwrap();

    let filenames_file = &args[3];

    let file_paths: Vec<String> = BufReader::new(File::open(filenames_file).unwrap())
        .lines()
        .map(|x| x.unwrap())
        .collect();
    let xref_file = format!("{}/crossref", tree_config.paths.index_path);
    let xref_ext_file = format!("{}/crossref-extra", tree_config.paths.index_path);
    let jump_file = format!("{}/jumps", tree_config.paths.index_path);
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
    // Not populated until phase 2 when we walk the above data-structures.
    let mut jumps = Vec::new();

    // As we process the source entries and build the SourceMeta, we keep a running list of what
    // cross-SourceMeta links need to be established.  We then process this after all of the files
    // have been processed and we know all symbols are known.

    // Pairs of [parent class sym, subclass sym] to add subclass to parent.
    let mut xref_link_subclass = Vec::new();
    // Pairs of [parent method sym, overridden by sym] to add the override to the parent.
    let mut xref_link_override = Vec::new();

    // Triples of [ipc sym, src src, target sym].
    let mut xref_link_ipc = Vec::new();

    for path in &file_paths {
        print!("File {}\n", path);

        let analysis_fname = format!("{}/analysis/{}", tree_config.paths.index_path, path);
        let analysis = read_analysis(&analysis_fname, &mut read_target);

        // Load the source file and chop it up into `lines` so that we extract
        // the `line` for each result.  In the future this could move to
        // dynamic extraction that uses the `peek_range` if available and this
        // line if it's not.
        let source_fname = find_source_file(
            path,
            &tree_config.paths.files_path,
            &tree_config.paths.objdir_path,
        );
        let source_file = match File::open(source_fname) {
            Ok(f) => f,
            Err(_) => {
                println!("Unable to open source file");
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
                let p: &str = &path;
                let t3 = t2.entry(p).or_insert(Vec::new());
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
                    line: ustr(&line),
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
                    // XXX these now either need to come from the dynamic
                    // "extra" or the "supers"/"overrides" should be explicitly
                    // mapped.
                    if !piece.supers.is_empty() {
                        for super_info in &piece.supers {
                            xref_link_subclass.push((super_info.sym, piece.sym));
                        }
                    }

                    if !piece.overrides.is_empty() {
                        for override_info in &piece.overrides {
                            xref_link_override.push((override_info.sym, piece.sym));
                        }
                    }

                    if let ("ipc", Some(src_sym), Some(target_sym)) =
                        (piece.kind.as_str(), piece.src_sym, piece.target_sym)
                    {
                        xref_link_ipc.push((piece.sym, src_sym, target_sym));
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

    for (ipc_sym, src_sym, target_sym) in xref_link_ipc {
        if let Some(src_meta) = meta_table.get_mut(&src_sym) {
            src_meta.idl_sym = Some(ipc_sym);
            src_meta.target_sym = Some(target_sym);
        }

        if let Some(target_meta) = meta_table.get_mut(&target_sym) {
            target_meta.idl_sym = Some(ipc_sym);
            target_meta.src_sym = Some(src_sym);
        }
    }

    // ## Write out the crossref database.
    let mut xref_out = File::create(xref_file).unwrap();
    let mut xref_ext_out = File::create(xref_ext_file).unwrap();
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

    for (id, id_data) in table {
        let mut kindmap = Map::new();
        for (kind, kind_data) in &id_data {
            let mut result = Vec::new();
            for (path, results) in kind_data {
                result.push(json!({
                    "path": path,
                    "lines": results,
                }));
            }
            let kindstr = match *kind {
                AnalysisKind::Use => "uses",
                AnalysisKind::Def => "defs",
                AnalysisKind::Assign => "assignments",
                AnalysisKind::Decl => "decls",
                AnalysisKind::Forward => "forwards",
                AnalysisKind::Idl => "idl",
                AnalysisKind::IPC => "ipc",
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
        if let Some(meta) = meta_table.get(&id) {
            kindmap.insert("meta".to_string(), json!(meta));
        }

        let kindmap = json!(kindmap);
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

        if id_data.contains_key(&AnalysisKind::Def) {
            let defs = id_data.get(&AnalysisKind::Def).unwrap();
            if defs.len() == 1 {
                for (path, results) in defs {
                    if results.len() == 1 {
                        let mut v = Vec::new();
                        v.push(json!(id));
                        v.push(json!(path));
                        v.push(json!(results[0].lineno));
                        let pretty = pretty_table.get(&id).unwrap();
                        v.push(json!(pretty));
                        jumps.push(json!(v));
                    }
                }
            }
        }
    }

    let mut jumpf = File::create(jump_file).unwrap();
    for jump in jumps {
        let _ = jumpf.write_all((jump.to_string() + "\n").as_bytes());
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
}
