use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs::File;
use std::io::BufRead;
use std::io::BufReader;
use std::io::BufWriter;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

extern crate env_logger;
extern crate getopts;
extern crate ipdl_parser;
extern crate tools;

use getopts::Options;
use lazy_static::lazy_static;
use regex::Regex;
use serde_json::{Value, json};

use ipdl_parser::ast::ProtocolSide;
use tools::file_format::analysis::{
    AnalysisKind, AnalysisTarget, WithLocation, read_analyses, read_target,
};
use tools::file_format::local_symbols::parameter_symbol;

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

/// Write a JSON record as a line.
fn emit(outputf: &mut BufWriter<File>, record: serde_json::Value) {
    writeln!(outputf, "{}", record).unwrap();
}

/// A location's `loc` for a name `len` long.
fn locstr(loc: &ast::Location, len: usize) -> String {
    format!("{}:{}-{}", loc.lineno, loc.colno, loc.colno + len)
}

/// The C++ symbols of a generated header's definitions and declarations, by
/// their pretty names (ex: "mozilla::_foo::SIntDouble::i" for both of a
/// struct field's accessors).
#[derive(Default)]
struct GeneratedSymbols {
    by_pretty: HashMap<String, Vec<String>>,
    /// Functions' parameters' names and types, by symbol (from their
    /// structured records).
    params: HashMap<String, Vec<(String, String)>>,
}

impl GeneratedSymbols {
    /// From the generated files' analysis (a data structures' header, and
    /// their `.cpp`, which defines unions' constructors).
    fn load(paths: &[String]) -> GeneratedSymbols {
        let mut syms = GeneratedSymbols::default();
        for datum in read_analyses(paths, &mut read_target) {
            for piece in datum.data {
                // (Not locals' and parameters'; see `local_symbols`.)
                if (piece.kind == AnalysisKind::Def || piece.kind == AnalysisKind::Decl)
                    && !piece.sym.starts_with("V_")
                {
                    let list = syms.by_pretty.entry(piece.pretty.to_string()).or_default();
                    if !list.iter().any(|s| s == piece.sym.as_str()) {
                        list.push(piece.sym.to_string());
                    }
                }
            }
        }
        let mut read_params = |value: Value, _: &_, _| -> Option<(String, Vec<(String, String)>)> {
            value.get("structured")?;
            let sym = value.get("sym")?.as_str()?.to_string();
            let params = value
                .get("args")?
                .as_array()?
                .iter()
                .filter_map(|arg| {
                    Some((
                        arg.get("name")?.as_str()?.to_string(),
                        arg.get("type")?.as_str()?.to_string(),
                    ))
                })
                .collect();
            Some((sym, params))
        };
        for datum in read_analyses(paths, &mut read_params) {
            syms.params.extend(datum.data);
        }
        syms
    }

    /// A struct's constructors from its fields: those whose parameters are
    /// the fields, in order, as IPDL names them (`_<field>`), ex:
    /// `SIntDouble(const int& _i, const double& _d)` and its `&&` overload.
    fn field_constructors(&self, pretty: &str, name: &str, fields: &[&str]) -> Vec<&str> {
        if fields.is_empty() {
            return vec![];
        }
        let names: Vec<String> = fields.iter().map(|f| format!("_{}", f)).collect();
        self.get(&format!("{}::{}", pretty, name))
            .iter()
            .filter(|sym| {
                self.params
                    .get(*sym)
                    .is_some_and(|params| params.iter().map(|(n, _)| n).eq(names.iter()))
            })
            .map(|sym| sym.as_str())
            .collect()
    }

    /// A union's constructors from one of its variants: those whose only
    /// parameter is of the variant's type (see `variant_cxx_type`), ex:
    /// `IntDouble(const int& aOther)` and its `&&` overload.
    fn variant_constructors(&self, pretty: &str, name: &str, ty: &str) -> Vec<&str> {
        self.get(&format!("{}::{}", pretty, name))
            .iter()
            .filter(|sym| match self.params.get(*sym).map(|p| p.as_slice()) {
                Some([(_, param_ty)]) => unqualified_type(param_ty) == ty,
                _ => false,
            })
            .map(|sym| sym.as_str())
            .collect()
    }

