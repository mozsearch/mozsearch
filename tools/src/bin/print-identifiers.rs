use std::env;

extern crate tools;
use tools::config;
use tools::file_format::identifiers::IdentMap;

fn main() {
    let cfg = config::load(&env::args().nth(1).unwrap(), false);
    let id_map = IdentMap::load(&cfg);
    let ids = id_map.get(&env::args().nth(2).unwrap()).unwrap();
    let results = ids.lookup(&env::args().nth(3).unwrap(), false, true, 20);
    for result in results {
        println!("R `{}` = `{}`", result.id, result.symbol);
    }
}
