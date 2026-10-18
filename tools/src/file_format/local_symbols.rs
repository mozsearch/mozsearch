//! C++ locals' and parameters' symbols.
//!
//! The clang plugin gives locals and parameters `V_` symbols whose hash
//! includes their locations, so a function's declaration (ex: in a header)
//! and definition (in a `.cpp`) name the same parameter differently, and so
//! do platforms whose generated files differ; and it marks them `no_crossref`
//! and gives them no target records.  As the tools read a file's analysis,
//! `process_local_symbols`:
//! - renames each parameter's symbols to one derived from its function's
//!   symbol and its position, `parameter_symbol`, which every file (and
//!   analyzer, ex: ipdl-analyze's binding slots for messages' parameters)
//!   agrees on;
//! - gives each other local one symbol (the least of its platforms');
//! - and gives them target records (definitions, or a declaration's
//!   parameters' declarations, and uses), in their functions' contexts, with
//!   pretty names qualified by their functions' (ex: `params::Scale::aFactor`),
//!   so they have crossref entries (and jumps),
//!
//! without changing the plugin (firefox's copy of which is what indexes
//! firefox).

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};

use super::analysis::parse_location;
use super::analysis_manglings::split_symbol_list;

/// A 64-bit FNV-1a hash (stable across runs and builds, unlike std's).
fn fnv1a64(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// The unified symbol of a function's parameter, by its index: still a `V_`
/// local's (`V_<hex>_<hex>`, which the UI recognizes).
pub fn parameter_symbol(function_sym: &str, index: usize) -> String {
    format!("V_{:016x}_{:x}", fnv1a64(function_sym), index)
}

/// The parameters' types in a function's type, without whitespace (ex:
/// ["constchar*", "_Bool*"] for "nsresult (const char *, _Bool *)"): the
/// parts of its last top-level parenthesized group between top-level commas
/// (not commas in template arguments or nested parentheses), or None if it
/// has none.
fn parameter_types(ty: &str) -> Option<Vec<String>> {
    let mut depth = 0i32;
    let mut group: Option<(usize, usize)> = None;
    let mut start = 0;
    for (i, c) in ty.char_indices() {
        match c {
            '(' => {
                if depth == 0 {
                    start = i + 1;
                }
                depth += 1;
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    group = Some((start, i));
                }
            }
            _ => {}
        }
    }
    let (start, end) = group?;
    let params = ty[start..end].trim();
    if params.is_empty() || params == "void" {
        return Some(vec![]);
    }
    let mut types = vec![];
    let mut current = String::new();
    let mut depth = 0i32;
    for c in params.chars() {
        match c {
            '(' | '<' | '[' => depth += 1,
            ')' | '>' | ']' => depth -= 1,
            ',' if depth == 0 => {
                types.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        if !c.is_whitespace() {
            current.push(c);
        }
    }
    types.push(current);
    Some(types)
}

/// The positions of a function's parameters (by their types, in order) if
/// there's exactly one way to place them among its type's parameters', in
/// order.
fn unique_type_assignment(param_types: &[Option<String>], types: &[String]) -> Option<Vec<usize>> {
    let (n, m) = (param_types.len(), types.len());
    let matches = |i: usize, j: usize| param_types[i].as_ref() == Some(&types[j]);
    // (How many ways there are to place parameters i.. among positions j..,
    // up to 2.)
    let mut ways = vec![vec![0u8; m + 1]; n + 1];
    for j in 0..=m {
        ways[n][j] = 1;
    }
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let mut w = ways[i][j + 1];
            if matches(i, j) {
                w += ways[i + 1][j + 1];
            }
            ways[i][j] = w.min(2);
        }
    }
    if ways[0][0] != 1 {
        return None;
    }
    let mut positions = vec![];
    let (mut i, mut j) = (0, 0);
    while i < n {
        if matches(i, j) && ways[i + 1][j + 1] == 1 {
            positions.push(j);
            i += 1;
        }
        j += 1;
    }
    Some(positions)
}

struct Function<'a> {
    loc: (u32, u32),
    /// Where its body starts and ends, for definitions.
    body: Option<((u32, u32), (u32, u32))>,
    /// Its declaration's last line (its target record's `peekRange`'s).
    last_line: Option<u32>,
    sym: &'a str,
    /// Its pretty name (ex: "params::Scale").
    pretty: &'a str,
    /// Its parameters' types, from its type.
    types: Option<Vec<String>>,
    /// Its parameters' names, from its structured record, if this file has
    /// it.
    arg_names: Option<Vec<&'a str>>,
}

/// A parameter's records at a location: its symbols (ex: several
/// platforms'), name, and type (without whitespace).
struct Param<'a> {
    syms: Vec<&'a str>,
    name: &'a str,
    ty: Option<String>,
}