    fn get(&self, pretty: &str) -> &[String] {
        self.by_pretty
            .get(pretty)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// A method's const overload and its other (mutable) one, ex: a struct
    /// field's accessors.
    fn const_and_mutable(&self, pretty: &str) -> (Option<&str>, Option<&str>) {
        let syms = self.get(pretty);
        let is_const = |s: &&String| s.starts_with("_ZNK");
        (
            syms.iter().find(is_const).map(|s| s.as_str()),
            syms.iter()
                .find(|s| s.starts_with("_Z") && !is_const(s))
                .map(|s| s.as_str()),
        )
    }

    fn first_with_prefix(&self, pretty: &str, prefix: &str) -> Option<&str> {
        self.get(pretty)
            .iter()
            .find(|s| s.starts_with(prefix))
            .map(|s| s.as_str())
    }
}

/// IPDL structs and unions, by their qualified and unqualified names, as
/// their `IPDL_` symbols and pretty names.
type IpdlTypes = HashMap<String, (String, String)>;

/// A C++ type without qualifiers, `const`, references or spaces (ex:
/// `nsTArray<Actors>` for `const nsTArray<mozilla::_foo::Unions::Actors> &`),
/// for comparing types however generated code spells them.
fn unqualified_type(ty: &str) -> String {
    lazy_static! {
        static ref QUALIFIER: Regex = Regex::new(r"[A-Za-z_][A-Za-z0-9_]*::").unwrap();
    }
    QUALIFIER
        .replace_all(&ty.replace("const ", ""), "")
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '&')
        .collect()
}

/// A union variant's C++ type, unqualified (see `unqualified_type`), ex:
/// `nsTArray<int>` for `int[]`, but for actors' (whose C++ types are
/// `SideVariant`s of their sides).
fn variant_cxx_type(type_spec: &ast::TypeSpec, actors: &HashSet<String>) -> Option<String> {
    let base = type_spec
        .spec
        .rsplit("::")
        .next()
        .unwrap_or(&type_spec.spec);
    if actors.contains(base) {
        return None;
    }
    let mut ty = base.to_string();
    for modifier in &type_spec.modifiers {
        ty = match modifier {
            ast::TypeSpecModifier::Array => format!("nsTArray<{}>", ty),
            ast::TypeSpecModifier::Maybe => format!("Maybe<{}>", ty),
            ast::TypeSpecModifier::UniquePtr => format!("UniquePtr<{}>", ty),
        };
    }
    Some(ty)
}

/// A data structure's (or its part's) `IPDL_` symbol: the namespaces, its
/// name, and its parts' names, joined with `_` (as messages' `IPC_` symbols
/// are).
fn ipdl_sym(ns: &ast::Namespace, parts: &[&str]) -> String {
    let mut sym = format!("IPDL_{}_{}", ns.namespaces.join("_"), ns.name.id);
    for part in parts {
        sym.push('_');
        sym.push_str(part);
    }
    sym
}

/// The C++ pretty name of a data structure (or its part).
fn cxx_pretty(ns: &ast::Namespace, parts: &[&str]) -> String {
    let mut pretty = format!("{}::{}", ns.namespaces.join("::"), ns.name.id);
    for part in parts {
        pretty.push_str("::");
        pretty.push_str(part);
    }
    pretty
}

/// The names IPDL might generate for a union's variant of a type (its
/// `T<name>` enum value and `get_<name>()` accessors): the type's name,
/// prefixed by its modifiers' (ex: "ArrayOfint" for `int[]`), and, for
/// actors (whose variants are generally named for the protocol, ex:
/// "ArrayOfPTestDataStructuresSub"), each side's too.
fn variant_names(type_spec: &ast::TypeSpec, actors: &HashSet<String>) -> Vec<String> {
    let base = type_spec
        .spec
        .rsplit("::")
        .next()
        .unwrap_or(&type_spec.spec);
    let bases = if actors.contains(base) {
        vec![
            base.to_string(),
            format!("{}Parent", base),
            format!("{}Child", base),
        ]
    } else {
        vec![base.to_string()]
    };
    bases
        .into_iter()
        .map(|mut name| {
            for modifier in &type_spec.modifiers {
                let prefix = match modifier {
                    ast::TypeSpecModifier::Array => "ArrayOf",
                    ast::TypeSpecModifier::Maybe => "Maybe",
                    ast::TypeSpecModifier::UniquePtr => "UniquePtr",
                };
                name = format!("{}{}", prefix, name);
            }
            name
        })
        .collect()
}

