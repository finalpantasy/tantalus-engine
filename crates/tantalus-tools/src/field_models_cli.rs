//! `tantalusctl assets inspect-bundle --field-models`: sweep every field event
//! script in `p0data7.bin`, collect immediate model and animation operands,
//! and resolve them against `p0data4.bin` and `p0data5.bin`.
//!
//! The pure resolution rules and result types are in
//! `tantalus_assets::pc::field_models`; this file only needs the event decoder.

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use tantalus_assets::pc::asset_bundle::AssetBundleMetadata;
use tantalus_assets::pc::field_models::{
    AnimationRole, FieldModelCoverage, FieldScriptUsage, build_coverage, model_dirs,
};
use tantalus_assets::pc::model_assembly::{
    AssembleOptions, ModelCatalog, SerializedStore, assemble_model, clip_entries,
};
use tantalus_assets::pc::serialized::SerializedFile;
use tantalus_assets::pc::text_asset::TextAsset;
use tantalus_assets::pc::unity_raw::UnityRawBundle;
use tantalus_legacy::{EventBinary, Expression, ExpressionOp, InstructionKind};

const MAX_BUNDLE_BYTES: usize = 600 * 1024 * 1024;
const EVENT_SEGMENT: &str = "eventbinary";
/// Language folder swept for the coverage; the rest are only compared.
const PRIMARY_LANGUAGE: &str = "us";

