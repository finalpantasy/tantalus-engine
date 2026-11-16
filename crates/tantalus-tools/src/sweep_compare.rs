//! Compare two battle sweep reports (`audit/battle-run*.json`) after a
//! behavioral landing: which battles changed outcome, spoils, or errors,
//! and where. Answers "did the sweep shift, and where?" without ad-hoc
//! scripts. Handles both report shapes: flat per-scene rows and
//! `--all-patterns` entries (a scene with a `patterns` array).

use serde_json::Value;

/// One battle's comparable facts.
#[derive(Debug, Clone, PartialEq)]
struct Row {
    scene: String,
    pattern: usize,
    outcome: String,
    gil: i64,
    exp: i64,
    error: String,
}

fn collect(report: &Value) -> Vec<Row> {
    let mut rows = Vec::new();
    let Some(results) = report.get("results").and_then(|r| r.as_array()) else {
        return rows;
    };
    for entry in results {
        let scene = entry
            .get("scene")
            .and_then(|s| s.as_str())
            .unwrap_or_default()
            .to_owned();
        if let Some(patterns) = entry.get("patterns").and_then(|p| p.as_array()) {
            for (pattern, report) in patterns.iter().enumerate() {
                rows.push(row_of(&scene, pattern, report));
            }
        } else {
            rows.push(row_of(&scene, 0, entry));
        }
    }
    rows
}

fn row_of(scene: &str, pattern: usize, report: &Value) -> Row {
    let num = |key: &str| report.get(key).and_then(|v| v.as_i64()).unwrap_or(0);
    Row {
        scene: scene.to_owned(),
        pattern,
        outcome: report
            .get("outcome")
            .and_then(|v| v.as_str())
            .unwrap_or("none")
            .to_owned(),
        gil: num("bonus_gil"),
        exp: num("bonus_exp"),
        error: report
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned(),
    }
}

/// Compare two sweep reports. Returns a JSON summary: counts plus every
/// delta keyed by scene and pattern.
pub fn compare(before: &Value, after: &Value) -> Value {
    let before_rows = collect(before);
    let after_rows = collect(after);
    let mut outcome_changes = 0u64;
    let mut spoils_changes = 0u64;
    let mut error_changes = 0u64;
    let mut deltas = Vec::new();
    for (index, (a, b)) in before_rows.iter().zip(&after_rows).enumerate() {
        if a == b {
            continue;
        }
        let mut kinds = Vec::new();
        if a.outcome != b.outcome {
            outcome_changes += 1;
            kinds.push("outcome");
        }
        if a.gil != b.gil || a.exp != b.exp {
            spoils_changes += 1;
            kinds.push("spoils");
        }
        if a.error != b.error {
            error_changes += 1;
            kinds.push("error");
        }
        deltas.push(serde_json::json!({
            "scene": a.scene,
            "pattern": a.pattern,
            "index": index,
            "kinds": kinds,
            "before": {"outcome": a.outcome, "bonus_gil": a.gil, "bonus_exp": a.exp, "error": a.error},
            "after": {"outcome": b.outcome, "bonus_gil": b.gil, "bonus_exp": b.exp, "error": b.error},
        }));
    }
    serde_json::json!({
        "battles": before_rows.len().min(after_rows.len()),
        "same_shape": before_rows.len() == after_rows.len(),
        "identical": deltas.is_empty(),
        "outcome_changes": outcome_changes,
        "spoils_changes": spoils_changes,
        "error_changes": error_changes,
        "deltas": deltas,
    })
}
