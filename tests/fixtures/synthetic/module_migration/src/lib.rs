use serde::Deserialize;
use serde_json::{Value, json};
use tantalus_sdk::{ABI_VERSION, input_bytes, output_bytes};

#[derive(Debug, Deserialize)]
struct Request {
    abi_version: i32,
    module_state: Value,
    input: MigrationInput,
}

#[derive(Debug, Deserialize)]
struct MigrationInput {
    operation: String,
    from_schema: u32,
    to_schema: u32,
}

#[unsafe(no_mangle)]
pub extern "C" fn tantalus_abi_version() -> i32 {
    ABI_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn alloc(length: i32) -> i32 {
    let buffer = Vec::<u8>::with_capacity(length as usize);
    let pointer = buffer.as_ptr() as i32;
    std::mem::forget(buffer);
    pointer
}

#[unsafe(no_mangle)]
pub extern "C" fn tantalus_invoke(packed: i64) -> i64 {
    let request: Request = serde_json::from_slice(unsafe { input_bytes(packed) }).unwrap();
    assert_eq!(request.abi_version, ABI_VERSION);
    assert_eq!(request.input.operation, "migrate");
    assert_eq!(request.input.from_schema, 1);
    assert_eq!(request.input.to_schema, 2);
    let mut state = request.module_state;
    state["migrated"] = json!(true);
    state["schema"] = json!(2);
    output_bytes(serde_json::to_vec(&json!({"state": state, "events": []})).unwrap())
}
