//! Unified symbols for C++ functions' parameters.
//!
//! The clang plugin gives parameters `V_` symbols (like other locals) whose
//! hash includes their locations, so a function's declaration (ex: in a
//! header) and definition (in a `.cpp`) name the same parameter differently,
//! and so do platforms whose generated files differ.  As the tools read a
//! file's analysis, `unify_parameter_symbols` renames each parameter's
//! symbols to one derived from its function's symbol and its position,
//! `parameter_symbol`, which every file (and analyzer, ex: ipdl-analyze's
//! binding slots for messages' parameters) agrees on, without changing the
//! plugin (firefox's copy of which is what indexes firefox).

use std::collections::HashMap;

use serde_json::Value;

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

struct Function {
    loc: (u32, u32),
    /// Where its body starts, for definitions.
    body: Option<(u32, u32)>,
    /// Its declaration's last line (its target record's `peekRange`'s).
    last_line: Option<u32>,
    sym: String,
    /// Its parameters' types, from its type.
    types: Option<Vec<String>>,
    /// Its parameters' names, from its structured record, if this file has
    /// it.
    arg_names: Option<Vec<String>>,
}

/// A parameter's records at a location: its symbols (ex: several
/// platforms'), name, and type (without whitespace).
struct Param {
    syms: Vec<String>,
    name: String,
    ty: Option<String>,
}

