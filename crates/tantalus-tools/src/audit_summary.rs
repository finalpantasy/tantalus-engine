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

/// Cross-path identity: compares a report-path sweep audit against its
/// world-hand-off twin. One matching pair = pass; any delta (or a missing
/// twin) = fail, so `--deny-fail` catches regeneration skew, not just errors.
fn cross_path_row(
    dir: &Path,
    class: &'static str,
    report_file: &'static str,
    world_file: &'static str,
) -> SummaryRow {
    let counts = match (load(dir, report_file), load(dir, world_file)) {
        (Some(report), Some(world)) => {
            let result = crate::sweep_compare::compare(&report, &world);
            let identical = result["identical"].as_bool() == Some(true);
            Some((u64::from(identical), u64::from(!identical)))
        }
        _ => None,
    };
    SummaryRow {
        class,
        unit: "pairs",
        source: "cross-path compare",
        counts,
    }
}

/// `pass` out of `total`; `None` when a field is missing or `pass > total`.
/// Battle clips the install itself lacks: `clip_missing` with no copy in any
/// other folder. Only `p0data5` holds named clips (`p0data2`-`p0data4` carry
/// only `Take 001` model clips, `resources.assets` none), so the game has no
/// such clip either, and `AnimationFactory.AddAnimWithAnimatioName` adds
/// nothing. A clip found under another folder still counts as a failure.
fn absent_battle_clips(report: &Value) -> std::collections::BTreeSet<String> {
    report["scenes"]["animations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|animation| {
            animation["status"] == "clip_missing"
                && animation["clip_found_in_other_folders"]
                    .as_array()
                    .is_some_and(|folders| folders.is_empty())
        })
        .filter_map(|animation| animation["name"].as_str().map(str::to_owned))
        .collect()
}

