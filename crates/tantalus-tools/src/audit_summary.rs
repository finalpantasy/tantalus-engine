//! `tantalusctl audit summary`: pass and fail counts per asset class (milestone W9).
//!
//! The summary reads the committed `audit/*.json` reports, so it needs no game install and can
//! run in CI. Each row names the report it came from. A report that is missing, unreadable or
//! lacks a field becomes a `missing` row, which counts as a failure of the summary itself: a
//! stale or half-deleted audit directory must not look clean. A `fail` count above zero is real
//! data (a record the Rust side could not decode or resolve), shown but not an error unless the
//! caller passes `--deny-fail`.

use serde_json::{Value, json};
use std::path::Path;

/// One asset class with its pass and fail counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryRow {
    pub class: &'static str,
    pub unit: &'static str,
    pub source: &'static str,
    /// `None` when the report or one of its fields was absent.
    pub counts: Option<(u64, u64)>,
}

impl SummaryRow {
    pub fn pass(&self) -> Option<u64> {
        self.counts.map(|(pass, _)| pass)
    }

    pub fn fail(&self) -> Option<u64> {
        self.counts.map(|(_, fail)| fail)
    }
}

fn load(dir: &Path, file: &str) -> Option<Value> {
    let bytes = std::fs::read(dir.join(file)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn number(value: &Value, path: &str) -> Option<u64> {
    let mut node = value;
    for key in path.split('.') {
        node = node.get(key)?;
    }
    node.as_u64()
}

fn length(value: &Value, path: &str) -> Option<u64> {
    let mut node = value;
    for key in path.split('.') {
        node = node.get(key)?;
    }
    node.as_array().map(|items| items.len() as u64)
}

/// `pass` out of `total`; `None` when a field is missing or `pass > total`.
fn of(total: Option<u64>, pass: Option<u64>) -> Option<(u64, u64)> {
    let (total, pass) = (total?, pass?);
    (pass <= total).then(|| (pass, total - pass))
}

fn row(
    class: &'static str,
    unit: &'static str,
    source: &'static str,
    counts: impl FnOnce(&Value) -> Option<(u64, u64)>,
    dir: &Path,
) -> SummaryRow {
    SummaryRow {
        class,
        unit,
        source,
        counts: load(dir, source).as_ref().and_then(counts),
    }
}

/// Builds every row from the reports in `dir`.
pub fn summarize(dir: &Path) -> Vec<SummaryRow> {
    vec![
        row(
            "Field composition",
            "fields",
            "field-composition.json",
            |r| {
                let total = number(r, "total.fields")?;
                of(Some(total), total.checked_sub(number(r, "total.failed")?))
            },
            dir,
        ),
        row(
            "Field scene, navigation and atlas load",
            "fields",
            "game-coverage.json",
            |r| {
                of(
                    number(r, "fields.scene_navigation_atlas_load.checked"),
                    number(r, "fields.scene_navigation_atlas_load.loaded"),
                )
            },
            dir,
        ),
        row(
            "Event function decode",
            "functions",
            "game-coverage.json",
            |r| {
                let decoded = number(r, "events.functions_decoded")?;
                let failed = number(r, "events.decode_errors")?;
                Some((decoded, failed))
            },
            dir,
        ),
        row(
            "Event dry run (zero-state host)",
            "functions",
            "game-coverage.json",
            |r| {
                let total = number(r, "events.probe.functions")?;
                let failed = number(r, "events.probe.instruction_budget_stops")?
                    + length(r, "events.probe.other_runtime_errors")?;
                of(Some(total), total.checked_sub(failed))
            },
            dir,
        ),
        row(
            "Field-script model references",
            "model ids",
            "field-model-coverage.json",
            |r| {
                of(
                    number(r, "coverage.totals.distinct_model_ids"),
                    number(r, "coverage.totals.model_status.resolved"),
                )
            },
            dir,
        ),
        row(
            "Field-script animation references",
            "animation ids",
            "field-model-coverage.json",
            |r| {
                of(
                    number(r, "coverage.totals.distinct_animation_ids"),
                    number(r, "coverage.totals.animation_status.resolved"),
                )
            },
            dir,
        ),
        row(
            "Battle scenes fully resolved",
            "scenes",
            "battle-model-coverage.json",
            |r| {
                of(
                    number(r, "scenes.totals.scenes"),
                    number(r, "scenes.totals.scenes_fully_resolved"),
                )
            },
            dir,
        ),
        row(
            "Battle enemy models",
            "models",
            "battle-model-coverage.json",
            |r| {
                of(
                    number(r, "scenes.totals.enemy_models.distinct"),
                    number(r, "scenes.totals.enemy_models.distinct_resolved"),
                )
            },
            dir,
        ),
        row(
            "Battle animation clips",
            "clips",
            "battle-model-coverage.json",
            |r| {
                of(
                    number(r, "scenes.totals.sequence_animations.distinct"),
                    number(r, "scenes.totals.sequence_animations.distinct_resolved"),
                )
            },
            dir,
        ),
        row(
            "Battle backgrounds",
            "backgrounds",
            "battle-model-coverage.json",
            |r| {
                of(
                    number(r, "scenes.totals.backgrounds.distinct"),
                    number(r, "scenes.totals.backgrounds.distinct_resolved"),
                )
            },
            dir,
        ),
        row(
            "Player battle animations",
            "references",
            "battle-model-coverage.json",
            |r| {
                of(
                    number(r, "players.totals.animation_references"),
                    number(r, "players.totals.animation_references_resolved"),
                )
            },
            dir,
        ),
        row(
            "Battle enemy action sequences",
            "slots",
            "battle-sequences.json",
            |r| {
                of(
                    number(r, "sequence_slots"),
                    number(r, "slots_with_sequence"),
                )
            },
            dir,
        ),
        row(
            "Battle scene sweep (ran to an end)",
            "scenes",
            "battle-run.json",
            |r| {
                let total = number(r, "scenes")?;
                let ended = number(r, "victories")? + number(r, "defeats")? + number(r, "escapes")?;
                of(Some(total), Some(ended))
            },
            dir,
        ),
        row(
            "Battle runs, all patterns (report path)",
            "battles",
            "battle-run-all-patterns.json",
            |r| {
                let total = number(r, "battles")?;
                of(Some(total), total.checked_sub(number(r, "errors")?))
            },
            dir,
        ),
        row(
            "Battle runs, all patterns (world hand-off)",
            "battles",
            "battle-world-run-all-patterns.json",
            |r| {
                let total = number(r, "battles")?;
                of(Some(total), total.checked_sub(number(r, "errors")?))
            },
            dir,
        ),
        row(
            "Battle runs, all patterns, weak party (report path)",
            "battles",
            "battle-run-all-patterns-weak.json",
            |r| {
                let total = number(r, "battles")?;
                of(Some(total), total.checked_sub(number(r, "errors")?))
            },
            dir,
        ),
        row(
            "Battle runs, all patterns, weak party (world hand-off)",
            "battles",
            "battle-world-run-all-patterns-weak.json",
            |r| {
                let total = number(r, "battles")?;
                of(Some(total), total.checked_sub(number(r, "errors")?))
            },
            dir,
        ),
        row(
            "Battle runs, all patterns, flee-first (report path)",
            "battles",
            "battle-run-all-patterns-flee.json",
            |r| {
                let total = number(r, "battles")?;
                of(Some(total), total.checked_sub(number(r, "errors")?))
            },
            dir,
        ),
        row(
            "Battle runs, all patterns, flee-first (world hand-off)",
            "battles",
            "battle-world-run-all-patterns-flee.json",
            |r| {
                let total = number(r, "battles")?;
                of(Some(total), total.checked_sub(number(r, "errors")?))
            },
            dir,
        ),
        row(
            "Sound asset decode",
            "assets",
            "audio-decode.json",
            |r| {
                of(
                    number(r, "totals.sound_assets"),
                    number(r, "totals.decoded_ok"),
                )
            },
            dir,
        ),
        row(
            "Movie-to-sound rows resolved",
            "rows",
            "audio-decode.json",
            |r| {
                let total = number(r, "totals.movie_rows")?;
                of(
                    Some(total),
                    total.checked_sub(number(r, "totals.movie_rows_unresolved")?),
                )
            },
            dir,
        ),
        row(
            "World block meshes",
            "meshes",
            "world-map-inventory.json",
            |r| {
                let decoded = number(r, "block_meshes_decoded")?;
                Some((decoded, number(r, "block_meshes_failed")?))
            },
            dir,
        ),
    ]
}

/// Renders rows as a fixed-width table.
pub fn render_table(rows: &[SummaryRow]) -> String {
    let width = rows.iter().map(|row| row.class.len()).max().unwrap_or(0);
    let mut out = format!(
        "{:<width$}  {:>8}  {:>6}  {:>8}  {:<13}  source\n",
        "asset class", "pass", "fail", "total", "unit"
    );
    for row in rows {
        match row.counts {
            Some((pass, fail)) => out.push_str(&format!(
                "{:<width$}  {:>8}  {:>6}  {:>8}  {:<13}  {}\n",
                row.class,
                pass,
                fail,
                pass + fail,
                row.unit,
                row.source
            )),
            None => out.push_str(&format!(
                "{:<width$}  {:>8}  {:>6}  {:>8}  {:<13}  {}\n",
                row.class, "missing", "-", "-", row.unit, row.source
            )),
        }
    }
    out
}

/// Machine-readable copy of the table.
pub fn to_json(rows: &[SummaryRow]) -> Value {
    json!({
        "schema": "tantalus.audit-summary.v1",
        "scope": "Counts read from the committed audit reports; an inventory, not a gameplay claim.",
        "rows": rows.iter().map(|row| json!({
            "class": row.class,
            "unit": row.unit,
            "source": row.source,
            "pass": row.pass(),
            "fail": row.fail(),
            "missing": row.counts.is_none(),
        })).collect::<Vec<_>>(),
    })
}

/// Rows with a nonzero fail count, and rows whose report was missing.
pub fn failure_counts(rows: &[SummaryRow]) -> (usize, usize) {
    let failing = rows
        .iter()
        .filter(|row| row.fail().is_some_and(|fail| fail > 0))
        .count();
    let missing = rows.iter().filter(|row| row.counts.is_none()).count();
    (failing, missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn of_requires_pass_not_above_total() {
        assert_eq!(of(Some(10), Some(7)), Some((7, 3)));
        assert_eq!(of(Some(10), Some(11)), None);
        assert_eq!(of(None, Some(1)), None);
    }

    #[test]
    fn a_missing_report_is_a_missing_row_not_a_clean_one() {
        let dir = tempfile::tempdir().unwrap();
        let rows = summarize(dir.path());
        assert!(rows.iter().all(|row| row.counts.is_none()));
        let (failing, missing) = failure_counts(&rows);
        assert_eq!(failing, 0);
        assert_eq!(missing, rows.len());
        assert!(render_table(&rows).contains("missing"));
    }

    #[test]
    fn counts_come_from_the_named_report_fields() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("battle-run.json"),
            r#"{"scenes":10,"victories":6,"defeats":2,"escapes":0,"errors":1,"unfinished":1}"#,
        )
        .unwrap();
        let rows = summarize(dir.path());
        let sweep = rows
            .iter()
            .find(|row| row.class.starts_with("Battle scene sweep"))
            .unwrap();
        assert_eq!(sweep.counts, Some((8, 2)));
        let json = to_json(&rows);
        assert_eq!(json["rows"].as_array().unwrap().len(), rows.len());
    }
}