/// A variant's name for its `IPDL_` symbol: its type's, without the sides
/// actors' generated names have.
fn variant_owner_name(type_spec: &ast::TypeSpec) -> String {
    let mut name = type_spec
        .spec
        .rsplit("::")
        .next()
        .unwrap_or(&type_spec.spec)
        .to_string();
    for modifier in &type_spec.modifiers {
        let prefix = match modifier {
            ast::TypeSpecModifier::Array => "ArrayOf",
            ast::TypeSpecModifier::Maybe => "Maybe",
            ast::TypeSpecModifier::UniquePtr => "UniquePtr",
        };
        name = format!("{}{}", prefix, name);
    }
    name
}

/// Write a use of an IPDL struct or union, if a type names one.
fn output_type_use(
    outputf: &mut BufWriter<File>,
    type_spec: &ast::TypeSpec,
    ipdl_types: &IpdlTypes,
    context: &str,
    contextsym: &str,
) {
    let found = ipdl_types.get(&type_spec.spec).or_else(|| {
        type_spec
            .spec
            .rsplit("::")
            .next()
            .and_then(|name| ipdl_types.get(name))
    });
    let Some((sym, pretty)) = found else {
        return;
    };
    let loc = locstr(&type_spec.loc, type_spec.spec.len());
    emit(
        outputf,
        json!({
            "loc": loc,
            "target": 1,
            "kind": "use",
            "pretty": pretty,
            "sym": sym,
            "context": context,
            "contextsym": contextsym,
        }),
    );
    emit(
        outputf,
        json!({
            "loc": loc,
            "source": 1,
            "syntax": "idl,type,use",
            "pretty": format!("ipdl type {}", pretty),
            "sym": sym,
        }),
    );
}

/// Write the records of an IPDL definition: its target and source records,
/// and its structured record (to which `structured` adds).
fn output_definition(
    outputf: &mut BufWriter<File>,
    loc: &str,
    syntax: &str,
    pretty: &str,
    sym: &str,
    mut structured: serde_json::Value,
) {
    emit(
        outputf,
        json!({
            "loc": loc,
            "target": 1,
            "kind": "idl",
            "pretty": pretty,
            "sym": sym,
        }),
    );
    emit(
        outputf,
        json!({
            "loc": loc,
            "source": 1,
            "syntax": format!("idl,{},def", syntax),
            "pretty": format!("ipdl {} {}", syntax, pretty),
            "sym": sym,
        }),
    );
    let obj = structured.as_object_mut().unwrap();
    obj.insert("loc".to_string(), json!(loc));
    obj.insert("structured".to_string(), json!(1));
    obj.insert("pretty".to_string(), json!(pretty));
    obj.insert("sym".to_string(), json!(sym));
    obj.insert("implKind".to_string(), json!("idl"));
    emit(outputf, structured);
}

fn cpp_slot(kind: &str, sym: &str) -> serde_json::Value {
    json!({
        "slotKind": kind,
        "slotLang": "cpp",
        "ownerLang": "idl",
        "sym": sym,
    })
}

