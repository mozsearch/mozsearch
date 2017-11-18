
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

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
