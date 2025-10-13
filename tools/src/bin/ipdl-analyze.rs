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
use serde_json::json;

use ipdl_parser::ast::ProtocolSide;
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
    opts.reqopt(
        "o",
        "objdir-list",
        "List of generated/objdir files, probably `objdir-files`.",
        "FILES_LIST",
    );
    opts
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
            .map(|id| mangle_simple(id))
            .collect::<Vec<_>>()
            .join(""),
        mangle_simple(protocol),
        mangle_simple(name)
    )
}

fn find_analysis<'a>(analysis: &'a TargetAnalysis, mangled: &str) -> Option<&'a AnalysisTarget> {
    // As a hack to deal with SendPFooConstructor having a single-arg variant
    // that takes an args struct which just news the actor and then calls the
    // 2-arg variant, passing the actor as the first variant, and because the
    // 1-arg variant comes before the 2-arg variant, we return the last variant
    // we see.  Will this cause even more problems?  Maybe!
    let mut best_piece = None;
    for datum in analysis {
        for piece in &datum.data {
            // Inline method definitions and pure virtual method declarations
            // will both be reported as definitions by the C++ indexer without a
            // declaration, so we need to accept both decls and defs.
            if (piece.kind == AnalysisKind::Decl || piece.kind == AnalysisKind::Def)
                && piece.sym.contains(mangled)
            {
                best_piece = Some(piece);
            }
        }
    }

    best_piece
}

fn output_ipc_data(
    outputf: &mut File,
    locstr: &str,
    ipc_pretty: &str,
    ipc_sym: &str,
    send_datum: &AnalysisTarget,
    recv_datum: &AnalysisTarget,
) {
    write!(
        outputf,
        "{}",
        json!({
            "loc": locstr,
            "target": 1,
            "kind": "idl",
            "pretty": ipc_pretty,
            "sym": ipc_sym,
        })
    )
    .unwrap();
    writeln!(outputf).unwrap();
    write!(
        outputf,
        "{}",
        json!({
            "loc": locstr,
            "source": 1,
            "syntax": "idl,ipc,def",
            "pretty": format!("ipc {}", ipc_pretty),
            "sym": ipc_sym,
        })
    )
    .unwrap();
    writeln!(outputf).unwrap();
    write!(
        outputf,
        "{}",
        json!({
            "loc": locstr,
            "structured": 1,
            "pretty": ipc_pretty,
            "sym": ipc_sym,
            // Note that this is different than the target record kind.
            "kind": "ipc",
            "implKind": "idl",
            "bindingSlots": [
                {
                    "slotKind": "send",
                    "slotLang": "cpp",
                    "ownerLang": "idl",
                    "sym": send_datum.sym,
                },
                {
                    "slotKind": "recv",
                    "slotLang": "cpp",
                    "ownerLang": "idl",
                    "sym": recv_datum.sym,
                },
            ]
        })
    )
    .unwrap();
    writeln!(outputf).unwrap();
}

#[allow(clippy::too_many_arguments)]
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
    if maybe_send_datum.is_none() {
        println!("No analysis target found for send: {}", mangled);
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
    if maybe_recv_datum.is_none() {
        println!(
            "No analysis target found for recv: {} or {}",
            mangled_no_p, mangled_yes_p
        );
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
        output_ipc_data(
            outputf,
            locstr,
            &ipc_pretty,
            &ipc_sym,
            send_datum,
            recv_datum,
        );
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

    let base_dir = matches.opt_str("b").unwrap();
    let analysis_dir = matches.opt_str("a").unwrap();
    let repo_file_list_fname = matches.opt_str("f").unwrap();
    let objdir_file_list_fname = matches.opt_str("o").unwrap();
    let base_path = Path::new(&base_dir);
    let analysis_path = Path::new(&analysis_dir);

    let mut file_names = Vec::new();
    for f in matches.free {
        file_names.push(PathBuf::from(f));
    }

    let repo_files_map = load_file_list(&repo_file_list_fname);
    let objdir_files_map = load_file_list(&objdir_file_list_fname);

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
            // https://searchfox.org/firefox-main/source/ipc/ipdl/ipdl/direct_call.py in the
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
            // truly generated files that stick around (and get labeled as generated).
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
            //
            // ### Platform variations resulting from preprocessed IPDL files
            //
            // Some IPDL files like PContent.ipdl are preprocessed which both means the IPDL parser
            // has to deal with the existence of directives, but also that we need to deal with the
            // merge logic being unable to unify the files.  Previously we use predicted path
            // locations to find `PFoo{Parent,Child}.h`, but now we perform a lookup from the
            // `objdir-files` list identically to how we use `repo-files` list for
            // "Foo{Parent,Child}.h".

            // ### Parent Analyses
            let mut parent_ana_files = vec![];
            if let Some(parent_fname) = objdir_files_map.get(&format!("{}Parent.h", &ns.name.id)) {
                let parent_path = analysis_path
                    .join(parent_fname)
                    .to_string_lossy()
                    .into_owned();
                println!("  Reading Parent header {:?}", &parent_path);
                parent_ana_files.push(parent_path);
            } else {
                println!(
                    "  Unable to find Parent header for protocol: {}",
                    &ns.name.id
                );
            }
            if let Some(parent_impl_fname) =
                repo_files_map.get(&format!("{}Parent.h", &ns.name.id[1..]))
            {
                let parent_impl_path = analysis_path
                    .join(parent_impl_fname)
                    .to_string_lossy()
                    .into_owned();
                println!("  Reading Parent impl header {:?}", &parent_impl_path);
                parent_ana_files.push(parent_impl_path);
            } else {
                println!(
                    "  Unable to find Parent impl header for protocol: {}",
                    &ns.name.id
                );
            }

            let parent_analysis = read_analyses(parent_ana_files.as_slice(), &mut read_target);

            // ### Child Analyses
            let mut child_ana_files = vec![];
            if let Some(child_fname) = objdir_files_map.get(&format!("{}Child.h", &ns.name.id)) {
                let child_path = analysis_path
                    .join(child_fname)
                    .to_string_lossy()
                    .into_owned();
                println!("  Reading Child header {:?}", &child_path);
                child_ana_files.push(child_path);
            } else {
                println!(
                    "  Unable to find Child header for protocol: {}",
                    &ns.name.id
                );
            }
            if let Some(child_impl_fname) =
                repo_files_map.get(&format!("{}Child.h", &ns.name.id[1..]))
            {
                let child_impl_path = analysis_path
                    .join(child_impl_fname)
                    .to_string_lossy()
                    .into_owned();
                println!("  Reading Child impl header {:?}", &child_impl_path);
                child_ana_files.push(child_impl_path);
            } else {
                println!(
                    "  Unable to find Child impl header for protocol: {}",
                    &ns.name.id
                );
            }
            let child_analysis = read_analyses(child_ana_files.as_slice(), &mut read_target);

            let is_toplevel = protocol.managers.is_empty();

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

                if message.direction == ast::Direction::To(ProtocolSide::Child)
                    || message.direction == ast::Direction::Both
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

                if message.direction == ast::Direction::To(ProtocolSide::Parent)
                    || message.direction == ast::Direction::Both
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
