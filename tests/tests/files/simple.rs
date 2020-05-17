extern crate test_rust_dependency;

use std::path::{Path, PathBuf};
use test_rust_dependency::{MyType, MyTrait};
use test_rust_dependency::my_mod::MyOtherType;

/* A grab-bug of rust code to exercise the searchfox indexer.
   Note how this comment ends up in the file description, too!
*/

mod build_time_generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

#[derive(Clone)]
pub struct Loader {
    #[allow(unused)]
    whatever: build_time_generated::GeneratedType,
    deps_dir: PathBuf,
    my_type: MyType,
}

impl MyTrait for Loader {
    fn do_bar() -> i32 { 10000 }

    fn do_foo() -> MyType {
        MyType::new().do_foo()
    }
}

extern "C" fn WithoutNoMangle() {}

#[no_mangle]
extern "C" fn WithNoMangle() {}

impl Loader {
    pub fn new(deps_dir: PathBuf) -> Self {
        Self {
            whatever: build_time_generated::GeneratedType{ some_num: 1 },
            deps_dir,
            my_type: MyType::new(),
        }
    }

    fn needs_hard_reload(&self, _: &Path) -> bool {
        unsafe {
            test_rust_dependency::ExternFunctionImplementedInCpp();
        }
        true
    }

    fn set_path_prefix(&mut self, _: &Path) {
        MyType::new().do_foo();
    }

    fn abs_path_prefix(&self) -> Option<PathBuf> { None }
    fn search_directories(&self) -> Vec<PathBuf> {
        vec![self.deps_dir.clone()]
    }
}

enum AnEnum {
    Variant1,
    Variant2,
}

fn simple_fn() {
    let my_enum = AnEnum::Variant1;
    match my_enum {
        AnEnum::Variant1 => println!("Yay"),
        _ => println!("Boo"),
    }
}

struct MultiParams<'a, T> {
    myvar: T,
    another_var: &'a u32,
}

impl<'a, T> MultiParams<'a, T> {
    fn fn_with_params_in_signature(&self) {
    }
}
