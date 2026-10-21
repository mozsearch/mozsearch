uniffi::include_scaffolding!("uniffied");

#[uniffi::export]
pub fn add(a: i32, b: i32) -> i32 { a + b }

// Exported via uniffied.udl
pub fn sub(a: i32, b: i32) -> i32 { a + b }
