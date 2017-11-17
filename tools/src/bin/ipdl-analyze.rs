use std::env;
use std::path::Path;
use std::path::PathBuf;
use std::fs::File;
use std::io::Write;

extern crate tools;
extern crate ipdl_parser;
extern crate getopts;

use getopts::Options;

use tools::file_format::analysis::{read_analysis, read_target, WithLocation, AnalysisTarget, AnalysisKind};

use ipdl_parser::parser;
use ipdl_parser::ast;

type TargetAnalysis = Vec<WithLocation<Vec<AnalysisTarget>>>;

fn get_options_parser() -> Options {
    let mut opts = Options::new();
    opts.optmulti("I", "include",
                  "Additional directory to search for included protocol specifications",
                  "DIR");
    opts.reqopt("d", "outheaders-dir",
                "Directory into which C++ headers analysis data is location.",
                "HDR_DIR");
    opts.reqopt("b", "base-input-prefix",
                "Base directory where IPDL input files are found.",
                "BASE_DIR");
    opts.reqopt("a", "analysis-prefix",
                "Base directory where analysis output files are found.",
                "ANALYSIS_DIR");
    opts
}

fn header_file_name(outheaders_dir: &str, ns: &ast::Namespace, parent_or_child: &str) -> String {
    format!("{}/{}/{}{}.h",
            outheaders_dir,
            ns.namespaces.clone().join("/"),
            ns.name.id,
            parent_or_child)
}

fn mangle_simple(s: &str) -> String {
    format!("{}{}", s.len(), s)
}

fn mangle_nested_name(ns: &[String], protocol: &str, name: &str) -> String {
    format!("_ZN{}{}{}E",
            ns.iter().map(|id| mangle_simple(&id)).collect::<Vec<_>>().join(""),
            mangle_simple(protocol),
            mangle_simple(name))
}

fn find_analysis<'a>(analysis: &'a TargetAnalysis, mangled: &str) -> Option<&'a AnalysisTarget>
{
    for datum in analysis {
        for piece in &datum.data {
            if piece.kind == AnalysisKind::Decl && piece.sym.contains(mangled) {
                return Some(&piece);
            }
        }
    }

    println!("No analysis target found for {}", mangled);
    return None
}

fn output_data(outputf: &mut File, locstr: &str, datum: &AnalysisTarget) {
    write!(outputf, r#"{{"loc": "{}", "target": 1, "kind": "idl", "pretty": "{}", "sym": "{}"}}"#,
           locstr, datum.pretty, datum.sym).unwrap();
    write!(outputf, "\n").unwrap();
    write!(outputf, r#"{{"loc": "{}", "source": 1, "pretty": "{}", "sym": "{}"}}"#,
           locstr, datum.pretty, datum.sym).unwrap();
    write!(outputf, "\n").unwrap();
}

fn output_send_recv(outputf: &mut File,
                    locstr: &str,
                    protocol: &ast::Namespace,
                    message: &ast::MessageDecl,
                    is_ctor: bool,
                    send_side: &str, send_analysis: &TargetAnalysis,
                    recv_side: &str, recv_analysis: &TargetAnalysis)
{
    let send_prefix = if message.send_semantics == ast::SendSemantics::Intr { "Call" } else { "Send" };
    let recv_prefix = if message.send_semantics == ast::SendSemantics::Intr { "Answer" } else { "Recv" };

    let ctor_suffix = if is_ctor { "Constructor" } else { "" };

    let mangled = mangle_nested_name(&protocol.namespaces,
                                     &format!("{}{}", protocol.name.id, send_side),
                                     &format!("{}{}{}", send_prefix, message.name.id, ctor_suffix));
    if let Some(send_datum) = find_analysis(send_analysis, &mangled) {
        output_data(outputf, &locstr, &send_datum);
    }

    let mangled = mangle_nested_name(&protocol.namespaces,
                                     &format!("{}{}", protocol.name.id, recv_side),
                                     &format!("{}{}{}", recv_prefix, message.name.id, ctor_suffix));
    if let Some(recv_datum) = find_analysis(recv_analysis, &mangled) {
        output_data(outputf, &locstr, &recv_datum);
    }
}

fn main() {
    let args : Vec<String> = env::args().collect();

    let opts = get_options_parser();

    let matches = match opts.parse(&args[1..]) {
        Ok(m) => { m },
        Err(f) => { panic!(f.to_string()) },
    };

    let mut include_dirs = Vec::new();
    for i in matches.opt_strs("I") {
        include_dirs.push(PathBuf::from(i))
    }

    let outheaders_dir = matches.opt_str("d").unwrap();
    let base_dir = matches.opt_str("b").unwrap();
    let analysis_dir = matches.opt_str("a").unwrap();
    let base_path = Path::new(&base_dir);
    let analysis_path = Path::new(&analysis_dir);

    let mut file_names = Vec::new();
    for f in matches.free {
        file_names.push(PathBuf::from(f));
    }

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
            let parent_fname = header_file_name(&outheaders_dir, &ns, "Parent");
            let parent_analysis = read_analysis(&parent_fname, &read_target);
            let child_fname = header_file_name(&outheaders_dir, &ns, "Child");
            let child_analysis = read_analysis(&child_fname, &read_target);

            let is_toplevel = protocol.managers.len() == 0;

            for message in protocol.messages {
                let loc = &message.name.loc;
                let locstr = format!("{}:{}-{}", loc.lineno, loc.colno, loc.colno + message.name.id.len());

                if is_toplevel && message.name.id == "__delete__" {
                    continue;
                }

                let is_ctor = protocol.manages.iter().any(|e| e.id == message.name.id);

                if message.direction == ast::Direction::ToChild || message.direction == ast::Direction::ToParentOrChild {
                    output_send_recv(&mut outputf, &locstr, &ns, &message, is_ctor,
                                     "Parent", &parent_analysis, "Child", &child_analysis);
                }

                if message.direction == ast::Direction::ToParent || message.direction == ast::Direction::ToParentOrChild {
                    output_send_recv(&mut outputf, &locstr, &ns, &message, is_ctor,
                                     "Child", &child_analysis, "Parent", &parent_analysis);
                }
            }
        }
    }
}
