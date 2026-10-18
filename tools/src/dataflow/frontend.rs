//! Turn tree-sitter parses plus analysis records into `FuncIr`s: tree-sitter
//! says what the syntax is (what's assigned to what, what's an argument of
//! what), and the records say what the names are (which local, which field,
//! which callee).
//!
//! C++ (tree-sitter-mozcpp, clang's records), JS (tree-sitter-typescript,
//! scip-typescript's records), and Kotlin (tree-sitter-kotlin-ng, scip-java's
//! records) share most of this; their syntax differs in node kinds and field
//! names, which `Builder::eval` and `Builder::stmt` dispatch on.

use std::collections::HashMap;

use tree_sitter::Node;
use ustr::{Ustr, ustr};

use super::ir::*;
use super::records::{FileRecords, SourceRec, short_name};
use crate::tree_sitter_support::cst_tokenizer::{
    SourceParse, parse_source_for_analysis, parse_with_profile_for_analysis, profile_for_lang,
};

/// The functions of a file.
pub struct FileIr {
    pub path: String,
    pub lang: Lang,
    pub funcs: Vec<FuncIr>,
    pub by_sym: HashMap<Ustr, usize>,
}

impl FileIr {
    pub fn func(&self, sym: &Ustr) -> Option<&FuncIr> {
        self.by_sym.get(sym).map(|&i| &self.funcs[i])
    }

    /// The innermost function containing a line.
    pub fn func_at_line(&self, line: u32) -> Option<&FuncIr> {
        self.funcs
            .iter()
            .filter(|f| f.line <= line && line <= f.end_line)
            .min_by_key(|f| f.end_line - f.line)
    }
}

pub fn lang_for_path(path: &str) -> Option<Lang> {
    let ext = path.rsplit('.').next()?;
    match ext {
        "c" | "cc" | "cpp" | "cxx" | "h" | "hh" | "hxx" | "hpp" => Some(Lang::Cpp),
        "js" | "jsm" | "mjs" | "sjs" | "ts" | "jsx" | "tsx" => Some(Lang::Js),
        "kt" | "kts" => Some(Lang::Kotlin),
        _ => None,
    }
}

/// Where a macro-defined function's name is in a macro's expansion (by
/// its key and platform), from its record.
fn expansion_offset(def: &SourceRec, key: &str, platform: &str) -> Option<usize> {
    def.expansion_offsets
        .iter()
        .find(|(k, p, _)| k == key && p == platform)
        .and_then(|(_, _, offsets)| offsets.first().copied())
}

/// The function definitions in a macro's expansion, by their names'
/// offsets (the first in the walk, for an offset).  (Indexed once for each
/// expansion: walking the expansion for each function's offset was quadratic
/// in X-macro expansions defining many functions, ex: 45 s of MIR.h's 65.)
fn macro_function_nodes(parse: &SourceParse) -> HashMap<usize, Node<'_>> {
    let mut nodes = HashMap::new();
    let mut stack = vec![parse.tree.root_node()];
    while let Some(node) = stack.pop() {
        if node.kind() == "function_definition"
            && let Some(name) = cpp_function_name(node)
        {
            nodes.entry(name.start_byte()).or_insert(node);
        }
        stack.extend(named(node));
    }
    nodes
}

pub fn build_file_ir(path: &str, source: &str, recs: &FileRecords) -> Result<FileIr, String> {
    let parse = parse_source_for_analysis(path, source)?;
    let lang = match parse.lang {
        "cpp" => Lang::Cpp,
        "js" | "jsx" => Lang::Js,
        "kotlin" => Lang::Kotlin,
        other => return Err(format!("No dataflow frontend for {}", other)),
    };
    let lines: Vec<&str> = source.lines().collect();
    let cx = Cx {
        path,
        lang,
        recs,
        lines: &lines,
        utf16_columns: Default::default(),
    };
    let mut funcs = vec![];
    let file_text: std::rc::Rc<str> = std::rc::Rc::from(parse.text.as_str());
    let mut stack = vec![parse.tree.root_node()];
    while let Some(node) = stack.pop() {
        if let Some((sym, pretty)) = cx.function_sym(node, &parse.text) {
            let mut builder = Builder::new(&cx, &file_text, node, sym, pretty);
            builder.build(node);
            stack.append(&mut builder.nested);
            funcs.push(builder.ir);
            continue;
        }
        stack.extend(named(node).into_iter().rev());
    }
    // Functions defined by macros, from their expansions.  (Each
    // expansion, by the macro's place, key and platform, is parsed once:
    // `NS_FORWARD_*`-style macros define hundreds of functions each.)
    let macro_defs = recs.macro_function_defs();
    let cpp = profile_for_lang("cpp").unwrap();
    let mut expansion_parses: HashMap<(u32, u32, &str, &str), Option<SourceParse>> = HashMap::new();
    // (Their functions' names' offsets.)
    let mut expansion_functions: HashMap<(u32, u32, &str, &str), std::collections::HashSet<usize>> =
        HashMap::new();
    let mut chosen: Vec<(usize, (u32, u32, &str, &str))> = vec![];
    let mut seen: std::collections::HashSet<Ustr> = funcs.iter().map(|f| f.sym).collect();
    for (di, (def, mac)) in macro_defs.iter().enumerate() {
        let sym = def.sym();
        if seen.contains(&sym) {
            continue;
        }
        let mut options: Vec<&(String, String, String)> = mac.expands_to.iter().collect();
        options.sort_by_key(|(key, platform, _)| {
            let rank = match platform.as_str() {
                "linux64" => 0,
                "linux64-opt" => 1,
                _ => 2,
            };
            (rank, platform.clone(), key.clone())
        });
        for (key, platform, expansion) in options {
            let Some(offset) = expansion_offset(def, key, platform) else {
                continue;
            };
            let cache_key = (mac.line, mac.col, key.as_str(), platform.as_str());
            let parse = expansion_parses
                .entry(cache_key)
                .or_insert_with(|| parse_with_profile_for_analysis(cpp, expansion).ok());
            let Some(parse) = parse else {
                continue;
            };
            let functions = expansion_functions
                .entry(cache_key)
                .or_insert_with(|| macro_function_nodes(parse).into_keys().collect());
            if !functions.contains(&offset) {
                continue;
            }
            chosen.push((di, cache_key));
            seen.insert(sym);
            break;
        }
    }
    let mut expansion_nodes: HashMap<(u32, u32, &str, &str), HashMap<usize, Node>> = HashMap::new();
    let mut expansion_texts: HashMap<(u32, u32, &str, &str), std::rc::Rc<str>> = HashMap::new();
    for (_, cache_key) in &chosen {
        if let Some(Some(parse)) = expansion_parses.get(cache_key) {
            expansion_nodes
                .entry(*cache_key)
                .or_insert_with(|| macro_function_nodes(parse));
            expansion_texts
                .entry(*cache_key)
                .or_insert_with(|| std::rc::Rc::from(parse.text.as_str()));
        }
    }
    for (di, cache_key) in chosen {
        let (def, mac) = &macro_defs[di];
        let (_, _, key, platform) = cache_key;
        let Some(offset) = expansion_offset(def, key, platform) else {
            continue;
        };
        let Some(&node) = expansion_nodes
            .get(&cache_key)
            .and_then(|nodes| nodes.get(&offset))
        else {
            continue;
        };
        let mut builder = Builder::new(
            &cx,
            &expansion_texts[&cache_key],
            node,
            def.sym(),
            def.pretty.clone(),
        );
        builder.macro_cx = Some(MacroCx {
            line: mac.line,
            col: mac.col,
            key: key.to_string(),
            platform: platform.to_string(),
            prefix: 0,
            name: mac.short_name().to_string(),
        });
        builder.ir.line = mac.line;
        builder.ir.end_line = mac.line;
        builder.build(node);
        funcs.push(builder.ir);
    }
    funcs.sort_by_key(|f| (f.line, f.end_line));
    let mut by_sym = HashMap::new();
    for (i, f) in funcs.iter().enumerate() {
        by_sym.entry(f.sym).or_insert(i);
    }
    Ok(FileIr {
        path: path.to_string(),
        lang,
        funcs,
        by_sym,
    })
}

fn named(node: Node) -> Vec<Node> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|n| !is_comment(n))
        .collect()
}

fn is_comment(node: &Node) -> bool {
    matches!(
        node.kind(),
        "comment" | "line_comment" | "block_comment" | "multiline_comment"
    )
}

/// Whether a scip symbol is a local or a parameter (scip's locals are
/// "<prefix><path>/#<n>"; parameters "...().(<name>)").
pub(crate) fn is_scip_local(sym: &str) -> bool {
    if let Some((_, n)) = sym.rsplit_once("/#")
        && !n.is_empty()
        && n.bytes().all(|b| b.is_ascii_digit())
    {
        return true;
    }
    sym.contains("().(") && sym.ends_with(')')
}

/// Macros whose expansions are assertions or logging: their arguments are
/// tested (or used), and what they expand to (crash reporting, log
/// plumbing) is noise.
const ASSERTION_MACROS: &[&str] = &[
    "MOZ_ASSERT",
    "MOZ_ASSERT_IF",
    "MOZ_DIAGNOSTIC_ASSERT",
    "MOZ_DIAGNOSTIC_ASSERT_IF",
    "MOZ_RELEASE_ASSERT",
    "MOZ_RELEASE_ASSERT_IF",
    "MOZ_ASSERT_UNREACHABLE",
    "MOZ_ASSERT_DEBUG_OR_FUZZING",
    "MOZ_CRASH",
    "MOZ_CRASH_UNSAFE",
    "MOZ_CRASH_UNSAFE_PRINTF",
    "MOZ_DIAGNOSTIC_CRASH",
    "MOZ_MAKE_COMPILER_ASSUME_IS_UNREACHABLE",
    "NS_ASSERTION",
    "NS_ERROR",
    "NS_WARNING",
    "NS_WARNING_ASSERTION",
    "MOZ_LOG",
    "MOZ_LOG_FMT",
    "NS_ASSERT_OWNINGTHREAD",
    "MOZ_ASSERT_UNREACHABLE",
];

/// Callees which return (a cast of) their argument.
const IDENTITY_CALLEES: &[&str] = &[
    "static_cast",
    "const_cast",
    "reinterpret_cast",
    "dynamic_cast",
    "move",
    "forward",
    "as_const",
    "WrapNotNull",
    "WrapNotNullUnchecked",
    "do_AddRef",
    "ToRefPtr",
];

struct Cx<'a> {
    path: &'a str,
    lang: Lang,
    recs: &'a FileRecords,
    lines: &'a [&'a str],
    /// Non-ASCII lines' UTF-16 columns by byte offset (`None` for ASCII
    /// lines), as `recs_at` needs them: computing them for each lookup was
    /// quadratic in minified JS's huge lines (69 s of 70 for a 35 MB
    /// bundle).
    utf16_columns: std::cell::RefCell<HashMap<usize, Option<std::rc::Rc<Vec<u32>>>>>,
}

impl<'a> Cx<'a> {
    fn line_text(&self, line: u32) -> String {
        let text = self
            .lines
            .get(line.wrapping_sub(1) as usize)
            .map(|l| l.trim())
            .unwrap_or("");
        truncate(text, 120)
    }

