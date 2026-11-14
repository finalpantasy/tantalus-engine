use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use tantalus_assets::inspect_install;
use tantalus_assets::install::{
    BuildSupport, EncodingObservation, LayoutStatus, ReferenceRuntime, ResourceStatus,
};
use tempfile::TempDir;

const REQUIRED_RESOURCES: &[&str] = &[
    "FF9_Launcher.exe",
    "x64/FF9.exe",
    "x64/FF9_Data/Managed/Assembly-CSharp.dll",
    "x64/FF9_Data/resources.assets",
    "x64/FF9_Data/sharedassets2.assets",
];

fn write_fixture(root: &Path, relative_path: &str, contents: &[u8]) {
    let path = root.join(relative_path);
    fs::create_dir_all(path.parent().expect("fixture file has a parent")).unwrap();
    fs::write(path, contents).unwrap();
}

fn complete_fixture() -> TempDir {
    let fixture = tempfile::tempdir().unwrap();
    write_fixture(fixture.path(), "FF9_Launcher.exe", b"abc");
    write_fixture(fixture.path(), "x64/FF9.exe", b"synthetic executable");
    write_fixture(
        fixture.path(),
        "x64/FF9_Data/Managed/Assembly-CSharp.dll",
        b"synthetic managed reference",
    );
    write_fixture(
        fixture.path(),
        "x64/FF9_Data/resources.assets",
        b"synthetic resources",
    );
    write_fixture(
        fixture.path(),
        "x64/FF9_Data/sharedassets2.assets",
        b"synthetic UI resources",
    );
    fs::create_dir(fixture.path().join("StreamingAssets")).unwrap();
    fixture
}

fn snapshot_files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, directory: &Path, output: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, output);
            } else {
                output.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(&path).unwrap(),
                );
            }
        }
    }

    let mut output = BTreeMap::new();
    visit(root, root, &mut output);
    output
}

#[test]
fn complete_layout_is_recognized_without_claiming_build_support() {
    let fixture = complete_fixture();
    let report = inspect_install(fixture.path()).unwrap();

    assert_eq!(report.layout, LayoutStatus::RecognizedPc);
    assert_eq!(report.build.support, BuildSupport::Unknown);
    assert_eq!(report.build.edition, None);
    assert!(report.missing_dependencies.is_empty());
    assert!(report.resources.iter().any(|resource| {
        resource.key == "launcher"
            && resource.status == ResourceStatus::Present
            && resource.sha256.as_deref()
                == Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
    }));
}

#[test]
fn inspection_is_repeatable_and_never_changes_files() {
    let fixture = complete_fixture();
    write_fixture(
        fixture.path(),
        "Memoria.ini",
        "[Mod]\nFolderNames = \"Créateur 日本語\"\n".as_bytes(),
    );
    let before = snapshot_files(fixture.path());

    let first = inspect_install(fixture.path()).unwrap();
    let second = inspect_install(fixture.path()).unwrap();

    assert_eq!(snapshot_files(fixture.path()), before);
    assert_eq!(first, second);
    let memoria = first
        .resources
        .iter()
        .find(|resource| resource.key == "memoria_configuration")
        .unwrap();
    assert_eq!(memoria.encoding, Some(EncodingObservation::Utf8));
}

#[test]
fn missing_resources_are_listed_individually() {
    let empty = tempfile::tempdir().unwrap();
    let empty_report = inspect_install(empty.path()).unwrap();
    assert_eq!(empty_report.layout, LayoutStatus::Unrecognized);
    assert_eq!(
        empty_report.missing_dependencies.len(),
        REQUIRED_RESOURCES.len() + 1
    );

    let partial = tempfile::tempdir().unwrap();
    write_fixture(partial.path(), "FF9_Launcher.exe", b"abc");
    let partial_report = inspect_install(partial.path()).unwrap();
    assert_eq!(partial_report.layout, LayoutStatus::IncompletePc);
    assert!(partial_report.missing_dependencies.iter().any(|missing| {
        missing.resource_key == "x64_executable" && missing.relative_path == "x64/FF9.exe"
    }));
}