/// Scenes whose every `scenes.missing` entry is a clip the install lacks.
fn scenes_missing_only_absent_clips(report: &Value) -> Option<u64> {
    let absent = absent_battle_clips(report);
    let mut only_absent: std::collections::BTreeMap<&str, bool> = std::collections::BTreeMap::new();
    for item in report["scenes"]["missing"].as_array()? {
        let expected = item["reason"] == "clip_missing"
            && item["name"]
                .as_str()
                .is_some_and(|name| absent.contains(name));
        let scene = item["scene"].as_str().unwrap_or_default();
        let entry = only_absent.entry(scene).or_insert(true);
        *entry &= expected;
    }
    Some(only_absent.values().filter(|only| **only).count() as u64)
}

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
                // Documented causes do not count as failures: the
                // ledger lists budget-stop causes per event, and WATCH
                // is an unsupported opcode the pinned reference
                // explains (world scripts 9100/9101, which nothing
                // loads). Anything outside those documented sets is a
                // real failure.
                let total = number(r, "events.probe.functions")?;
                let stops = number(r, "events.probe.instruction_budget_stops")?;
                let documented_stops: u64 = r["events"]["probe"]["instruction_budget_stop_causes"]
                    .as_array()
                    .map(|causes| {
                        causes
                            .iter()
                            .filter_map(|cause| cause["functions"].as_u64())
                            .sum()
                    })
                    .unwrap_or(0);
                let undocumented_stops = stops.saturating_sub(documented_stops);
                let undocumented_errors = r["events"]["probe"]["other_runtime_errors"]
                    .as_array()
                    .map(|errors| {
                        errors
                            .iter()
                            .filter(|error| {
                                !error["error"]
                                    .as_str()
                                    .is_some_and(|text| text.contains("WATCH"))
                            })
                            .count() as u64
                    })
                    .unwrap_or(0);
                of(
                    Some(total),
                    total.checked_sub(undocumented_stops + undocumented_errors),
                )
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
                // A scene whose only gaps are clips the install lacks
                // counts as resolved (see `absent_battle_clips`).
                let resolved = number(r, "scenes.totals.scenes_fully_resolved")?;
                of(
                    number(r, "scenes.totals.scenes"),
                    Some(resolved + scenes_missing_only_absent_clips(r)?),
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
                let absent_clips = absent_battle_clips(r);
                let absent: std::collections::BTreeSet<&str> = r["scenes"]["missing"]
                    .as_array()?
                    .iter()
                    .filter(|item| item["kind"] == "sequence_animation")
                    .filter_map(|item| item["name"].as_str())
                    .filter(|name| absent_clips.contains(*name))
                    .collect();
                let resolved = number(r, "scenes.totals.sequence_animations.distinct_resolved")?;
                of(
                    number(r, "scenes.totals.sequence_animations.distinct"),
                    Some(resolved + absent.len() as u64),
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
        cross_path_row(
            dir,
            "Cross-path identity, armed pair",
            "battle-run-all-patterns.json",
            "battle-world-run-all-patterns.json",
        ),
        cross_path_row(
            dir,
            "Cross-path identity, weak pair",
            "battle-run-all-patterns-weak.json",
            "battle-world-run-all-patterns-weak.json",
        ),
        cross_path_row(
            dir,
            "Cross-path identity, flee-first pair",
            "battle-run-all-patterns-flee.json",
            "battle-world-run-all-patterns-flee.json",
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
                // A movie with no asset and no entry in the game's own
                // `MovieAudioMetaData.txt` has no audio in the game:
                // `SoundMetaData.GetSoundIndex` returns -1 and
                // `SoundLib.PlayMovieMusic` logs it and plays nothing.
                let total = number(r, "totals.movie_rows")?;
                let silent = r["movie_audio"]
                    .as_array()?
                    .iter()
                    .filter(|row| {
                        row["resolved"] == false
                            && row["metadata_sound_index"].is_null()
                            && row["bundle"].is_null()
                    })
                    .count() as u64;
                let unresolved = number(r, "totals.movie_rows_unresolved")?;
                of(
                    Some(total),
                    total.checked_sub(unresolved.saturating_sub(silent)),
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
            dir.path().join("battle-run-all-patterns.json"),
            r#"{"battles":10,"victories":6,"defeats":2,"escapes":0,"errors":1}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("battle-world-run-all-patterns.json"),
            r#"{"battles":10,"victories":6,"defeats":2,"escapes":0,"errors":1}"#,
        )
        .unwrap();
        let rows = summarize(dir.path());
        let armed = rows
            .iter()
            .find(|row| row.class == "Battle runs, all patterns (report path)")
            .unwrap();
        assert_eq!(armed.counts, Some((9, 1)));
        // The twins are identical, so the armed cross-path row passes.
        let identity = rows
            .iter()
            .find(|row| row.class == "Cross-path identity, armed pair")
            .unwrap();
        assert_eq!(identity.counts, Some((1, 0)));
        let json = to_json(&rows);
        assert_eq!(json["rows"].as_array().unwrap().len(), rows.len());
    }

    #[test]
    fn data_the_install_lacks_is_expected_but_other_gaps_still_fail() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("battle-model-coverage.json"),
            r#"{"scenes":{"totals":{"scenes":3,"scenes_fully_resolved":1,
                "sequence_animations":{"distinct":4,"distinct_resolved":2}},
              "animations":[
                {"name":"ANH_A","status":"clip_missing","clip_found_in_other_folders":[]},
                {"name":"ANH_B","status":"clip_missing","clip_found_in_other_folders":[7]}],
              "missing":[
                {"scene":"S1","kind":"sequence_animation","name":"ANH_A","reason":"clip_missing"},
                {"scene":"S2","kind":"sequence_animation","name":"ANH_B","reason":"clip_missing"}]}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("audio-decode.json"),
            r#"{"totals":{"movie_rows":3,"movie_rows_unresolved":2},"movie_audio":[
                {"resolved":true,"metadata_sound_index":1,"bundle":"b"},
                {"resolved":false,"metadata_sound_index":null,"bundle":null},
                {"resolved":false,"metadata_sound_index":4,"bundle":null}]}"#,
        )
        .unwrap();
        let rows = summarize(dir.path());
        let counts = |class: &str| rows.iter().find(|row| row.class == class).unwrap().counts;
        // ANH_A is nowhere in the install; ANH_B exists in another folder.
        assert_eq!(counts("Battle scenes fully resolved"), Some((2, 1)));
        assert_eq!(counts("Battle animation clips"), Some((3, 1)));
        // The silent movie is expected; the one with metadata but no asset fails.
        assert_eq!(counts("Movie-to-sound rows resolved"), Some((2, 1)));
    }

    #[test]
    fn cross_path_identity_row_fails_on_divergence() {
        let dir = tempfile::tempdir().unwrap();
        for (file, gil) in [
            ("battle-run-all-patterns.json", 100),
            ("battle-world-run-all-patterns.json", 200),
        ] {
            std::fs::write(
                dir.path().join(file),
                format!(
                    r#"{{"results":[{{"scene":"S","patterns":[{{"outcome":"victory","bonus_gil":{gil},"bonus_exp":5,"error":null}}]}}]}}"#
                ),
            )
            .unwrap();
        }
        let rows = summarize(dir.path());
        let identity = rows
            .iter()
            .find(|row| row.class == "Cross-path identity, armed pair")
            .unwrap();
        assert_eq!(identity.counts, Some((0, 1)));
        let (failing, _) = failure_counts(&rows);
        assert!(failing > 0);
    }
}