/// Write a translation unit's structs and unions: an `IPDL_` symbol for each,
/// and for each of their fields and variants, with binding slots for the C++
/// IPDL generates for them (found by their pretty names in the generated
/// header's analysis).
fn output_data_structures(
    outputf: &mut BufWriter<File>,
    tu: &ast::TranslationUnit,
    generated: &GeneratedSymbols,
    ipdl_types: &IpdlTypes,
    actors: &HashSet<String>,
) {
    for (ns, _attrs, fields) in &tu.structs {
        let sym = ipdl_sym(ns, &[]);
        let pretty = cxx_pretty(ns, &[]);
        let mut slots = vec![];
        if let Some(class) = generated.first_with_prefix(&pretty, "T_") {
            slots.push(cpp_slot("class", class));
        }
        // (Whose arguments are the fields' values, by their `ipdlIndex`es.)
        let field_names: Vec<&str> = fields.iter().map(|f| f.name.id.as_str()).collect();
        for constructor in generated.field_constructors(&pretty, &ns.name.id, &field_names) {
            slots.push(cpp_slot("constructor", constructor));
        }
        let mut field_infos = vec![];
        for (index, field) in fields.iter().enumerate() {
            let name = field.name.id.as_str();
            let field_sym = ipdl_sym(ns, &[name]);
            let field_pretty = cxx_pretty(ns, &[name]);
            let mut field_slots = vec![];
            let (getter, setter) = generated.const_and_mutable(&field_pretty);
            if let Some(getter) = getter {
                field_slots.push(cpp_slot("getter", getter));
            }
            if let Some(setter) = setter {
                field_slots.push(cpp_slot("setter", setter));
            }
            if let Some(member) = generated.first_with_prefix(&format!("{}_", field_pretty), "F_") {
                field_slots.push(cpp_slot("member", member));
            }
            output_definition(
                outputf,
                &locstr(&field.name.loc, name.len()),
                "field",
                &field_pretty,
                &field_sym,
                json!({
                    "kind": "field",
                    "parentsym": sym,
                    "type_pretty": field.type_spec.spec,
                    "bindingSlots": field_slots,
                    // (Its constructor parameter's.)
                    "ipdlIndex": index,
                }),
            );
            output_type_use(
                outputf,
                &field.type_spec,
                ipdl_types,
                &field_pretty,
                &field_sym,
            );
            field_infos.push(json!({ "pretty": field_pretty, "sym": field_sym }));
        }
        output_definition(
            outputf,
            &locstr(&ns.name.loc, ns.name.id.len()),
            "struct",
            &pretty,
            &sym,
            json!({
                "kind": "struct",
                "bindingSlots": slots,
                "fields": field_infos,
            }),
        );
    }

    for (ns, _attrs, variants) in &tu.unions {
        let sym = ipdl_sym(ns, &[]);
        let pretty = cxx_pretty(ns, &[]);
        let mut slots = vec![];
        if let Some(class) = generated.first_with_prefix(&pretty, "T_") {
            slots.push(cpp_slot("class", class));
        }
        let mut variant_infos = vec![];
        for (index, type_spec) in variants.iter().enumerate() {
            let owner_name = variant_owner_name(type_spec);
            let variant_sym = ipdl_sym(ns, &[&owner_name]);
            let variant_pretty = cxx_pretty(ns, &[&owner_name]);
            let mut variant_slots = vec![];
            // (Constructing the union from the variant's value writes it.)
            if let Some(ty) = variant_cxx_type(type_spec, actors) {
                for constructor in generated.variant_constructors(&pretty, &ns.name.id, &ty) {
                    variant_slots.push(cpp_slot("constructor", constructor));
                }
            }
            for name in variant_names(type_spec, actors) {
                let (getter, setter) =
                    generated.const_and_mutable(&cxx_pretty(ns, &[&format!("get_{}", name)]));
                if let Some(getter) = getter {
                    variant_slots.push(cpp_slot("getter", getter));
                }
                if let Some(setter) = setter {
                    variant_slots.push(cpp_slot("setter", setter));
                }
                // (The type's `T<name>` enum value.)
                if let Some(value) = generated
                    .first_with_prefix(&cxx_pretty(ns, &["Type", &format!("T{}", name)]), "E_")
                    .or_else(|| {
                        generated.first_with_prefix(&cxx_pretty(ns, &[&format!("T{}", name)]), "E_")
                    })
                {
                    variant_slots.push(cpp_slot("const", value));
                }
            }
            output_definition(
                outputf,
                &locstr(&type_spec.loc, type_spec.spec.len()),
                "variant",
                &variant_pretty,
                &variant_sym,
                json!({
                    "kind": "field",
                    "parentsym": sym,
                    "type_pretty": type_spec.spec,
                    "bindingSlots": variant_slots,
                    "ipdlIndex": index,
                }),
            );
            output_type_use(
                outputf,
                type_spec,
                ipdl_types,
                &variant_pretty,
                &variant_sym,
            );
            variant_infos.push(json!({ "pretty": variant_pretty, "sym": variant_sym }));
        }
        output_definition(
            outputf,
            &locstr(&ns.name.loc, ns.name.id.len()),
            "union",
            &pretty,
            &sym,
            json!({
                "kind": "union",
                "bindingSlots": slots,
                "fields": variant_infos,
            }),
        );
    }
}

