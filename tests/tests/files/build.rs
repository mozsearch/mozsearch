use std::fs::File;
use std::env;
use std::path::Path;
use std::io::Write;

fn main() {
    let path = Path::new(&env::var_os("OUT_DIR").unwrap()).join("generated.rs");
    let mut file = File::create(path).unwrap();
    file.write_all("#[derive(Copy, Clone)]\npub struct GeneratedType {\n  pub some_num: i32,\n}".as_bytes()).unwrap();
}
