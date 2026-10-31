use std::env;
use std::fs::File;
use std::io::Write;
use std::path::Path;

fn main() {
    let path = Path::new(&env::var_os("OUT_DIR").unwrap()).join("generated.rs");
    let mut file = File::create(path).unwrap();
    file.write_all("#[derive(Copy, Clone)]\n#[allow(dead_code)]\npub struct GeneratedType {\n  pub some_num: i32,\n}".as_bytes()).unwrap();
}
