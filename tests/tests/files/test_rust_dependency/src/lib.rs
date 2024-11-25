//! A description of this very interesting module.
//!
//! This is an overview of yada yada yada...

#[derive(Copy, Clone)]
pub struct MyType(());

impl MyType {
    pub fn new() -> Self {
        MyType(())
    }

    pub fn do_foo(self) -> MyType {
        unimplemented!()
    }
}

pub mod my_mod {
    pub struct MyOtherType;
}

pub trait MyTrait {
    fn do_bar() -> i32;

    fn do_foo() -> MyType {
        MyType::new()
    }
}

extern "C" {
    pub fn ExternFunctionImplementedInCpp();
}

impl MyTrait for MyType {
    fn do_bar() -> i32 {
        100
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
