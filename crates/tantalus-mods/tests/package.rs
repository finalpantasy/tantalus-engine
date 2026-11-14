use std::fs;
use std::path::PathBuf;
use tantalus_contracts::ContentId;
use tantalus_core::{Registry, RegistryRecord};
use tantalus_mods::{PackageLoader, StagedPackage};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/synthetic/raptor_package")
}

fn write_json(path: &std::path::Path, value: serde_json::Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

#[test]
fn raptor_package_adds_content_without_overwriting_vanilla() {
    let staged = PackageLoader::stage(&fixture()).unwrap();
    assert_eq!(staged.patch.additions.len(), 6);
    assert_eq!(staged.content_hash.len(), 64);
    assert_eq!(staged.manifest.world.spawn_regions.len(), 1);

    let vanilla = RegistryRecord::new(
        ContentId::parse("ff9:item/potion").unwrap(),
        serde_json::json!({"healing": 100}),
    );
    let mut registry = Registry::new();
    registry
        .apply(tantalus_core::RegistryPatch::add(vanilla))
        .unwrap();
    registry.apply(staged.patch).unwrap();

    assert_eq!(registry.len(), 7);
    assert_eq!(
        registry
            .get(&ContentId::parse("ff9:item/potion").unwrap())
            .unwrap()
            .data["healing"],
        100
    );
    assert!(
        registry
            .get(&ContentId::parse("my_mod:enemy/iron_raptor").unwrap())
            .is_some()
    );
}

#[test]
fn malformed_manifest_has_a_source_diagnostic() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("tantalus-package.json"), b"{bad").unwrap();

    let report = PackageLoader::stage(directory.path()).unwrap_err();
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.code == "resource-json-invalid")
    );
    assert!(
        report.issues[0]
            .source
            .as_ref()
            .unwrap()
            .contains("tantalus-package.json")
    );
}

#[test]
fn missing_and_escaping_resources_fail_before_staging() {
    let directory = tempfile::tempdir().unwrap();
    write_json(
        &directory.path().join("tantalus-package.json"),
        serde_json::json!({
            "schema_version": 1,
            "package_id": "fixture.bad",
            "version": "1.0.0",
            "dependencies": [],
            "world": {"spawn_regions": []},
            "records": [
                {"id": "fixture:content/missing", "source_path": "missing.json", "operation": "add"},
                {"id": "fixture:content/escape", "source_path": "../outside.json", "operation": "add"}
            ]
        }),
    );

    let report = PackageLoader::stage(directory.path()).unwrap_err();
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.code == "package-resource-unavailable")
    );
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.code == "invalid-package-path")
    );
}

#[test]
fn duplicate_content_ids_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    write_json(
        &directory.path().join("record.json"),
        serde_json::json!({"value": 1}),
    );
    write_json(
        &directory.path().join("tantalus-package.json"),
        serde_json::json!({
            "schema_version": 1,
            "package_id": "fixture.duplicate",
            "version": "1.0.0",
            "dependencies": [],
            "world": {"spawn_regions": []},
            "records": [
                {"id": "fixture:content/same", "source_path": "record.json", "operation": "add"},
                {"id": "fixture:content/same", "source_path": "record.json", "operation": "add"}
            ]
        }),
    );

    let report = PackageLoader::stage(directory.path()).unwrap_err();
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.code == "duplicate-package-content")
    );
}

#[test]
fn non_ascii_package_roots_are_supported() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("パッケージ");
    fs::create_dir(&root).unwrap();
    write_json(
        &root.join("record.json"),
        serde_json::json!({"text": "日本語"}),
    );
    write_json(
        &root.join("tantalus-package.json"),
        serde_json::json!({
            "schema_version": 1,
            "package_id": "fixture.unicode",
            "version": "1.0.0",
            "dependencies": [],
            "world": {"spawn_regions": []},
            "records": [
                {"id": "fixture:content/text", "source_path": "record.json", "operation": "add"}
            ]
        }),
    );

    let staged = PackageLoader::stage(&root).unwrap();
    assert_eq!(staged.patch.additions[0].data["text"], "日本語");
}

#[test]
fn staged_package_is_repeatable() {
    let first: StagedPackage = PackageLoader::stage(&fixture()).unwrap();
    let second = PackageLoader::stage(&fixture()).unwrap();
    assert_eq!(first.content_hash, second.content_hash);
    assert_eq!(first.manifest, second.manifest);
}

#[test]
fn content_hash_changes_when_a_record_changes() {
    let directory = tempfile::tempdir().unwrap();
    write_json(
        &directory.path().join("record.json"),
        serde_json::json!({"value": 1}),
    );
    write_json(
        &directory.path().join("tantalus-package.json"),
        serde_json::json!({
            "schema_version": 1,
            "package_id": "fixture.hash",
            "version": "1.0.0",
            "dependencies": [],
            "world": {"spawn_regions": []},
            "records": [
                {"id": "fixture:content/value", "source_path": "record.json", "operation": "add"}
            ]
        }),
    );

    let first = PackageLoader::stage(directory.path()).unwrap().content_hash;
    write_json(
        &directory.path().join("record.json"),
        serde_json::json!({"value": 2}),
    );
    let second = PackageLoader::stage(directory.path()).unwrap().content_hash;
    assert_ne!(first, second);
}