/// The kinds of structured records that declare locals.
const LOCAL_KINDS: [&str; 2] = ["parameter", "localVar"];

/// What `process_local_symbols` does to a record.
#[derive(Default)]
pub(crate) struct Edit {
    drop: bool,
    sym: Option<String>,
    pretty: Option<String>,
    /// Whether it's a local's source record (which loses its `no_crossref`).
    local_source: bool,
    /// Target records to add before it.
    targets: Vec<Value>,
}

fn parse_point(text: &str) -> Option<(u32, u32)> {
    let (line, col) = text.split_once(':')?;
    Some((line.parse().ok()?, col.parse().ok()?))
}

fn loc_at(loc: Option<&str>) -> Option<(u32, u32)> {
    let loc = parse_location(loc?);
    Some((loc.lineno, loc.col_start))
}

/// The fields of a record that `plan_local_edits` looks at, found in one pass
/// over its keys (cheaper than looking each up by its hash).
#[derive(Default)]
struct Fields<'a> {
    loc: Option<&'a str>,
    sym: Option<&'a str>,
    source: bool,
    target: bool,
    structured: bool,
    syntax: Option<&'a str>,
    peek_range: Option<&'a str>,
    kind: Option<&'a str>,
    args: Option<&'a Vec<Value>>,
    in_expansion: bool,
}

fn fields(obj: &Map<String, Value>) -> Fields<'_> {
    let mut fields = Fields::default();
    for (key, value) in obj {
        match key.as_str() {
            "loc" => fields.loc = value.as_str(),
            "sym" => fields.sym = value.as_str(),
            "source" => fields.source = true,
            "target" => fields.target = true,
            "structured" => fields.structured = true,
            "syntax" => fields.syntax = value.as_str(),
            "peekRange" => fields.peek_range = value.as_str(),
            "kind" => fields.kind = value.as_str(),
            "args" => fields.args = value.as_array(),
            "inExpansionAt" => fields.in_expansion = true,
            _ => {}
        }
    }
    fields
}

/// Rename the locals' and parameters' `V_` symbols in a file's analysis
/// records, and give them target records (see the module's documentation).
///
/// Parameters' symbols become `parameter_symbol`s.
/// A parameter is a symbol with a structured record of kind "parameter"; its
/// function is the nearest function definition or declaration (a source
/// record with "def"/"decl" and "function" syntax) before it, if it's before
/// the definition's body (its `nestingRange`) or within the declaration's
/// lines (its `peekRange`).  Its index is its name's position in the
/// function's structured record's `args`, if the file has it (definitions'),
/// else its position, if all of the function's parameters (by its type) have
/// records, else the position its type (and the others') can only have among
/// the function's type's parameters.  (Unnamed parameters have no records,
/// so a declaration's are otherwise ambiguous, and so are a function
/// pointer's parameters' parameters.)  Parameters without an index keep their
/// symbols.  Other locals' symbols (several platforms', at a location)
/// become the least of them.  A local's structured records at a location
/// (several platforms') become one.
///
/// Then the locals' symbols that have no target records get them: a
/// definition (or, for a declaration's parameter, a declaration) where its
/// structured record is (or its first source record, if the file has none),
/// and uses elsewhere, in the innermost function definition's context (or,
/// for a parameter's declaration, its function's), with its function's
/// pretty name and its own (ex: `params::Scale::aFactor`), which its
/// structured records get too.  Its source records lose their `no_crossref`.
///
/// (Most records aren't locals', so this finds the records it needs in one
/// pass, and edits only those.)
pub fn process_local_symbols(records: &mut Vec<Value>) {
    let mut edits = plan_local_edits(records);
    if edits.iter().all(Option::is_none) {
        return;
    }
    let old = std::mem::take(records);
    let targets: usize = edits.iter().flatten().map(|e| e.targets.len()).sum();
    records.reserve(old.len() + targets);
    for (index, record) in old.into_iter().enumerate() {
        match edits[index].take() {
            Some(edit) => apply_local_edit(*edit, record, &mut |r| records.push(r)),
            None => records.push(record),
        }
    }
}

/// Give `out` what `process_local_symbols` makes of a record with an edit:
/// its target records, and it, edited (or nothing, if it's dropped).
pub(crate) fn apply_local_edit(edit: Edit, mut record: Value, out: &mut dyn FnMut(Value)) {
    if edit.drop {
        return;
    }
    for target in edit.targets {
        out(target);
    }
    if let Some(obj) = record.as_object_mut() {
        if let Some(sym) = edit.sym
            && let Some(value) = obj.get_mut("sym")
        {
            *value = Value::String(sym);
        }
        if let Some(pretty) = edit.pretty
            && let Some(value) = obj.get_mut("pretty")
        {
            *value = Value::String(pretty);
        }
        if edit.local_source {
            obj.shift_remove("no_crossref");
        }
    }
    out(record);
}

