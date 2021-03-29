use std::collections::BTreeMap;
use std::env;
use std::fs::File;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

extern crate env_logger;
extern crate getopts;
extern crate ipdl_parser;
extern crate tools;

use getopts::Options;

use tools::file_format::analysis::{
    read_analyses, read_target, AnalysisKind, AnalysisTarget, WithLocation,
};

use ipdl_parser::ast;
use ipdl_parser::parser;

type TargetAnalysis = Vec<WithLocation<Vec<AnalysisTarget>>>;

fn get_options_parser() -> Options {
    let mut opts = Options::new();
    opts.optmulti(
        "I",
        "include",
        "Additional directory to search for included protocol specifications",
        "DIR",
    );
    opts.reqopt(
        "d",
        "outheaders-dir",
        "Directory into which C++ headers analysis data is location.",
        "HDR_DIR",
    );
    opts.reqopt(
        "b",
        "base-input-prefix",
        "Base directory where IPDL input files are found.",
        "BASE_DIR",
    );
    opts.reqopt(
        "a",
        "analysis-prefix",
        "Base directory where analysis output files are found.",
        "ANALYSIS_DIR",
    );
    opts.reqopt(
        "f",
        "files-list",
        "List of source files, probably `repo-files`.",
        "FILES_LIST",
    );
    opts
}

/**
 * Derive where the auto-generated PFooParent.h/PFooChild.h files will show up.
 */
fn header_file_name(outheaders_dir: &str, ns: &ast::Namespace, parent_or_child: &str) -> String {
    format!(
        "{}/{}/{}{}.h",
        outheaders_dir,
        ns.namespaces.clone().join("/"),
        ns.name.id,
        parent_or_child
    )
}

fn load_file_list(filenames_file: &str) -> BTreeMap<String, String> {
    BufReader::new(File::open(filenames_file).unwrap())
        .lines()
        // In theory we could use Path for this but I am too sleepy to deal with the resulting type
        // nightmare of file_name()'s return value. Also, I lost my rust book so I don't know rust.
        .map(|maybe_name| {
            let name = maybe_name.unwrap();
            (name.rsplit("/").next().unwrap().into(), name)
        })
        .collect()
}

fn mangle_simple(s: &str) -> String {
    format!("{}{}", s.len(), s)
}

fn mangle_nested_name(ns: &[String], protocol: &str, name: &str) -> String {
    format!(
        "_ZN{}{}{}E",
        ns.iter()
            .map(|id| mangle_simple(&id))
            .collect::<Vec<_>>()
            .join(""),
        mangle_simple(protocol),
        mangle_simple(name)
    )
}

fn find_analysis<'a>(analysis: &'a TargetAnalysis, mangled: &str) -> Option<&'a AnalysisTarget> {
    for datum in analysis {
        for piece in &datum.data {
            if piece.kind == AnalysisKind::Decl && piece.sym.contains(mangled) {
                return Some(&piece);
            }
        }
    }

    println!("  No analysis target found for {}", mangled);
    return None;
}

fn output_binding_target_data(outputf: &mut File, locstr: &str, datum: &AnalysisTarget) {
    write!(
        outputf,
        r#"{{"loc": "{}", "target": 1, "kind": "idl", "pretty": "{}", "sym": "{}"}}"#,
        locstr, datum.pretty, datum.sym
    )
    .unwrap();
    write!(outputf, "\n").unwrap();
}

fn output_ipc_data(outputf: &mut File, locstr: &str, ipc_pretty: &str, ipc_sym: &str, send_datum: &AnalysisTarget, recv_datum: &AnalysisTarget) {
    // It might make sense to change the kind to "ipc", but if so, we probably want to change the
    // binding target records as well.
    write!(
        outputf,
        r#"{{"loc": "{}", "target": 1, "kind": "idl", "pretty": "{}", "sym": "{}"}}"#,
        locstr, ipc_pretty, ipc_sym
    )
    .unwrap();
    write!(outputf, "\n").unwrap();
    write!(
        outputf,
        r#"{{"loc": "{}", "source": 1, "syntax": "idl,ipc,def", "pretty": "ipc {}", "sym": "{}"}}"#,
        locstr, ipc_pretty, ipc_sym
    )
    .unwrap();
    write!(outputf, "\n").unwrap();
    write!(
        outputf,
        r#"{{"loc": "{}", "structured": 1, "pretty": "{}", "sym": "{}", "kind": "ipc", "implKind": "idl", "srcsym": "{}", "targetsym": "{}"}}"#,
        locstr, ipc_pretty, ipc_sym, send_datum.sym, recv_datum.sym
    )
    .unwrap();
    write!(outputf, "\n").unwrap();
}