pub struct FieldModelsRequest {
    /// `p0data4.bin`; defaults to a sibling of the event bundle.
    pub models: Option<PathBuf>,
    /// `p0data5.bin`; defaults to a sibling of the event bundle.
    pub clips: Option<PathBuf>,
    pub json: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DecodeFailure {
    pub field: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LanguageDifference {
    pub language: String,
    pub field: String,
    pub detail: String,
}

/// Unresolved ids that another bundle's container has a path for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Elsewhere {
    pub model_ids: BTreeSet<u16>,
    pub animation_ids: BTreeSet<u16>,
    /// Up to three example container paths.
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FieldModelsReport {
    pub event_bundle: String,
    pub models_bundle: Option<String>,
    pub clips_bundle: Option<String>,
    pub primary_language: String,
    pub languages: BTreeMap<String, usize>,
    /// Event scripts in the primary language, all kinds.
    pub primary_scripts: usize,
    /// Scripts under `eventbinary/field/` in the primary language.
    pub field_scripts: usize,
    /// Other script folders (battle, world) in the primary language, by count.
    pub excluded_scripts: BTreeMap<String, usize>,
    pub decode_failures: Vec<DecodeFailure>,
    pub language_differences: Vec<LanguageDifference>,
    /// Other `p0data*.bin` bundles whose container holds a path for an
    /// unresolved id (see [`Elsewhere`]).
    pub unresolved_found_elsewhere: BTreeMap<String, Elsewhere>,
    pub coverage: FieldModelCoverage,
}

fn constant(expression: &Expression) -> Option<i32> {
    let mut ops = expression
        .0
        .iter()
        .filter(|op| !matches!(op, ExpressionOp::End | ExpressionOp::Pad));
    match (ops.next(), ops.next()) {
        (Some(ExpressionOp::Constant(value)), None) => Some(*value),
        _ => None,
    }
}

fn note_animation(usage: &mut FieldScriptUsage, role: AnimationRole, operand: &Expression) {
    match constant(operand) {
        // Memoria casts the operand with `(UInt16)getv2()`.
        Some(value) => {
            usage
                .animation_ids
                .entry(value as u16)
                .or_default()
                .insert(role);
        }
        None => *usage.dynamic_animations.entry(role).or_insert(0) += 1,
    }
}

/// Collects the model and animation operands of one decoded instruction.
pub fn collect_instruction(usage: &mut FieldScriptUsage, kind: &InstructionKind) {
    match kind {
        InstructionKind::SetActorModel { model, .. } => match constant(model) {
            Some(value) => {
                usage.model_ids.insert(value as u16);
            }
            None => usage.dynamic_models += 1,
        },
        InstructionKind::SetIdleAnimation(a) => note_animation(usage, AnimationRole::Idle, a),
        InstructionKind::SetWalkAnimation(a) => note_animation(usage, AnimationRole::Walk, a),
        InstructionKind::SetRunAnimation(a) => note_animation(usage, AnimationRole::Run, a),
        InstructionKind::PlayAnimation(a) => note_animation(usage, AnimationRole::Play, a),
        InstructionKind::SetLeftTurnAnimation(a) => {
            note_animation(usage, AnimationRole::TurnLeft, a)
        }
        InstructionKind::SetRightTurnAnimation(a) => {
            note_animation(usage, AnimationRole::TurnRight, a)
        }
        InstructionKind::SetJumpAnimation { animation, .. } => {
            note_animation(usage, AnimationRole::Jump, animation)
        }
        InstructionKind::SetInactiveAnimation { animation } => {
            note_animation(usage, AnimationRole::Inactive, animation)
        }
        InstructionKind::PlayObjectAnimation { animation, .. } => {
            note_animation(usage, AnimationRole::PlayObject, animation)
        }
        _ => {}
    }
}

/// Decodes one event archive and gathers its operands. Per-function decode
/// errors are returned so they are never silently dropped.
pub fn scan_script(field: &str, script: &[u8]) -> Result<(FieldScriptUsage, Vec<String>), String> {
    let binary = EventBinary::parse(script).map_err(|error| error.to_string())?;
    let mut usage = FieldScriptUsage {
        field: field.to_owned(),
        ..FieldScriptUsage::default()
    };
    let mut errors = Vec::new();
    for entry in &binary.entries {
        match binary.decode_entry_with_diagnostics(entry.index) {
            Ok(report) => {
                for function in &report.functions {
                    for instruction in function.program.instructions.values() {
                        collect_instruction(&mut usage, &instruction.kind);
                    }
                }
                errors.extend(report.errors.iter().map(|f| f.error.message.clone()));
            }
            Err(error) => errors.push(error.to_string()),
        }
    }
    Ok((usage, errors))
}

struct EventAsset {
    /// Folder under `eventbinary/`: `field`, `battle` or `world`.
    kind: String,
    language: String,
    name: String,
    path_id: i64,
}

fn event_assets(container: &[(String, i64)]) -> Vec<EventAsset> {
    let mut out = Vec::new();
    for (path, path_id) in container {
        let segments: Vec<&str> = path.split('/').collect();
        let Some(position) = segments.iter().position(|s| *s == EVENT_SEGMENT) else {
            continue;
        };
        let rest = &segments[position + 1..];
        let [kind, language, file] = rest else {
            continue;
        };
        let name = file
            .strip_suffix(".bytes")
            .unwrap_or(file)
            .strip_suffix(".eb")
            .unwrap_or(file)
            .to_owned();
        out.push(EventAsset {
            kind: (*kind).to_owned(),
            language: (*language).to_owned(),
            name,
            path_id: *path_id,
        });
    }
    out
}

fn sibling(path: &Path, name: &str) -> Option<PathBuf> {
    let candidate = path.parent()?.join(name);
    candidate.is_file().then_some(candidate)
}

fn open_store<T>(
    path: &Path,
    run: impl FnOnce(&SerializedStore<'_, '_>) -> Result<T, String>,
) -> Result<T, String> {
    let source = path.to_string_lossy().into_owned();
    let bundle = UnityRawBundle::open(path).map_err(|error| error.to_string())?;
    let bytes = bundle
        .read_entry(0, MAX_BUNDLE_BYTES)
        .map_err(|error| error.to_string())?;
    let file = SerializedFile::parse(&bytes, &source).map_err(|error| error.to_string())?;
    let store = SerializedStore::new(&file, &source).map_err(|error| error.to_string())?;
    run(&store)
}

pub fn run_field_models(
    bundle: &UnityRawBundle,
    path: &Path,
    request: &FieldModelsRequest,
) -> Result<(), String> {
    let report = build_report(bundle, path, request)?;
    if request.json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    } else {
        print_summary(&report);
    }
    Ok(())
}

pub fn build_report(
    bundle: &UnityRawBundle,
    path: &Path,
    request: &FieldModelsRequest,
) -> Result<FieldModelsReport, String> {
    let source = path.to_string_lossy().into_owned();
    let bytes = bundle
        .read_entry(0, MAX_BUNDLE_BYTES)
        .map_err(|error| error.to_string())?;
    let file = SerializedFile::parse(&bytes, &source).map_err(|error| error.to_string())?;
    let objects = &file.report().objects;
    let index_of: HashMap<i64, usize> = objects
        .iter()
        .enumerate()
        .map(|(index, object)| (object.path_id, index))
        .collect();
    let bundle_object = objects
        .iter()
        .position(|object| object.class_id == 142)
        .ok_or_else(|| "AssetBundle object is missing".to_owned())?;
    let metadata = AssetBundleMetadata::parse(
        file.object_data(bundle_object)
            .map_err(|error| error.to_string())?,
        &source,
    )
    .map_err(|error| error.to_string())?;
    let container: Vec<(String, i64)> = metadata
        .container
        .iter()
        .map(|(path, info)| (path.clone(), info.path_id))
        .collect();
    let assets = event_assets(&container);

    let mut languages: BTreeMap<String, usize> = BTreeMap::new();
    for asset in &assets {
        *languages.entry(asset.language.clone()).or_insert(0) += 1;
    }

    // language -> field -> usage
    let mut by_language: BTreeMap<String, BTreeMap<String, FieldScriptUsage>> = BTreeMap::new();
    let mut decode_failures = Vec::new();
    let mut excluded: BTreeMap<String, usize> = BTreeMap::new();
    for asset in &assets {
        if asset.kind != "field" {
            if asset.language == PRIMARY_LANGUAGE {
                *excluded.entry(asset.kind.clone()).or_insert(0) += 1;
            }
            continue;
        }
        let Some(index) = index_of.get(&asset.path_id) else {
            decode_failures.push(DecodeFailure {
                field: asset.name.clone(),
                message: format!("container path id {} has no object", asset.path_id),
            });
            continue;
        };
        let text = TextAsset::parse(
            file.object_data(*index)
                .map_err(|error| error.to_string())?,
            &asset.name,
        )
        .map_err(|error| error.to_string())?;
        match scan_script(&asset.name, &text.script) {
            Ok((usage, errors)) => {
                if asset.language == PRIMARY_LANGUAGE {
                    for message in errors {
                        decode_failures.push(DecodeFailure {
                            field: asset.name.clone(),
                            message,
                        });
                    }
                }
                by_language
                    .entry(asset.language.clone())
                    .or_default()
                    .insert(asset.name.clone(), usage);
            }
            Err(message) => decode_failures.push(DecodeFailure {
                field: asset.name.clone(),
                message,
            }),
        }
    }

    let primary = by_language
        .get(PRIMARY_LANGUAGE)
        .ok_or_else(|| {
            format!(
                "no `{PRIMARY_LANGUAGE}` event scripts found in {source}; languages {languages:?}; {} container entries; sample {:?}",
                container.len(),
                container.iter().filter(|(p, _)| p.contains(".eb")).take(3).map(|(p, _)| p).collect::<Vec<_>>()
            )
        })?;
    let mut language_differences = Vec::new();
    for (language, fields) in &by_language {
        if language == PRIMARY_LANGUAGE {
            continue;
        }
        for (name, base) in primary {
            match fields.get(name) {
                None => language_differences.push(LanguageDifference {
                    language: language.clone(),
                    field: name.clone(),
                    detail: "script missing".to_owned(),
                }),
                Some(other) => {
                    let same = FieldScriptUsage {
                        field: base.field.clone(),
                        ..other.clone()
                    } == *base;
                    if !same {
                        language_differences.push(LanguageDifference {
                            language: language.clone(),
                            field: name.clone(),
                            detail: format!(
                                "models {:?} vs {:?}; animations {} vs {}; dynamic {}+{} vs {}+{}",
                                other.model_ids,
                                base.model_ids,
                                other.animation_ids.len(),
                                base.animation_ids.len(),
                                other.dynamic_models,
                                other.dynamic_animations.values().sum::<usize>(),
                                base.dynamic_models,
                                base.dynamic_animations.values().sum::<usize>(),
                            ),
                        });
                    }
                }
            }
        }
        for name in fields.keys() {
            if !primary.contains_key(name) {
                language_differences.push(LanguageDifference {
                    language: language.clone(),
                    field: name.clone(),
                    detail: "script not present in primary language".to_owned(),
                });
            }
        }
    }

    let usages: Vec<FieldScriptUsage> = primary.values().cloned().collect();

    let models_path = request
        .models
        .clone()
        .or_else(|| sibling(path, "p0data4.bin"));
    let clips_path = request
        .clips
        .clone()
        .or_else(|| sibling(path, "p0data5.bin"));

    let clips = match &clips_path {
        Some(clips_path) => Some(open_store(clips_path, |store| {
            let (clips, _other) = clip_entries(&store.container().map_err(|e| e.to_string())?);
            Ok(clips)
        })?),
        None => None,
    };

    let coverage = match &models_path {
        Some(models_path) => open_store(models_path, |store| {
            let catalog =
                ModelCatalog::from_container(&store.container().map_err(|e| e.to_string())?);
            // Assemble only the directories the scripts reference.
            let wanted: BTreeSet<u16> = usages
                .iter()
                .flat_map(|usage| usage.model_ids.iter().copied())
                .collect();
            let mut assembled: HashMap<String, bool> = HashMap::new();
            for id in wanted {
                for dir in model_dirs(u32::from(id), &catalog) {
                    let ok = dir.roots.first().is_some_and(|root| {
                        assemble_model(store, &dir.label, *root, AssembleOptions::default()).is_ok()
                    });
                    assembled.insert(dir.label.clone(), ok);
                }
            }
            Ok(build_coverage(
                &usages,
                Some(&catalog),
                &|dir| assembled.get(&dir.label).copied().unwrap_or(false),
                clips.as_deref(),
            ))
        })?,
        None => build_coverage(&usages, None, &|_| false, clips.as_deref()),
    };

    let unresolved_found_elsewhere = locate_elsewhere(
        path,
        [models_path.as_deref(), clips_path.as_deref()],
        &coverage,
    );
    Ok(FieldModelsReport {
        event_bundle: file_name(path),
        models_bundle: models_path.as_deref().map(file_name),
        clips_bundle: clips_path.as_deref().map(file_name),
        primary_language: PRIMARY_LANGUAGE.to_owned(),
        languages,
        primary_scripts: assets
            .iter()
            .filter(|a| a.language == PRIMARY_LANGUAGE)
            .count(),
        field_scripts: usages.len(),
        excluded_scripts: excluded,
        decode_failures,
        language_differences,
        unresolved_found_elsewhere,
        coverage,
    })
}

/// Container paths of one bundle; unreadable bundles are skipped.
fn container_paths(path: &Path) -> Option<Vec<String>> {
    let bundle = UnityRawBundle::open(path).ok()?;
    let bytes = bundle.read_entry(0, MAX_BUNDLE_BYTES).ok()?;
    let source = path.to_string_lossy();
    let file = SerializedFile::parse(&bytes, &source).ok()?;
    let index = file
        .report()
        .objects
        .iter()
        .position(|object| object.class_id == 142)?;
    let metadata = AssetBundleMetadata::parse(file.object_data(index).ok()?, &source).ok()?;
    Some(metadata.container.into_iter().map(|(p, _)| p).collect())
}

/// Looks in the sibling `p0data*.bin` bundles (other than the event, model
/// and clip bundles) for container paths of unresolved ids.
fn locate_elsewhere(
    event_path: &Path,
    skip: [Option<&Path>; 2],
    coverage: &FieldModelCoverage,
) -> BTreeMap<String, Elsewhere> {
    use tantalus_assets::pc::field_models::{AnimationStatus, ModelStatus};
    let mut found = BTreeMap::new();
    let Some(dir) = event_path.parent() else {
        return found;
    };
    let models: Vec<u16> = coverage
        .models
        .iter()
        .filter(|m| {
            matches!(
                m.status,
                ModelStatus::NoDirectory | ModelStatus::DirectoryNotAssembled
            )
        })
        .map(|m| m.model_id)
        .collect();
    let clips: Vec<(u16, Option<u32>)> = coverage
        .animations
        .iter()
        .filter(|a| a.status == AnimationStatus::ClipMissing)
        .map(|a| (a.animation_id, a.folder_geo_id))
        .collect();
    if models.is_empty() && clips.is_empty() {
        return found;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for candidate in paths {
        let name = file_name(&candidate);
        if !(name.starts_with("p0data") && name.ends_with(".bin"))
            || candidate == event_path
            || skip.iter().flatten().any(|skipped| *skipped == candidate)
        {
            continue;
        }
        let Some(container) = container_paths(&candidate) else {
            continue;
        };
        let mut hit = Elsewhere::default();
        let model_set: BTreeSet<u16> = models.iter().copied().collect();
        let clip_set: BTreeMap<(u32, u16), ()> = clips
            .iter()
            .filter_map(|(id, folder)| Some(((((*folder)?), *id), ())))
            .collect();
        for path in &container {
            let segments: Vec<&str> = path.split('/').collect();
            if let Some(at) = segments.iter().position(|s| *s == "models")
                && let Some(id) = segments.get(at + 2).and_then(|s| s.parse::<u16>().ok())
                && model_set.contains(&id)
            {
                hit.model_ids.insert(id);
                if hit.examples.len() < 3 {
                    hit.examples.push(path.clone());
                }
            }
            if let [.., "animations", folder, file] = segments.as_slice()
                && let (Ok(folder), Some(id)) = (
                    folder.parse::<u32>(),
                    file.strip_suffix(".anim")
                        .and_then(|n| n.parse::<u16>().ok()),
                )
                && clip_set.contains_key(&(folder, id))
            {
                hit.animation_ids.insert(id);
                if hit.examples.len() < 3 {
                    hit.examples.push(path.clone());
                }
            }
        }
        if !hit.model_ids.is_empty() || !hit.animation_ids.is_empty() {
            found.insert(name, hit);
        }
    }
    found
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn print_summary(report: &FieldModelsReport) {
    let totals = &report.coverage.totals;
    println!(
        "field scripts ({}): {} of {} event scripts; decode failures {}; language differences {}",
        report.primary_language,
        report.field_scripts,
        report.primary_scripts,
        report.decode_failures.len(),
        report.language_differences.len()
    );
    println!(
        "model ids: {} distinct, status {:?}, dynamic operands {}",
        totals.distinct_model_ids, totals.model_status, totals.dynamic_model_operands
    );
    println!(
        "animation ids: {} distinct, status {:?}, dynamic operands {}",
        totals.distinct_animation_ids, totals.animation_status, totals.dynamic_animation_operands
    );
    println!(
        "fields with unresolved ids: {}",
        totals.fields_with_unresolved
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use tantalus_legacy::{Expression, ExpressionOp, InstructionKind};

    fn imm(value: i32) -> Expression {
        Expression(vec![ExpressionOp::Constant(value), ExpressionOp::End])
    }

    #[test]
    fn immediate_and_dynamic_operands_are_separated() {
        let mut usage = FieldScriptUsage::default();
        collect_instruction(
            &mut usage,
            &InstructionKind::SetActorModel {
                model: imm(98),
                eye_height: imm(4),
            },
        );
        collect_instruction(
            &mut usage,
            &InstructionKind::SetActorModel {
                model: Expression(vec![
                    ExpressionOp::Constant(1),
                    ExpressionOp::Constant(2),
                    ExpressionOp::Add,
                    ExpressionOp::End,
                ]),
                eye_height: imm(4),
            },
        );
        collect_instruction(&mut usage, &InstructionKind::SetIdleAnimation(imm(77)));
        collect_instruction(
            &mut usage,
            &InstructionKind::SetWalkAnimation(Expression(vec![ExpressionOp::End])),
        );
        collect_instruction(&mut usage, &InstructionKind::PlayAnimation(imm(-1)));
        assert_eq!(usage.model_ids, BTreeSet::from([98]));
        assert_eq!(usage.dynamic_models, 1);
        assert!(usage.animation_ids.contains_key(&77));
        assert!(usage.animation_ids.contains_key(&65535));
        assert_eq!(usage.dynamic_animations[&AnimationRole::Walk], 1);
    }

    #[test]
    fn container_paths_split_into_language_and_name() {
        let container = vec![
            (
                "assets/resources/commonasset/eventengine/eventbinary/field/us/evt_a_1.eb.bytes"
                    .to_owned(),
                1,
            ),
            ("assets/resources/other/x.bytes".to_owned(), 2),
        ];
        let assets = event_assets(&container);
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].language, "us");
        assert_eq!(assets[0].kind, "field");
        assert_eq!(assets[0].name, "evt_a_1");
    }

    #[test]
    fn rejects_non_event_bytes() {
        assert!(scan_script("x", b"nope").is_err());
    }

    #[test]
    fn real_install_sweep_when_present() {
        let root = std::env::var_os("TANTALUS_FFIX_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("E:/SteamLibrary/steamapps/common/FINAL FANTASY IX"));
        let events = root.join("StreamingAssets").join("p0data7.bin");
        if !events.is_file() {
            return;
        }
        let bundle = UnityRawBundle::open(&events).expect("open p0data7");
        let report = build_report(
            &bundle,
            &events,
            &FieldModelsRequest {
                models: None,
                clips: None,
                json: true,
            },
        )
        .expect("sweep");
        assert!(report.field_scripts > 0);
    }
}