/// Rename the parameters' `V_` symbols in a file's analysis records to
/// `parameter_symbol`s.
///
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
/// symbols.  Structured records that renaming made identical (several
/// platforms') are dropped.
pub fn unify_parameter_symbols(records: &mut Vec<Value>) {
    let mut functions: Vec<Function> = vec![];
    let mut param_records: Vec<((u32, u32), String, String)> = vec![];
    let mut structured_args: HashMap<String, Vec<String>> = HashMap::new();
    let mut last_lines: HashMap<(u32, u32), u32> = HashMap::new();
    let mut types_at: HashMap<(u32, u32), String> = HashMap::new();
    for record in records.iter() {
        let Some(loc) = record.get("loc").and_then(|l| l.as_str()) else {
            continue;
        };
        let loc = parse_location(loc);
        let at = (loc.lineno, loc.col_start);
        let sym = record.get("sym").and_then(|s| s.as_str()).unwrap_or("");
        if record.get("source").is_some() {
            if sym.starts_with("V_")
                && let Some(ty) = record.get("type").and_then(|t| t.as_str())
            {
                types_at.insert(at, ty.split_whitespace().collect());
                continue;
            }
            let syntax = record.get("syntax").and_then(|s| s.as_str()).unwrap_or("");
            let kinds: Vec<&str> = syntax.split(',').collect();
            if !(kinds.contains(&"def") || kinds.contains(&"decl"))
                || !kinds.iter().any(|k| *k == "function" || *k == "destructor")
                || record.get("inExpansionAt").is_some()
            {
                continue;
            }
            // (The least of several platforms' symbols, so every file
            // agrees.)
            let Some(sym) = split_symbol_list(sym).into_iter().min() else {
                continue;
            };
            let body = record
                .get("nestingRange")
                .and_then(|r| r.as_str())
                .and_then(|r| r.split_once('-'))
                .and_then(|(start, _)| start.split_once(':'))
                .and_then(|(l, c)| Some((l.parse().ok()?, c.parse().ok()?)));
            let types = record
                .get("type")
                .and_then(|t| t.as_str())
                .and_then(parameter_types);
            functions.push(Function {
                loc: at,
                body,
                last_line: None,
                sym: sym.to_string(),
                types,
                arg_names: None,
            });
        } else if record.get("target").is_some() {
            if let Some(last) = record
                .get("peekRange")
                .and_then(|r| r.as_str())
                .and_then(|r| r.split_once('-'))
                .and_then(|(_, end)| end.parse::<u32>().ok())
            {
                let entry = last_lines.entry(at).or_default();
                *entry = (*entry).max(last);
            }
        } else if record.get("structured").is_some() {
            let kind = record.get("kind").and_then(|k| k.as_str()).unwrap_or("");
            if kind == "parameter" && sym.starts_with("V_") {
                let name = record
                    .get("pretty")
                    .and_then(|p| p.as_str())
                    .unwrap_or("")
                    .to_string();
                param_records.push((at, sym.to_string(), name));
            } else if let Some(args) = record.get("args").and_then(|a| a.as_array()) {
                structured_args.insert(
                    sym.to_string(),
                    args.iter()
                        .map(|a| {
                            a.get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("")
                                .to_string()
                        })
                        .collect(),
                );
            }
        }
    }
    if param_records.is_empty() || functions.is_empty() {
        return;
    }
    for function in &mut functions {
        function.arg_names = structured_args.get(&function.sym).cloned();
        function.last_line = last_lines.get(&function.loc).copied();
    }
    functions.sort_by_key(|f| f.loc);
    functions.dedup_by(|a, b| a.loc == b.loc);
    param_records.sort();

    // Each function's parameters, in order.
    let mut by_function: HashMap<usize, Vec<Param>> = HashMap::new();
    let mut i = 0;
    while i < param_records.len() {
        let at = param_records[i].0;
        let mut j = i;
        while j < param_records.len() && param_records[j].0 == at {
            j += 1;
        }
        let param = Param {
            syms: param_records[i..j].iter().map(|p| p.1.clone()).collect(),
            name: param_records[i].2.clone(),
            ty: types_at.get(&at).cloned(),
        };
        i = j;
        // (The functions are sorted by location.)
        let Some(fi) = functions.partition_point(|f| f.loc < at).checked_sub(1) else {
            continue;
        };
        let function = &functions[fi];
        let within = match (function.body, function.last_line) {
            (Some(body), _) => at < body,
            (None, Some(last_line)) => at.0 <= last_line,
            (None, None) => true,
        };
        if within {
            by_function.entry(fi).or_default().push(param);
        }
    }

    // (Each parameter location's symbols, ex: several platforms', share a
    // new symbol.)
    let mut renames: HashMap<String, String> = HashMap::new();
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
                        let earlier = taken.entry(param.name.as_str()).or_default();
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
            let new_sym = parameter_symbol(&function.sym, index);
            for sym in &param.syms {
                renames.insert(sym.clone(), new_sym.clone());
            }
        }
    }
    if renames.is_empty() {
        return;
    }
    let mut renamed_structured: Vec<bool> = vec![false; records.len()];
    for (ri, record) in records.iter_mut().enumerate() {
        let Some(obj) = record.as_object_mut() else {
            continue;
        };
        let Some(syms) = obj.get("sym").and_then(|s| s.as_str()) else {
            continue;
        };
        if !syms.contains("V_") {
            continue;
        }
        let mut changed = false;
        let mut new_syms: Vec<String> = vec![];
        for sym in split_symbol_list(syms) {
            let new_sym = match renames.get(sym) {
                Some(new_sym) => {
                    changed = true;
                    new_sym.clone()
                }
                None => sym.to_string(),
            };
            if !new_syms.contains(&new_sym) {
                new_syms.push(new_sym);
            }
        }
        if changed {
            obj.insert("sym".to_string(), Value::String(new_syms.join(",")));
            renamed_structured[ri] = obj.contains_key("structured");
        }
    }
    let mut kept: HashMap<String, Vec<Value>> = HashMap::new();
    let mut ri = 0;
    records.retain(|record| {
        let renamed = renamed_structured[ri];
        ri += 1;
        if !renamed {
            return true;
        }
        let sym = record["sym"].as_str().unwrap_or("").to_string();
        let same = kept.entry(sym).or_default();
        if same.contains(record) {
            return false;
        }
        same.push(record.clone());
        true
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        unify_parameter_symbols(&mut records);
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
        unify_parameter_symbols(&mut records);
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
        unify_parameter_symbols(&mut records);
        assert_eq!(records[1]["sym"], json!("V_aaaa_1"));
        assert_eq!(records[4]["sym"], json!(parameter_symbol("_Z3Barib", 1)));
        assert_eq!(records[5]["sym"], json!(parameter_symbol("_Z3Barib", 1)));
    }
}
