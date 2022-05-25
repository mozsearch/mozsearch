use std::env;

extern crate env_logger;
extern crate tools;
use tools::config;
use tools::file_format::identifiers::IdentMap;

fn main() {
    env_logger::init();
    let tree_name = &env::args().nth(2).unwrap();
    let cfg = config::load(&env::args().nth(1).unwrap(), false, Some(tree_name));
    let id_map = IdentMap::load(&cfg);
    let ids = id_map.get(tree_name).unwrap();
    let results = ids.lookup(&env::args().nth(3).unwrap(), false, true, 20);
    for result in results {
        println!("R `{}` = `{}`", result.id, result.symbol);
    }
}
