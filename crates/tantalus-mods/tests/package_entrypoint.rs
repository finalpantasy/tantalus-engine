use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use tantalus_contracts::{ContentRevision, ProfileLock};
use tantalus_mods::*;

const ACTION_WASM: &[u8] =
    include_bytes!("../../../tests/fixtures/synthetic/action_mode/action_mode.wasm");

fn manifest(entrypoints: serde_json::Value) -> serde_json::Value {
    json!({
        "schema_version": 1,
        "package_id": "fixture.action",
        "version": "1.0.0",
        "save_schema": 1,
        "entrypoints": entrypoints,
        "world": {"spawn_regions": []},
        "records": []
    })
}

fn write_package(root: &Path, entrypoints: serde_json::Value) {
    fs::write(
        root.join("tantalus-package.json"),
        serde_json::to_vec(&manifest(entrypoints)).unwrap(),
    )
    .unwrap();
}

fn stage_action(root: &Path) -> StagedPackage {
    fs::write(root.join("action.wasm"), ACTION_WASM).unwrap();
    write_package(
        root,
        json!([{"name": "action", "source_path": "action.wasm", "abi_version": 1}]),
    );
    PackageLoader::stage(root).unwrap()
}

fn resolve(staged: &StagedPackage) -> ProfileLock {
    ModResolver::resolve(&ProfileRequest {
        engine_version: "0.1.0".to_owned(),
        sdk_version: "1.0.0".to_owned(),
        base_revision: ContentRevision::from_u64(1),
        available: vec![PackageCandidate::from(staged.clone())],
        requested: vec!["fixture.action".to_owned()],
        enabled_features: BTreeMap::new(),
        load_order: Vec::new(),
    })
    .unwrap()
    .lock
}

#[test]
fn module_only_package_stages_resolves_and_executes_named_entrypoint() {
    let temp = tempfile::tempdir().unwrap();
    let staged = stage_action(temp.path());
    assert_eq!(staged.modules.len(), 1);
    assert_eq!(
        staged.modules[0].sha256,
        ModuleHost::wasm_sha256(ACTION_WASM)
    );
    assert_eq!(staged.content_hash, stage_action(temp.path()).content_hash);
    let lock = resolve(&staged);
    assert_eq!(lock.modules[0].content_hash, staged.content_hash);

    let limits = ModuleLimits {
        fuel: 5_000_000,
        ..ModuleLimits::default()
    };
    let mut host = ModuleHost::instantiate_entrypoint(&lock, &staged, "action", limits).unwrap();
    host.invoke(&json!({
        "tick": 1,
        "move_x": 2,
        "attack": true,
        "player_x": 0,
        "enemies": [{"id": 7, "x": 4, "hp": 2}]
    }))
    .unwrap();
    assert_eq!(
        host.authoritative_state()["fixture:world/player_x"],
        json!(2)
    );
    assert_eq!(
        host.authoritative_state()["fixture:world/enemy"]["hp"],
        json!(1)
    );
}

#[test]
fn entrypoint_rejects_tampering_and_stale_profile_identity() {
    let temp = tempfile::tempdir().unwrap();
    let mut staged = stage_action(temp.path());
    let lock = resolve(&staged);
    staged.modules[0].wasm[10] ^= 1;
    let error =
        ModuleHost::instantiate_entrypoint(&lock, &staged, "action", ModuleLimits::default())
            .err()
            .unwrap();
    assert_eq!(error.kind, ModuleErrorKind::HostValidation);

    let staged = stage_action(temp.path());
    let mut stale = resolve(&staged);
    stale.modules[0].content_hash = "0".repeat(64);
    assert!(
        ModuleHost::instantiate_entrypoint(&stale, &staged, "action", ModuleLimits::default())
            .is_err()
    );
}

#[test]
fn malformed_entrypoints_fail_before_staging() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("invalid.wasm"), b"not wasm").unwrap();
    write_package(
        temp.path(),
        json!([
            {"name": "bad", "source_path": "invalid.wasm", "abi_version": 1},
            {"name": "bad", "source_path": "invalid.wasm", "abi_version": 1},
            {"name": "escape", "source_path": "../outside.wasm", "abi_version": 1},
            {"name": "missing", "source_path": "missing.wasm", "abi_version": 1},
            {"name": "old", "source_path": "invalid.wasm", "abi_version": 99}
        ]),
    );
    let report = PackageLoader::stage(temp.path()).unwrap_err();
    for code in [
        "invalid-wasm",
        "invalid-entrypoint",
        "invalid-package-path",
        "package-resource-unavailable",
    ] {
        assert!(report.issues.iter().any(|issue| issue.code == code));
    }
}

#[test]
fn changing_module_bytes_changes_package_content_identity() {
    let first = tempfile::tempdir().unwrap();
    let staged = stage_action(first.path());
    let second = tempfile::tempdir().unwrap();
    let mut changed = ACTION_WASM.to_vec();
    let last = changed.last_mut().unwrap();
    *last ^= 1;
    fs::write(second.path().join("action.wasm"), changed).unwrap();
    write_package(
        second.path(),
        json!([{"name": "action", "source_path": "action.wasm", "abi_version": 1}]),
    );
    let changed = PackageLoader::stage(second.path()).unwrap();
    assert_ne!(staged.content_hash, changed.content_hash);
}