fn output_send_recv(
    outputf: &mut File,
    locstr: &str,
    protocol: &ast::Namespace,
    message: &ast::MessageDecl,
    is_ctor: bool,
    send_side: &str,
    send_analysis: &TargetAnalysis,
    recv_side: &str,
    recv_analysis: &TargetAnalysis,
) {
    let send_prefix = if message.send_semantics == ast::SendSemantics::Intr {
        "Call"
    } else {
        "Send"
    };
    let recv_prefix = if message.send_semantics == ast::SendSemantics::Intr {
        "Answer"
    } else {
        "Recv"
    };

    let ctor_suffix = if is_ctor { "Constructor" } else { "" };

    let mangled = mangle_nested_name(
        &protocol.namespaces,
        &format!("{}{}", protocol.name.id, send_side),
        &format!("{}{}{}", send_prefix, message.name.id, ctor_suffix),
    );
    let maybe_send_datum = find_analysis(send_analysis, &mangled);
    if let Some(send_datum) = maybe_send_datum {
        output_binding_target_data(outputf, &locstr, &send_datum);
    }

    // Depending on whether the protocol is a legacy virtual implementation or direct-call, the
    // "P" prefix of the protocol may need to be sliced off to find the symbol.  See the block
    // comment in `main()` for more info.
    //
    // Our hacky heuristic here is we try without the P and failover to trying with the P.
    let mangled_no_p = mangle_nested_name(
        &protocol.namespaces,
        &format!("{}{}", /* sliced */ &protocol.name.id[1..], recv_side),
        &format!("{}{}{}", recv_prefix, message.name.id, ctor_suffix),
    );
    let mangled_yes_p = mangle_nested_name(
        &protocol.namespaces,
        &format!("{}{}", /* not sliced */ protocol.name.id, recv_side),
        &format!("{}{}{}", recv_prefix, message.name.id, ctor_suffix),
    );
    let maybe_recv_datum = find_analysis(recv_analysis, &mangled_no_p)
                           .or_else(|| find_analysis(recv_analysis, &mangled_yes_p));
    if let Some(recv_datum) = maybe_recv_datum {
        output_binding_target_data(outputf, &locstr, &recv_datum);
    }

    if let (Some(send_datum), Some(recv_datum)) = (maybe_send_datum, maybe_recv_datum) {
        let ipc_pretty = format!(
            "{}::{}::{}",
            protocol.namespaces.join("_"),
            protocol.name.id,
            message.name.id
        );
        let ipc_sym = format!(
            "IPC_{}_{}_{}",
            protocol.namespaces.join("_"),
            protocol.name.id,
            message.name.id
        );
        output_ipc_data(outputf, &locstr, &ipc_pretty, &ipc_sym, &send_datum, &recv_datum);
    }
}