/// Whether a record's line could be one `plan_local_edits` uses (a
/// superset, by its text, so that the other lines needn't be parsed for it).
pub(crate) fn may_plan_from(line: &str) -> bool {
    line.contains("\"V_")
        || line.contains("\"peekRange\"")
        || line.contains("\"args\"")
        || line.contains("function")
        || line.contains("destructor")
}

/// Whether `plan_local_edits` uses a record: it ignores the others, so it
/// plans the same edits for those (as `plan_local_edits_of`) as for all.
pub(crate) fn plans_from(record: &Value) -> bool {
    let Some(obj) = record.as_object() else {
        return false;
    };
    let f = fields(obj);
    let Some(sym) = f.sym else {
        return false;
    };
    let local = sym.starts_with("V_") && sym.split(',').all(|s| s.starts_with("V_"));
    if local {
        return f.source || f.target || f.structured;
    }
    if f.source {
        f.syntax
            .is_some_and(|syntax| syntax.contains("function") || syntax.contains("destructor"))
    } else if f.target {
        f.peek_range.is_some()
    } else if f.structured {
        f.args.is_some()
    } else {
        false
    }
}

/// `plan_local_edits` of some of a file's records (the ones it uses; see
/// `plans_from`), by their indices among all of the file's (`indices`).
pub(crate) fn plan_local_edits_of(
    indices: &[usize],
    records: &[Value],
) -> HashMap<usize, Box<Edit>> {
    plan_local_edits(records)
        .into_iter()
        .enumerate()
        .filter_map(|(i, edit)| Some((indices[i], edit?)))
        .collect()
}

/// A list of locals' symbols, renamed (without duplicates), and whether any
/// were.
fn renamed<'r>(syms: &'r str, renames: &'r HashMap<&str, String>) -> (Vec<&'r str>, bool) {
    let mut changed = false;
    let mut new_syms: Vec<&str> = vec![];
    for sym in syms.split(',') {
        let new_sym = match renames.get(sym) {
            Some(new_sym) => {
                changed = true;
                new_sym.as_str()
            }
            None => sym,
        };
        if !new_syms.contains(&new_sym) {
            new_syms.push(new_sym);
        }
    }
    (new_syms, changed)
}

/// A record's edit, by its index.
fn edit(edits: &mut [Option<Box<Edit>>], index: usize) -> &mut Edit {
    edits[index].get_or_insert_with(Default::default)
}