    /// The source records at a node (in the file), whose symbols are spelled
    /// there.  scip's columns are UTF-16 code units, so for lines with
    /// non-ASCII text we look there too.
    fn recs_at(&self, node: Node) -> Vec<&'a SourceRec> {
        let pos = node.start_position();
        let line = pos.row as u32 + 1;
        let mut out: Vec<&SourceRec> = self.recs.sources_at(line, pos.column as u32).collect();
        if out.is_empty()
            && self.lang != Lang::Cpp
            && let Some(columns) = self.utf16_columns(pos.row)
            && let Some(&col16) = columns.get(pos.column)
            && col16 != u32::MAX
        {
            out = self.recs.sources_at(line, col16).collect();
        }
        out
    }

    /// A line's UTF-16 column at each byte offset (`u32::MAX` inside a
    /// character), if it isn't ASCII (see `utf16_columns`).
    fn utf16_columns(&self, row: usize) -> Option<std::rc::Rc<Vec<u32>>> {
        if let Some(columns) = self.utf16_columns.borrow().get(&row) {
            return columns.clone();
        }
        let columns = self
            .lines
            .get(row)
            .filter(|text| !text.is_ascii())
            .map(|text| {
                let mut columns = vec![u32::MAX; text.len() + 1];
                let mut col16 = 0u32;
                for (offset, c) in text.char_indices() {
                    columns[offset] = col16;
                    col16 += c.len_utf16() as u32;
                }
                columns[text.len()] = col16;
                std::rc::Rc::new(columns)
            });
        self.utf16_columns.borrow_mut().insert(row, columns.clone());
        columns
    }

    /// The symbol and pretty name of a function node, if it's a function
    /// with a definition record (or a name; see `fallback_sym`).
    fn function_sym(&self, node: Node, text: &str) -> Option<(Ustr, String)> {
        let name = match (self.lang, node.kind()) {
            (Lang::Cpp, "function_definition") => cpp_function_name(node)?,
            (
                Lang::Js,
                "function_declaration" | "generator_function_declaration" | "method_definition",
            ) => node.child_by_field_name("name")?,
            (Lang::Js, "function_expression" | "function" | "arrow_function") => {
                // (Named by what they're assigned to, if they have no names
                // of their own: `const f = () => ...`, `{ f() {...} }`,
                // `foo: function() {...}`.)
                let parent = node.parent()?;
                match parent.kind() {
                    "variable_declarator" => parent.child_by_field_name("name")?,
                    "pair" => parent.child_by_field_name("key")?,
                    _ => node.child_by_field_name("name")?,
                }
            }
            (Lang::Kotlin, "function_declaration") => node.child_by_field_name("name")?,
            (Lang::Kotlin, "secondary_constructor") => node,
            _ => return None,
        };
        let recs = self.recs_at(name);
        let def = recs.iter().find(|r| {
            r.has_syntax("def")
                && (r.has_syntax("function")
                    || r.has_syntax("method")
                    || r.has_syntax("var")
                    || r.has_syntax("field"))
        });
        match def {
            Some(rec) => Some((rec.sym(), rec.pretty.clone())),
            None => {
                // Named functions without records (ex: C++ templates never
                // instantiated) still get IR, for the debugging view.
                let name_text = &text[name.byte_range()];
                if name_text.is_empty() || name_text.len() > 200 {
                    return None;
                }
                let line = name.start_position().row + 1;
                Some((
                    ustr(&format!("?{}:{}:{}", self.path, line, name_text)),
                    name_text.to_string(),
                ))
            }
        }
    }
}

