use std::process::Command;
use tempfile::TempDir;

#[test]
fn malformed_scenario_reports_the_case_path() {
    let fixture = TempDir::new().unwrap();
    let case = fixture.path().join("bad-scenario.json");
    std::fs::write(&case, b"{not-json").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_tantalusctl"))
        .args(["scenario", "run", "--case"])
        .arg(&case)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("bad-scenario.json"));
    assert!(stderr.contains("could not parse"));
}
