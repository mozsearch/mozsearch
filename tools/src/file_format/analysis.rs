use std::fs::File;
use std::io::BufReader;
use std::io::BufRead;
use std::collections::HashMap;

extern crate rustc_serialize;
use self::rustc_serialize::json::{Json, Object};

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
    pub context: String,
    pub contextsym: String,
}

#[derive(Debug)]
pub struct AnalysisSource {
    pub pretty: String,
    pub sym: String,
    pub syntax: Vec<String>,
    pub no_crossref: bool,
}

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
    let file = match File::open(filename) {
        Ok(f) => f,
        Err(_) => return vec![],
    };
    let reader = BufReader::new(&file);
    let mut result = Vec::new();
    let mut lineno = 1;
    for line in reader.lines() {
        let line = line.unwrap();
        let data = Json::from_str(&line);
        let data = match data {
            Ok(data) => data,
            Err(_) => panic!("error on line {}: {}", lineno, &line),
        };
        lineno += 1;
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

    match last_loc {
        Some(ll) => result2.push(WithLocation { loc: ll, data: last_vec }),
        None => {},
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
    let context = match obj.get("context") {
        Some(json) => json.as_string().unwrap().to_string(),
        None => "".to_string()
    };
    let contextsym = match obj.get("contextsym") {
        Some(json) => json.as_string().unwrap().to_string(),
        None => "".to_string()
    };
    let sym = obj.get("sym").unwrap().as_string().unwrap().to_string();

    Some(AnalysisTarget { kind: kind, pretty: pretty, sym: sym, context: context, contextsym: contextsym })
}

pub fn read_source(obj : &Object) -> Option<AnalysisSource> {
    if !obj.contains_key("source") {
        return None;
    }

    let syntax = match obj.get("syntax") {
        Some(json) => json.as_string().unwrap().to_string(),
        None => "".to_string()
    };
    let syntax = syntax.split(',').map(|x| x.to_string()).collect::<Vec<_>>();

    let pretty = match obj.get("pretty") {
        Some(json) => json.as_string().unwrap().to_string(),
        None => "".to_string()
    };
    let sym = obj.get("sym").unwrap().as_string().unwrap().to_string();

    let no_crossref = match obj.get("no_crossref") {
        Some(_) => true,
        None => false,
    };

    Some(AnalysisSource { pretty: pretty, sym: sym, syntax: syntax, no_crossref: no_crossref })
}

pub struct Jump {
    pub id: String,
    pub path: String,
    pub lineno: u64,
    pub pretty: String,
}

pub fn read_jumps(filename: &str) -> HashMap<String, Jump> {
    let file = File::open(filename).unwrap();
    let reader = BufReader::new(&file);
    let mut result = HashMap::new();
    let mut lineno = 1;
    for line in reader.lines() {
        let line = line.unwrap();
        let data = Json::from_str(&line);
        let data = match data {
            Ok(data) => data,
            Err(_) => panic!("error on line {}: {}", lineno, &line),
        };
        lineno += 1;

        let array = data.as_array().unwrap();
        let id = array[0].as_string().unwrap().to_string();
        let data = Jump {
            id: id.clone(),
            path: array[1].as_string().unwrap().to_string(),
            lineno: array[2].as_u64().unwrap(),
            pretty: array[3].as_string().unwrap().to_string(),
        };

        result.insert(id, data);
    }
    result
}