fn truncate(text: &str, max: usize) -> String {
    let collapsed;
    let text = if text.contains('\n') {
        collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
        collapsed.as_str()
    } else {
        text
    };
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// The name node of a C++ function definition's declarator.
fn cpp_function_name(node: Node) -> Option<Node> {
    let mut decl = node.child_by_field_name("declarator")?;
    loop {
        match decl.kind() {
            "function_declarator" => {
                decl = decl.child_by_field_name("declarator")?;
                break;
            }
            "pointer_declarator" | "reference_declarator" | "attributed_declarator" => {
                decl = match decl.child_by_field_name("declarator") {
                    Some(d) => d,
                    None => *named(decl).last()?,
                };
            }
            _ => return None,
        }
    }
    Some(cpp_name_node(decl))
}

/// The node of the last name in a (possibly qualified) C++ name.
fn cpp_name_node(mut node: Node) -> Node {
    loop {
        match node.kind() {
            "qualified_identifier" | "template_function" | "template_method" => {
                match node.child_by_field_name("name") {
                    Some(name) => node = name,
                    None => return node,
                }
            }
            "destructor_name" => return node,
            _ => return node,
        }
    }
}

/// The identifier a C++ declarator declares.
fn cpp_declarator_ident(mut node: Node) -> Option<Node> {
    loop {
        match node.kind() {
            "identifier" | "field_identifier" => return Some(node),
            "qualified_identifier" => return Some(cpp_name_node(node)),
            "pointer_declarator"
            | "array_declarator"
            | "init_declarator"
            | "attributed_declarator"
            | "function_declarator" => {
                node = node.child_by_field_name("declarator")?;
            }
            "reference_declarator" | "variadic_declarator" | "parenthesized_declarator" => {
                node = *named(node).last()?;
            }
            _ => return None,
        }
    }
}

/// What an expression evaluates to: its values, and the place it is, if
/// it's one (`x`, `*p`, `a.b`), or is the address of (`&x`).
#[derive(Default)]
struct Ev {
    vals: Vec<Val>,
    place: Option<Place>,
    addr: bool,
}

impl Ev {
    fn vals(vals: Vec<Val>) -> Self {
        Ev {
            vals,
            place: None,
            addr: false,
        }
    }

    fn place(place: Place) -> Self {
        Ev {
            vals: vec![Val::direct(Atom::Read(place.clone()))],
            place: Some(place),
            addr: false,
        }
    }

    fn addr(place: Place) -> Self {
        Ev {
            vals: vec![Val::direct(Atom::Addr(place.clone()))],
            place: Some(place),
            addr: true,
        }
    }

    fn konst(text: &str) -> Self {
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        Ev::vals(vec![Val::direct(Atom::Const(truncate(&text, 40)))])
    }
}

/// Walking a C++ macro's expansion (reparsed), whose records are at the
/// macro's location with offsets in the expansion.
struct MacroCx {
    line: u32,
    col: u32,
    key: String,
    platform: String,
    /// The length of what we wrapped the expansion in before it.
    prefix: usize,
    name: String,
}

struct Builder<'c, 'a> {
    cx: &'c Cx<'a>,
    /// The text the current nodes are of: the file's, or a macro expansion's.
    /// (Shared: copying the file's for each function's builder was most of
    /// the time for files of many functions, ex: 2/3 of rlbox.wasm.c's.)
    text: std::rc::Rc<str>,
    ir: FuncIr,
    lambda_depth: u32,
    macro_cx: Option<MacroCx>,
    /// Names of locals and parameters seen so far, for resolving names
    /// without records.
    scope: HashMap<String, Place>,
    /// The class the function's a method of, for recognizing calls of its
    /// other methods without receivers (C++: "nsPrefBranch"; scip: the
    /// symbol up to the method).
    class_prefix: Option<String>,
    /// Named functions in this one, which get their own IR.
    nested: Vec<Node<'c>>,
}

impl<'c, 'a> Builder<'c, 'a> {
    fn new(cx: &'c Cx<'a>, text: &std::rc::Rc<str>, node: Node, sym: Ustr, pretty: String) -> Self {
        let class_prefix = match cx.lang {
            Lang::Cpp => pretty.rsplit_once("::").map(|(class, _)| class.to_string()),
            _ => sym.rsplit_once('#').map(|(class, _)| class.to_string()),
        };
        Builder {
            cx,
            text: text.clone(),
            ir: FuncIr {
                sym,
                pretty,
                path: cx.path.to_string(),
                lang: cx.lang,
                line: node.start_position().row as u32 + 1,
                end_line: node.end_position().row as u32 + 1,
                params: vec![],
                has_body: true,
                events: vec![],
                calls: vec![],
                questions: vec![],
                names: HashMap::new(),
                types: HashMap::new(),
            },
            lambda_depth: 0,
            macro_cx: None,
            scope: HashMap::new(),
            class_prefix,
            nested: vec![],
        }
    }

    fn t(&self, node: Node) -> &str {
        self.text.get(node.byte_range()).unwrap_or("")
    }

    fn pos(&self, node: Node) -> (u32, u32) {
        match &self.macro_cx {
            Some(m) => (m.line, m.col),
            None => {
                let p = node.start_position();
                (p.row as u32 + 1, p.column as u32)
            }
        }
    }

    /// The source records for a node: at it, or, in a macro expansion, at
    /// the macro, spelled at its offset in the expansion.
    fn recs(&self, node: Node) -> Vec<&'a SourceRec> {
        match &self.macro_cx {
            None => self.cx.recs_at(node),
            Some(m) => {
                let Some(offset) = node.start_byte().checked_sub(m.prefix) else {
                    return vec![];
                };
                self.cx
                    .recs
                    .all_sources_at(m.line, m.col)
                    .iter()
                    .filter(|r| {
                        r.expansion_offsets.iter().any(|(key, platform, offsets)| {
                            *key == m.key && *platform == m.platform && offsets.contains(&offset)
                        })
                    })
                    .collect()
            }
        }
    }

    fn question(&mut self, node: Node, kind: QuestionKind, text: String) {
        let (line, _) = self.pos(node);
        self.ir.questions.push(Question { line, kind, text });
    }

    /// The last line of a node (in the file).
    fn end_line(&self, node: Node) -> u32 {
        match &self.macro_cx {
            Some(m) => m.line,
            None => node.end_position().row as u32 + 1,
        }
    }

    fn event(&mut self, node: Node, kind: EventKind) {
        let (line, col) = self.pos(node);
        let end_line = self.end_line(node);
        self.ir.events.push(Event {
            line,
            col,
            end_line,
            kind,
            in_lambda: self.lambda_depth > 0,
            macro_name: self.macro_cx.as_ref().map(|m| m.name.clone()),
            text: self.cx.line_text(line),
        });
    }

    fn assign(&mut self, node: Node, place: Place, vals: Vec<Val>, op: &str) {
        self.event(
            node,
            EventKind::Assign {
                place,
                vals,
                op: op.to_string(),
            },
        );
    }

    fn test(&mut self, node: Node, vals: Vec<Val>) {
        if !vals.is_empty() {
            self.event(node, EventKind::Test { vals });
        }
    }

    fn name_sym(&mut self, sym: Ustr, name: &str) {
        self.ir.names.entry(sym).or_insert_with(|| name.to_string());
    }

    // ## Functions

    fn build(&mut self, node: Node<'c>) {
        match self.cx.lang {
            Lang::Cpp => self.cpp_function(node),
            Lang::Js => self.js_function(node),
            Lang::Kotlin => self.kt_function(node),
        }
    }

    fn add_param(&mut self, name_node: Node, type_hint: Option<String>) {
        let name = self.t(name_node).to_string();
        let recs = self.recs(name_node);
        let rec = recs.iter().find(|r| !r.syms.is_empty());
        let place = match rec {
            Some(rec) => {
                let sym = rec.sym();
                self.name_sym(sym, &name);
                if let Some(ty) = &rec.type_pretty {
                    self.ir.types.insert(sym, ty.clone());
                }
                Place::local(sym)
            }
            None => self.synth_local(name_node, &name),
        };
        self.scope.insert(name.clone(), place.clone());
        self.ir.params.push(Param {
            name,
            place,
            type_pretty: rec.and_then(|r| r.type_pretty.clone()).or(type_hint),
        });
    }

    /// A key for a local without a symbol.
    fn synth_local(&mut self, node: Node, name: &str) -> Place {
        let (line, _) = self.pos(node);
        let sym = ustr(&format!("L_{}_{}", line, name));
        self.name_sym(sym, name);
        Place::local(sym)
    }

    /// The place a name without a record is: a local seen so far, or else a
    /// local we make up.
    fn unknown_name(&mut self, node: Node) -> Place {
        let name = self.t(node).to_string();
        if let Some(place) = self.scope.get(&name) {
            return place.clone();
        }
        let sym = ustr(&format!("name:{}", name));
        Place::local(sym)
    }

    /// Declare a local at its name node, returning its place.
    fn declare_local(&mut self, name_node: Node) -> Place {
        let name = self.t(name_node).to_string();
        let recs = self.recs(name_node);
        // (A C++ local's record, rather than its constructor's, which clang
        // puts at the local's name for `T x(a, b);`.)
        let rec = recs
            .iter()
            .find(|r| r.sym().starts_with("V_"))
            .or_else(|| {
                recs.iter()
                    .find(|r| !r.syms.is_empty() && !r.has_syntax("constructor"))
            })
            .or_else(|| recs.iter().find(|r| !r.syms.is_empty()));
        let place = match rec {
            Some(rec) => {
                let sym = rec.sym();
                self.name_sym(sym, &name);
                if let Some(ty) = &rec.type_pretty {
                    self.ir.types.insert(sym, ty.clone());
                }
                Place::local(sym)
            }
            None => self.synth_local(name_node, &name),
        };
        self.scope.insert(name, place.clone());
        place
    }

    fn lambda_body(&mut self, body: Node<'c>, params: Option<Node<'c>>) {
        self.lambda_depth += 1;
        if let Some(params) = params {
            for param in named(params) {
                if let Some(ident) = self.param_ident(param) {
                    self.declare_local(ident);
                }
            }
        }
        match body.kind() {
            "compound_statement" | "statement_block" | "block" | "function_body" => self.stmt(body),
            "statements" => self.stmts(body),
            _ => {
                let vals = self.eval(body).vals;
                if !vals.is_empty() {
                    self.event(body, EventKind::Return { vals });
                }
            }
        }
        self.lambda_depth -= 1;
    }

    /// The name of a parameter node, in any language.
    fn param_ident(&self, param: Node<'c>) -> Option<Node<'c>> {
        match param.kind() {
            "parameter_declaration"
            | "optional_parameter_declaration"
            | "variadic_parameter_declaration" => {
                cpp_declarator_ident(param.child_by_field_name("declarator")?)
            }
            "required_parameter" | "optional_parameter" => {
                let name = param
                    .child_by_field_name("pattern")
                    .or_else(|| param.child_by_field_name("name"))?;
                (name.kind() == "identifier").then_some(name)
            }
            "identifier" => Some(param),
            "parameter" | "class_parameter" | "variable_declaration" => {
                named(param).into_iter().find(|n| n.kind() == "identifier")
            }
            _ => None,
        }
    }

    fn stmts(&mut self, node: Node<'c>) {
        for child in named(node) {
            self.stmt(child);
        }
    }

    fn stmt(&mut self, node: Node<'c>) {
        match self.cx.lang {
            Lang::Cpp => self.cpp_stmt(node),
            Lang::Js => self.js_stmt(node),
            Lang::Kotlin => self.kt_stmt(node),
        }
    }

    fn eval(&mut self, node: Node<'c>) -> Ev {
        match self.cx.lang {
            Lang::Cpp => self.cpp_eval(node),
            Lang::Js => self.js_eval(node),
            Lang::Kotlin => self.kt_eval(node),
        }
    }

    /// Evaluate nodes for their values, derived from them all if there are
    /// several.
    fn eval_union(&mut self, nodes: Vec<Node<'c>>) -> Vec<Val> {
        let several = nodes.len() > 1;
        let mut vals = vec![];
        for node in nodes {
            vals.extend(self.eval(node).vals);
        }
        if several { derive(vals) } else { vals }
    }

    fn arg(&mut self, node: Node<'c>) -> Arg {
        let ev = self.eval(node);
        Arg {
            vals: ev.vals,
            place: ev.place,
            addr: ev.addr,
            text: truncate(self.t(node), 60),
            line: self.pos(node).0,
            end_line: self.end_line(node),
        }
    }

    fn push_call(
        &mut self,
        node: Node,
        callee_recs: &[&SourceRec],
        callee_text: &str,
        kind: CallKind,
        receiver: Option<Arg>,
        args: Vec<Arg>,
    ) -> usize {
        let (line, col) = self.pos(node);
        let mut callee: Vec<Ustr> = vec![];
        let mut pretty = String::new();
        for rec in callee_recs {
            for sym in &rec.syms {
                if !callee.contains(sym) {
                    callee.push(*sym);
                }
            }
            if pretty.is_empty() {
                pretty = rec.pretty.clone();
            }
        }
        if pretty.is_empty() {
            pretty = callee_text.to_string();
        }
        let id = self.ir.calls.len();
        if callee.is_empty() && kind != CallKind::Macro {
            self.question(
                node,
                QuestionKind::UnresolvedCall,
                format!("call of {}", truncate(callee_text, 60)),
            );
        }
        let end_line = self.end_line(node);
        self.ir.calls.push(Call {
            id,
            line,
            col,
            end_line,
            callee,
            callee_pretty: pretty,
            kind,
            receiver,
            args,
            text: truncate(self.t(node), 80),
        });
        self.event(node, EventKind::Call(id));
        id
    }

    /// The receiver for a call of a method without one (`Foo()` in a method
    /// of the same class), if the callee is the function's class's.
    fn implicit_receiver(&self, recs: &[&SourceRec]) -> Option<Arg> {
        let class = self.class_prefix.as_ref()?;
        let rec = recs.first()?;
        let callee_class = match self.cx.lang {
            Lang::Cpp => rec.pretty.rsplit_once("::")?.0.to_string(),
            _ => rec.sym().rsplit_once('#')?.0.to_string(),
        };
        (callee_class == *class).then(|| Arg {
            vals: vec![Val::direct(Atom::Read(Place::this()))],
            place: Some(Place::this()),
            addr: false,
            text: "this".to_string(),
            line: 0,
            end_line: 0,
        })
    }

    // ## C++

    fn cpp_function(&mut self, node: Node<'c>) {
        if let Some(decl) = node.child_by_field_name("declarator") {
            let mut d = decl;
            // (Down to the function declarator.)
            while d.kind() != "function_declarator" {
                match d
                    .child_by_field_name("declarator")
                    .or_else(|| named(d).last().copied())
                {
                    Some(next) => d = next,
                    None => break,
                }
            }
            if let Some(params) = d.child_by_field_name("parameters") {
                for param in named(params) {
                    if let Some(ident) = self.param_ident(param) {
                        let ty = param
                            .child_by_field_name("type")
                            .map(|t| self.t(t).to_string());
                        self.add_param(ident, ty);
                    }
                }
            }
        }
        for child in named(node) {
            if child.kind() == "field_initializer_list" {
                for init in named(child) {
                    self.cpp_field_initializer(init);
                }
            }
        }
        if let Some(body) = node.child_by_field_name("body") {
            self.stmt(body);
        }
    }

    /// `: mFoo(aFoo)` in a constructor.
    fn cpp_field_initializer(&mut self, init: Node<'c>) {
        let children = named(init);
        let Some(name) = children.first().copied() else {
            return;
        };
        let name = cpp_name_node(name);
        let recs = self.recs(name);
        let args: Vec<Node> = children[1..].iter().flat_map(|n| named(*n)).collect();
        let vals = self.eval_union(args);
        if let Some(field) = recs.iter().find(|r| r.has_syntax("field")) {
            let sym = field.sym();
            self.name_sym(sym, field.short_name());
            self.assign(init, Place::this().with(Step::Field(sym)), vals, "init");
        }
    }

    fn cpp_stmt(&mut self, node: Node<'c>) {
        match node.kind() {
            "compound_statement" | "translation_unit" | "declaration_list" => self.stmts(node),
            "declaration" => {
                self.cpp_declaration(node);
            }
            "expression_statement" => {
                for child in named(node) {
                    self.eval(child);
                }
            }
            "return_statement" | "co_return_statement" => {
                let vals = self.eval_union(named(node));
                self.event(node, EventKind::Return { vals });
            }
            "if_statement" => {
                if let Some(cond) = node.child_by_field_name("condition") {
                    self.cpp_condition(cond);
                }
                if let Some(c) = node.child_by_field_name("consequence") {
                    self.stmt(c);
                }
                if let Some(alt) = node.child_by_field_name("alternative") {
                    self.stmts(alt);
                }
            }
            "while_statement" | "switch_statement" => {
                if let Some(cond) = node.child_by_field_name("condition") {
                    self.cpp_condition(cond);
                }
                if let Some(body) = node.child_by_field_name("body") {
                    self.stmt(body);
                }
            }
            "do_statement" => {
                if let Some(body) = node.child_by_field_name("body") {
                    self.stmt(body);
                }
                if let Some(cond) = node.child_by_field_name("condition") {
                    let vals = self.eval(cond).vals;
                    self.test(cond, vals);
                }
            }
            "for_statement" => {
                if let Some(init) = node.child_by_field_name("initializer") {
                    if init.kind() == "declaration" {
                        self.stmt(init);
                    } else {
                        self.eval(init);
                    }
                }
                if let Some(cond) = node.child_by_field_name("condition") {
                    let vals = self.eval(cond).vals;
                    self.test(cond, vals);
                }
                if let Some(update) = node.child_by_field_name("update") {
                    self.eval(update);
                }
                if let Some(body) = node.child_by_field_name("body") {
                    self.stmt(body);
                }
            }
            "for_range_loop" => {
                if let Some(init) = node.child_by_field_name("initializer") {
                    self.stmts(init);
                }
                let right = node
                    .child_by_field_name("right")
                    .map(|r| self.eval(r))
                    .unwrap_or_default();
                let vals = match right.place {
                    Some(place) => vec![Val::direct(Atom::Read(place.with(Step::Index)))],
                    None => derive(right.vals),
                };
                if let Some(decl) = node.child_by_field_name("declarator") {
                    self.cpp_declare_targets(decl, vals, node);
                }
                if let Some(body) = node.child_by_field_name("body") {
                    self.stmt(body);
                }
            }
            "case_statement" => {
                let value = node.child_by_field_name("value").map(|v| v.id());
                for child in named(node) {
                    if Some(child.id()) != value {
                        self.stmt(child);
                    }
                }
            }
            "labeled_statement" | "attributed_statement" | "else_clause" => {
                for child in named(node) {
                    if child.kind() != "statement_identifier" {
                        self.stmt(child);
                    }
                }
            }
            "try_statement" => {
                for child in named(node) {
                    match child.kind() {
                        "catch_clause" => {
                            if let Some(body) = child.child_by_field_name("body") {
                                self.stmt(body);
                            }
                        }
                        _ => self.stmt(child),
                    }
                }
            }
            "throw_statement" => {
                for child in named(node) {
                    self.eval(child);
                }
            }
            "macro_invocation" => {
                self.cpp_macro(node, node.child_by_field_name("name"), true);
            }
            "break_statement"
            | "continue_statement"
            | "goto_statement"
            | "type_definition"
            | "alias_declaration"
            | "using_declaration"
            | "static_assert_declaration"
            | "class_specifier"
            | "struct_specifier"
            | "enum_specifier"
            | "union_specifier"
            | "namespace_alias_definition"
            | "empty_statement"
            | "preproc_call" => {}
            "function_definition" => {
                // (A local class's methods.)
                self.nested.push(node);
            }
            "ERROR" => {
                self.question(node, QuestionKind::Unparsed, truncate(self.t(node), 60));
                for child in named(node) {
                    self.stmt(child);
                }
            }
            _ => {
                self.eval(node);
            }
        }
    }

    /// An `if`/`while`/`switch` condition clause: a declaration or an
    /// expression, which is tested.
    fn cpp_condition(&mut self, cond: Node<'c>) {
        if cond.kind() != "condition_clause" {
            let vals = self.eval(cond).vals;
            self.test(cond, vals);
            return;
        }
        if let Some(init) = cond.child_by_field_name("initializer") {
            self.stmts(init);
        }
        if let Some(value) = cond.child_by_field_name("value") {
            if value.kind() == "declaration" {
                let places = self.cpp_declaration(value);
                let vals = places
                    .into_iter()
                    .map(|p| Val::direct(Atom::Read(p)))
                    .collect();
                self.test(value, vals);
            } else {
                let vals = self.eval(value).vals;
                self.test(value, vals);
            }
        }
    }

    /// A declaration's initializations: returns the places declared.
    fn cpp_declaration(&mut self, node: Node<'c>) -> Vec<Place> {
        let mut places = vec![];
        let mut cursor = node.walk();
        let declarators: Vec<Node> = node
            .children_by_field_name("declarator", &mut cursor)
            .collect();
        for decl in declarators {
            if decl.kind() == "init_declarator" {
                let vals = match decl.child_by_field_name("value") {
                    Some(value) => match value.kind() {
                        // `T x(a, b);`, `T x{a, b};`: a constructor's
                        // arguments.
                        "argument_list" | "initializer_list" => {
                            let args = named(value);
                            let ctor_recs: Vec<&SourceRec> = cpp_declarator_ident(decl)
                                .map(|ident| self.recs(ident))
                                .unwrap_or_default()
                                .into_iter()
                                .filter(|r| r.has_syntax("constructor"))
                                .collect();
                            if ctor_recs.is_empty() {
                                self.eval_union(args)
                            } else {
                                let args: Vec<Arg> =
                                    args.into_iter().map(|a| self.arg(a)).collect();
                                let id = self.push_call(
                                    value,
                                    &ctor_recs,
                                    "constructor",
                                    CallKind::Construct,
                                    None,
                                    args,
                                );
                                vec![Val::direct(Atom::Call(id))]
                            }
                        }
                        _ => self.eval(value).vals,
                    },
                    None => vec![],
                };
                if let Some(place) = self.cpp_declare_targets(decl, vals, decl) {
                    places.push(place);
                }
            } else if decl.kind() == "function_declarator"
                && let Some(ident) = cpp_declarator_ident(decl)
                && let Some(place) = self.cpp_vexing_declaration(decl, ident)
            {
                places.push(place);
            } else if let Some(ident) = cpp_declarator_ident(decl) {
                places.push(self.declare_local(ident));
            } else if decl.kind() == "structured_binding_declarator" {
                for ident in named(decl) {
                    places.push(self.declare_local(ident));
                }
            }
        }
        // (`T x = v;` is sometimes a declaration with a `value` field.)
        if let Some(value) = node.child_by_field_name("value")
            && let Some(place) = places.last().cloned()
        {
            let vals = self.eval(value).vals;
            self.assign(node, place, vals, "init");
        }
        places
    }

    /// `T x(y);`, which parses as a function's declaration (C++'s "most
    /// vexing parse"), when the records say it's a variable constructed from
    /// the variables the parameters' types name: the local, initialized by
    /// the constructor's call.
    fn cpp_vexing_declaration(&mut self, decl: Node<'c>, ident: Node<'c>) -> Option<Place> {
        let recs = self.recs(ident);
        if !recs.iter().any(|r| r.sym().starts_with("V_")) {
            return None;
        }
        let ctor_recs: Vec<&SourceRec> = recs
            .into_iter()
            .filter(|r| r.has_syntax("constructor"))
            .collect();
        if ctor_recs.is_empty() {
            return None;
        }
        let params = decl.child_by_field_name("parameters")?;
        let mut args = vec![];
        for param in named(params) {
            if param.kind() != "parameter_declaration"
                || param.child_by_field_name("declarator").is_some()
            {
                return None;
            }
            let name = param.child_by_field_name("type")?;
            let ev = self.cpp_ident(name);
            args.push(Arg {
                vals: ev.vals,
                place: ev.place,
                addr: ev.addr,
                text: truncate(self.t(name), 60),
                line: self.pos(name).0,
                end_line: self.end_line(name),
            });
        }
        let id = self.push_call(
            params,
            &ctor_recs,
            "constructor",
            CallKind::Construct,
            None,
            args,
        );
        let place = self.declare_local(ident);
        self.assign(
            decl,
            place.clone(),
            vec![Val::direct(Atom::Call(id))],
            "init",
        );
        Some(place)
    }

    /// Declare what a declarator declares, initialized with `vals`.
    fn cpp_declare_targets(&mut self, decl: Node<'c>, vals: Vec<Val>, at: Node) -> Option<Place> {
        let target = match decl.kind() {
            "init_declarator" => decl.child_by_field_name("declarator")?,
            _ => decl,
        };
        if target.kind() == "structured_binding_declarator" {
            for ident in named(target) {
                let place = self.declare_local(ident);
                self.assign(at, place, derive(vals.clone()), "init");
            }
            return None;
        }
        let ident = cpp_declarator_ident(target)?;
        let place = self.declare_local(ident);
        if !vals.is_empty() {
            self.assign(at, place.clone(), vals, "init");
        }
        Some(place)
    }

    fn cpp_ident(&mut self, node: Node<'c>) -> Ev {
        let text = self.t(node).to_string();
        let recs = self.recs(node);
        for rec in &recs {
            let sym = rec.sym();
            if sym.is_empty() {
                continue;
            }
            if sym.starts_with("V_") {
                self.name_sym(sym, &text);
                if let Some(ty) = &rec.type_pretty {
                    self.ir.types.entry(sym).or_insert_with(|| ty.clone());
                }
                return Ev::place(Place::local(sym));
            }
            if rec.has_syntax("field") {
                self.name_sym(sym, rec.short_name());
                return Ev::place(Place::this().with(Step::Field(sym)));
            }
            if rec.has_syntax("variable") {
                let constant = rec
                    .type_pretty
                    .as_deref()
                    .is_some_and(|t| t.starts_with("const ") && !t.contains('*'));
                if constant {
                    return Ev::konst(&text);
                }
                self.name_sym(sym, rec.short_name());
                return Ev::place(Place::global(sym));
            }
            if rec.has_syntax("enum") || rec.has_syntax("function") {
                return Ev::konst(&text);
            }
        }
        match text.as_str() {
            "true" | "false" | "nullptr" | "NULL" => Ev::konst(&text),
            _ => Ev::place(self.unknown_name(node)),
        }
    }

    fn cpp_eval(&mut self, node: Node<'c>) -> Ev {
        match node.kind() {
            "parenthesized_expression" => {
                let children = named(node);
                match children.as_slice() {
                    // A GNU statement expression: `({ ...; value; })`.
                    [block] if block.kind() == "compound_statement" => {
                        self.statement_expression(*block)
                    }
                    [inner] => self.eval(*inner),
                    _ => Ev::vals(self.eval_union(children)),
                }
            }
            "identifier" => self.cpp_ident(node),
            "qualified_identifier" => {
                let name = cpp_name_node(node);
                if name.id() == node.id() {
                    Ev::konst(self.t(node))
                } else {
                    let ev = self.cpp_ident(name);
                    match &ev.place {
                        // (A qualified name that isn't a member or a local
                        // isn't a local: `Foo::sBar`.)
                        Some(p) if matches!(p.root, Root::Local(s) if s.starts_with("name:")) => {
                            Ev::konst(self.t(node))
                        }
                        _ => ev,
                    }
                }
            }
            "this" => Ev::addr(Place::this()),
            "true"
            | "false"
            | "null"
            | "nullptr"
            | "number_literal"
            | "string_literal"
            | "char_literal"
            | "concatenated_string"
            | "raw_string_literal"
            | "user_defined_literal"
            | "sizeof_expression"
            | "alignof_expression"
            | "offsetof_expression"
            | "template_function"
            | "template_method"
            | "requires_expression" => Ev::konst(self.t(node)),
            "field_expression" => self.cpp_field(node),
            "pointer_expression" => {
                let op = node
                    .child_by_field_name("operator")
                    .map(|o| o.kind())
                    .unwrap_or("");
                let Some(arg) = node.child_by_field_name("argument") else {
                    return Ev::default();
                };
                let ev = self.eval(arg);
                match (op, ev.place) {
                    ("*", Some(place)) if ev.addr => Ev::place(place),
                    ("*", Some(place)) => Ev::place(place.with(Step::Deref)),
                    ("&", Some(place)) if !ev.addr => Ev::addr(place),
                    (_, _) => Ev::vals(derive(ev.vals)),
                }
            }
            "subscript_expression" => {
                if let Some(indices) = node.child_by_field_name("indices") {
                    for index in named(indices) {
                        self.eval(index);
                    }
                }
                let Some(arg) = node.child_by_field_name("argument") else {
                    return Ev::default();
                };
                let ev = self.eval(arg);
                match ev.place {
                    Some(place) => Ev::place(place.with(Step::Index)),
                    None => Ev::vals(ev.vals),
                }
            }
            "call_expression" => self.cpp_call(node),
            "new_expression" => {
                let args = node
                    .child_by_field_name("arguments")
                    .map(named)
                    .unwrap_or_default();
                let args: Vec<Arg> = args.into_iter().map(|a| self.arg(a)).collect();
                let ty = node.child_by_field_name("type");
                let recs: Vec<&SourceRec> = ty
                    .map(|t| self.recs(t))
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|r| r.has_syntax("constructor"))
                    .collect();
                let text = ty.map(|t| self.t(t).to_string()).unwrap_or_default();
                let id = self.push_call(node, &recs, &text, CallKind::Construct, None, args);
                Ev::vals(vec![Val::direct(Atom::Call(id))])
            }
            "assignment_expression" => {
                let op = node
                    .child_by_field_name("operator")
                    .map(|o| o.kind().to_string())
                    .unwrap_or_else(|| "=".to_string());
                let right = node
                    .child_by_field_name("right")
                    .map(|r| self.eval(r).vals)
                    .unwrap_or_default();
                let left = node
                    .child_by_field_name("left")
                    .map(|l| self.eval(l))
                    .unwrap_or_default();
                // (`x.f() = v`: an assignment to the reference an accessor
                // returns, ex: an IPDL struct's field's mutable one, as one to
                // the part of `x` named by the accessor's symbol.)
                let through = match left.vals.as_slice() {
                    [
                        Val {
                            atom: Atom::Call(id),
                            derived: false,
                        },
                    ] if left.place.is_none() => {
                        let call = &self.ir.calls[*id];
                        match (&call.receiver, call.callee.first()) {
                            (
                                Some(Arg {
                                    place: Some(receiver),
                                    addr: false,
                                    ..
                                }),
                                Some(sym),
                            ) if call.args.is_empty() && call.kind == CallKind::Call => {
                                Some(receiver.with(Step::Field(*sym)))
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                };
                match left.place.filter(|_| !left.addr).or(through) {
                    Some(place) => {
                        let vals = if op == "=" {
                            right
                        } else {
                            let mut vals = derive(right);
                            vals.push(Val {
                                atom: Atom::Read(place.clone()),
                                derived: true,
                            });
                            vals
                        };
                        self.assign(node, place.clone(), vals, &op);
                        Ev::place(place)
                    }
                    _ => {
                        self.question(
                            node,
                            QuestionKind::Unparsed,
                            format!("assignment to {}", truncate(self.t(node), 60)),
                        );
                        Ev::vals(right)
                    }
                }
            }
            "update_expression" => {
                let Some(arg) = node.child_by_field_name("argument") else {
                    return Ev::default();
                };
                let ev = self.eval(arg);
                let op = node
                    .child_by_field_name("operator")
                    .map(|o| o.kind().to_string())
                    .unwrap_or_default();
                if let Some(place) = ev.place.clone() {
                    let vals = derive(ev.vals.clone());
                    self.assign(node, place, vals, &op);
                }
                ev
            }
            "binary_expression" => {
                let children: Vec<Node> = ["left", "right"]
                    .iter()
                    .filter_map(|f| node.child_by_field_name(f))
                    .collect();
                let mut vals = vec![];
                for child in children {
                    vals.extend(self.eval(child).vals);
                }
                Ev::vals(derive(vals))
            }
            "unary_expression" => {
                let vals = node
                    .child_by_field_name("argument")
                    .map(|a| self.eval(a).vals)
                    .unwrap_or_default();
                Ev::vals(derive(vals))
            }
            "conditional_expression" => {
                if let Some(cond) = node.child_by_field_name("condition") {
                    let vals = self.eval(cond).vals;
                    self.test(cond, vals);
                }
                let mut vals = vec![];
                for f in ["consequence", "alternative"] {
                    if let Some(child) = node.child_by_field_name(f) {
                        vals.extend(self.eval(child).vals);
                    }
                }
                Ev::vals(vals)
            }
            "comma_expression" => {
                if let Some(left) = node.child_by_field_name("left") {
                    self.eval(left);
                }
                node.child_by_field_name("right")
                    .map(|r| self.eval(r))
                    .unwrap_or_default()
            }
            "cast_expression" => node
                .child_by_field_name("value")
                .map(|v| self.eval(v))
                .unwrap_or_default(),
            "co_await_expression" => node
                .child_by_field_name("argument")
                .map(|a| self.eval(a))
                .unwrap_or_default(),
            "delete_expression" => {
                for child in named(node) {
                    self.eval(child);
                }
                Ev::default()
            }
            "lambda_expression" => {
                let params = node
                    .child_by_field_name("declarator")
                    .and_then(|d| d.child_by_field_name("parameters"));
                if let Some(body) = node.child_by_field_name("body") {
                    self.lambda_body(body, params);
                }
                Ev::konst(&format!("lambda@{}", self.pos(node).0))
            }
            "initializer_list" | "compound_literal_expression" | "argument_list" => {
                let mut children = named(node);
                if node.kind() == "compound_literal_expression" {
                    children = node
                        .child_by_field_name("value")
                        .map(named)
                        .unwrap_or_default();
                }
                Ev::vals(self.eval_union(children))
            }
            "initializer_pair" => node
                .child_by_field_name("value")
                .map(|v| self.eval(v))
                .unwrap_or_default(),
            "macro_invocation" => {
                Ev::vals(self.cpp_macro(node, node.child_by_field_name("name"), false))
            }
            "ERROR" => {
                self.question(node, QuestionKind::Unparsed, truncate(self.t(node), 60));
                Ev::vals(self.eval_union(named(node)))
            }
            _ => Ev::vals(self.eval_union(named(node))),
        }
    }

    /// `({ ...; value; })`: its statements, and its last expression's value.
    fn statement_expression(&mut self, block: Node<'c>) -> Ev {
        let children = named(block);
        let Some((last, rest)) = children.split_last() else {
            return Ev::default();
        };
        for child in rest {
            self.stmt(*child);
        }
        if last.kind() == "expression_statement" {
            match named(*last).first() {
                Some(expr) => self.eval(*expr),
                None => Ev::default(),
            }
        } else {
            self.stmt(*last);
            Ev::default()
        }
    }

    /// `a.b`, `a->b`: the place, if `a` is one.
    fn cpp_field(&mut self, node: Node<'c>) -> Ev {
        let Some(arg) = node.child_by_field_name("argument") else {
            return Ev::default();
        };
        let arrow = node
            .child_by_field_name("operator")
            .is_some_and(|o| o.kind() == "->");
        let base = self.eval(arg);
        let Some(field) = node.child_by_field_name("field") else {
            return Ev::vals(base.vals);
        };
        let field_name = cpp_name_node(field);
        let recs = self.recs(field_name);
        let step = match recs.iter().find(|r| !r.syms.is_empty()) {
            Some(rec) => {
                let sym = rec.sym();
                self.name_sym(sym, rec.short_name());
                Step::Field(sym)
            }
            None => Step::Field(ustr(&format!("name:{}", self.t(field_name)))),
        };
        let base_place = match (base.place, arrow) {
            // (`this->x`, `(&y)->x`.)
            (Some(place), true) if base.addr => Some(place),
            (Some(place), true) => Some(place.with(Step::Deref)),
            (Some(place), false) => Some(place),
            (None, _) => None,
        };
        match base_place {
            Some(place) => Ev::place(place.with(step)),
            None => Ev::vals(derive(base.vals)),
        }
    }

    fn cpp_call(&mut self, node: Node<'c>) -> Ev {
        let Some(function) = node.child_by_field_name("function") else {
            return Ev::default();
        };
        let arg_nodes = node
            .child_by_field_name("arguments")
            .map(named)
            .unwrap_or_default();
        let (name_node, receiver) = match function.kind() {
            "field_expression" => {
                let arrow = function
                    .child_by_field_name("operator")
                    .is_some_and(|o| o.kind() == "->");
                let receiver = function.child_by_field_name("argument").map(|a| {
                    let mut arg = self.arg(a);
                    if arrow {
                        match (&arg.place, arg.addr) {
                            (Some(_), true) => arg.addr = false,
                            (Some(place), false) => {
                                let deref = place.with(Step::Deref);
                                arg.vals = vec![Val::direct(Atom::Read(deref.clone()))];
                                arg.place = Some(deref);
                            }
                            _ => {}
                        }
                    }
                    arg
                });
                (
                    function.child_by_field_name("field").map(cpp_name_node),
                    receiver,
                )
            }
            "identifier" | "qualified_identifier" | "template_function" => {
                (Some(cpp_name_node(function)), None)
            }
            // (Functional casts: `int32_t(x)`, `nsDependentCString(s)`.)
            "primitive_type" | "type_identifier" | "template_type" | "sized_type_specifier" => {
                let ctor_recs: Vec<&SourceRec> = self
                    .recs(function)
                    .into_iter()
                    .filter(|r| r.has_syntax("constructor"))
                    .collect();
                if ctor_recs.is_empty() {
                    return match arg_nodes.as_slice() {
                        [arg] => self.eval(*arg),
                        _ => Ev::vals(derive(self.eval_union(arg_nodes))),
                    };
                }
                let args: Vec<Arg> = arg_nodes.into_iter().map(|a| self.arg(a)).collect();
                let text = self.t(function).to_string();
                let id = self.push_call(node, &ctor_recs, &text, CallKind::Construct, None, args);
                return Ev::vals(vec![Val::direct(Atom::Call(id))]);
            }
            _ => {
                // (Calls of call results, lambdas, function pointers.)
                let callee = self.eval(function);
                let args: Vec<Arg> = arg_nodes.into_iter().map(|a| self.arg(a)).collect();
                let receiver = Arg {
                    vals: callee.vals,
                    place: callee.place,
                    addr: false,
                    text: truncate(self.t(function), 60),
                    line: self.pos(function).0,
                    end_line: self.end_line(function),
                };
                let id = self.push_call(
                    node,
                    &[],
                    self.t(function).to_string().as_str(),
                    CallKind::Call,
                    Some(receiver),
                    args,
                );
                return Ev::vals(vec![Val::direct(Atom::Call(id))]);
            }
        };
        let Some(name_node) = name_node else {
            return Ev::vals(self.eval_union(arg_nodes));
        };
        let name = self.t(name_node).to_string();
        let recs = self.recs(name_node);
        if recs.iter().any(|r| r.has_syntax("macro")) {
            return Ev::vals(self.cpp_macro(node, Some(name_node), false));
        }
        // (`static_cast<T>(x)`'s name is in a template function.)
        let base_name = match function.kind() {
            "template_function" => function
                .child_by_field_name("name")
                .map(|n| self.t(n).to_string())
                .unwrap_or_default(),
            _ => name.clone(),
        };
        if IDENTITY_CALLEES.contains(&base_name.as_str()) && arg_nodes.len() == 1 {
            return self.eval(arg_nodes[0]);
        }
        let callee_recs: Vec<&SourceRec> = recs
            .iter()
            .copied()
            .filter(|r| r.has_syntax("function") || r.has_syntax("constructor"))
            .collect();
        // (A functional cast to a class without a constructor of its own:
        // `Key(aKey)`.)
        if callee_recs.is_empty() && recs.iter().any(|r| r.has_syntax("type")) {
            return match arg_nodes.as_slice() {
                [arg] => self.eval(*arg),
                _ => Ev::vals(derive(self.eval_union(arg_nodes))),
            };
        }
        let kind = if callee_recs.iter().any(|r| r.has_syntax("constructor")) {
            CallKind::Construct
        } else {
            CallKind::Call
        };
        let receiver = receiver.or_else(|| self.implicit_receiver(&callee_recs));
        let args: Vec<Arg> = arg_nodes.into_iter().map(|a| self.arg(a)).collect();
        let id = self.push_call(node, &callee_recs, &name, kind, receiver, args);
        Ev::vals(vec![Val::direct(Atom::Call(id))])
    }

    /// A macro invocation: analyze its expansion if the records have one,
    /// else treat it as an opaque call of its arguments.
    fn cpp_macro(&mut self, node: Node<'c>, name: Option<Node<'c>>, is_stmt: bool) -> Vec<Val> {
        let name_text = name.map(|n| self.t(n).to_string()).unwrap_or_default();
        if ASSERTION_MACROS.contains(&name_text.as_str()) {
            let args = node
                .child_by_field_name("arguments")
                .map(named)
                .unwrap_or_default();
            let args: Vec<Node> = args
                .into_iter()
                .flat_map(|a| {
                    if a.kind() == "macro_arguments" {
                        named(a)
                    } else {
                        vec![a]
                    }
                })
                .filter(|a| !matches!(a.kind(), "string_literal" | "concatenated_string"))
                .collect();
            let mut vals = vec![];
            for arg in args {
                vals.extend(self.eval(arg).vals);
            }
            let saved = self.macro_cx.take();
            let (line, col) = self.pos(node);
            let end_line = self.end_line(node);
            self.macro_cx = saved;
            if !vals.is_empty() {
                self.ir.events.push(Event {
                    line,
                    col,
                    end_line,
                    kind: EventKind::Test { vals },
                    in_lambda: self.lambda_depth > 0,
                    macro_name: Some(name_text),
                    text: self.cx.line_text(line),
                });
            }
            return vec![];
        }
        let expansion = name.and_then(|n| {
            if self.macro_cx.is_some() {
                return None;
            }
            let recs = self.recs(n);
            let rec = recs.iter().find(|r| !r.expands_to.is_empty())?;
            let mut options: Vec<&(String, String, String)> = rec.expands_to.iter().collect();
            options.sort_by_key(|(key, platform, _)| {
                let rank = match platform.as_str() {
                    "linux64" => 0,
                    "linux64-opt" => 1,
                    _ => 2,
                };
                (rank, platform.clone(), key.clone())
            });
            let (key, platform, text) = options.first()?;
            let (line, col) = self.pos(n);
            Some((key.clone(), platform.clone(), text.clone(), line, col))
        });
        // (Macros called like functions in statements, ex: `NS_ENSURE_ARG(x);`,
        // often expand to statements, which aren't expressions.)
        let is_stmt = is_stmt
            || node
                .parent()
                .is_some_and(|p| p.kind() == "expression_statement")
            || expansion.as_ref().is_some_and(|(_, _, text, _, _)| {
                let text = text.trim_start();
                [
                    "do ", "do{", "{", "if ", "if(", "while", "for ", "for(", "switch", "return",
                ]
                .iter()
                .any(|p| text.starts_with(p))
            });
        let Some((key, platform, expansion, line, col)) = expansion else {
            // (Unexpanded: its arguments are used, and it might do anything
            // with them.)
            let args = node
                .child_by_field_name("arguments")
                .map(named)
                .unwrap_or_default();
            let args: Vec<Node> = args
                .into_iter()
                .flat_map(|a| {
                    if a.kind() == "macro_arguments" {
                        named(a)
                    } else {
                        vec![a]
                    }
                })
                .collect();
            let args: Vec<Arg> = args.into_iter().map(|a| self.arg(a)).collect();
            if self.macro_cx.is_none() {
                self.question(
                    node,
                    QuestionKind::Macro,
                    format!("macro {} without an expansion", name_text),
                );
            }
            let id = self.push_call(node, &[], &name_text, CallKind::Macro, None, args);
            return vec![Val::direct(Atom::Call(id))];
        };
        let (prefix, suffix) = if is_stmt {
            ("void __dataflow_macro() {\n", ";\n}\n")
        } else {
            ("void __dataflow_macro() {\n(", ");\n}\n")
        };
        let wrapped = format!("{}{}{}", prefix, expansion, suffix);
        let Ok(parse) = parse_with_profile_for_analysis(profile_for_lang("cpp").unwrap(), &wrapped)
        else {
            return vec![];
        };
        let saved_text = std::mem::replace(&mut self.text, std::rc::Rc::from(parse.text.as_str()));
        self.macro_cx = Some(MacroCx {
            line,
            col,
            key,
            platform,
            prefix: prefix.len(),
            name: name_text,
        });
        let mut vals = vec![];
        // SAFETY of lifetimes: the expansion's nodes don't outlive this
        // block, but `Builder` is parameterized by the file tree's lifetime,
        // so walk them with a builder-agnostic helper.
        let root = parse.tree.root_node();
        let body = find_kind(root, "compound_statement");
        if let Some(body) = body {
            vals = self.walk_expansion(body, is_stmt);
        }
        self.macro_cx = None;
        self.text = saved_text;
        vals
    }

    fn walk_expansion(&mut self, body: Node, is_stmt: bool) -> Vec<Val> {
        // Nodes of the expansion's tree have a shorter lifetime than 'c; a
        // sub-builder borrowing our state walks them.
        let mut sub = Builder {
            cx: self.cx,
            text: self.text.clone(),
            ir: std::mem::replace(&mut self.ir, empty_ir()),
            lambda_depth: self.lambda_depth,
            macro_cx: self.macro_cx.take(),
            scope: std::mem::take(&mut self.scope),
            class_prefix: self.class_prefix.clone(),
            nested: vec![],
        };
        let vals = if is_stmt {
            sub.stmt(body);
            vec![]
        } else {
            match named(body).first() {
                Some(stmt) if stmt.kind() == "expression_statement" => match named(*stmt).first() {
                    Some(expr) => sub.eval(*expr).vals,
                    None => vec![],
                },
                _ => {
                    sub.stmt(body);
                    vec![]
                }
            }
        };
        self.text = sub.text;
        self.ir = sub.ir;
        self.macro_cx = sub.macro_cx;
        self.scope = sub.scope;
        vals
    }

    // ## JS

    fn js_function(&mut self, node: Node<'c>) {
        self.ir.has_body = node.child_by_field_name("body").is_some();
        if let Some(params) = node.child_by_field_name("parameters") {
            for param in named(params) {
                match self.param_ident(param) {
                    Some(ident) => self.add_param(ident, None),
                    None => {
                        // (A destructuring parameter: its names are locals.)
                        if let Some(pattern) = param.child_by_field_name("pattern") {
                            self.js_pattern(pattern, vec![], None, param);
                        }
                    }
                }
            }
        } else if let Some(param) = node.child_by_field_name("parameter") {
            self.add_param(param, None);
        }
        if let Some(body) = node.child_by_field_name("body") {
            if body.kind() == "statement_block" {
                self.stmt(body);
            } else {
                let vals = self.eval(body).vals;
                self.event(body, EventKind::Return { vals });
            }
        }
    }

    fn js_stmt(&mut self, node: Node<'c>) {
        match node.kind() {
            "statement_block" | "program" => self.stmts(node),
            "lexical_declaration" | "variable_declaration" => {
                for decl in named(node) {
                    if decl.kind() != "variable_declarator" {
                        continue;
                    }
                    let Some(name) = decl.child_by_field_name("name") else {
                        continue;
                    };
                    let value = decl.child_by_field_name("value");
                    if let Some(value) = value
                        && matches!(
                            value.kind(),
                            "arrow_function" | "function_expression" | "function"
                        )
                        && self.cx.function_sym(value, &self.text).is_some()
                    {
                        self.nested.push(value);
                        continue;
                    }
                    let ev = value.map(|v| self.eval(v)).unwrap_or_default();
                    self.js_pattern(name, ev.vals, ev.place, decl);
                }
            }
            "expression_statement" => {
                for child in named(node) {
                    self.eval(child);
                }
            }
            "return_statement" => {
                let vals = self.eval_union(named(node));
                self.event(node, EventKind::Return { vals });
            }
            "if_statement" => {
                if let Some(cond) = node.child_by_field_name("condition") {
                    let vals = self.eval(cond).vals;
                    self.test(cond, vals);
                }
                if let Some(c) = node.child_by_field_name("consequence") {
                    self.stmt(c);
                }
                if let Some(alt) = node.child_by_field_name("alternative") {
                    self.stmts(alt);
                }
            }
            "while_statement" | "do_statement" => {
                if let Some(cond) = node.child_by_field_name("condition") {
                    let vals = self.eval(cond).vals;
                    self.test(cond, vals);
                }
                if let Some(body) = node.child_by_field_name("body") {
                    self.stmt(body);
                }
            }
            "for_statement" => {
                if let Some(init) = node.child_by_field_name("initializer") {
                    self.stmt(init);
                }
                if let Some(cond) = node.child_by_field_name("condition") {
                    let vals = self.eval(cond).vals;
                    self.test(cond, vals);
                }
                if let Some(inc) = node.child_by_field_name("increment") {
                    self.eval(inc);
                }
                if let Some(body) = node.child_by_field_name("body") {
                    self.stmt(body);
                }
            }
            "for_in_statement" => {
                let right = node
                    .child_by_field_name("right")
                    .map(|r| self.eval(r))
                    .unwrap_or_default();
                let of = node
                    .child_by_field_name("operator")
                    .is_some_and(|o| o.kind() == "of");
                let vals = match (right.place, of) {
                    (Some(place), true) => vec![Val::direct(Atom::Read(place.with(Step::Index)))],
                    (_, _) => derive(right.vals),
                };
                if let Some(left) = node.child_by_field_name("left") {
                    self.js_pattern(left, vals, None, node);
                }
                if let Some(body) = node.child_by_field_name("body") {
                    self.stmt(body);
                }
            }
            "switch_statement" => {
                if let Some(value) = node.child_by_field_name("value") {
                    let vals = self.eval(value).vals;
                    self.test(value, vals);
                }
                if let Some(body) = node.child_by_field_name("body") {
                    for case in named(body) {
                        let value = case.child_by_field_name("value").map(|v| v.id());
                        for child in named(case) {
                            if Some(child.id()) != value {
                                self.stmt(child);
                            }
                        }
                    }
                }
            }
            "try_statement" => {
                for child in named(node) {
                    match child.kind() {
                        "catch_clause" => {
                            if let Some(param) = child.child_by_field_name("parameter") {
                                self.js_pattern(param, vec![], None, child);
                            }
                            if let Some(body) = child.child_by_field_name("body") {
                                self.stmt(body);
                            }
                        }
                        "finally_clause" => self.stmts(child),
                        _ => self.stmt(child),
                    }
                }
            }
            "labeled_statement" | "else_clause" => {
                for child in named(node) {
                    if child.kind() != "statement_identifier" {
                        self.stmt(child);
                    }
                }
            }
            "throw_statement" => {
                for child in named(node) {
                    self.eval(child);
                }
            }
            "function_declaration"
            | "generator_function_declaration"
            | "class_declaration"
            | "class" => {
                self.nested.push(node);
            }
            "break_statement" | "continue_statement" | "empty_statement" | "debugger_statement"
            | "import_statement" | "export_statement" => {}
            "ERROR" => {
                self.question(node, QuestionKind::Unparsed, truncate(self.t(node), 60));
                for child in named(node) {
                    self.stmt(child);
                }
            }
            _ => {
                self.eval(node);
            }
        }
    }

    /// Assign `vals` (from `base`, if it's a place) to a binding pattern.
    fn js_pattern(&mut self, pattern: Node<'c>, vals: Vec<Val>, base: Option<Place>, at: Node) {
        match pattern.kind() {
            "identifier" | "shorthand_property_identifier_pattern" => {
                let place = self.declare_or_lookup(pattern);
                if pattern.kind() == "shorthand_property_identifier_pattern" {
                    // (`{ a } = b`: `a` is `b.a`.)
                    let vals = match &base {
                        Some(base) => vec![Val::direct(Atom::Read(
                            base.with(Step::Field(ustr(&format!("name:{}", self.t(pattern))))),
                        ))],
                        None => derive(vals),
                    };
                    self.assign(at, place, vals, "init");
                } else {
                    self.assign(at, place, vals, "init");
                }
            }
            "object_pattern" => {
                for child in named(pattern) {
                    match child.kind() {
                        "shorthand_property_identifier_pattern" => {
                            self.js_pattern(child, vals.clone(), base.clone(), at)
                        }
                        "pair_pattern" => {
                            let key = child
                                .child_by_field_name("key")
                                .map(|k| self.t(k).to_string())
                                .unwrap_or_default();
                            let sub_base = base
                                .as_ref()
                                .map(|b| b.with(Step::Field(ustr(&format!("name:{}", key)))));
                            let sub_vals = match &sub_base {
                                Some(p) => vec![Val::direct(Atom::Read(p.clone()))],
                                None => derive(vals.clone()),
                            };
                            if let Some(value) = child.child_by_field_name("value") {
                                self.js_pattern(value, sub_vals, sub_base, at);
                            }
                        }
                        "object_assignment_pattern" | "assignment_pattern" => {
                            let mut sub_vals = vals.clone();
                            if let Some(right) = child.child_by_field_name("right") {
                                sub_vals.extend(self.eval(right).vals);
                            }
                            if let Some(left) = child.child_by_field_name("left") {
                                self.js_pattern(left, sub_vals, base.clone(), at);
                            }
                        }
                        _ => {
                            for ident in named(child) {
                                self.js_pattern(ident, derive(vals.clone()), None, at);
                            }
                        }
                    }
                }
            }
            "array_pattern" => {
                let element = base.as_ref().map(|b| b.with(Step::Index));
                let sub_vals = match &element {
                    Some(p) => vec![Val::direct(Atom::Read(p.clone()))],
                    None => derive(vals.clone()),
                };
                for child in named(pattern) {
                    self.js_pattern(child, sub_vals.clone(), element.clone(), at);
                }
            }
            "assignment_pattern" => {
                let mut sub_vals = vals;
                if let Some(right) = pattern.child_by_field_name("right") {
                    sub_vals.extend(self.eval(right).vals);
                }
                if let Some(left) = pattern.child_by_field_name("left") {
                    self.js_pattern(left, sub_vals, base, at);
                }
            }
            "rest_pattern" => {
                for child in named(pattern) {
                    self.js_pattern(child, derive(vals.clone()), None, at);
                }
            }
            _ => {
                // (`a.b = ...` in `for (a.b of ...)`, etc.)
                let ev = self.eval(pattern);
                if let Some(place) = ev.place {
                    self.assign(at, place, vals, "=");
                }
            }
        }
    }

    /// A JS or Kotlin name being declared (or assigned): its place.
    fn declare_or_lookup(&mut self, node: Node) -> Place {
        let recs = self.recs(node);
        if let Some(rec) = recs.iter().find(|r| !r.syms.is_empty()) {
            let sym = rec.sym();
            let name = self.t(node).to_string();
            if is_scip_local(&sym) {
                self.name_sym(sym, &name);
                self.scope.insert(name, Place::local(sym));
                return Place::local(sym);
            }
            self.name_sym(sym, rec.short_name());
            return Place::global(sym);
        }
        self.declare_local(node)
    }

    /// A JS or Kotlin name being read.
    fn scip_ident(&mut self, node: Node<'c>) -> Ev {
        let text = self.t(node).to_string();
        let recs = self.recs(node);
        for rec in &recs {
            let sym = rec.sym();
            if sym.is_empty() {
                continue;
            }
            if is_scip_local(&sym) {
                self.name_sym(sym, &text);
                return Ev::place(Place::local(sym));
            }
            if rec.has_syntax("field") || rec.has_syntax("var") || rec.has_syntax("property") {
                self.name_sym(sym, rec.short_name());
                // (A member of the function's class, without `this.`, is
                // `this`'s, in Kotlin.)
                if self.cx.lang == Lang::Kotlin
                    && let Some(class) = &self.class_prefix
                    && sym.rsplit_once('#').is_some_and(|(c, _)| c == class)
                {
                    return Ev::place(Place::this().with(Step::Field(sym)));
                }
                return Ev::place(Place::global(sym));
            }
            if rec.has_syntax("method") || rec.has_syntax("function") || rec.has_syntax("class") {
                return Ev::konst(&text);
            }
        }
        match text.as_str() {
            "undefined" | "null" | "true" | "false" | "NaN" | "Infinity" => Ev::konst(&text),
            _ => Ev::place(self.unknown_name(node)),
        }
    }

    /// The field step for a property name node.
    fn member_step(&mut self, prop: Node) -> Step {
        let recs = self.recs(prop);
        match recs
            .iter()
            .find(|r| !r.syms.is_empty() && (r.has_syntax("field") || r.has_syntax("property")))
            .or_else(|| recs.iter().find(|r| !r.syms.is_empty()))
        {
            Some(rec) => {
                let sym = rec.sym();
                self.name_sym(sym, rec.short_name());
                Step::Field(sym)
            }
            None => Step::Field(ustr(&format!("name:{}", self.t(prop)))),
        }
    }

    fn js_eval(&mut self, node: Node<'c>) -> Ev {
        match node.kind() {
            "parenthesized_expression"
            | "non_null_expression"
            | "await_expression"
            | "spread_element"
            | "yield_expression" => match named(node).first() {
                Some(inner) => self.eval(*inner),
                None => Ev::default(),
            },
            "as_expression" | "satisfies_expression" | "type_assertion" => {
                match named(node).first() {
                    Some(inner) => self.eval(*inner),
                    None => Ev::default(),
                }
            }
            "identifier" | "shorthand_property_identifier" => self.scip_ident(node),
            "this" => Ev::place(Place::this()),
            "string"
            | "number"
            | "true"
            | "false"
            | "null"
            | "undefined"
            | "regex"
            | "template_literal_type"
            | "super"
            | "import"
            | "meta_property" => Ev::konst(self.t(node)),
            "template_string" => {
                let subs: Vec<Node> = named(node)
                    .into_iter()
                    .filter(|n| n.kind() == "template_substitution")
                    .flat_map(named)
                    .collect();
                let mut vals = vec![];
                for sub in subs {
                    vals.extend(self.eval(sub).vals);
                }
                Ev::vals(derive(vals))
            }
            "member_expression" => {
                let base = node
                    .child_by_field_name("object")
                    .map(|o| self.eval(o))
                    .unwrap_or_default();
                let Some(prop) = node.child_by_field_name("property") else {
                    return Ev::vals(base.vals);
                };
                let step = self.member_step(prop);
                match base.place {
                    Some(place) => Ev::place(place.with(step)),
                    None => Ev::vals(derive(base.vals)),
                }
            }
            "subscript_expression" => {
                if let Some(index) = node.child_by_field_name("index") {
                    self.eval(index);
                }
                let base = node
                    .child_by_field_name("object")
                    .map(|o| self.eval(o))
                    .unwrap_or_default();
                match base.place {
                    Some(place) => Ev::place(place.with(Step::Index)),
                    None => Ev::vals(derive(base.vals)),
                }
            }
            "call_expression" => self.js_call(node),
            "new_expression" => {
                let args = node
                    .child_by_field_name("arguments")
                    .map(named)
                    .unwrap_or_default();
                let args: Vec<Arg> = args.into_iter().map(|a| self.arg(a)).collect();
                let ctor = node.child_by_field_name("constructor");
                let recs: Vec<&SourceRec> = ctor
                    .map(|c| {
                        let name = if c.kind() == "member_expression" {
                            c.child_by_field_name("property").unwrap_or(c)
                        } else {
                            c
                        };
                        self.recs(name)
                    })
                    .unwrap_or_default();
                let text = ctor.map(|c| self.t(c).to_string()).unwrap_or_default();
                let id = self.push_call(node, &recs, &text, CallKind::Construct, None, args);
                Ev::vals(vec![Val::direct(Atom::Call(id))])
            }
            "assignment_expression" | "augmented_assignment_expression" => {
                let op = node
                    .child_by_field_name("operator")
                    .map(|o| o.kind().to_string())
                    .unwrap_or_else(|| "=".to_string());
                let right = node.child_by_field_name("right");
                let left = node.child_by_field_name("left");
                if let (Some(left), Some(right)) = (left, right)
                    && matches!(
                        right.kind(),
                        "arrow_function" | "function_expression" | "function"
                    )
                {
                    // (`this.foo = function() {...}`: a lambda in this one.)
                    let _ = left;
                }
                let right_ev = right.map(|r| self.eval(r)).unwrap_or_default();
                let Some(left) = left else {
                    return Ev::vals(right_ev.vals);
                };
                if matches!(left.kind(), "object_pattern" | "array_pattern") {
                    self.js_pattern(left, right_ev.vals.clone(), right_ev.place, node);
                    return Ev::vals(right_ev.vals);
                }
                let left_ev = self.eval(left);
                match left_ev.place {
                    Some(place) => {
                        let vals = if op == "=" {
                            right_ev.vals
                        } else {
                            let mut vals = derive(right_ev.vals);
                            vals.push(Val {
                                atom: Atom::Read(place.clone()),
                                derived: true,
                            });
                            vals
                        };
                        self.assign(node, place.clone(), vals, &op);
                        Ev::place(place)
                    }
                    None => Ev::vals(right_ev.vals),
                }
            }
            "update_expression" => {
                let Some(arg) = node.child_by_field_name("argument") else {
                    return Ev::default();
                };
                let ev = self.eval(arg);
                if let Some(place) = ev.place.clone() {
                    let vals = derive(ev.vals.clone());
                    self.assign(node, place, vals, "++");
                }
                ev
            }
            "binary_expression" => {
                let op = node
                    .child_by_field_name("operator")
                    .map(|o| o.kind())
                    .unwrap_or("");
                let mut vals = vec![];
                for f in ["left", "right"] {
                    if let Some(child) = node.child_by_field_name(f) {
                        vals.extend(self.eval(child).vals);
                    }
                }
                // (`a || b`, `a ?? b`: either value itself.)
                if matches!(op, "||" | "??" | "&&") {
                    Ev::vals(vals)
                } else {
                    Ev::vals(derive(vals))
                }
            }
            "unary_expression" => {
                let vals = node
                    .child_by_field_name("argument")
                    .map(|a| self.eval(a).vals)
                    .unwrap_or_default();
                Ev::vals(derive(vals))
            }
            "ternary_expression" => {
                if let Some(cond) = node.child_by_field_name("condition") {
                    let vals = self.eval(cond).vals;
                    self.test(cond, vals);
                }
                let mut vals = vec![];
                for f in ["consequence", "alternative"] {
                    if let Some(child) = node.child_by_field_name(f) {
                        vals.extend(self.eval(child).vals);
                    }
                }
                Ev::vals(vals)
            }
            "sequence_expression" => {
                let children = named(node);
                let Some((last, rest)) = children.split_last() else {
                    return Ev::default();
                };
                for child in rest {
                    self.eval(*child);
                }
                self.eval(*last)
            }
            "arrow_function" | "function_expression" | "function" | "generator_function" => {
                let params = node.child_by_field_name("parameters");
                if let Some(param) = node.child_by_field_name("parameter") {
                    self.lambda_depth += 1;
                    self.declare_or_lookup(param);
                    self.lambda_depth -= 1;
                }
                if let Some(body) = node.child_by_field_name("body") {
                    self.lambda_body(body, params);
                }
                Ev::konst(&format!("function@{}", self.pos(node).0))
            }
            "object" => {
                let mut vals = vec![];
                for child in named(node) {
                    match child.kind() {
                        "pair" => {
                            if let Some(value) = child.child_by_field_name("value") {
                                vals.extend(self.eval(value).vals);
                            }
                        }
                        "method_definition" => self.nested.push(child),
                        _ => vals.extend(self.eval(child).vals),
                    }
                }
                Ev::vals(derive(vals))
            }
            "array" => Ev::vals(derive(self.eval_union(named(node)))),
            "class" => {
                self.nested.push(node);
                Ev::konst("class")
            }
            "ERROR" => {
                self.question(node, QuestionKind::Unparsed, truncate(self.t(node), 60));
                Ev::vals(self.eval_union(named(node)))
            }
            _ => Ev::vals(self.eval_union(named(node))),
        }
    }

    fn js_call(&mut self, node: Node<'c>) -> Ev {
        let Some(function) = node.child_by_field_name("function") else {
            return Ev::default();
        };
        let arg_nodes = node
            .child_by_field_name("arguments")
            .map(named)
            .unwrap_or_default();
        let (name_node, receiver) = match function.kind() {
            "member_expression" => {
                let receiver = function.child_by_field_name("object").map(|o| self.arg(o));
                (function.child_by_field_name("property"), receiver)
            }
            "identifier" => (Some(function), None),
            _ => {
                let callee = self.eval(function);
                let args: Vec<Arg> = arg_nodes.into_iter().map(|a| self.arg(a)).collect();
                let receiver = Arg {
                    vals: callee.vals,
                    place: callee.place,
                    addr: false,
                    text: truncate(self.t(function), 60),
                    line: self.pos(function).0,
                    end_line: self.end_line(function),
                };
                let text = self.t(function).to_string();
                let id = self.push_call(node, &[], &text, CallKind::Call, Some(receiver), args);
                return Ev::vals(vec![Val::direct(Atom::Call(id))]);
            }
        };
        let Some(name_node) = name_node else {
            return Ev::vals(self.eval_union(arg_nodes));
        };
        let name = self.t(name_node).to_string();
        let recs = self.recs(name_node);
        let callee_recs: Vec<&SourceRec> = recs
            .iter()
            .copied()
            .filter(|r| {
                !r.syms.is_empty()
                    && (r.has_syntax("method") || r.has_syntax("function") || r.has_syntax("var"))
            })
            .take(1)
            .collect();
        let args: Vec<Arg> = arg_nodes.into_iter().map(|a| self.arg(a)).collect();
        let id = self.push_call(node, &callee_recs, &name, CallKind::Call, receiver, args);
        Ev::vals(vec![Val::direct(Atom::Call(id))])
    }

    // ## Kotlin

    fn kt_function(&mut self, node: Node<'c>) {
        self.ir.has_body = named(node)
            .iter()
            .any(|c| matches!(c.kind(), "function_body" | "block"));
        for child in named(node) {
            match child.kind() {
                "function_value_parameters" => {
                    for param in named(child) {
                        if let Some(ident) = self.param_ident(param) {
                            let ty = named(param)
                                .into_iter()
                                .find(|n| n.kind() != "identifier")
                                .map(|t| self.t(t).to_string());
                            self.add_param(ident, ty);
                        }
                    }
                }
                "function_body" => {
                    let children = named(child);
                    match children.as_slice() {
                        [block] if block.kind() == "block" => self.stmt(*block),
                        [expr] => {
                            let vals = self.eval(*expr).vals;
                            self.event(*expr, EventKind::Return { vals });
                        }
                        _ => self.stmts(child),
                    }
                }
                "block" => self.stmt(child),
                _ => {}
            }
        }
    }

    fn kt_stmt(&mut self, node: Node<'c>) {
        match node.kind() {
            "block" | "statements" | "source_file" | "class_body" => self.stmts(node),
            "property_declaration" => self.kt_property(node),
            "assignment" => {
                self.eval(node);
            }
            "for_statement" => {
                let children = named(node);
                let mut vars = vec![];
                let mut iterable = None;
                let mut bodies = vec![];
                for child in children {
                    match child.kind() {
                        "variable_declaration" | "multi_variable_declaration" => vars.push(child),
                        "block" | "statement" => bodies.push(child),
                        "annotation" | "label" => {}
                        _ if iterable.is_none() => iterable = Some(child),
                        _ => bodies.push(child),
                    }
                }
                let right = iterable.map(|i| self.eval(i)).unwrap_or_default();
                let vals = match right.place {
                    Some(place) => vec![Val::direct(Atom::Read(place.with(Step::Index)))],
                    None => derive(right.vals),
                };
                for var in vars {
                    let idents: Vec<Node> = if var.kind() == "multi_variable_declaration" {
                        named(var)
                            .into_iter()
                            .filter_map(|v| self.param_ident(v))
                            .collect()
                    } else {
                        self.param_ident(var).into_iter().collect()
                    };
                    for ident in idents {
                        let place = self.declare_or_lookup(ident);
                        self.assign(var, place, vals.clone(), "init");
                    }
                }
                for body in bodies {
                    self.stmt(body);
                }
            }
            "while_statement" | "do_while_statement" => {
                for child in named(node) {
                    match child.kind() {
                        "block" => self.stmt(child),
                        _ => {
                            let vals = self.eval(child).vals;
                            self.test(child, vals);
                        }
                    }
                }
            }
            "function_declaration" | "class_declaration" | "object_declaration" => {
                self.nested.push(node);
            }
            "import" | "package_header" | "type_alias" => {}
            _ => {
                self.eval(node);
            }
        }
    }

    /// `val x = v`, `var (a, b) = v`, `val x by lazy { ... }`.
    fn kt_property(&mut self, node: Node<'c>) {
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        let mut value = None;
        let mut after_eq = false;
        let mut targets = vec![];
        for child in &children {
            if child.kind() == "=" {
                after_eq = true;
                continue;
            }
            if !child.is_named() || is_comment(child) {
                continue;
            }
            match child.kind() {
                "variable_declaration" => targets.extend(self.param_ident(*child)),
                "multi_variable_declaration" => {
                    for var in named(*child) {
                        targets.extend(self.param_ident(var));
                    }
                }
                "property_delegate" => value = named(*child).first().copied(),
                _ if after_eq && value.is_none() => value = Some(*child),
                _ => {}
            }
        }
        let vals = value.map(|v| self.eval(v).vals).unwrap_or_default();
        let several = targets.len() > 1;
        for ident in targets {
            let place = self.declare_or_lookup(ident);
            let vals = if several {
                derive(vals.clone())
            } else {
                vals.clone()
            };
            if !vals.is_empty() {
                self.assign(node, place, vals, "init");
            }
        }
    }

    /// A Kotlin block's statements, and its last expression's value (for
    /// `if`, `when`, `try` and lambda bodies, which are expressions).
    fn kt_block_value(&mut self, block: Node<'c>) -> Vec<Val> {
        if block.kind() != "block" {
            return self.eval(block).vals;
        }
        let children = named(block);
        let Some((last, rest)) = children.split_last() else {
            return vec![];
        };
        for child in rest {
            self.stmt(*child);
        }
        match last.kind() {
            "property_declaration"
            | "assignment"
            | "for_statement"
            | "while_statement"
            | "do_while_statement"
            | "function_declaration"
            | "class_declaration" => {
                self.stmt(*last);
                vec![]
            }
            _ => self.eval(*last).vals,
        }
    }

    fn kt_eval(&mut self, node: Node<'c>) -> Ev {
        match node.kind() {
            "parenthesized_expression"
            | "annotated_expression"
            | "labeled_expression"
            | "spread_expression" => match named(node)
                .into_iter()
                .rev()
                .find(|n| !matches!(n.kind(), "annotation" | "label"))
            {
                Some(inner) => self.eval(inner),
                None => Ev::default(),
            },
            "as_expression" => node
                .child_by_field_name("left")
                .map(|l| self.eval(l))
                .unwrap_or_default(),
            "identifier" => self.scip_ident(node),
            "this_expression" => Ev::place(Place::this()),
            "number_literal" | "float_literal" | "character_literal" | "boolean_literal"
            | "null_literal" | "callable_reference" | "super_expression" => Ev::konst(self.t(node)),
            "string_literal" | "multiline_string_literal" => {
                let subs: Vec<Node> = named(node)
                    .into_iter()
                    .filter(|n| n.kind() == "interpolation")
                    .flat_map(named)
                    .collect();
                if subs.is_empty() {
                    return Ev::konst(self.t(node));
                }
                let mut vals = vec![];
                for sub in subs {
                    vals.extend(self.eval(sub).vals);
                }
                Ev::vals(derive(vals))
            }
            "navigation_expression" => {
                let children = named(node);
                let (Some(base), Some(prop)) = (children.first(), children.last()) else {
                    return Ev::default();
                };
                let base = self.eval(*base);
                if prop.kind() != "identifier" || children.len() < 2 {
                    return Ev::vals(base.vals);
                }
                let step = self.member_step(*prop);
                match base.place {
                    Some(place) => Ev::place(place.with(step)),
                    None => Ev::vals(derive(base.vals)),
                }
            }
            "index_expression" => {
                let children = named(node);
                let Some((base, indices)) = children.split_first() else {
                    return Ev::default();
                };
                for index in indices {
                    self.eval(*index);
                }
                let base = self.eval(*base);
                match base.place {
                    Some(place) => Ev::place(place.with(Step::Index)),
                    None => Ev::vals(derive(base.vals)),
                }
            }
            "call_expression" => self.kt_call(node),
            "assignment" => {
                let op = node
                    .child_by_field_name("operator")
                    .map(|o| o.kind().to_string())
                    .unwrap_or_else(|| "=".to_string());
                let right = node
                    .child_by_field_name("right")
                    .map(|r| self.eval(r).vals)
                    .unwrap_or_default();
                let left = node
                    .child_by_field_name("left")
                    .map(|l| self.eval(l))
                    .unwrap_or_default();
                if let Some(place) = left.place {
                    let vals = if op == "=" {
                        right
                    } else {
                        let mut vals = derive(right);
                        vals.push(Val {
                            atom: Atom::Read(place.clone()),
                            derived: true,
                        });
                        vals
                    };
                    self.assign(node, place, vals, &op);
                }
                Ev::default()
            }
            "unary_expression" => {
                let op = node
                    .child_by_field_name("operator")
                    .map(|o| o.kind())
                    .unwrap_or("");
                let Some(arg) = node.child_by_field_name("argument") else {
                    return Ev::default();
                };
                let ev = self.eval(arg);
                match op {
                    "!!" => ev,
                    "++" | "--" => {
                        if let Some(place) = ev.place.clone() {
                            let vals = derive(ev.vals.clone());
                            self.assign(node, place, vals, op);
                        }
                        ev
                    }
                    _ => Ev::vals(derive(ev.vals)),
                }
            }
            "binary_expression" => {
                let op = node
                    .child_by_field_name("operator")
                    .map(|o| o.kind())
                    .unwrap_or("");
                let mut vals = vec![];
                for child in named(node) {
                    vals.extend(self.eval(child).vals);
                }
                // (`a ?: b`: either value itself.)
                if op == "?:" {
                    Ev::vals(vals)
                } else {
                    Ev::vals(derive(vals))
                }
            }
            "if_expression" => {
                let cond = node.child_by_field_name("condition");
                if let Some(cond) = cond {
                    let vals = self.eval(cond).vals;
                    self.test(cond, vals);
                }
                let mut vals = vec![];
                for child in named(node) {
                    if Some(child.id()) == cond.map(|c| c.id()) {
                        continue;
                    }
                    vals.extend(self.kt_block_value(child));
                }
                Ev::vals(vals)
            }
            "when_expression" => {
                let mut vals = vec![];
                for child in named(node) {
                    match child.kind() {
                        "when_subject" => {
                            for sub in named(child) {
                                if sub.kind() == "variable_declaration" {
                                    continue;
                                }
                                let v = self.eval(sub).vals;
                                self.test(sub, v);
                            }
                        }
                        "when_entry" => {
                            let mut cursor = child.walk();
                            let conds: Vec<Node> = child
                                .children_by_field_name("condition", &mut cursor)
                                .collect();
                            let cond_ids: Vec<usize> = conds.iter().map(|c| c.id()).collect();
                            for cond in conds {
                                let v = self.eval(cond).vals;
                                self.test(cond, v);
                            }
                            for body in named(child) {
                                if !cond_ids.contains(&body.id()) {
                                    vals.extend(self.kt_block_value(body));
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Ev::vals(vals)
            }
            "try_expression" => {
                let mut vals = vec![];
                for child in named(node) {
                    match child.kind() {
                        "block" => vals.extend(self.kt_block_value(child)),
                        "catch_block" => {
                            for sub in named(child) {
                                match sub.kind() {
                                    "identifier" => {
                                        self.declare_or_lookup(sub);
                                    }
                                    "block" => vals.extend(self.kt_block_value(sub)),
                                    _ => {}
                                }
                            }
                        }
                        "finally_block" => self.stmts(child),
                        _ => {}
                    }
                }
                Ev::vals(vals)
            }
            "return_expression" => {
                let vals = match named(node)
                    .into_iter()
                    .find(|n| node.child_by_field_name("label").map(|l| l.id()) != Some(n.id()))
                {
                    Some(expr) => self.eval(expr).vals,
                    None => vec![],
                };
                self.event(node, EventKind::Return { vals });
                Ev::default()
            }
            "throw_expression" => {
                for child in named(node) {
                    self.eval(child);
                }
                Ev::default()
            }
            "lambda_literal" | "annotated_lambda" | "anonymous_function" => {
                let lambda = if node.kind() == "annotated_lambda" {
                    named(node)
                        .into_iter()
                        .find(|n| n.kind() == "lambda_literal")
                } else {
                    Some(node)
                };
                if let Some(lambda) = lambda {
                    self.kt_lambda(lambda);
                }
                Ev::konst(&format!("lambda@{}", self.pos(node).0))
            }
            "object_literal" => {
                self.nested.push(node);
                Ev::konst("object")
            }
            "collection_literal" => Ev::vals(derive(self.eval_union(named(node)))),
            "is_expression" | "in_expression" | "range_expression" => {
                Ev::vals(derive(self.eval_union(named(node))))
            }
            "ERROR" => {
                self.question(node, QuestionKind::Unparsed, truncate(self.t(node), 60));
                Ev::vals(self.eval_union(named(node)))
            }
            _ => Ev::vals(self.eval_union(named(node))),
        }
    }

    fn kt_lambda(&mut self, lambda: Node<'c>) {
        self.lambda_depth += 1;
        let children = named(lambda);
        let mut statements = vec![];
        for child in children {
            match child.kind() {
                "lambda_parameters" => {
                    for param in named(child) {
                        let idents: Vec<Node> = if param.kind() == "multi_variable_declaration" {
                            named(param)
                                .into_iter()
                                .filter_map(|v| self.param_ident(v))
                                .collect()
                        } else {
                            self.param_ident(param).into_iter().collect()
                        };
                        for ident in idents {
                            self.declare_or_lookup(ident);
                        }
                    }
                }
                _ => statements.push(child),
            }
        }
        if let Some((last, rest)) = statements.split_last() {
            for child in rest {
                self.stmt(*child);
            }
            let vals = match last.kind() {
                "property_declaration" | "assignment" | "for_statement" | "while_statement" => {
                    self.stmt(*last);
                    vec![]
                }
                _ => self.eval(*last).vals,
            };
            if !vals.is_empty() {
                self.event(*last, EventKind::Return { vals });
            }
        }
        self.lambda_depth -= 1;
    }

    fn kt_call(&mut self, node: Node<'c>) -> Ev {
        let children = named(node);
        let Some((function, rest)) = children.split_first() else {
            return Ev::default();
        };
        let function = *function;
        let mut arg_nodes = vec![];
        let mut lambdas = vec![];
        for child in rest {
            match child.kind() {
                "value_arguments" => {
                    for arg in named(*child) {
                        // (`name = value`: the value.)
                        if let Some(value) = named(arg).last() {
                            arg_nodes.push(*value);
                        }
                    }
                }
                "annotated_lambda" | "lambda_literal" => lambdas.push(*child),
                _ => {}
            }
        }
        let (name_node, receiver) = match function.kind() {
            "navigation_expression" => {
                let parts = named(function);
                let receiver = parts.first().map(|b| self.arg(*b));
                (parts.last().copied().filter(|_| parts.len() > 1), receiver)
            }
            "identifier" => (Some(function), None),
            _ => {
                let callee = self.eval(function);
                let mut args: Vec<Arg> = arg_nodes.into_iter().map(|a| self.arg(a)).collect();
                for lambda in lambdas {
                    args.push(self.arg(lambda));
                }
                let receiver = Arg {
                    vals: callee.vals,
                    place: callee.place,
                    addr: false,
                    text: truncate(self.t(function), 60),
                    line: self.pos(function).0,
                    end_line: self.end_line(function),
                };
                let text = self.t(function).to_string();
                let id = self.push_call(node, &[], &text, CallKind::Call, Some(receiver), args);
                return Ev::vals(vec![Val::direct(Atom::Call(id))]);
            }
        };
        let Some(name_node) = name_node else {
            return Ev::vals(self.eval_union(arg_nodes));
        };
        let name = self.t(name_node).to_string();
        let recs = self.recs(name_node);
        // (A property of function type, called: `getSettingsProvider()`,
        // whose records are the property's and `invoke`'s.)
        let callee_recs: Vec<&SourceRec> = recs
            .iter()
            .copied()
            .filter(|r| {
                !r.syms.is_empty()
                    && r.has_syntax("method")
                    && !r.sym().starts_with("S_jvm_kotlin/Function")
                    && !r.sym().ends_with("#get().")
            })
            .take(1)
            .collect();
        let kind = if callee_recs.iter().any(|r| r.pretty.ends_with("<init>")) {
            CallKind::Construct
        } else {
            CallKind::Call
        };
        let receiver = receiver.or_else(|| self.implicit_receiver(&callee_recs));
        let mut args: Vec<Arg> = arg_nodes.into_iter().map(|a| self.arg(a)).collect();
        for lambda in lambdas {
            args.push(self.arg(lambda));
        }
        let id = self.push_call(node, &callee_recs, &name, kind, receiver, args);
        Ev::vals(vec![Val::direct(Atom::Call(id))])
    }
}

fn empty_ir() -> FuncIr {
    FuncIr {
        sym: Ustr::default(),
        pretty: String::new(),
        path: String::new(),
        lang: Lang::Cpp,
        line: 0,
        end_line: 0,
        params: vec![],
        has_body: true,
        events: vec![],
        calls: vec![],
        questions: vec![],
        names: HashMap::new(),
        types: HashMap::new(),
    }
}

/// The first node of a kind, depth-first.
fn find_kind<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    if node.kind() == kind {
        return Some(node);
    }
    for child in named(node) {
        if let Some(found) = find_kind(child, kind) {
            return Some(found);
        }
    }
    None
}

#[allow(dead_code)]
fn short(pretty: &str) -> &str {
    short_name(pretty)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    /// A source record for `needle` (the `nth` on its line) in `source`.
    fn rec(
        source: &str,
        line: usize,
        needle: &str,
        syntax: &str,
        pretty: &str,
        sym: &str,
        ty: Option<&str>,
    ) -> Value {
        let text = source.lines().nth(line - 1).unwrap();
        let col = text.find(needle).unwrap();
        let mut obj = json!({
            "loc": format!("{:05}:{}-{}", line, col, col + needle.len()),
            "source": 1,
            "syntax": syntax,
            "pretty": pretty,
            "sym": sym,
        });
        if let Some(ty) = ty {
            obj["type"] = json!(ty);
        }
        obj
    }

    #[test]
    fn test_cpp_ir() {
        let source = "nsresult Foo::Get(const char* aName, bool* aRetVal) {\n  \
                      nsresult rv = Bar(aName, aRetVal);\n  \
                      if (rv != 0) { *aRetVal = false; }\n  \
                      return rv;\n\
                      }\n";
        let values = vec![
            rec(
                source,
                1,
                "Get",
                "def,function",
                "function Foo::Get",
                "_ZGet",
                None,
            ),
            rec(
                source,
                1,
                "aName",
                "",
                "variable aName",
                "V_name",
                Some("const char *"),
            ),
            rec(
                source,
                1,
                "aRetVal",
                "",
                "variable aRetVal",
                "V_retval",
                Some("_Bool *"),
            ),
            rec(source, 2, "rv", "", "variable rv", "V_rv", Some("nsresult")),
            rec(
                source,
                2,
                "Bar",
                "function,use",
                "function Foo::Bar",
                "_ZBar",
                None,
            ),
            rec(source, 2, "aName", "", "variable aName", "V_name", None),
            rec(
                source,
                2,
                "aRetVal",
                "",
                "variable aRetVal",
                "V_retval",
                None,
            ),
            rec(source, 3, "rv", "", "variable rv", "V_rv", None),
            rec(
                source,
                3,
                "aRetVal",
                "",
                "variable aRetVal",
                "V_retval",
                None,
            ),
            rec(source, 4, "rv", "", "variable rv", "V_rv", None),
        ];
        let recs = FileRecords::from_values(values);
        let file = build_file_ir("a.cpp", source, &recs).unwrap();
        assert_eq!(file.funcs.len(), 1);
        assert_eq!(
            file.funcs[0].to_string(),
            "Foo::Get (a.cpp:1-5)\n  \
             param 0: aName : const char *\n  \
             param 1: aRetVal : _Bool *\n      \
             2:16  call#0 Call this.Foo::Bar({aName}@aName, {aRetVal}@aRetVal) -> _ZBar\n      \
             2:11  rv init {Foo::Bar(…)}\n      \
             3:6   test {~rv, ~0}\n      \
             3:17  *aRetVal = {false}\n      \
             4:2   return {rv}\n"
        );
    }
}
