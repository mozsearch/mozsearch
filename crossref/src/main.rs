use std::fs::File;
use std::env;
use std::io::BufReader;
use std::io::BufRead;
use std::io::Write;
use std::collections::HashMap;
use std::collections::BTreeMap;

extern crate rustc_serialize;
use rustc_serialize::json::{Json, Object, ToJson};

#[derive(Eq, PartialEq, PartialOrd, Ord, Debug)]
struct Location {
    lineno: u32,
    col_start: u32,
    col_end: u32,
}

#[derive(Debug)]
struct WithLocation<T> {
    data: T,
    loc: Location,
}

#[derive(Debug, Eq, PartialEq, Ord, PartialOrd)]
enum AnalysisKind {
    Use,
    Def,
    Assign,
    Decl,
    Idl,
}

#[derive(Debug)]
struct AnalysisTarget {
    kind: AnalysisKind,
    pretty: String,
    sym: String,
}

#[derive(Debug, RustcEncodable, RustcDecodable)]
struct SearchResult {
    lineno: u32,
    line: String,
}

impl ToJson for SearchResult {
    fn to_json(&self) -> Json {
        let mut obj = BTreeMap::new();
        obj.insert("lno".to_string(), self.lineno.to_json());
        obj.insert("line".to_string(), self.line.to_json());
        Json::Object(obj)
    }
}

/*
struct AnalysisSource {
    loc: String,
    pretty: String,
    sym: String,
}
*/

/*
struct AnalysisIterator<T, LinesIter> {
    filter: &Fn(Json) -> Option<T>,
    lines: LinesIter,
}
 */

fn parse_location(loc: &str) -> Location {
    let v : Vec<&str> = loc.split(":").collect();
    let lineno = v[0].parse::<u32>().unwrap();
    let (col_start, col_end) = if v[1].contains("-") {
        let v : Vec<&str> = v[1].split("-").collect();
        (v[0], v[1])
    } else {
        (v[1], v[1])
    };
    let col_start = col_start.parse::<u32>().unwrap();
    let col_end = col_end.parse::<u32>().unwrap();
    Location { lineno: lineno, col_start: col_start, col_end: col_end }
}

fn read_analysis<T>(filename: &str, filter: &Fn(&Object) -> Option<T>) -> Vec<WithLocation<Vec<T>>> {
    let f = File::open(filename).unwrap();
    let file = BufReader::new(&f);
    let mut result = Vec::new();
    for line in file.lines() {
        let data = Json::from_str(&line.unwrap()).unwrap();
        let obj = data.as_object().unwrap();
        let loc = parse_location(obj.get("loc").unwrap().as_string().unwrap());
        match filter(obj) {
            Some(v) => result.push(WithLocation { data: v, loc: loc }),
            None => {}
        }
    }

    result.sort_by(|x1, x2| {
        x1.loc.cmp(&x2.loc)
    });

    let mut result2 = Vec::new();
    let mut last_loc = None;
    let mut last_vec = Vec::new();
    for r in result {
        match last_loc {
            Some(ll) => {
                if ll == r.loc {
                    last_loc = Some(ll);
                } else {
                    result2.push(WithLocation { loc: ll, data: last_vec });
                    last_vec = Vec::new();
                    last_loc = Some(r.loc);
                }
            },
            None => {
                last_loc = Some(r.loc);
            }
        }
        last_vec.push(r.data);
    }

    result2
}

fn read_target(obj : &Object) -> Option<AnalysisTarget> {
    if !obj.contains_key("target") {
        return None;
    }

    let kindstr = obj.get("kind").unwrap().as_string().unwrap();
    let kind = match kindstr {
        "use" => AnalysisKind::Use,
        "def" => AnalysisKind::Def,
        "assign" => AnalysisKind::Assign,
        "decl" => AnalysisKind::Decl,
        "idl" => AnalysisKind::Idl,
        _ => panic!("bad target kind")
    };

    let pretty = match obj.get("pretty") {
        Some(json) => json.as_string().unwrap().to_string(),
        None => "".to_string()
    };
    let sym = obj.get("sym").unwrap().as_string().unwrap().to_string();

    Some(AnalysisTarget { kind: kind, pretty: pretty, sym: sym })
}

fn source_path(path: &str, tree_root: &str, objdir: &str) -> String {
    if path.starts_with("__GENERATED__") {
        return path.replace("__GENERATED__", objdir);
    }
    (tree_root.to_string() + "/").to_string() + path
}

fn main() {
    let args: Vec<_> = env::args().collect();

    let tree_root = &args[1];
    let index_root = &args[2];
    //let mozsearch_root = &args[3];
    let objdir = &args[4];
    let filenames_file = &args[5];

    let analysis_root = index_root.to_string() + "/analysis";
    let output_file = index_root.to_string() + "/crossref";
    let jump_file = index_root.to_string() + "/jumps";

    let mut table = HashMap::new();
    let mut pretty_table = HashMap::new();
    let mut jumps = Vec::new();

    {
        let mut process_file = |path: &str| {
            print!("File {}\n", path);

            let analysis_file = (analysis_root.to_string() + "/").to_string() + path;

            let analysis = read_analysis(&analysis_file, &read_target);

            let source = source_path(path, tree_root, objdir);
            let f = match File::open(source) {
                Ok(f) => f,
                Err(_) => return,
            };
            let file = BufReader::new(&f);
            let mut lines = Vec::new();
            for line in file.lines() {
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
                    let line_cut = line.trim();
                    let mut buf = String::new();
                    let mut i = 0;
                    for c in line_cut.chars() {
                        buf.push(c);
                        i += 1;
                        if i > 100 {
                            break;
                        }
                    }
                    t3.push(SearchResult { lineno: datum.loc.lineno, line: buf });

                    pretty_table.insert(piece.sym.to_owned(), piece.pretty.to_owned());
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
}
