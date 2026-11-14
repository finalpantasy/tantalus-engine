use serde_json::json;
use std::process::Command;
use tempfile::TempDir;

fn command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_tantalusctl"))
}

#[test]
fn assets_import_runs_a_real_local_plan() {
    let fixture = TempDir::new().unwrap();
    let source = fixture.path().join("source");
    let cache = fixture.path().join("cache");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("event.txt"), "synthetic event").unwrap();
    let plan_path = fixture.path().join("import.json");
    let plan = json!({
        "target_schema": 1,
        "source_id": "cli-fixture",
        "source_root": source,
        "cache_root": cache,
        "requests": [{
            "id": "fixture:event/opening",
            "source_path": "event.txt",
            "kind": "event",
            "codec": "utf8",
            "dependencies": []
        }],
        "limits": {
            "max_file_bytes": 1024,
            "max_total_bytes": 4096,
            "max_assets": 4
        }
    });
    std::fs::write(&plan_path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();

    let output = command()
        .args(["assets", "import", "--plan"])
        .arg(&plan_path)
        .arg("--json")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(manifest["source_id"], "cli-fixture");
    assert_eq!(manifest["assets"][0]["id"], "fixture:event/opening");
    let pack_id = manifest["pack_id"].as_str().unwrap();
    assert!(
        cache
            .join("revisions")
            .join(pack_id)
            .join("manifest.json")
            .is_file()
    );
}

#[test]
fn malformed_import_plan_fails_with_plan_context() {
    let fixture = TempDir::new().unwrap();
    let plan_path = fixture.path().join("bad-plan.json");
    std::fs::write(&plan_path, b"{not-json").unwrap();

    let output = command()
        .args(["assets", "import", "--plan"])
        .arg(&plan_path)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("bad-plan.json"));
    assert!(stderr.contains("could not parse"));
}