/// Write a message's parameters' symbols (`IPC_…_<Message>__<param>`), with
/// binding slots for the send and recv methods' parameters (whose symbols
/// are unified; see `local_symbols`): in-parameters in order, then sync
/// messages' out-parameters (pointers), and async messages' return values
/// without slots (their send methods take resolvers), and uses of the IPDL
/// structs and unions their types name.
fn output_message_params(
    outputf: &mut BufWriter<File>,
    message: &ast::MessageDecl,
    message_pretty: &str,
    message_sym: &str,
    slots: &[(&AnalysisTarget, &AnalysisTarget)],
    ipdl_types: &IpdlTypes,
) {
    let sync = message.send_semantics != ast::SendSemantics::Async;
    let params = message
        .in_params
        .iter()
        .map(|p| (p, false))
        .chain(message.out_params.iter().map(|p| (p, true)));
    for (position, (param, is_out)) in params.enumerate() {
        let name = param.name.id.as_str();
        let param_sym = format!("{}__{}", message_sym, name);
        let param_pretty = format!("{}::{}", message_pretty, name);
        let mut param_slots = vec![];
        if !is_out || sync {
            for (send, recv) in slots {
                param_slots.push(cpp_slot("param", &parameter_symbol(&send.sym, position)));
                param_slots.push(cpp_slot("param", &parameter_symbol(&recv.sym, position)));
            }
        }
        output_definition(
            outputf,
            &locstr(&param.name.loc, name.len()),
            "parameter",
            &param_pretty,
            &param_sym,
            json!({
                "kind": "parameter",
                "parentsym": message_sym,
                "type_pretty": param.type_spec.spec,
                "bindingSlots": param_slots,
                "ipdlIndex": position,
                "ipdlOut": is_out,
            }),
        );
        output_type_use(
            outputf,
            &param.type_spec,
            ipdl_types,
            &param_pretty,
            &param_sym,
        );
    }
}

/// Write a message's `IPC_` symbol's records, with binding slots for its
/// send and recv methods (for both directions' for `both:` messages).
fn output_ipc_data(
    outputf: &mut BufWriter<File>,
    locstr: &str,
    ipc_pretty: &str,
    ipc_sym: &str,
    slots: &[(&AnalysisTarget, &AnalysisTarget)],
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
    let mut binding_slots = vec![];
    for (send_datum, recv_datum) in slots {
        binding_slots.push(json!({
            "slotKind": "send",
            "slotLang": "cpp",
            "ownerLang": "idl",
            "sym": send_datum.sym,
        }));
        binding_slots.push(json!({
            "slotKind": "recv",
            "slotLang": "cpp",
            "ownerLang": "idl",
            "sym": recv_datum.sym,
        }));
    }
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
            "bindingSlots": binding_slots,
        })
    )
    .unwrap();
    writeln!(outputf).unwrap();
}

