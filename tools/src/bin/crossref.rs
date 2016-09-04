use std::fs::File;
use std::env;
use std::io::BufReader;
use std::io::BufRead;
use std::io::Write;
use std::collections::HashMap;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

extern crate tools;
use tools::find_source_file;
use tools::analysis::{read_analysis, read_target, AnalysisKind};
use tools::config;

extern crate rustc_serialize;
use rustc_serialize::json::{Json, ToJson};

#[derive(Debug, RustcEncodable, RustcDecodable)]
struct SearchResult {
    lineno: u32,
    bounds: (u32, u32),
    line: String,
    context: String,
    contextsym: String,
}

impl ToJson for SearchResult {
    fn to_json(&self) -> Json {
        let (st, en) = self.bounds;
        let bounds = vec![st, en];

        let mut obj = BTreeMap::new();
        obj.insert("lno".to_string(), self.lineno.to_json());
        obj.insert("bounds".to_string(), bounds.to_json());
        obj.insert("line".to_string(), self.line.to_json());
        obj.insert("context".to_string(), self.context.to_json());
        obj.insert("contextsym".to_string(), self.contextsym.to_json());
        Json::Object(obj)
    }
}

fn split_scopes(id: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut argument_nesting = 0;
    for (index, m) in id.match_indices(|c| c == ':' || c == '<' || c == '>') {
        if m == ":" && argument_nesting == 0 {
            if start != index {
                result.push(id[start .. index].to_owned());
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
    result.push(id[start ..].to_owned());
    return result;
}

fn main() {
    let args: Vec<_> = env::args().collect();

    let cfg = config::load(&args[1], false);

    let tree_name = &args[2];
    let tree_config = cfg.trees.get(tree_name).unwrap();

    let filenames_file = &args[3];

    let output_file = format!("{}/crossref", tree_config.paths.index_path);
    let jump_file = format!("{}/jumps", tree_config.paths.index_path);
    let id_file = format!("{}/identifiers", tree_config.paths.index_path);

    let mut table = HashMap::new();
    let mut pretty_table = HashMap::new();
    let mut id_table = HashMap::new();
    let mut jumps = Vec::new();

    {
        let mut process_file = |path: &str| {
            print!("File {}\n", path);

            let analysis_fname = format!("{}/analysis/{}", tree_config.paths.index_path, path);
            let analysis = read_analysis(&analysis_fname, &read_target);

            let source_fname = find_source_file(path, &tree_config.paths.files_path, &tree_config.paths.objdir_path);
            let source_file = match File::open(source_fname) {
                Ok(f) => f,
                Err(_) => {
                    println!("Unable to open source file");
                    return;
                },
            };
            let reader = BufReader::new(&source_file);
            let mut lines = Vec::new();
            for line in reader.lines() {
                match line {
                    Ok(l) => lines.push(l),
                    Err(_) => lines.push("".to_string()),
                }
            }

            for datum in analysis {
                for piece in datum.data {
                    let t1 = table.entry(piece.sym.to_owned()).or_insert(BTreeMap::new());
                    let t2 = t1.entry(piece.kind).or_insert(BTreeMap::new());
                    let t3 = t2.entry(path.to_owned()).or_insert(Vec::new());
                    let lineno = (datum.loc.lineno - 1) as usize;
                    if lineno >= lines.len() {
                        print!("Bad line number in file {} (line {})\n", path, lineno);
                        return;
                    }
                    let line = lines[lineno].clone();
                    let line_cut = line.trim_right();
                    let len = line_cut.len();
                    let line_cut = line_cut.trim_left();
                    let offset = (len - line_cut.len()) as u32;
                    let mut buf = String::new();
                    let mut i = 0;
                    for c in line_cut.chars() {
                        buf.push(c);
                        i += 1;
                        if i > 100 {
                            break;
                        }
                    }
                    t3.push(SearchResult {
                        lineno: datum.loc.lineno,
                        bounds: (datum.loc.col_start - offset, datum.loc.col_end - offset),
                        line: buf,
                        context: piece.context,
                        contextsym: piece.contextsym,
                    });

                    pretty_table.insert(piece.sym.to_owned(), piece.pretty.to_owned());

                    let ch = piece.sym.chars().nth(0).unwrap();
                    if !(ch >= '0' && ch <= '9') && !piece.sym.contains(' ') {
                        let t1 = id_table.entry(piece.pretty.to_owned()).or_insert(BTreeSet::new());
                        t1.insert(piece.sym.to_owned());
                    }
                }
            }
        };

        let f = File::open(filenames_file).unwrap();
        let file = BufReader::new(&f);
        for line in file.lines() {
            process_file(&line.unwrap());
        }
    }

    let mut outputf = File::create(output_file).unwrap();

    for (id, id_data) in table {
        let mut kindmap = BTreeMap::new();
        for (kind, kind_data) in &id_data {
            let mut result = Vec::new();
            for (path, results) in kind_data {
                let mut obj = BTreeMap::new();
                obj.insert("path".to_string(), path.to_json());
                obj.insert("lines".to_string(), results.to_json());
                result.push(Json::Object(obj));
            }
            let kindstr = match *kind {
                AnalysisKind::Use => "Uses",
                AnalysisKind::Def => "Definitions",
                AnalysisKind::Assign => "Assignments",
                AnalysisKind::Decl => "Declarations",
                AnalysisKind::Idl => "IDL",
            };
            kindmap.insert(kindstr.to_string(), Json::Array(result));
        }
        let kindmap = Json::Object(kindmap);

        let _ = outputf.write_all(format!("{}\n{}\n", id, kindmap.to_string()).as_bytes());

        if id_data.contains_key(&AnalysisKind::Def) {
            let defs = id_data.get(&AnalysisKind::Def).unwrap();
            if defs.len() == 1 {
                for (path, results) in defs {
                    if results.len() == 1 {
                        let mut v = Vec::new();
                        v.push(id.to_json());
                        v.push(path.to_json());
                        v.push(results[0].lineno.to_json());
                        let pretty = pretty_table.get(&id).unwrap();
                        v.push(pretty.to_json());
                        jumps.push(Json::Array(v));
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
            let components = split_scopes(&id);
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
