//! Field script model and animation binding coverage (data side).
//!
//! The event decoder lives in `tantalus-legacy`, which this crate does not
//! depend on, so the script sweep itself is in `tantalus-tools`
//! (`field_models_cli`). This module holds the typed results and the
//! resolution rules, which only need the model and clip catalogs.
//!
//! Rules, from the pinned Memoria reference:
//!
//! * `SetModel` (`EventEngine.DoEventCode.cs`, opcode `MODEL` 0x2F) stores
//!   `po.model = (UInt16)getv2()` and loads the model with
//!   `FF9BattleDB.GEO.GetValue(po.model)`: the model id **is** the geo id, and
//!   the geo name is `model_assembly::geo_name`.
//! * Animation opcodes (`ANIM`, `DANIM`, `AIDLE`, `AWALK`, `ARUN`, turn, jump,
//!   `ASLEEP`) read `(UInt16)getv2()` and call
//!   `FF9DBAll.AnimationDB.GetValue(id)`; the name `ANH_<a>_<b>_<c>_...` maps
//!   to the folder `GEO_<a>_<b>_<c>` (`AnimationFactory.GetAnimationFolder`)
//!   and `GetRenameAnimationPath` turns the path into
//!   `Animations/<geo id of that folder>/<animation id>`.

use super::model_assembly::{ClipEntry, ModelCatalog, ModelDir, geo_name};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

mod animation_names;

/// `AnimationFactory.GetRenameAnimationDirectory` special cases: geo name to
/// animation folder number.
const FOLDER_OVERRIDES: &[(&str, u32)] = &[("GEO_MON_B3_110", 347), ("GEO_MON_B3_109", 5461)];

/// The kind of script operand an animation id came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnimationRole {
    Idle,
    Walk,
    Run,
    Play,
    TurnLeft,
    TurnRight,
    Jump,
    Inactive,
    PlayObject,
}