/// A message's send and recv methods for one direction, if both are found.
fn find_send_recv<'a>(
    protocol: &ast::Namespace,
    message: &ast::MessageDecl,
    is_ctor: bool,
    send_side: &str,
    send_analysis: &'a TargetAnalysis,
    recv_side: &str,
    recv_analysis: &'a TargetAnalysis,
) -> Option<(&'a AnalysisTarget, &'a AnalysisTarget)> {
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

    match (maybe_send_datum, maybe_recv_datum) {
        (Some(send_datum), Some(recv_datum)) => Some((send_datum, recv_datum)),
        _ => None,
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

    // (Every translation unit's structs and unions, for their uses' symbols,
    // and protocols, whose actors are types too.)
    let mut ipdl_types: IpdlTypes = HashMap::new();
    let mut actors: HashSet<String> = HashSet::new();
    for tu in tus.values() {
        for (ns, _, _) in &tu.structs {
            let entry = (ipdl_sym(ns, &[]), cxx_pretty(ns, &[]));
            ipdl_types.insert(cxx_pretty(ns, &[]), entry.clone());
            ipdl_types.entry(ns.name.id.clone()).or_insert(entry);
        }
        for (ns, _, _) in &tu.unions {
            let entry = (ipdl_sym(ns, &[]), cxx_pretty(ns, &[]));
            ipdl_types.insert(cxx_pretty(ns, &[]), entry.clone());
            ipdl_types.entry(ns.name.id.clone()).or_insert(entry);
        }
        if let Some((ns, _)) = &tu.protocol {
            actors.insert(ns.name.id.clone());
        }
    }

    for (_, tu) in tus {
        println!("Analyzing {:?}", tu.file_name);

        let path = tu.file_name.as_path();
        let relative = path.strip_prefix(base_path).unwrap();
        let absolute = analysis_path.join(relative);
        // (Buffered: a record's JSON is written in many small pieces.)
        let mut outputf = BufWriter::new(File::create(absolute).unwrap());

        // ### Structs and unions
        //
        // Their C++ is in the header named for the file (ex:
        // `PTestDataStructuresCommon.h` for `PTestDataStructuresCommon.ipdlh`).
        if !tu.structs.is_empty() || !tu.unions.is_empty() {
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let generated = match objdir_files_map.get(&format!("{}.h", stem)) {
                Some(header) => {
                    let mut paths = vec![analysis_path.join(header).to_string_lossy().into_owned()];
                    println!("  Reading data structures' header {:?}", &paths[0]);
                    // (And their `.cpp`, which defines unions' constructors.)
                    if let Some(cpp) = objdir_files_map.get(&format!("{}.cpp", stem)) {
                        paths.push(analysis_path.join(cpp).to_string_lossy().into_owned());
                    }
                    GeneratedSymbols::load(&paths)
                }
                None => {
                    println!("  Unable to find the header for {:?}", path);
                    GeneratedSymbols::default()
                }
            };
            output_data_structures(&mut outputf, &tu, &generated, &ipdl_types, &actors);
        }

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

                let message_pretty = format!(
                    "{}::{}::{}",
                    ns.namespaces.join("_"),
                    ns.name.id,
                    message.name.id
                );
                let message_sym = format!(
                    "IPC_{}_{}_{}",
                    ns.namespaces.join("_"),
                    ns.name.id,
                    message.name.id
                );

                // (One symbol for the message, with the send and recv slots of
                // each direction it goes in: both for `both:` messages.)
                let mut slots = vec![];
                if message.direction == ast::Direction::To(ProtocolSide::Child)
                    || message.direction == ast::Direction::Both
                {
                    slots.extend(find_send_recv(
                        &ns,
                        &message,
                        is_ctor,
                        "Parent",
                        &parent_analysis,
                        "Child",
                        &child_analysis,
                    ));
                }

                if message.direction == ast::Direction::To(ProtocolSide::Parent)
                    || message.direction == ast::Direction::Both
                {
                    slots.extend(find_send_recv(
                        &ns,
                        &message,
                        is_ctor,
                        "Child",
                        &child_analysis,
                        "Parent",
                        &parent_analysis,
                    ));
                }

                if !slots.is_empty() {
                    let ipc_pretty = format!(
                        "{}::{}::{}",
                        ns.namespaces.join("_"),
                        ns.name.id,
                        message.name.id
                    );
                    let ipc_sym = format!(
                        "IPC_{}_{}_{}",
                        ns.namespaces.join("_"),
                        ns.name.id,
                        message.name.id
                    );
                    output_ipc_data(&mut outputf, &locstr, &ipc_pretty, &ipc_sym, &slots);
                }

                output_message_params(
                    &mut outputf,
                    &message,
                    &message_pretty,
                    &message_sym,
                    &slots,
                    &ipdl_types,
                );
            }
        }
        outputf.flush().unwrap();
    }
}
