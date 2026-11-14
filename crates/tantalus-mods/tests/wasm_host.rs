use serde_json::json;
use tantalus_contracts::{ContentRevision, ModuleLock, ProfileLock};
use tantalus_mods::*;

fn lock() -> ProfileLock {
    ProfileLock {
        content_revision: ContentRevision::from_u64(1),
        content_fingerprint: "fixture".to_owned(),
        modules: vec![ModuleLock {
            package: "fixture.action".to_owned(),
            version: "1.0.0".to_owned(),
            required: true,
            state_schema: 1,
            content_hash: "a".repeat(64),
            features: Vec::new(),
        }],
    }
}

fn data(offset: usize, bytes: &[u8]) -> String {
    let escaped = bytes
        .iter()
        .map(|byte| format!("\\{byte:02x}"))
        .collect::<String>();
    format!("(data (i32.const {offset}) \"{escaped}\")")
}

fn transactional_module() -> Vec<u8> {
    let output =
        br#"{"state":{"calls":1},"events":[{"name":"action.completed","data":{"ok":true}}]}"#;
    let set_state = br#"{"kind":"set_state","key":"fixture:progress/gil","value":100}"#;
    let allocate = br#"{"kind":"allocate_id"}"#;
    let packed = (output.len() as i64) << 32 | 1024;
    let behavior_offset = br#"{"abi_version":1,"input":""#.len();
    wat::parse_str(format!(
        r#"(module
            (import "tantalus" "emit" (func $emit (param i32 i32)))
            (memory (export "memory") 1)
            {output_data}
            {set_data}
            {allocate_data}
            (func (export "tantalus_abi_version") (result i32) i32.const 1)
            (func (export "alloc") (param i32) (result i32) i32.const 4096)
            (func (export "tantalus_invoke") (param i64) (result i64)
                i32.const 512 i32.const {set_len} call $emit
                i32.const 768 i32.const {allocate_len} call $emit
                local.get 0 i32.wrap_i64 i32.load8_u offset={behavior_offset}
                i32.const 116 i32.eq
                if unreachable end
                i64.const {packed})
        )"#,
        output_data = data(1024, output),
        set_data = data(512, set_state),
        allocate_data = data(768, allocate),
        set_len = set_state.len(),
        allocate_len = allocate.len(),
    ))
    .unwrap()
}

fn host(wasm: &[u8]) -> ModuleHost {
    ModuleHost::instantiate(&lock(), "fixture.action", wasm, ModuleLimits::default()).unwrap()
}

#[test]
fn real_wasm_callback_commits_state_ids_events_and_fuel_accounting() {
    let bytes = transactional_module();
    assert_eq!(ModuleHost::wasm_sha256(&bytes).len(), 64);
    let mut host = host(&bytes);
    let result = host.invoke(&json!("okay")).unwrap();

    assert_eq!(result.module_state, json!({"calls": 1}));
    assert_eq!(result.events[0].name, "action.completed");
    assert_eq!(
        host.authoritative_state()["fixture:progress/gil"],
        json!(100)
    );
    assert_eq!(host.next_id(), 2);
    assert!(result.fuel_consumed > 0);
}

#[test]
fn identical_inputs_produce_identical_state_and_fuel_use() {
    let bytes = transactional_module();
    let mut left = host(&bytes);
    let mut right = host(&bytes);
    for input in [json!("okay"), json!("trap"), json!("okay")] {
        let left_result = left.invoke(&input);
        let right_result = right.invoke(&input);
        assert_eq!(left_result.is_ok(), right_result.is_ok());
        if let (Ok(left), Ok(right)) = (left_result, right_result) {
            assert_eq!(left, right);
        }
        assert_eq!(left.authoritative_state(), right.authoritative_state());
        assert_eq!(left.module_state(), right.module_state());
        assert_eq!(left.next_id(), right.next_id());
    }
}

#[test]
fn trap_after_staged_mutations_rolls_back_guest_and_host_state() {
    let mut host = host(&transactional_module());
    host.invoke(&json!("okay")).unwrap();
    let state_before = host.authoritative_state().clone();
    let module_before = host.module_state().clone();

    let error = host.invoke(&json!("trap")).unwrap_err();
    assert_eq!(error.kind, ModuleErrorKind::Trap);
    assert_eq!(host.authoritative_state(), &state_before);
    assert_eq!(host.module_state(), &module_before);
    assert_eq!(host.next_id(), 2);

    host.invoke(&json!("okay")).unwrap();
    assert_eq!(host.next_id(), 3);
}