#[test]
fn wrong_kind_has_resource_and_path_context() {
    let fixture = complete_fixture();
    let launcher = fixture.path().join("FF9_Launcher.exe");
    fs::remove_file(&launcher).unwrap();
    fs::create_dir(&launcher).unwrap();

    let report = inspect_install(fixture.path()).unwrap();
    let resource = report
        .resources
        .iter()
        .find(|resource| resource.key == "launcher")
        .unwrap();
    assert_eq!(resource.status, ResourceStatus::WrongKind);
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "resource-kind-mismatch"
            && diagnostic.resource_key.as_deref() == Some("launcher")
            && diagnostic.path.contains("FF9_Launcher.exe")
    }));
}

#[test]
fn non_ascii_installation_paths_are_preserved() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("Fïnal Fantasy Ⅸ 日本語");
    fs::create_dir(&root).unwrap();
    write_fixture(&root, "FF9_Launcher.exe", b"abc");

    let report = inspect_install(&root).unwrap();
    assert_eq!(report.root, root.to_string_lossy());
    assert!(
        report
            .build
            .evidence
            .iter()
            .any(|entry| { entry == "launcher:FF9_Launcher.exe" })
    );
}

#[cfg(not(windows))]
#[test]
fn case_sensitive_layouts_do_not_silently_accept_wrong_case() {
    let fixture = complete_fixture();
    fs::rename(
        fixture.path().join("FF9_Launcher.exe"),
        fixture.path().join("ff9_launcher.exe"),
    )
    .unwrap();

    let report = inspect_install(fixture.path()).unwrap();
    assert_eq!(report.layout, LayoutStatus::IncompletePc);
    assert!(
        report
            .missing_dependencies
            .iter()
            .any(|missing| { missing.resource_key == "launcher" })
    );
}

#[test]
fn memoria_components_are_identified_without_executing_them() {
    let fixture = complete_fixture();
    write_fixture(
        fixture.path(),
        "x64/FF9_Data/Managed/Memoria.Prime.dll",
        b"synthetic Memoria component",
    );

    let report = inspect_install(fixture.path()).unwrap();
    assert_eq!(
        report.build.reference_runtime,
        ReferenceRuntime::MemoriaModified
    );
    assert!(report.build.support == BuildSupport::Unknown);
}

#[test]
fn special_effect_plugin_alone_does_not_confirm_memoria() {
    let fixture = complete_fixture();
    write_fixture(
        fixture.path(),
        "x64/FF9_Data/Plugins/FF9SpecialEffectPlugin.dll",
        b"synthetic associated plugin",
    );

    let report = inspect_install(fixture.path()).unwrap();
    assert_eq!(
        report.build.reference_runtime,
        ReferenceRuntime::OriginalOrUnknown
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "memoria-runtime-unconfirmed")
    );
}

#[test]
fn non_utf8_text_is_observed_without_being_mislabeled() {
    let fixture = complete_fixture();
    write_fixture(fixture.path(), "Memoria.ini", &[0xff, 0xfe, 0x00]);

    let report = inspect_install(fixture.path()).unwrap();
    let resource = report
        .resources
        .iter()
        .find(|resource| resource.key == "memoria_configuration")
        .unwrap();
    assert_eq!(resource.encoding, Some(EncodingObservation::NonUtf8));
}

#[test]
fn report_is_versioned_and_json_serializable() {
    let fixture = complete_fixture();
    let report = inspect_install(fixture.path()).unwrap();

    let value = serde_json::to_value(report).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["layout"], "recognized_pc");
    assert!(value["resources"].as_array().unwrap().len() >= 6);
}

#[test]
fn invalid_roots_return_contextual_errors() {
    let fixture = tempfile::tempdir().unwrap();
    let missing = fixture.path().join("not here");
    let missing_error = inspect_install(&missing).unwrap_err().to_string();
    assert!(missing_error.contains("not here"));

    let file_root = fixture.path().join("game.txt");
    fs::write(&file_root, b"not an installation").unwrap();
    let file_error = inspect_install(&file_root).unwrap_err().to_string();
    assert!(file_error.contains("not a directory"));
    assert!(file_error.contains("game.txt"));
}
