use std::fs::File;
use std::io::BufReader;
use std::io::BufRead;

extern crate rustc_serialize;
use rustc_serialize::json::{Json, Object};

#[derive(Eq, PartialEq, PartialOrd, Ord, Debug)]
pub struct Location {
    pub lineno: u32,
    pub col_start: u32,
    pub col_end: u32,
}

#[derive(Debug)]
pub struct WithLocation<T> {
    pub data: T,
    pub loc: Location,
}

#[derive(Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum AnalysisKind {
    Use,
    Def,
    Assign,
    Decl,
    Idl,
}

#[derive(Debug)]
pub struct AnalysisTarget {
    pub kind: AnalysisKind,
    pub pretty: String,
    pub sym: String,
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

pub fn read_analysis<T>(filename: &str, filter: &Fn(&Object) -> Option<T>) -> Vec<WithLocation<Vec<T>>> {
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

pub fn read_target(obj : &Object) -> Option<AnalysisTarget> {
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

pub fn source_path(path: &str, tree_root: &str, objdir: &str) -> String {
    if path.starts_with("__GENERATED__") {
        return path.replace("__GENERATED__", objdir);
    }
    (tree_root.to_string() + "/").to_string() + path
}