fn main() {
    env_logger::init();

    let args: Vec<String> = env::args().collect();

    let opts = get_options_parser();

    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(f) => panic!("{}", f.to_string()),
    };

    let mut include_dirs = Vec::new();
    for i in matches.opt_strs("I") {
        include_dirs.push(PathBuf::from(i))
    }

    let outheaders_dir = matches.opt_str("d").unwrap();
    let base_dir = matches.opt_str("b").unwrap();
    let analysis_dir = matches.opt_str("a").unwrap();
    let file_list_fname = matches.opt_str("f").unwrap();
    let base_path = Path::new(&base_dir);
    let analysis_path = Path::new(&analysis_dir);

    let mut file_names = Vec::new();
    for f in matches.free {
        file_names.push(PathBuf::from(f));
    }

    let repo_files_map = load_file_list(&file_list_fname);

    let maybe_tus = parser::parse(&include_dirs, file_names);

    if maybe_tus.is_none() {
        println!("Specification could not be parsed.");
        return;
    }

    let tus = maybe_tus.unwrap();

    for (_, tu) in tus {
        println!("Analyzing {:?}", tu.file_name);

        let path = tu.file_name.as_path();
        let relative = path.strip_prefix(base_path).unwrap();
        let absolute = analysis_path.join(relative);
        let mut outputf = File::create(absolute).unwrap();

        if let Some((ns, protocol)) = tu.protocol {
            // ## Analyses Linkage
            //
            // Originally, IPDL analysis worked by being able to know exactly where the protocol
            // autogenerated headers would end up.  These would contain (virtual) declarations for
            // all the Send and Recv methods.  Because of how searchfox indexes overridden methods,
            // this allowed locating the concrete recv methods.
            //
            // Bug 1512990 (ipc-devirt) changed this so that the recv methods would only exist on
            // the concrete implementation class and `thisCall` in `lower.py` would know to
            // static_cast to that class.  This broke finding the "recv" methods for any
            // implementations not grandfathered into the legacy virtual call mechanism.  The
            // legacy methods can be found in
            // https://searchfox.org/mozilla-central/source/ipc/ipdl/ipdl/direct_call.py in the
            // `VIRTUAL_CALL_CLASSES` set.  There's also a `DIRECT_CALL_OVERRIDES` section that
            // explains to the binding generator how to find the subclass's header file for
            // inclusion for build purposes.  Those entries only exist when the header file isn't
            // predictably publicly exposed via `EXPORTS.mozilla.dom`/similar in `moz.build` files
            // based on the IPDL namespace and protocol name.
            //
            // This IPDL analysis logic doesn't currently understand `direct_call.py`, but that
            // also doesn't quite matter because our analysis operates in terms of the source
            // locations of files, not where they get installed to.  Admittedly here, I (asuth),
            // am somewhat confused as to whether we have a reverse mapping somewhere or things
            // just work out.  In our objdirs, the install process generates symlinks back to the
            // original source location which makes the indexer's life easy.  And the symlinks
            // at least disappear by the time the webserver gets the merged objdir.  It's only
            // truly generated files that stick aroudn (and get labeled as generated).
            //
            // In any event, the approach I'm taking here is to load up the contents of the
            // `repo-files` list and create a map from the filename to the (source) path.  If
            // we find such a mapping, we add it to the list of analysis files to read.
            //
            // The right solution is likely what's been proposed for WebIDL in bug 1416899 wherein
            // the IPDL compiler should just be creating JSON build artifacts for searchfox when
            // requested and it can perform symlink resolution as part of its process so we always
            // have source paths when possible.
            //
            // Another option for symbol resolution just to depend on the `crossref` process.
            // It knows about all symbols, so the main issue is that it also really wants to know
            // the byproducts of this analysis.  One possibility is to support a type of record
            // that is linked/fixed-up by the crossref process.

            // Parent Analyses
            let parent_fname = header_file_name(&outheaders_dir, &ns, "Parent");
            println!("  Reading Parent header {:?}", parent_fname);
            let mut parent_ana_files = vec![parent_fname];
            if let Some(parent_impl_fname) = repo_files_map.get(&format!("{}Parent.h", &ns.name.id[1..])) {
                let parent_impl_path = analysis_path.join(parent_impl_fname).to_string_lossy().into_owned();
                println!("  Reading Parent impl header {:?}", &parent_impl_path);
                parent_ana_files.push(parent_impl_path);
            }

            let parent_analysis = read_analyses(parent_ana_files.as_slice(), &mut read_target);

            // Child Analyses
            let child_fname = header_file_name(&outheaders_dir, &ns, "Child");
            println!("  Reading Child header {:?}", child_fname);
            let mut child_ana_files = vec![child_fname];
            if let Some(child_impl_fname) = repo_files_map.get(&format!("{}Child.h", &ns.name.id[1..])) {
                let child_impl_path = analysis_path.join(child_impl_fname).to_string_lossy().into_owned();
                println!("  Reading Child impl header {:?}", &child_impl_path);
                child_ana_files.push(child_impl_path);
            }
            let child_analysis = read_analyses(child_ana_files.as_slice(), &mut read_target);

            let is_toplevel = protocol.managers.len() == 0;

            for message in protocol.messages {
                let loc = &message.name.loc;
                let locstr = format!(
                    "{}:{}-{}",
                    loc.lineno,
                    loc.colno,
                    loc.colno + message.name.id.len()
                );

                if is_toplevel && message.name.id == "__delete__" {
                    continue;
                }

                let is_ctor = protocol.manages.iter().any(|e| e.id == message.name.id);

                if message.direction == ast::Direction::ToChild
                    || message.direction == ast::Direction::ToParentOrChild
                {
                    output_send_recv(
                        &mut outputf,
                        &locstr,
                        &ns,
                        &message,
                        is_ctor,
                        "Parent",
                        &parent_analysis,
                        "Child",
                        &child_analysis,
                    );
                }

                if message.direction == ast::Direction::ToParent
                    || message.direction == ast::Direction::ToParentOrChild
                {
                    output_send_recv(
                        &mut outputf,
                        &locstr,
                        &ns,
                        &message,
                        is_ctor,
                        "Child",
                        &child_analysis,
                        "Parent",
                        &parent_analysis,
                    );
                }
            }
        }
    }
}
