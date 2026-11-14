use std::process::Command;
use tempfile::TempDir;

fn command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_tantalusctl"))
}

#[test]
fn install_inspect_emits_parseable_json() {
    let fixture = TempDir::new().unwrap();
    let output = command()
        .args(["install", "inspect", "--root"])
        .arg(fixture.path())
        .arg("--json")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["layout"], "unrecognized");
    assert_eq!(report["missing_dependencies"].as_array().unwrap().len(), 6);
}

#[test]
fn install_inspect_supports_non_ascii_cli_paths() {
    let parent = TempDir::new().unwrap();
    let root = parent.path().join("インストール-Ⅸ");
    std::fs::create_dir(&root).unwrap();

    let output = command()
        .args(["install", "inspect", "--root"])
        .arg(&root)
        .arg("--json")
        .output()
        .unwrap();

    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["root"], root.to_string_lossy().as_ref());
}

#[test]
fn missing_root_argument_is_a_usage_error() {
    let output = command().args(["install", "inspect"]).output().unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--root"));
}

#[test]
fn nonexistent_root_reports_the_path_and_fails() {
    let fixture = TempDir::new().unwrap();
    let missing = fixture.path().join("missing-game");
    let output = command()
        .args(["install", "inspect", "--root"])
        .arg(&missing)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("does not exist"));
    assert!(stderr.contains("missing-game"));
}