#[test]
fn infinite_loop_exhausts_deterministic_fuel_without_mutating_host() {
    let wasm = wat::parse_str(
        r#"(module
            (memory (export "memory") 1)
            (func (export "tantalus_abi_version") (result i32) i32.const 1)
            (func (export "alloc") (param i32) (result i32) i32.const 4096)
            (func (export "tantalus_invoke") (param i64) (result i64)
                (loop $forever br $forever)
                unreachable)
        )"#,
    )
    .unwrap();
    let mut host = host(&wasm);
    let error = host.invoke(&json!(null)).unwrap_err();
    assert_eq!(error.kind, ModuleErrorKind::FuelExhausted);
    assert!(host.authoritative_state().is_empty());
    assert_eq!(host.next_id(), 1);
}

#[test]
fn abi_and_required_exports_are_checked_before_callback_commit() {
    let wrong_version = wat::parse_str(
        r#"(module
            (memory (export "memory") 1)
            (func (export "tantalus_abi_version") (result i32) i32.const 99)
            (func (export "alloc") (param i32) (result i32) i32.const 4096)
            (func (export "tantalus_invoke") (param i64) (result i64) i64.const 0)
        )"#,
    )
    .unwrap();
    assert_eq!(
        host(&wrong_version).invoke(&json!(null)).unwrap_err().kind,
        ModuleErrorKind::AbiMismatch
    );

    let missing = wat::parse_str(r#"(module (memory (export "memory") 1))"#).unwrap();
    assert_eq!(
        host(&missing).invoke(&json!(null)).unwrap_err().kind,
        ModuleErrorKind::MissingExport
    );
}

#[test]
fn unapproved_nondeterministic_import_is_not_available() {
    let wasm = wat::parse_str(
        r#"(module
            (import "env" "clock" (func $clock))
            (memory (export "memory") 1)
            (func (export "tantalus_abi_version") (result i32) i32.const 1)
            (func (export "alloc") (param i32) (result i32) i32.const 4096)
            (func (export "tantalus_invoke") (param i64) (result i64) call $clock i64.const 0)
        )"#,
    )
    .unwrap();
    let error = host(&wasm).invoke(&json!(null)).unwrap_err();
    assert_eq!(error.kind, ModuleErrorKind::InvalidModule);
    assert!(error.context.contains("clock"));
}

#[test]
fn input_output_and_command_limits_fail_without_commit() {
    let bytes = transactional_module();
    let small_input = ModuleLimits {
        max_input_bytes: 16,
        ..ModuleLimits::default()
    };
    let mut host = ModuleHost::instantiate(&lock(), "fixture.action", &bytes, small_input).unwrap();
    assert_eq!(
        host.invoke(&json!("okay")).unwrap_err().kind,
        ModuleErrorKind::ResourceLimit
    );

    let small_output = ModuleLimits {
        max_output_bytes: 16,
        ..ModuleLimits::default()
    };
    let mut host =
        ModuleHost::instantiate(&lock(), "fixture.action", &bytes, small_output).unwrap();
    assert_eq!(
        host.invoke(&json!("okay")).unwrap_err().kind,
        ModuleErrorKind::ResourceLimit
    );
    assert!(host.authoritative_state().is_empty());

    let no_commands = ModuleLimits {
        max_commands: 0,
        ..ModuleLimits::default()
    };
    let mut host = ModuleHost::instantiate(&lock(), "fixture.action", &bytes, no_commands).unwrap();
    assert_eq!(
        host.invoke(&json!("okay")).unwrap_err().kind,
        ModuleErrorKind::HostValidation
    );
    assert_eq!(host.next_id(), 1);
}

#[test]
fn malformed_guest_output_and_memory_growth_do_not_commit() {
    let output = b"not json";
    let packed = (output.len() as i64) << 32 | 1024;
    let malformed = wat::parse_str(format!(
        r#"(module
            (memory (export "memory") 1)
            {}
            (func (export "tantalus_abi_version") (result i32) i32.const 1)
            (func (export "alloc") (param i32) (result i32) i32.const 4096)
            (func (export "tantalus_invoke") (param i64) (result i64) i64.const {})
        )"#,
        data(1024, output),
        packed
    ))
    .unwrap();
    assert_eq!(
        host(&malformed).invoke(&json!(null)).unwrap_err().kind,
        ModuleErrorKind::HostValidation
    );

    let grow = wat::parse_str(
        r#"(module
            (memory (export "memory") 1)
            (func (export "tantalus_abi_version") (result i32) i32.const 1)
            (func (export "alloc") (param i32) (result i32) i32.const 4096)
            (func (export "tantalus_invoke") (param i64) (result i64)
                i32.const 100 memory.grow drop
                i32.const 10000000 i32.load drop
                i64.const 0)
        )"#,
    )
    .unwrap();
    assert_eq!(
        host(&grow).invoke(&json!(null)).unwrap_err().kind,
        ModuleErrorKind::ResourceLimit
    );
}

#[test]
fn module_must_belong_to_the_profile_lock() {
    let error = ModuleHost::instantiate(
        &lock(),
        "fixture.unselected",
        &transactional_module(),
        ModuleLimits::default(),
    )
    .err()
    .unwrap();
    assert_eq!(error.kind, ModuleErrorKind::HostValidation);
}