fn plan_local_edits<'a>(records: &'a [Value]) -> Vec<Option<Box<Edit>>> {
    let mut edits: Vec<Option<Box<Edit>>> = (0..records.len()).map(|_| None).collect();
    let mut functions: Vec<Function> = vec![];
    let mut structured_args: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut last_lines: HashMap<(u32, u32), u32> = HashMap::new();
    // Locals' records: source records (all of whose symbols are locals'),
    // structured records (and whether they're parameters'), and target
    // records (ex: an earlier read's, for a merged file).
    let mut local_sources: Vec<(usize, (u32, u32), &str)> = vec![];
    let mut local_structured: Vec<(usize, (u32, u32), &str, bool)> = vec![];
    let mut local_targets: Vec<(usize, &str)> = vec![];
    for (index, record) in records.iter().enumerate() {
        let Some(obj) = record.as_object() else {
            continue;
        };
        let f = fields(obj);
        let Some(sym) = f.sym else {
            continue;
        };
        // (Locals' symbols have no commas of their own.)
        let local = sym.starts_with("V_") && sym.split(',').all(|s| s.starts_with("V_"));
        if f.source {
            if local {
                if let Some(at) = loc_at(f.loc) {
                    local_sources.push((index, at, sym));
                }
                continue;
            }
            let Some(syntax) = f.syntax else {
                continue;
            };
            if !syntax.contains("function") && !syntax.contains("destructor") {
                continue;
            }
            let kinds: Vec<&str> = syntax.split(',').collect();
            if !(kinds.contains(&"def") || kinds.contains(&"decl"))
                || !kinds.iter().any(|k| *k == "function" || *k == "destructor")
                || f.in_expansion
            {
                continue;
            }
            let Some(at) = loc_at(f.loc) else {
                continue;
            };
            // (The least of several platforms' symbols, so every file
            // agrees.)
            let Some(sym) = split_symbol_list(sym).into_iter().min() else {
                continue;
            };
            let body = obj
                .get("nestingRange")
                .and_then(|r| r.as_str())
                .and_then(|r| r.split_once('-'))
                .and_then(|(start, end)| Some((parse_point(start)?, parse_point(end)?)));
            let pretty = obj.get("pretty").and_then(|p| p.as_str()).unwrap_or("");
            let pretty = pretty.split_once(' ').map_or(pretty, |(_, name)| name);
            let types = obj
                .get("type")
                .and_then(|t| t.as_str())
                .and_then(parameter_types);
            functions.push(Function {
                loc: at,
                body,
                last_line: None,
                sym,
                pretty,
                types,
                arg_names: None,
            });
        } else if f.target {
            if local {
                local_targets.push((index, sym));
                continue;
            }
            if let Some(last) = f
                .peek_range
                .and_then(|r| r.split_once('-'))
                .and_then(|(_, end)| end.parse::<u32>().ok())
                && let Some(at) = loc_at(f.loc)
            {
                let entry = last_lines.entry(at).or_default();
                *entry = (*entry).max(last);
            }
        } else if f.structured {
            let kind = f.kind.unwrap_or("");
            if local {
                if LOCAL_KINDS.contains(&kind)
                    && let Some(at) = loc_at(f.loc)
                {
                    local_structured.push((index, at, sym, kind == "parameter"));
                }
            } else if let Some(args) = f.args {
                structured_args.insert(
                    sym,
                    args.iter()
                        .map(|a| a.get("name").and_then(|n| n.as_str()).unwrap_or(""))
                        .collect(),
                );
            }
        }
    }
    if local_sources.is_empty() && local_structured.is_empty() {
        return edits;
    }
    for function in &mut functions {
        function.arg_names = structured_args.get(function.sym).cloned();
        function.last_line = last_lines.get(&function.loc).copied();
    }
    functions.sort_by_key(|f| f.loc);
    functions.dedup_by(|a, b| a.loc == b.loc);

    // Locals' names, from their source records' pretty names (ex: "variable
    // aFactor"), and types, by location.
    let source_at: HashMap<(u32, u32), usize> = local_sources
        .iter()
        .map(|(index, at, _)| (*at, *index))
        .collect();
    let source_field = |at: &(u32, u32), key: &str| -> Option<&str> {
        records[*source_at.get(at)?].get(key)?.as_str()
    };
    let name_at = |at: &(u32, u32)| -> Option<&str> {
        let pretty = source_field(at, "pretty")?;
        Some(pretty.split_once(' ').map_or(pretty, |(_, name)| name))
    };

    // Each function's parameters, in order, and each parameter's function.
    let mut param_records: Vec<((u32, u32), usize, &str)> = local_structured
        .iter()
        .filter(|(_, _, _, is_param)| *is_param)
        .map(|(index, at, sym, _)| (*at, *index, *sym))
        .collect();
    param_records.sort();
    let mut by_function: HashMap<usize, Vec<Param>> = HashMap::new();
    let mut param_functions: HashMap<(u32, u32), usize> = HashMap::new();
    let mut i = 0;
    while i < param_records.len() {
        let (at, index, _) = param_records[i];
        let mut j = i;
        while j < param_records.len() && param_records[j].0 == at {
            j += 1;
        }
        // (Its name, from its source record, or its structured record's
        // pretty name's last part.)
        let name = name_at(&at).unwrap_or_else(|| {
            let pretty = records[index]
                .get("pretty")
                .and_then(|p| p.as_str())
                .unwrap_or("");
            pretty.rsplit("::").next().unwrap_or(pretty)
        });
        let param = Param {
            syms: param_records[i..j].iter().map(|p| p.2).collect(),
            name,
            ty: source_field(&at, "type").map(|ty| ty.split_whitespace().collect()),
        };
        i = j;
        // (The functions are sorted by location.)
        let Some(fi) = functions.partition_point(|f| f.loc < at).checked_sub(1) else {
            continue;
        };
        let function = &functions[fi];
        let within = match (function.body, function.last_line) {
            (Some((body, _)), _) => at < body,
            (None, Some(last_line)) => at.0 <= last_line,
            (None, None) => true,
        };
        if within {
            by_function.entry(fi).or_default().push(param);
            param_functions.insert(at, fi);
        }
    }

    // (Each parameter location's symbols, ex: several platforms', share a
    // new symbol.)
    let mut renames: HashMap<&str, String> = HashMap::new();
    for (fi, params) in &by_function {
        let function = &functions[*fi];
        let indexes: Vec<Option<usize>> = match (&function.arg_names, &function.types) {
            (Some(names), _) => {
                // (Its name's next position: the first not taken by an
                // earlier parameter of the same name.)
                let mut taken: HashMap<&str, usize> = HashMap::new();
                params
                    .iter()
                    .map(|param| {
                        let earlier = taken.entry(param.name).or_default();
                        let index = names
                            .iter()
                            .enumerate()
                            .filter(|(_, n)| **n == param.name)
                            .nth(*earlier)
                            .map(|(i, _)| i);
                        *earlier += 1;
                        index
                    })
                    .collect()
            }
            (None, Some(types)) if params.len() == types.len() => {
                (0..params.len()).map(Some).collect()
            }
            (None, Some(types)) if params.len() < types.len() => {
                let param_types: Vec<Option<String>> =
                    params.iter().map(|p| p.ty.clone()).collect();
                match unique_type_assignment(&param_types, types) {
                    Some(positions) => positions.into_iter().map(Some).collect(),
                    None => vec![None; params.len()],
                }
            }
            _ => vec![None; params.len()],
        };
        for (param, index) in params.iter().zip(indexes) {
            let Some(index) = index else {
                continue;
            };
            if function
                .types
                .as_ref()
                .is_some_and(|types| index >= types.len())
            {
                continue;
            }
            let new_sym = parameter_symbol(function.sym, index);
            for sym in &param.syms {
                renames.insert(sym, new_sym.clone());
            }
        }
    }
    // (Other locals' symbols, ex: several platforms' in a generated file,
    // become the least of those they appear with.)
    unify_platform_symbols(local_sources.iter().map(|(_, _, syms)| *syms), &mut renames);
    let mut targeted: HashSet<&str> = HashSet::new();
    for (index, syms) in &local_targets {
        let (syms, changed) = renamed(syms, &renames);
        if changed {
            edit(&mut edits, *index).sym = Some(syms.join(","));
        }
        targeted.extend(syms);
    }
    // (Where locals' structured records are, dropping several platforms'
    // that are now the same.)
    let mut declared: HashMap<(u32, u32), Vec<&str>> = HashMap::new();
    let mut declarations: HashMap<&str, (u32, u32)> = HashMap::new();
    let mut kept: HashSet<(&str, (u32, u32))> = HashSet::new();
    let mut structured_syms: Vec<(usize, &str)> = vec![];
    for (index, at, sym, _) in &local_structured {
        let (syms, changed) = renamed(sym, &renames);
        let sym = syms[0];
        // (Several platforms' records for a local at a location are one.)
        if !kept.insert((sym, *at)) {
            edit(&mut edits, *index).drop = true;
            continue;
        }
        if changed {
            edit(&mut edits, *index).sym = Some(sym.to_string());
        }
        declared.entry(*at).or_default().push(sym);
        let entry = declarations.entry(sym).or_insert(*at);
        *entry = (*entry).min(*at);
        structured_syms.push((*index, sym));
    }
    let mut source_syms: Vec<(usize, (u32, u32), Vec<&str>)> = vec![];
    let mut first_uses: HashMap<&str, (u32, u32)> = HashMap::new();
    for (index, at, syms) in &local_sources {
        let (syms, changed) = renamed(syms, &renames);
        if changed {
            edit(&mut edits, *index).sym = Some(syms.join(","));
        }
        for sym in &syms {
            let entry = first_uses.entry(sym).or_insert(*at);
            *entry = (*entry).min(*at);
        }
        source_syms.push((*index, *at, syms));
    }

    // The innermost function definition whose body contains a location.
    let mut bodies: Vec<((u32, u32), (u32, u32), usize)> = functions
        .iter()
        .enumerate()
        .filter_map(|(fi, f)| f.body.map(|(start, end)| (start, end, fi)))
        .collect();
    bodies.sort();
    let enclosing = |at: (u32, u32)| -> Option<usize> {
        let before = bodies.partition_point(|(start, _, _)| *start <= at);
        bodies[..before]
            .iter()
            .rev()
            .find(|(_, end, _)| at < *end)
            .map(|(_, _, fi)| *fi)
    };
    // A location's function: a parameter's, or the enclosing definition.
    let function_at = |at: (u32, u32)| param_functions.get(&at).copied().or_else(|| enclosing(at));
    // Each local's pretty name: its declaration's (or first use's)
    // function's, and its own.
    let mut prettys: HashMap<&str, String> = HashMap::new();
    for sym in declarations.keys().chain(first_uses.keys()) {
        if prettys.contains_key(sym) {
            continue;
        }
        let Some(at) = declarations.get(sym).or_else(|| first_uses.get(sym)) else {
            continue;
        };
        let Some(name) = name_at(at) else {
            continue;
        };
        let pretty = match function_at(*at) {
            Some(fi) => format!("{}::{}", functions[fi].pretty, name),
            None => name.to_string(),
        };
        prettys.insert(sym, pretty);
    }
    let pretty_of = |sym: &str| prettys.get(sym).cloned();

    for (index, sym) in &structured_syms {
        if let Some(pretty) = pretty_of(sym) {
            edit(&mut edits, *index).pretty = Some(pretty);
        }
    }
    for (index, at, syms) in source_syms {
        let loc = records[index]
            .get("loc")
            .and_then(|l| l.as_str())
            .unwrap_or("");
        let mut targets = vec![];
        for sym in syms {
            if targeted.contains(sym) {
                continue;
            }
            let Some(pretty) = pretty_of(sym) else {
                continue;
            };
            let is_declaration = match declared.get(&at) {
                Some(syms) => syms.contains(&sym),
                None => !declarations.contains_key(sym) && first_uses.get(sym) == Some(&at),
            };
            let kind = match param_functions.get(&at) {
                Some(fi) if is_declaration && functions[*fi].body.is_none() => "decl",
                _ if is_declaration => "def",
                _ => "use",
            };
            let mut target = Map::new();
            target.insert("loc".to_string(), Value::String(loc.to_string()));
            target.insert("target".to_string(), Value::from(1));
            target.insert("kind".to_string(), Value::String(kind.to_string()));
            target.insert("pretty".to_string(), Value::String(pretty));
            target.insert("sym".to_string(), Value::String(sym.to_string()));
            let context = if is_declaration {
                function_at(at)
            } else {
                enclosing(at)
            };
            if let Some(fi) = context {
                let function = &functions[fi];
                target.insert(
                    "context".to_string(),
                    Value::String(function.pretty.to_string()),
                );
                target.insert(
                    "contextsym".to_string(),
                    Value::String(function.sym.to_string()),
                );
            }
            targets.push(Value::Object(target));
        }
        let edit = edit(&mut edits, index);
        edit.local_source = true;
        edit.targets = targets;
    }
    edits
}