/// Immediate operands of one field script plus the count of non-constant ones.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FieldScriptUsage {
    pub field: String,
    /// Distinct immediate `SetModel` ids.
    pub model_ids: BTreeSet<u16>,
    /// `SetModel` operands that are not a constant expression.
    pub dynamic_models: usize,
    /// Distinct immediate animation ids with the roles that used them.
    pub animation_ids: BTreeMap<u16, BTreeSet<AnimationRole>>,
    /// Animation operands that are not a constant expression, by role.
    pub dynamic_animations: BTreeMap<AnimationRole, usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelStatus {
    /// A model directory with a root object exists and assembled cleanly.
    Resolved,
    /// A directory exists but did not assemble (or has no root object).
    DirectoryNotAssembled,
    /// The id has a geo name but no directory in the supplied model bundle.
    NoDirectory,
    /// The id is not a key of `FF9BattleDB.GEO`.
    NoGeoName,
    /// No model bundle was supplied; only the geo name was checked.
    NotChecked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelResolution {
    pub model_id: u16,
    pub geo_name: Option<&'static str>,
    pub directories: Vec<String>,
    pub status: ModelStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnimationStatus {
    /// `animations/<folder geo id>/<id>.anim` exists.
    Resolved,
    /// The id is not a key of `FF9DBAll.AnimationDB`.
    NotInAnimationDb,
    /// The animation name's model folder is not a `FF9BattleDB.GEO` name.
    UnknownFolder,
    /// Name and folder resolve but the clip is absent from the clip bundle.
    ClipMissing,
    /// No clip bundle was supplied; only the name was checked.
    NotChecked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnimationResolution {
    pub animation_id: u16,
    pub name: Option<&'static str>,
    pub folder_geo_id: Option<u32>,
    /// Folders that hold a clip with this id when the expected one does not.
    pub clip_found_in_other_folders: Vec<u32>,
    pub status: AnimationStatus,
}

/// One id of one field with its resolution status; names, directories and
/// clip folders are in the top-level tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FieldItem<S> {
    pub id: u16,
    pub status: S,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldCoverage {
    pub field: String,
    pub models: Vec<FieldItem<ModelStatus>>,
    pub dynamic_models: usize,
    pub animations: Vec<FieldItem<AnimationStatus>>,
    pub animation_roles: BTreeMap<u16, BTreeSet<AnimationRole>>,
    pub dynamic_animations: BTreeMap<AnimationRole, usize>,
    pub unresolved_models: usize,
    pub unresolved_animations: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CoverageTotals {
    pub fields: usize,
    pub fields_with_unresolved: usize,
    pub distinct_model_ids: usize,
    pub model_status: BTreeMap<String, usize>,
    pub dynamic_model_operands: usize,
    pub distinct_animation_ids: usize,
    pub animation_status: BTreeMap<String, usize>,
    pub dynamic_animation_operands: usize,
}

/// One animation folder that lacks clips the scripts reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MissingClipFolder {
    pub geo_id: u32,
    pub geo_name: Option<&'static str>,
    pub missing_animation_ids: usize,
    /// Clips the clip bundle holds for this folder (0: no folder at all).
    pub clips_in_bundle: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FieldModelCoverage {
    pub missing_clip_folders: Vec<MissingClipFolder>,
    pub totals: CoverageTotals,
    pub models: Vec<ModelResolution>,
    pub animations: Vec<AnimationResolution>,
    pub fields: Vec<FieldCoverage>,
}

/// Geo name from an animation name: `ANH_MAIN_F0_VIV_001` is `GEO_MAIN_F0_VIV`.
pub fn animation_geo_name(animation_name: &str) -> Option<String> {
    let mut tokens = animation_name.split('_');
    let _prefix = tokens.next()?;
    let (a, b, c) = (tokens.next()?, tokens.next()?, tokens.next()?);
    Some(format!("GEO_{a}_{b}_{c}"))
}

/// `FF9DBAll.AnimationDB` name for an animation id.
pub fn animation_name(animation_id: u32) -> Option<&'static str> {
    animation_names::ANIMATION_NAMES
        .binary_search_by_key(&animation_id, |(id, _)| *id)
        .ok()
        .map(|index| animation_names::ANIMATION_NAMES[index].1)
}

/// Number of entries in the embedded animation table.
pub fn animation_table_len() -> usize {
    animation_names::ANIMATION_NAMES.len()
}

/// Reverse lookup `GEO_...` name to geo id over `FF9BattleDB.GEO`.
fn geo_ids_by_name() -> HashMap<&'static str, u32> {
    (0..=u32::from(u16::MAX))
        .filter_map(|id| geo_name(id).map(|name| (name, id)))
        .collect()
}

/// Resolves one model id. `assembled` says whether a directory assembles; it
/// is only consulted for directories found in `catalog`.
pub fn resolve_model(
    model_id: u16,
    catalog: Option<&ModelCatalog>,
    assembled: &dyn Fn(&ModelDir) -> bool,
) -> ModelResolution {
    let name = geo_name(u32::from(model_id));
    let mut directories = Vec::new();
    let status = match (name, catalog) {
        (None, _) => ModelStatus::NoGeoName,
        (Some(_), None) => ModelStatus::NotChecked,
        (Some(_), Some(catalog)) => {
            let dirs = catalog.dirs_for_geo(u32::from(model_id));
            directories = dirs.iter().map(|dir| dir.label.clone()).collect();
            if dirs.is_empty() {
                ModelStatus::NoDirectory
            } else if dirs
                .iter()
                .any(|dir| !dir.roots.is_empty() && assembled(dir))
            {
                ModelStatus::Resolved
            } else {
                ModelStatus::DirectoryNotAssembled
            }
        }
    };
    ModelResolution {
        model_id,
        geo_name: name,
        directories,
        status,
    }
}

/// Resolves one animation id against the clip set (`None` skips the clip check).
pub fn resolve_animation(
    animation_id: u16,
    clips: Option<&HashSet<(u32, u32)>>,
    by_id: &HashMap<u32, Vec<u32>>,
    geo_ids: &HashMap<&'static str, u32>,
) -> AnimationResolution {
    let id = u32::from(animation_id);
    let name = animation_name(id);
    let mut folder_geo_id = None;
    let mut other = Vec::new();
    let status = match name {
        None => AnimationStatus::NotInAnimationDb,
        Some(name) => {
            let geo = animation_geo_name(name);
            folder_geo_id = geo.as_deref().and_then(|geo| {
                FOLDER_OVERRIDES
                    .iter()
                    .find(|(over, _)| *over == geo)
                    .map(|(_, folder)| *folder)
                    .or_else(|| geo_ids.get(geo).copied())
            });
            match (folder_geo_id, clips) {
                (None, _) => AnimationStatus::UnknownFolder,
                (Some(_), None) => AnimationStatus::NotChecked,
                (Some(folder), Some(clips)) => {
                    if clips.contains(&(folder, id)) {
                        AnimationStatus::Resolved
                    } else {
                        if let Some(folders) = by_id.get(&id) {
                            other = folders.clone();
                        }
                        AnimationStatus::ClipMissing
                    }
                }
            }
        }
    };
    AnimationResolution {
        animation_id,
        name,
        folder_geo_id,
        clip_found_in_other_folders: other,
        status,
    }
}

fn model_ok(status: ModelStatus) -> bool {
    matches!(status, ModelStatus::Resolved | ModelStatus::NotChecked)
}

fn animation_ok(status: AnimationStatus) -> bool {
    matches!(
        status,
        AnimationStatus::Resolved | AnimationStatus::NotChecked
    )
}

fn key<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Resolves every operand of every field. `catalog` and `clips` are optional;
/// without them the model/clip checks report `not_checked`.
pub fn build_coverage(
    usages: &[FieldScriptUsage],
    catalog: Option<&ModelCatalog>,
    assembled: &dyn Fn(&ModelDir) -> bool,
    clips: Option<&[ClipEntry]>,
) -> FieldModelCoverage {
    let clip_set: Option<HashSet<(u32, u32)>> = clips.map(|clips| {
        clips
            .iter()
            .filter_map(|clip| Some((clip.geo_id, clip.name.parse::<u32>().ok()?)))
            .collect()
    });
    let mut by_id: HashMap<u32, Vec<u32>> = HashMap::new();
    if let Some(set) = &clip_set {
        for (folder, id) in set {
            by_id.entry(*id).or_default().push(*folder);
        }
        for folders in by_id.values_mut() {
            folders.sort_unstable();
        }
    }
    let geo_ids = geo_ids_by_name();

    let mut model_ids = BTreeSet::new();
    let mut animation_ids = BTreeSet::new();
    for usage in usages {
        model_ids.extend(usage.model_ids.iter().copied());
        animation_ids.extend(usage.animation_ids.keys().copied());
    }
    let models: BTreeMap<u16, ModelResolution> = model_ids
        .iter()
        .map(|id| (*id, resolve_model(*id, catalog, assembled)))
        .collect();
    let animations: BTreeMap<u16, AnimationResolution> = animation_ids
        .iter()
        .map(|id| {
            (
                *id,
                resolve_animation(*id, clip_set.as_ref(), &by_id, &geo_ids),
            )
        })
        .collect();

    let mut totals = CoverageTotals {
        fields: usages.len(),
        distinct_model_ids: models.len(),
        distinct_animation_ids: animations.len(),
        ..CoverageTotals::default()
    };
    for model in models.values() {
        *totals.model_status.entry(key(&model.status)).or_insert(0) += 1;
    }
    for animation in animations.values() {
        *totals
            .animation_status
            .entry(key(&animation.status))
            .or_insert(0) += 1;
    }

    let mut per_folder: BTreeMap<u32, (usize, usize)> = BTreeMap::new();
    if let Some(set) = &clip_set {
        for (folder, _) in set {
            per_folder.entry(*folder).or_default().1 += 1;
        }
    }
    let mut missing: BTreeMap<u32, usize> = BTreeMap::new();
    for animation in animations.values() {
        if animation.status == AnimationStatus::ClipMissing
            && let Some(folder) = animation.folder_geo_id
        {
            *missing.entry(folder).or_insert(0) += 1;
        }
    }
    let missing_clip_folders = missing
        .into_iter()
        .map(|(geo_id, missing_animation_ids)| MissingClipFolder {
            geo_id,
            geo_name: geo_name(geo_id),
            missing_animation_ids,
            clips_in_bundle: per_folder.get(&geo_id).map_or(0, |entry| entry.1),
        })
        .collect();
    let mut fields = Vec::with_capacity(usages.len());
    for usage in usages {
        let field_models: Vec<FieldItem<ModelStatus>> = usage
            .model_ids
            .iter()
            .map(|id| FieldItem {
                id: *id,
                status: models[id].status,
            })
            .collect();
        let field_animations: Vec<FieldItem<AnimationStatus>> = usage
            .animation_ids
            .keys()
            .map(|id| FieldItem {
                id: *id,
                status: animations[id].status,
            })
            .collect();
        let unresolved_models = field_models.iter().filter(|m| !model_ok(m.status)).count();
        let unresolved_animations = field_animations
            .iter()
            .filter(|a| !animation_ok(a.status))
            .count();
        totals.dynamic_model_operands += usage.dynamic_models;
        totals.dynamic_animation_operands += usage.dynamic_animations.values().sum::<usize>();
        if unresolved_models + unresolved_animations > 0 {
            totals.fields_with_unresolved += 1;
        }
        fields.push(FieldCoverage {
            field: usage.field.clone(),
            models: field_models,
            dynamic_models: usage.dynamic_models,
            animations: field_animations,
            animation_roles: usage.animation_ids.clone(),
            dynamic_animations: usage.dynamic_animations.clone(),
            unresolved_models,
            unresolved_animations,
        });
    }
    FieldModelCoverage {
        missing_clip_folders,
        totals,
        models: models.into_values().collect(),
        animations: animations.into_values().collect(),
        fields,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pc::model_assembly::ContainerEntry;

    fn entry(path: &str, path_id: i64, class_id: u16) -> ContainerEntry {
        ContainerEntry {
            path: path.to_owned(),
            path_id,
            class_id,
        }
    }

    #[test]
    fn animation_table_is_sorted_and_named() {
        assert!(animation_table_len() > 14000);
        assert!(
            animation_names::ANIMATION_NAMES
                .windows(2)
                .all(|pair| pair[0].0 < pair[1].0)
        );
        assert_eq!(animation_name(77), Some("ANH_ACC_F0_EGG_P"));
        assert_eq!(
            animation_geo_name("ANH_ACC_F0_EGG_P").as_deref(),
            Some("GEO_ACC_F0_EGG")
        );
        assert_eq!(animation_geo_name("ANH_ONLY"), None);
    }

    #[test]
    fn fixture_resolution_reports_each_status() {
        // geo 8 is GEO_MAIN_F0_VIV; 3 is GEO_ACC_F0_FEL; 6 is GEO_ACC_F0_FLR.
        let catalog = ModelCatalog::from_container(&[
            entry("assets/resources/models/2/8/mesh0", 1, 1),
            entry("assets/resources/models/1/6/mesh0", 2, 1),
        ]);
        let ok = |dir: &ModelDir| dir.geo_id == 8;
        let usage = FieldScriptUsage {
            field: "EVT_TEST".into(),
            model_ids: BTreeSet::from([8, 6, 3, 65000]),
            dynamic_models: 2,
            animation_ids: BTreeMap::from([
                (77, BTreeSet::from([AnimationRole::Idle])),
                (65535, BTreeSet::from([AnimationRole::Play])),
            ]),
            dynamic_animations: BTreeMap::from([(AnimationRole::Walk, 1)]),
        };
        let name = animation_name(77).unwrap();
        let geo = animation_geo_name(name).unwrap();
        let folder = (0..=u32::from(u16::MAX))
            .find(|id| geo_name(*id) == Some(geo.as_str()))
            .expect("fixture geo exists");
        let clips = vec![ClipEntry {
            geo_id: folder,
            name: "77".into(),
            path_id: 9,
        }];
        let coverage = build_coverage(
            std::slice::from_ref(&usage),
            Some(&catalog),
            &ok,
            Some(&clips),
        );
        let status: BTreeMap<u16, ModelStatus> = coverage
            .models
            .iter()
            .map(|m| (m.model_id, m.status))
            .collect();
        assert_eq!(status[&8], ModelStatus::Resolved);
        assert_eq!(status[&6], ModelStatus::DirectoryNotAssembled);
        assert_eq!(status[&3], ModelStatus::NoDirectory);
        assert_eq!(status[&65000], ModelStatus::NoGeoName);
        let anim: BTreeMap<u16, AnimationStatus> = coverage
            .animations
            .iter()
            .map(|a| (a.animation_id, a.status))
            .collect();
        assert_eq!(anim[&77], AnimationStatus::Resolved);
        assert_eq!(anim[&65535], AnimationStatus::NotInAnimationDb);
        assert_eq!(coverage.totals.distinct_model_ids, 4);
        assert_eq!(coverage.totals.dynamic_model_operands, 2);
        assert_eq!(coverage.totals.dynamic_animation_operands, 1);
        assert_eq!(coverage.totals.fields_with_unresolved, 1);
        assert_eq!(coverage.fields[0].unresolved_models, 3);

        // Without a clip bundle the clip check is skipped, not failed.
        let unchecked = build_coverage(&[usage], None, &ok, None);
        assert!(
            unchecked
                .models
                .iter()
                .filter(|m| m.geo_name.is_some())
                .all(|m| m.status == ModelStatus::NotChecked)
        );
        assert_eq!(unchecked.animations[0].status, AnimationStatus::NotChecked);
    }

    #[test]
    fn missing_clip_lists_other_folders() {
        let clips = vec![ClipEntry {
            geo_id: 1,
            name: "77".into(),
            path_id: 1,
        }];
        let coverage = build_coverage(
            &[FieldScriptUsage {
                field: "F".into(),
                animation_ids: BTreeMap::from([(77, BTreeSet::from([AnimationRole::Run]))]),
                ..FieldScriptUsage::default()
            }],
            None,
            &|_| true,
            Some(&clips),
        );
        let animation = &coverage.animations[0];
        assert_eq!(animation.status, AnimationStatus::ClipMissing);
        assert_eq!(animation.clip_found_in_other_folders, vec![1]);
    }
}