/// Add to `renames` each local symbol that appears with others (ex: several
/// platforms' at a location) and isn't renamed already, to the least symbol
/// it's connected to.
fn unify_platform_symbols<'a>(
    sym_lists: impl Iterator<Item = &'a str>,
    renames: &mut HashMap<&'a str, String>,
) {
    // (A union-find of symbols, by index.)
    let mut index: HashMap<&str, usize> = HashMap::new();
    let mut parent: Vec<usize> = vec![];
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for syms in sym_lists {
        if !syms.contains(',') {
            continue;
        }
        let locals: Vec<&str> = syms
            .split(',')
            .filter(|sym| !renames.contains_key(*sym))
            .collect();
        if locals.len() < 2 {
            continue;
        }
        let mut ids = vec![];
        for sym in locals {
            let next = parent.len();
            let id = *index.entry(sym).or_insert(next);
            if id == next {
                parent.push(id);
            }
            ids.push(id);
        }
        for id in &ids[1..] {
            let (a, b) = (find(&mut parent, ids[0]), find(&mut parent, *id));
            parent[b] = a;
        }
    }
    let mut least: HashMap<usize, &str> = HashMap::new();
    for (sym, id) in &index {
        let root = find(&mut parent, *id);
        let entry = least.entry(root).or_insert(sym);
        if *sym < *entry {
            *entry = sym;
        }
    }
    for (sym, id) in &index {
        let root = find(&mut parent, *id);
        if least[&root] != *sym {
            renames.insert(sym, least[&root].to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The records without locals' target records (see `add_local_targets`).
    fn without_local_targets(records: Vec<Value>) -> Vec<Value> {
        records
            .into_iter()
            .filter(|r| {
                r.get("target").is_none() || !r["sym"].as_str().unwrap_or("").starts_with("V_")
            })
            .collect()
    }

    #[test]
    fn test_parameter_types() {
        assert_eq!(
            parameter_types("nsresult (const char *, _Bool *)"),
            Some(vec!["constchar*".to_string(), "_Bool*".to_string()])
        );
        assert_eq!(parameter_types("void (void)"), Some(vec![]));
        assert_eq!(parameter_types("void ()"), Some(vec![]));
        assert_eq!(
            parameter_types("void (Pair<A, B>, int)").map(|t| t.len()),
            Some(2)
        );
        assert_eq!(
            parameter_types("void (void (*)(int, int), int) const"),
            Some(vec!["void(*)(int,int)".to_string(), "int".to_string()])
        );
        assert_eq!(parameter_types("int"), None);
    }

    #[test]
    fn test_unify_parameter_symbols() {
        // A definition (with two platforms' symbols for its first
        // parameter, and a use of it), a declaration, and a lambda's
        // parameter in the body (not the function's).
        let mut records = vec![
            json!({"loc": "00002:5-8", "source": 1, "syntax": "def,function", "pretty": "function Foo",
                   "sym": "_Z3Fooib", "nestingRange": "2:30-6:0", "type": "void (int, _Bool)"}),
            json!({"loc": "00002:13-14", "source": 1, "syntax": "", "pretty": "variable a",
                   "sym": "V_aaaa_1,V_bbbb_1", "no_crossref": 1}),
            json!({"loc": "00002:13-14", "structured": 1, "pretty": "a", "sym": "V_aaaa_1",
                   "kind": "parameter"}),
            json!({"loc": "00002:13-14", "structured": 1, "pretty": "a", "sym": "V_bbbb_1",
                   "kind": "parameter"}),
            json!({"loc": "00002:21-22", "source": 1, "syntax": "", "pretty": "variable b",
                   "sym": "V_cccc_2", "no_crossref": 1}),
            json!({"loc": "00002:21-22", "structured": 1, "pretty": "b", "sym": "V_cccc_2",
                   "kind": "parameter"}),
            json!({"loc": "00003:2-3", "source": 1, "syntax": "", "pretty": "variable a",
                   "sym": "V_aaaa_1", "no_crossref": 1}),
            json!({"loc": "00004:20-21", "structured": 1, "pretty": "x", "sym": "V_dddd_3",
                   "kind": "parameter"}),
            json!({"loc": "00008:5-8", "source": 1, "syntax": "decl,function", "pretty": "function Bar",
                   "sym": "_Z3Bari", "type": "void (int)"}),
            json!({"loc": "00008:5-8", "target": 1, "kind": "decl", "pretty": "Bar",
                   "sym": "_Z3Bari", "peekRange": "8-8"}),
            json!({"loc": "00008:13-14", "structured": 1, "pretty": "n", "sym": "V_eeee_4",
                   "kind": "parameter"}),
            // (A function pointer's parameter after the declaration's lines:
            // not Bar's.)
            json!({"loc": "00009:30-31", "structured": 1, "pretty": "y", "sym": "V_ffff_5",
                   "kind": "parameter"}),
        ];
        process_local_symbols(&mut records);
        let records = without_local_targets(records);
        let a = parameter_symbol("_Z3Fooib", 0);
        let b = parameter_symbol("_Z3Fooib", 1);
        // (The two platforms' structured records for `a` are now one.)
        assert_eq!(records.len(), 11);
        assert_eq!(records[1]["sym"], json!(a));
        assert_eq!(records[2]["sym"], json!(a));
        assert_eq!(records[3]["sym"], json!(b));
        assert_eq!(records[5]["sym"], json!(a));
        assert_eq!(records[6]["sym"], json!("V_dddd_3"));
        assert_eq!(records[9]["sym"], json!(parameter_symbol("_Z3Bari", 0)));
        assert_eq!(records[10]["sym"], json!("V_ffff_5"));
        assert!(a.starts_with("V_") && a.split('_').count() == 3);
    }

    #[test]
    fn test_unify_with_structured_args() {
        // An unnamed first parameter: the structured record's args say the
        // named one is second.
        let mut records = vec![
            json!({"loc": "00002:5-8", "source": 1, "syntax": "def,function", "pretty": "function Foo",
                   "sym": "_Z3Fooib", "nestingRange": "2:30-6:0", "type": "void (int, _Bool)"}),
            json!({"loc": "00002:5-8", "structured": 1, "pretty": "Foo", "sym": "_Z3Fooib",
                   "kind": "function", "args": [{"name": "", "type": "int"}, {"name": "b", "type": "_Bool"}]}),
            json!({"loc": "00002:18-19", "structured": 1, "pretty": "b", "sym": "V_cccc_2",
                   "kind": "parameter"}),
        ];
        process_local_symbols(&mut records);
        let records = without_local_targets(records);
        assert_eq!(records[2]["sym"], json!(parameter_symbol("_Z3Fooib", 1)));
    }

    #[test]
    fn test_unnamed_declaration_parameter() {
        // Declarations with an unnamed first parameter: the named one's
        // index is its type's only position, if it has one.
        let mut records = vec![
            json!({"loc": "00002:4-7", "source": 1, "syntax": "decl,function", "pretty": "function Foo",
                   "sym": "_Z3Fooii", "type": "int (int, int)"}),
            json!({"loc": "00002:17-18", "source": 1, "syntax": "", "pretty": "variable b",
                   "sym": "V_aaaa_1", "no_crossref": 1, "type": "int"}),
            json!({"loc": "00002:17-18", "structured": 1, "pretty": "b", "sym": "V_aaaa_1",
                   "kind": "parameter"}),
            json!({"loc": "00003:4-7", "source": 1, "syntax": "decl,function", "pretty": "function Bar",
                   "sym": "_Z3Barib", "type": "int (int, _Bool)"}),
            json!({"loc": "00003:18-19", "source": 1, "syntax": "", "pretty": "variable b",
                   "sym": "V_bbbb_2", "no_crossref": 1, "type": "_Bool"}),
            json!({"loc": "00003:18-19", "structured": 1, "pretty": "b", "sym": "V_bbbb_2",
                   "kind": "parameter"}),
        ];
        process_local_symbols(&mut records);
        let records = without_local_targets(records);
        assert_eq!(records[1]["sym"], json!("V_aaaa_1"));
        assert_eq!(records[4]["sym"], json!(parameter_symbol("_Z3Barib", 1)));
        assert_eq!(records[5]["sym"], json!(parameter_symbol("_Z3Barib", 1)));
    }

    #[test]
    fn test_local_targets() {
        // A definition with a parameter and a local (with two platforms'
        // symbols), and a declaration with a parameter.
        let mut records = vec![
            json!({"loc": "00002:5-8", "source": 1, "syntax": "def,function", "pretty": "function ns::Foo",
                   "sym": "_ZN2ns3FooEi", "nestingRange": "2:16-6:0", "type": "void (int)"}),
            json!({"loc": "00002:13-14", "source": 1, "syntax": "", "pretty": "variable a",
                   "sym": "V_aaaa_1", "no_crossref": 1, "type": "int"}),
            json!({"loc": "00002:13-14", "structured": 1, "pretty": "a", "sym": "V_aaaa_1",
                   "kind": "parameter"}),
            json!({"loc": "00003:6-7", "source": 1, "syntax": "", "pretty": "variable x",
                   "sym": "V_xxxx_2,V_yyyy_2", "no_crossref": 1, "type": "int"}),
            json!({"loc": "00003:6-7", "structured": 1, "pretty": "x", "sym": "V_xxxx_2",
                   "kind": "localVar"}),
            json!({"loc": "00003:6-7", "structured": 1, "pretty": "x", "sym": "V_yyyy_2",
                   "kind": "localVar"}),
            json!({"loc": "00004:2-3", "source": 1, "syntax": "", "pretty": "variable x",
                   "sym": "V_xxxx_2,V_yyyy_2", "no_crossref": 1, "type": "int"}),
            json!({"loc": "00004:6-7", "source": 1, "syntax": "", "pretty": "variable a",
                   "sym": "V_aaaa_1", "no_crossref": 1, "type": "int"}),
            json!({"loc": "00008:5-8", "source": 1, "syntax": "decl,function", "pretty": "function ns::Bar",
                   "sym": "_ZN2ns3BarEi", "type": "void (int)"}),
            json!({"loc": "00008:13-14", "source": 1, "syntax": "", "pretty": "variable n",
                   "sym": "V_nnnn_3", "no_crossref": 1, "type": "int"}),
            json!({"loc": "00008:13-14", "structured": 1, "pretty": "n", "sym": "V_nnnn_3",
                   "kind": "parameter"}),
        ];
        process_local_symbols(&mut records);
        let a = parameter_symbol("_ZN2ns3FooEi", 0);
        let n = parameter_symbol("_ZN2ns3BarEi", 0);
        let targets: Vec<(String, String, String, String, String)> = records
            .iter()
            .filter(|r| r.get("target").is_some())
            .map(|r| {
                let field = |key: &str| r[key].as_str().unwrap_or("").to_string();
                (
                    field("loc"),
                    field("kind"),
                    field("sym"),
                    field("pretty"),
                    field("contextsym"),
                )
            })
            .collect();
        let target = |loc: &str, kind: &str, sym: &str, pretty: &str, contextsym: &str| {
            (
                loc.to_string(),
                kind.to_string(),
                sym.to_string(),
                pretty.to_string(),
                contextsym.to_string(),
            )
        };
        assert_eq!(
            targets,
            vec![
                target("00002:13-14", "def", &a, "ns::Foo::a", "_ZN2ns3FooEi"),
                target("00003:6-7", "def", "V_xxxx_2", "ns::Foo::x", "_ZN2ns3FooEi"),
                target("00004:2-3", "use", "V_xxxx_2", "ns::Foo::x", "_ZN2ns3FooEi"),
                target("00004:6-7", "use", &a, "ns::Foo::a", "_ZN2ns3FooEi"),
                target("00008:13-14", "decl", &n, "ns::Bar::n", "_ZN2ns3BarEi"),
            ]
        );
        // (The local's platforms' structured records are now one, and
        // structured records have the qualified pretty names.)
        let structured: Vec<(String, String)> = records
            .iter()
            .filter(|r| r.get("structured").is_some())
            .map(|r| {
                (
                    r["sym"].as_str().unwrap().to_string(),
                    r["pretty"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        assert_eq!(
            structured,
            vec![
                (a.clone(), "ns::Foo::a".to_string()),
                ("V_xxxx_2".to_string(), "ns::Foo::x".to_string()),
                (n.clone(), "ns::Bar::n".to_string()),
            ]
        );
        assert!(records.iter().all(|r| r.get("no_crossref").is_none()));

        // Reading the records again (ex: a merged file's) changes nothing.
        let again = records.clone();
        process_local_symbols(&mut records);
        assert_eq!(records, again);
    }
}
