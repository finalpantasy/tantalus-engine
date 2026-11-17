//! Battle model, animation and background coverage (data side), plus the player rows.
//!
//! Everything here is pure: the bundle readers live in `tantalus-tools`
//! (`battle_models_cli`) and hand over catalogs. Rules, from the pinned Memoria reference:
//!
//! * Enemy model: `btl_init.InitEnemyData` sets `dms_geo_id = BTL_SCENE.GetMonGeoID(placement)`,
//!   which is `MonAddr[TypeNo].Geo` unless the placement is a multipart slave
//!   (`TypeNo > 0` and `FLG_MULTIPART`), which gets -1 and no model. The model name is
//!   `FF9BattleDB.GEO.GetValue(geo)`. `ModelFactory.CreateModel` loads the directory of that
//!   name's `upscaleTable`/`revertUpscaleTable` target (`model_factory`), which is `models/3/<id>`
//!   for most monsters but the party member's field model for the party-member stand-ins (`ModelFactory.IsUseAsEnemyCharacter`)
//!   character stand-ins.
//! * Enemy animations: `SetupBattleEnemy` calls `SetMonsterParameter` for every type, which does
//!   `mot[i] = FF9BattleDB.Animation[Mot[i]]` (six ids, a missing key throws). Sequence streams
//!   add `Anim` operands: `FF9BattleDB.Animation[AnmAddrList[AnmOfsList[slot] + code]]` (code
//!   255 means the type's own idle motion and loads nothing).
//! * Animation name to clip (`AnimationFactory.AddAnimWithAnimatioName`): path
//!   `Animations/GEO_<a>_<b>_<c>/<name>`; `GetRenameAnimationPath` replaces the file name with
//!   the `FF9DBAll.AnimationDB` key of the name and the folder with the `FF9BattleDB.GEO` key
//!   of the folder name (two special folders in `FOLDER_OVERRIDES`). The container stores
//!   `animations/<geo id>/<key>.anim`.
//! * Background: `HonoluluBattleMain.InitBattleScene` uses `FF9BattleDB.MapModel["BSC_<scene>"]`
//!   (`BBG_Bnnn`; `BattleBackground` in `BTL_SCENE_INFO` is a Memoria patch field and absent
//!   from the vanilla scene format), loads `BattleMap/BattleModel/battleMap_all/<bbg>/<bbg>`,
//!   reads `BattleMap/BattleInfo/INB_Bnnn.inb`, and `nf_InitBattleBG` loads
//!   `BBG_B<bbgnumber>_OBJ1..objanim` and, when `texanim != 0`, plays the animations from
//!   `BattleMap/BattleTexAnim/TAM_Bnnn.tab`. A missing `.tab` with `texanim == 0` only logs.
//! * Players: `btl_mot.BattleParameterList[serial]` (Memoria's `BattleParameters.csv`): model,
//!   trance model and 34 animation names (`SetPlayerDefMotion`). The model names resolve
//!   through the same `CreateModel` rules: `GEO_MAIN_B0_000` (Zidane) loads the field model `2/98`. Weapon: `btl_eqp.InitWeapon`
//!   uses `FF9BattleDB.GEO.GetValue(weapon.ModelId)` and, for `GEO_WEP*`, the directory
//!   `BattleMap/BattleModel/battle_weapon/<name>`; a weapon without a model name gets a dummy
//!   object and is not a failure.
//! * Player abilities do not use `.raw17`: `btl_vfx.SelectCommandVfx` calls
//!   `btlseq.RunSequence` only for enemy commands; player commands pick a `SpecialEffect` id
//!   (`GetPlayerCommandSFX`: `AA_DATA.Info.VfxIndex`, `Vfx2`, the character's `AttackSequence`,
//!   fixed ids for Steal, Throw and Trance) and `SFX.SetEffectArgs` loads the `TextAsset`
//!   `SpecialEffects/ef<id:D3>` and passes it to the native `SFX_Play`.

use super::battle_scene::{BattleInfo, BattleScene, Placement};
use super::battle_sequence::{SeqOp, SequenceFile};
use super::field_models::{FOLDER_OVERRIDES, ModelResolution, ModelStatus, resolve_model};
use super::model_assembly::{ClipEntry, ContainerEntry, ModelCatalog, ModelDir};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

mod names;

const BATTLE_MODEL_PREFIX: &str = "assets/resources/battlemap/battlemodel/";
/// Container root of the weapon models (`<type 6>/<geo id>/...`).
pub const WEAPON_MODEL_PREFIX: &str = BATTLE_MODEL_PREFIX;
const BACKGROUND_PREFIX: &str = "assets/resources/battlemap/battlemodel/battlemap_all/";
const TEX_ANIM_PREFIX: &str = "assets/resources/battlemap/battletexanim/";

// ---------------------------------------------------------------------------
// Name tables
// ---------------------------------------------------------------------------

/// `FF9BattleDB.Animation[id]`.
pub fn battle_animation_name(id: u32) -> Option<&'static str> {
    names::BATTLE_ANIMATIONS
        .binary_search_by_key(&id, |(key, _)| *key)
        .ok()
        .map(|index| names::BATTLE_ANIMATIONS[index].1)
}

/// Number of rows in `FF9BattleDB.Animation`.
pub fn battle_animation_count() -> usize {
    names::BATTLE_ANIMATIONS.len()
}

/// `FF9DBAll.AnimationDB.GetKey(name)` for the names this module knows.
pub fn animation_db_key(name: &str) -> Option<u32> {
    names::ANIMATION_DB_FIRST_KEY
        .binary_search_by_key(&name, |(key, _)| *key)
        .ok()
        .map(|index| names::ANIMATION_DB_FIRST_KEY[index].1)
}

/// `FF9BattleDB.GEO.GetKey(name)` (first key in source order).
pub fn geo_id_by_name(name: &str) -> Option<u32> {
    names::GEO_FIRST_ID
        .binary_search_by_key(&name, |(key, _)| *key)
        .ok()
        .map(|index| names::GEO_FIRST_ID[index].1)
}

/// `FF9BattleDB.MapModel["BSC_<scene>"]` for a bare scene name (any case).
pub fn background_name(scene: &str) -> Option<&'static str> {
    let key = format!("BSC_{}", scene.to_ascii_uppercase());
    names::MAP_MODEL
        .binary_search_by_key(&key.as_str(), |(scene, _)| *scene)
        .ok()
        .map(|index| names::MAP_MODEL[index].1)
}

/// Number of rows in `FF9BattleDB.MapModel`.
pub fn background_table_len() -> usize {
    names::MAP_MODEL.len()
}

/// `SpecialEffect` enum name of an id (the first when several aliases share it).
pub fn special_effect_name(id: i32) -> Option<&'static str> {
    let start = names::SPECIAL_EFFECTS.partition_point(|(value, _)| *value < id);
    names::SPECIAL_EFFECTS
        .get(start)
        .filter(|(value, _)| *value == id)
        .map(|(_, name)| *name)
}

/// `SpecialEffect` id of an enum name.
pub fn special_effect_id(name: &str) -> Option<i32> {
    names::SPECIAL_EFFECTS
        .iter()
        .find(|(_, candidate)| *candidate == name)
        .map(|(id, _)| *id)
}

// ---------------------------------------------------------------------------
// Clips
// ---------------------------------------------------------------------------

/// Clip lookup: `(folder geo id, AnimationDB key)` pairs.
#[derive(Debug, Clone, Default)]
pub struct ClipIndex {
    set: HashSet<(u32, u32)>,
    by_key: HashMap<u32, Vec<u32>>,
}

impl ClipIndex {
    pub fn new(clips: &[ClipEntry]) -> Self {
        let mut index = Self::default();
        for clip in clips {
            if let Ok(key) = clip.name.parse::<u32>()
                && index.set.insert((clip.geo_id, key))
            {
                index.by_key.entry(key).or_default().push(clip.geo_id);
            }
        }
        for folders in index.by_key.values_mut() {
            folders.sort_unstable();
        }
        index
    }

    pub fn len(&self) -> usize {
        self.set.len()
    }

    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnimationStatus {
    /// `animations/<folder geo id>/<key>.anim` exists.
    Resolved,
    /// The id is not a key of `FF9BattleDB.Animation` (the source throws).
    NotInBattleAnimationTable,
    /// The name has no `FF9DBAll.AnimationDB` key, so the path keeps the name as file name and
    /// cannot match a numeric container entry.
    NameNotInAnimationDb,
    /// The name has fewer than four tokens or its folder is not a `FF9BattleDB.GEO` name.
    UnknownFolder,
    /// Name and folder resolve but the clip is absent from the clip bundle.
    ClipMissing,
    /// No clip bundle was supplied; only the names were checked.
    NotChecked,
}

impl AnimationStatus {
    pub fn is_ok(self) -> bool {
        matches!(self, Self::Resolved | Self::NotChecked)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnimationResolution {
    /// `FF9BattleDB.Animation` id; absent for player rows, which carry names directly.
    pub animation_id: Option<u32>,
    pub name: Option<String>,
    pub folder_geo_id: Option<u32>,
    /// `FF9DBAll.AnimationDB` key used as the clip file name.
    pub clip_key: Option<u32>,
    /// Folders holding a clip with this key when the expected folder does not.
    pub clip_found_in_other_folders: Vec<u32>,
    pub status: AnimationStatus,
}

/// Resolves a battle animation name to a clip (see the module rules).
pub fn resolve_animation_name(
    name: &str,
    animation_id: Option<u32>,
    clips: Option<&ClipIndex>,
) -> AnimationResolution {
    let mut folder_geo_id = None;
    let mut clip_key = None;
    let mut other = Vec::new();
    let tokens: Vec<&str> = name.split('_').collect();
    let status = if tokens.len() < 4 {
        AnimationStatus::UnknownFolder
    } else {
        let folder = format!("GEO_{}_{}_{}", tokens[1], tokens[2], tokens[3]);
        clip_key = animation_db_key(name);
        folder_geo_id = FOLDER_OVERRIDES
            .iter()
            .find(|(over, _)| *over == folder)
            .map(|(_, id)| *id)
            .or_else(|| geo_id_by_name(&folder));
        match (clip_key, folder_geo_id) {
            (None, _) => AnimationStatus::NameNotInAnimationDb,
            (Some(_), None) => AnimationStatus::UnknownFolder,
            (Some(_), Some(_)) if clips.is_none() => AnimationStatus::NotChecked,
            (Some(key), Some(folder)) => {
                let clips = clips.expect("checked above");
                if clips.set.contains(&(folder, key)) {
                    AnimationStatus::Resolved
                } else {
                    if let Some(folders) = clips.by_key.get(&key) {
                        other = folders.clone();
                    }
                    AnimationStatus::ClipMissing
                }
            }
        }
    };
    AnimationResolution {
        animation_id,
        name: Some(name.to_owned()),
        folder_geo_id,
        clip_key,
        clip_found_in_other_folders: other,
        status,
    }
}

/// Resolves a `FF9BattleDB.Animation` id.
pub fn resolve_animation_id(id: u32, clips: Option<&ClipIndex>) -> AnimationResolution {
    match battle_animation_name(id) {
        Some(name) => resolve_animation_name(name, Some(id), clips),
        None => AnimationResolution {
            animation_id: Some(id),
            name: None,
            folder_geo_id: None,
            clip_key: None,
            clip_found_in_other_folders: Vec::new(),
            status: AnimationStatus::NotInBattleAnimationTable,
        },
    }
}

fn model_ok(status: ModelStatus) -> bool {
    matches!(status, ModelStatus::Resolved | ModelStatus::NotChecked)
}

// ---------------------------------------------------------------------------
// Backgrounds
// ---------------------------------------------------------------------------

/// Container facts about one `battlemap_all/<bbg>/` directory.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct BackgroundDir {
    pub roots: usize,
    pub meshes: usize,
    pub materials: usize,
    pub textures: usize,
    pub clips: usize,
    /// Objects the reader failed on for a reason other than an unsupported format (set by the
    /// caller).
    pub unreadable: Vec<String>,
    /// Textures that are present but use a pixel format the reader does not decode yet (set by
    /// the caller). The data exists; this is a reader limit, not a missing resource.
    pub undecoded_textures: Vec<String>,
}

/// Background directories, battle infos and texture animation tables of the battle bundle.
#[derive(Debug, Clone, Default)]
pub struct BackgroundCatalog {
    /// Upper-case `BBG_Bnnn` (also `BBG_Bnnn_OBJ1`, `BBG_NOFUJII`).
    pub dirs: BTreeMap<String, BackgroundDir>,
    /// Upper-case `TAM_Bnnn`.
    pub tex_anims: BTreeSet<String>,
    /// Upper-case `INB_Bnnn`.
    pub infos: BTreeMap<String, Result<BattleInfo, String>>,
}

impl BackgroundCatalog {
    /// Directories and texture tables from the container paths; `infos` is filled by the caller.
    pub fn from_container(entries: &[ContainerEntry]) -> Self {
        let mut catalog = Self::default();
        for entry in entries {
            if let Some(rest) = entry.path.strip_prefix(BACKGROUND_PREFIX) {
                let mut parts = rest.split('/');
                let (Some(dir), Some(_file)) = (parts.next(), parts.next()) else {
                    continue;
                };
                let slot = catalog.dirs.entry(dir.to_ascii_uppercase()).or_default();
                match entry.class_id {
                    1 => slot.roots += 1,
                    43 => slot.meshes += 1,
                    21 => slot.materials += 1,
                    28 => slot.textures += 1,
                    74 => slot.clips += 1,
                    _ => {}
                }
            } else if let Some(file) = entry.path.strip_prefix(TEX_ANIM_PREFIX)
                && let Some(name) = file.strip_suffix(".tab.bytes")
            {
                catalog.tex_anims.insert(name.to_ascii_uppercase());
            }
        }
        catalog
    }

    fn dir_problem(&self, name: &str) -> Option<String> {
        match self.dirs.get(name) {
            None => Some(format!("{name}: no directory in the bundle")),
            Some(dir) if dir.roots == 0 => Some(format!("{name}: no root GameObject")),
            Some(dir) if dir.meshes == 0 => Some(format!("{name}: no mesh")),
            Some(dir) if dir.materials == 0 || dir.textures == 0 => Some(format!(
                "{name}: {} materials and {} textures",
                dir.materials, dir.textures
            )),
            Some(dir) if !dir.unreadable.is_empty() => Some(format!(
                "{name}: {} unreadable objects, first {}",
                dir.unreadable.len(),
                dir.unreadable[0]
            )),
            Some(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundStatus {
    Resolved,
    /// The scene has no `FF9BattleDB.MapModel` row.
    NoMapModelRow,
    /// The `BBG_Bnnn` directory is missing or incomplete.
    ModelMissing,
    /// `INB_Bnnn.inb` is missing or unparsable.
    InfoMissing,
    /// A `BBG_Bnnn_OBJk` object model the info asks for is missing.
    ObjectModelMissing,
    /// `texanim != 0` but `TAM_Bnnn.tab` is absent.
    TexAnimMissing,
    /// No battle bundle catalog was supplied.
    NotChecked,
}

impl BackgroundStatus {
    pub fn is_ok(self) -> bool {
        matches!(self, Self::Resolved | Self::NotChecked)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BackgroundResolution {
    pub background: Option<String>,
    pub status: BackgroundStatus,
    pub bbgnumber: Option<i16>,
    pub texanim: Option<i16>,
    pub objanim: Option<i16>,
    /// The name's number differs from `INB.bbgnumber` (the source builds object names from the
    /// info number and the texture-animation table name from the model name).
    pub info_number_differs: bool,
    pub problems: Vec<String>,
    /// Textures of the background and its object models that are present but not decodable
    /// with the current reader.
    pub undecoded_textures: usize,
}

/// Resolves the background of one scene (`scene` is the bare name, any case).
pub fn resolve_background(
    scene: &str,
    catalog: Option<&BackgroundCatalog>,
) -> BackgroundResolution {
    let mut out = BackgroundResolution {
        background: None,
        status: BackgroundStatus::Resolved,
        bbgnumber: None,
        texanim: None,
        objanim: None,
        info_number_differs: false,
        problems: Vec::new(),
        undecoded_textures: 0,
    };
    let Some(bbg) = background_name(scene) else {
        out.status = BackgroundStatus::NoMapModelRow;
        out.problems
            .push(format!("no MapModel row for BSC_{scene}"));
        return out;
    };
    out.background = Some(bbg.to_owned());
    let Some(catalog) = catalog else {
        out.status = BackgroundStatus::NotChecked;
        return out;
    };
    let bbg_upper = bbg.to_ascii_uppercase();
    let mut status = BackgroundStatus::Resolved;
    let mut worst = |candidate: BackgroundStatus, out: &mut BackgroundResolution, text: String| {
        if status == BackgroundStatus::Resolved {
            status = candidate;
        }
        out.problems.push(text);
    };
    if let Some(problem) = catalog.dir_problem(&bbg_upper) {
        worst(BackgroundStatus::ModelMissing, &mut out, problem);
    }
    out.undecoded_textures += catalog
        .dirs
        .get(&bbg_upper)
        .map_or(0, |dir| dir.undecoded_textures.len());
    let info_key = bbg_upper.replacen("BBG", "INB", 1);
    match catalog.infos.get(&info_key) {
        None => worst(
            BackgroundStatus::InfoMissing,
            &mut out,
            format!("{info_key}: not in the bundle"),
        ),
        Some(Err(error)) => worst(
            BackgroundStatus::InfoMissing,
            &mut out,
            format!("{info_key}: {error}"),
        ),
        Some(Ok(info)) => {
            out.bbgnumber = Some(info.bbgnumber);
            out.texanim = Some(info.texanim);
            out.objanim = Some(info.objanim);
            let name_number = bbg_upper
                .strip_prefix("BBG_B")
                .and_then(|digits| digits.parse::<i16>().ok());
            out.info_number_differs = name_number != Some(info.bbgnumber);
            for object in 1..=info.objanim.max(0) {
                let name = format!("BBG_B{:03}_OBJ{object}", info.bbgnumber);
                if let Some(problem) = catalog.dir_problem(&name) {
                    worst(BackgroundStatus::ObjectModelMissing, &mut out, problem);
                }
                out.undecoded_textures += catalog
                    .dirs
                    .get(&name)
                    .map_or(0, |dir| dir.undecoded_textures.len());
            }
            let tab = bbg_upper.replacen("BBG", "TAM", 1);
            if info.texanim != 0 && !catalog.tex_anims.contains(&tab) {
                worst(
                    BackgroundStatus::TexAnimMissing,
                    &mut out,
                    format!("{tab}: texanim is {} but the table is absent", info.texanim),
                );
            }
        }
    }
    out.status = status;
    out
}

// ---------------------------------------------------------------------------
// Scenes
// ---------------------------------------------------------------------------

/// One scene as the sweep sees it.
pub struct SceneInput<'a> {
    /// Bare scene name, upper case (`EVT_BATTLE_` removed).
    pub name: &'a str,
    pub scene: Result<&'a BattleScene, String>,
    pub sequence: Option<Result<&'a SequenceFile, String>>,
}

/// Resolution inputs shared by every scene.
pub struct Facts<'a> {
    pub catalog: Option<&'a ModelCatalog>,
    pub assembled: &'a dyn Fn(&ModelDir) -> bool,
    pub clips: Option<&'a ClipIndex>,
    pub backgrounds: Option<&'a BackgroundCatalog>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnemyUse {
    pub type_no: usize,
    /// Some pattern places this type (first `monster_count` placements).
    pub in_pattern: bool,
    /// Some placement of the type is not a multipart slave, so it needs a model.
    pub model_needed: bool,
    pub geo_id: i16,
    /// The six `Mot` animation ids (`FF9BattleDB.Animation` keys).
    pub mot: [u16; 6],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SceneCoverage {
    pub scene: String,
    /// Why the scene (or its sequence file) could not be read.
    pub scene_error: Option<String>,
    pub sequence_error: Option<String>,
    pub enemies: Vec<EnemyUse>,
    /// Distinct animation ids referenced by used sequence slots through `Anim` operands.
    pub sequence_animation_ids: BTreeSet<u32>,
    /// `Anim` operands whose `base + code` index is past `animList` (the source reads past it).
    pub sequence_anim_index_out_of_range: usize,
    pub background: BackgroundResolution,
    pub unresolved: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MissingItem {
    pub scene: String,
    /// `enemy_model`, `enemy_animation`, `sequence_animation`, `sequence_anim_index`,
    /// `background`, `scene_unreadable`, `sequence_unreadable`.
    pub kind: &'static str,
    pub type_no: Option<usize>,
    pub id: Option<i64>,
    pub name: Option<String>,
    pub reason: String,
    /// The type is placed by a pattern (false: the table row exists but no pattern uses it).
    pub in_pattern: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Count {
    pub references: usize,
    pub references_resolved: usize,
    pub distinct: usize,
    pub distinct_resolved: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SceneTotals {
    pub scenes: usize,
    pub scenes_read: usize,
    pub scenes_with_sequence_file: usize,
    pub scenes_fully_resolved: usize,
    pub enemy_types: usize,
    pub enemy_types_in_pattern: usize,
    pub enemy_models: Count,
    pub enemy_animations: Count,
    pub sequence_animations: Count,
    pub backgrounds: Count,
    /// Distinct backgrounds whose textures include a format the reader does not decode.
    pub backgrounds_with_undecoded_textures: usize,
    pub enemy_model_status: BTreeMap<String, usize>,
    pub animation_status: BTreeMap<String, usize>,
    pub background_status: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SceneReport {
    pub totals: SceneTotals,
    pub models: Vec<ModelResolution>,
    pub animations: Vec<AnimationResolution>,
    pub backgrounds: Vec<BackgroundResolution>,
    pub scenes: Vec<SceneCoverage>,
    pub missing: Vec<MissingItem>,
}

fn status_key<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Which types a pattern places and whether any placement needs a model.
fn type_usage(scene: &BattleScene) -> BTreeMap<usize, bool> {
    let mut usage: BTreeMap<usize, bool> = BTreeMap::new();
    for pattern in &scene.patterns {
        let count = usize::from(pattern.monster_count).min(pattern.monsters.len());
        for placement in &pattern.monsters[..count] {
            let slave = placement.type_no > 0 && placement.flags & Placement::FLG_MULTIPART != 0;
            let needs = usage.entry(usize::from(placement.type_no)).or_insert(false);
            *needs |= !slave;
        }
    }
    usage
}

/// Resolves every scene (see the module rules).
pub fn build_scene_report(inputs: &[SceneInput<'_>], facts: &Facts<'_>) -> SceneReport {
    let mut models: BTreeMap<u16, ModelResolution> = BTreeMap::new();
    let mut animations: BTreeMap<u32, AnimationResolution> = BTreeMap::new();
    let mut backgrounds: BTreeMap<String, BackgroundResolution> = BTreeMap::new();
    let mut totals = SceneTotals {
        scenes: inputs.len(),
        ..SceneTotals::default()
    };
    let mut scenes = Vec::with_capacity(inputs.len());
    let mut missing = Vec::new();
    let mut enemy_model_ids: BTreeSet<u16> = BTreeSet::new();
    let mut enemy_animation_ids: BTreeSet<u32> = BTreeSet::new();
    let mut sequence_animation_ids: BTreeSet<u32> = BTreeSet::new();
    let mut background_names: BTreeSet<String> = BTreeSet::new();

    for input in inputs {
        let background = resolve_background(input.name, facts.backgrounds);
        let mut coverage = SceneCoverage {
            scene: input.name.to_owned(),
            scene_error: None,
            sequence_error: None,
            enemies: Vec::new(),
            sequence_animation_ids: BTreeSet::new(),
            sequence_anim_index_out_of_range: 0,
            background: background.clone(),
            unresolved: 0,
        };
        let mut local: Vec<MissingItem> = Vec::new();
        let item = |kind: &'static str,
                    type_no: Option<usize>,
                    id: Option<i64>,
                    name: Option<String>,
                    reason: String,
                    in_pattern: bool| MissingItem {
            scene: input.name.to_owned(),
            kind,
            type_no,
            id,
            name,
            reason,
            in_pattern,
        };

        match &input.scene {
            Err(error) => {
                coverage.scene_error = Some(error.clone());
                local.push(item(
                    "scene_unreadable",
                    None,
                    None,
                    None,
                    error.clone(),
                    true,
                ));
            }
            Ok(scene) => {
                totals.scenes_read += 1;
                let usage = type_usage(scene);
                totals.enemy_types += scene.monster_types.len();
                for (type_no, monster) in scene.monster_types.iter().enumerate() {
                    let in_pattern = usage.contains_key(&type_no);
                    let model_needed = usage.get(&type_no).copied().unwrap_or(false);
                    totals.enemy_types_in_pattern += usize::from(in_pattern);
                    coverage.enemies.push(EnemyUse {
                        type_no,
                        in_pattern,
                        model_needed,
                        geo_id: monster.geo,
                        mot: monster.mot,
                    });
                    if model_needed {
                        totals.enemy_models.references += 1;
                        match u16::try_from(monster.geo) {
                            Ok(geo) => {
                                enemy_model_ids.insert(geo);
                                let resolution = models.entry(geo).or_insert_with(|| {
                                    resolve_model(geo, facts.catalog, facts.assembled)
                                });
                                if model_ok(resolution.status) {
                                    totals.enemy_models.references_resolved += 1;
                                } else {
                                    local.push(item(
                                        "enemy_model",
                                        Some(type_no),
                                        Some(i64::from(geo)),
                                        resolution.geo_name.map(str::to_owned),
                                        status_key(&resolution.status),
                                        in_pattern,
                                    ));
                                }
                            }
                            Err(_) => local.push(item(
                                "enemy_model",
                                Some(type_no),
                                Some(i64::from(monster.geo)),
                                None,
                                "negative_geo_id".to_owned(),
                                in_pattern,
                            )),
                        }
                    }
                    for animation_id in monster.mot {
                        let id = u32::from(animation_id);
                        totals.enemy_animations.references += 1;
                        enemy_animation_ids.insert(id);
                        let resolution = animations
                            .entry(id)
                            .or_insert_with(|| resolve_animation_id(id, facts.clips));
                        if resolution.status.is_ok() {
                            totals.enemy_animations.references_resolved += 1;
                        } else {
                            local.push(item(
                                "enemy_animation",
                                Some(type_no),
                                Some(i64::from(id)),
                                resolution.name.clone(),
                                status_key(&resolution.status),
                                in_pattern,
                            ));
                        }
                    }
                }
            }
        }

        match &input.sequence {
            None => {}
            Some(Err(error)) => {
                coverage.sequence_error = Some(error.clone());
                local.push(item(
                    "sequence_unreadable",
                    None,
                    None,
                    None,
                    error.clone(),
                    true,
                ));
            }
            Some(Ok(file)) => {
                totals.scenes_with_sequence_file += 1;
                let mut referenced: BTreeMap<u32, usize> = BTreeMap::new();
                for slot in &file.slots {
                    let Some(sequence) = file.sequence(slot.index) else {
                        continue;
                    };
                    for placed in &sequence.ops {
                        let SeqOp::Anim { code } = placed.op else {
                            continue;
                        };
                        if code == 255 {
                            continue;
                        }
                        let index = usize::from(slot.base_anim) + usize::from(code);
                        match file.header.anim_list.get(index) {
                            Some(raw) => match u32::try_from(*raw) {
                                Ok(id) => *referenced.entry(id).or_insert(0) += 1,
                                Err(_) => {
                                    coverage.sequence_anim_index_out_of_range += 1;
                                    local.push(item(
                                        "sequence_anim_index",
                                        None,
                                        Some(i64::from(*raw)),
                                        None,
                                        format!("negative animation id in animList[{index}]"),
                                        true,
                                    ));
                                }
                            },
                            None => {
                                coverage.sequence_anim_index_out_of_range += 1;
                                local.push(item(
                                    "sequence_anim_index",
                                    None,
                                    Some(index as i64),
                                    None,
                                    format!(
                                        "slot {} code {code} base {}: index {index} past animList of {}",
                                        slot.index,
                                        slot.base_anim,
                                        file.header.anim_list.len()
                                    ),
                                    true,
                                ));
                            }
                        }
                    }
                }
                for (id, count) in referenced {
                    totals.sequence_animations.references += count;
                    sequence_animation_ids.insert(id);
                    coverage.sequence_animation_ids.insert(id);
                    let resolution = animations
                        .entry(id)
                        .or_insert_with(|| resolve_animation_id(id, facts.clips));
                    if resolution.status.is_ok() {
                        totals.sequence_animations.references_resolved += count;
                    } else {
                        local.push(item(
                            "sequence_animation",
                            None,
                            Some(i64::from(id)),
                            resolution.name.clone(),
                            status_key(&resolution.status),
                            true,
                        ));
                    }
                }
            }
        }

        totals.backgrounds.references += 1;
        if background.status.is_ok() {
            totals.backgrounds.references_resolved += 1;
        } else {
            local.push(item(
                "background",
                None,
                None,
                background.background.clone(),
                format!(
                    "{}: {}",
                    status_key(&background.status),
                    background.problems.join("; ")
                ),
                true,
            ));
        }
        if let Some(name) = &background.background {
            background_names.insert(name.clone());
            backgrounds
                .entry(name.clone())
                .or_insert_with(|| background.clone());
        }
        *totals
            .background_status
            .entry(status_key(&background.status))
            .or_insert(0) += 1;

        coverage.unresolved = local.len();
        if local.is_empty() {
            totals.scenes_fully_resolved += 1;
        }
        missing.extend(local);
        scenes.push(coverage);
    }

    totals.enemy_models.distinct = enemy_model_ids.len();
    totals.enemy_models.distinct_resolved = enemy_model_ids
        .iter()
        .filter(|id| model_ok(models[id].status))
        .count();
    totals.enemy_animations.distinct = enemy_animation_ids.len();
    totals.enemy_animations.distinct_resolved = enemy_animation_ids
        .iter()
        .filter(|id| animations[id].status.is_ok())
        .count();
    totals.sequence_animations.distinct = sequence_animation_ids.len();
    totals.sequence_animations.distinct_resolved = sequence_animation_ids
        .iter()
        .filter(|id| animations[id].status.is_ok())
        .count();
    totals.backgrounds.distinct = background_names.len();
    totals.backgrounds.distinct_resolved = backgrounds
        .values()
        .filter(|background| background.status.is_ok())
        .count();
    totals.backgrounds_with_undecoded_textures = backgrounds
        .values()
        .filter(|background| background.undecoded_textures > 0)
        .count();
    for id in &enemy_model_ids {
        *totals
            .enemy_model_status
            .entry(status_key(&models[id].status))
            .or_insert(0) += 1;
    }
    for resolution in animations.values() {
        *totals
            .animation_status
            .entry(status_key(&resolution.status))
            .or_insert(0) += 1;
    }
    SceneReport {
        totals,
        models: models.into_values().collect(),
        animations: animations.into_values().collect(),
        backgrounds: backgrounds.into_values().collect(),
        scenes,
        missing,
    }
}

// ---------------------------------------------------------------------------
// Players and weapons
// ---------------------------------------------------------------------------

/// A name-addressed model (player model, trance model or weapon).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NamedModel {
    pub name: String,
    pub geo_id: Option<u32>,
    pub resolution: Option<ModelResolution>,
    /// `NoGeoName` stands for a name without a `FF9BattleDB.GEO` key.
    pub status: ModelStatus,
}

fn resolve_named_model(
    name: &str,
    catalog: Option<&ModelCatalog>,
    assembled: &dyn Fn(&ModelDir) -> bool,
) -> NamedModel {
    match geo_id_by_name(name).and_then(|id| u16::try_from(id).ok().map(|short| (id, short))) {
        None => NamedModel {
            name: name.to_owned(),
            geo_id: None,
            resolution: None,
            status: ModelStatus::NoGeoName,
        },
        Some((id, short)) => {
            let resolution = resolve_model(short, catalog, assembled);
            NamedModel {
                name: name.to_owned(),
                geo_id: Some(id),
                status: resolution.status,
                resolution: Some(resolution),
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlayerCoverage {
    pub serial: u32,
    pub label: &'static str,
    pub model: NamedModel,
    pub trance_model: NamedModel,
    pub animations: Vec<AnimationResolution>,
    /// `AttackSequence` (`SpecialEffect` id) from the same row.
    pub attack_sequence: i32,
    pub weapon_bone: u8,
    pub unresolved: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WeaponInput {
    pub id: u32,
    /// `None` when the row has no model name (dummy weapon, not a failure).
    pub model_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WeaponCoverage {
    pub id: u32,
    pub model_name: Option<String>,
    pub status: WeaponStatus,
    pub geo_id: Option<u32>,
    pub directories: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WeaponStatus {
    Resolved,
    /// Row without a model name: `InitWeapon` makes a dummy object.
    NoModelByDesign,
    NoGeoName,
    NoDirectory,
    DirectoryNotAssembled,
    NotChecked,
}

impl WeaponStatus {
    pub fn is_ok(self) -> bool {
        matches!(
            self,
            Self::Resolved | Self::NoModelByDesign | Self::NotChecked
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlayerReport {
    pub players: Vec<PlayerCoverage>,
    pub weapons: Vec<WeaponCoverage>,
    pub totals: PlayerTotals,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PlayerTotals {
    pub players: usize,
    pub model_references: usize,
    pub model_references_resolved: usize,
    pub animation_references: usize,
    pub animation_references_resolved: usize,
    pub distinct_animations: usize,
    pub distinct_animations_resolved: usize,
    pub weapons: usize,
    pub weapons_with_model: usize,
    pub weapons_resolved: usize,
    pub distinct_weapon_models: usize,
    pub distinct_weapon_models_resolved: usize,
    pub animation_status: BTreeMap<String, usize>,
    pub weapon_status: BTreeMap<String, usize>,
}

/// Resolves the 19 `BattleParameters.csv` rows and the weapon rows.
pub fn build_player_report(
    player_catalog: Option<&ModelCatalog>,
    player_assembled: &dyn Fn(&ModelDir) -> bool,
    weapon_catalog: Option<&ModelCatalog>,
    weapon_assembled: &dyn Fn(&ModelDir) -> bool,
    clips: Option<&ClipIndex>,
    weapons: &[WeaponInput],
) -> PlayerReport {
    let mut totals = PlayerTotals::default();
    let mut animation_cache: BTreeMap<String, AnimationResolution> = BTreeMap::new();
    let mut players = Vec::new();
    for row in names::PLAYERS {
        let model = resolve_named_model(row.model, player_catalog, player_assembled);
        let trance_model = resolve_named_model(row.trance_model, player_catalog, player_assembled);
        let mut unresolved = 0;
        for named in [&model, &trance_model] {
            totals.model_references += 1;
            if model_ok(named.status) {
                totals.model_references_resolved += 1;
            } else {
                unresolved += 1;
            }
        }
        let mut animations = Vec::new();
        for name in row.animations {
            totals.animation_references += 1;
            let resolution = animation_cache
                .entry(name.to_owned())
                .or_insert_with(|| resolve_animation_name(name, None, clips))
                .clone();
            if resolution.status.is_ok() {
                totals.animation_references_resolved += 1;
            } else {
                unresolved += 1;
            }
            animations.push(resolution);
        }
        players.push(PlayerCoverage {
            serial: row.serial,
            label: row.label,
            model,
            trance_model,
            animations,
            attack_sequence: row.attack_sequence,
            weapon_bone: row.weapon_bone,
            unresolved,
        });
    }
    totals.players = players.len();
    totals.distinct_animations = animation_cache.len();
    for resolution in animation_cache.values() {
        totals.distinct_animations_resolved += usize::from(resolution.status.is_ok());
        *totals
            .animation_status
            .entry(status_key(&resolution.status))
            .or_insert(0) += 1;
    }

    let mut weapon_rows = Vec::new();
    let mut distinct: BTreeMap<String, bool> = BTreeMap::new();
    for weapon in weapons {
        totals.weapons += 1;
        let (status, geo_id, directories) = match &weapon.model_name {
            None => (WeaponStatus::NoModelByDesign, None, Vec::new()),
            Some(name) => {
                totals.weapons_with_model += 1;
                let named = resolve_named_model(name, weapon_catalog, weapon_assembled);
                let directories = named
                    .resolution
                    .as_ref()
                    .map(|resolution| resolution.directories.clone())
                    .unwrap_or_default();
                let status = match named.status {
                    ModelStatus::Resolved => WeaponStatus::Resolved,
                    ModelStatus::NoGeoName => WeaponStatus::NoGeoName,
                    ModelStatus::NoDirectory => WeaponStatus::NoDirectory,
                    ModelStatus::DirectoryNotAssembled => WeaponStatus::DirectoryNotAssembled,
                    ModelStatus::NotChecked => WeaponStatus::NotChecked,
                };
                distinct.insert(name.clone(), status.is_ok());
                (status, named.geo_id, directories)
            }
        };
        totals.weapons_resolved += usize::from(status.is_ok() && weapon.model_name.is_some());
        *totals.weapon_status.entry(status_key(&status)).or_insert(0) += 1;
        weapon_rows.push(WeaponCoverage {
            id: weapon.id,
            model_name: weapon.model_name.clone(),
            status,
            geo_id,
            directories,
        });
    }
    totals.distinct_weapon_models = distinct.len();
    totals.distinct_weapon_models_resolved = distinct.values().filter(|ok| **ok).count();
    PlayerReport {
        players,
        weapons: weapon_rows,
        totals,
    }
}

// ---------------------------------------------------------------------------
// Player ability effects
// ---------------------------------------------------------------------------

/// One reference to a `SpecialEffect` id and where it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EffectReference {
    /// `aa_data[<id>].info.vfx_no`, `aa_data[<id>].vfx2`, `item_use[<id>]`, `player[<serial>]`
    /// or `fixed:<name>`.
    pub source: String,
    pub effect_id: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EffectResolution {
    pub effect_id: i32,
    pub enum_name: Option<&'static str>,
    /// `ef<id:D3>` TextAsset exists in `resources.assets`.
    pub asset_present: bool,
    pub asset_bytes: Option<usize>,
    pub referenced_by: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EffectTotals {
    pub references: usize,
    /// References with a negative id (`Special_No_Effect`: nothing to play).
    pub no_effect_references: usize,
    pub distinct_effects: usize,
    pub distinct_effects_present: usize,
    pub effects_not_in_enum: usize,
    pub assets_in_resources: usize,
    pub assets_unreferenced: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EffectReport {
    pub totals: EffectTotals,
    pub effects: Vec<EffectResolution>,
    pub missing: Vec<EffectReference>,
}

/// Matches effect references against the `ef<id>` TextAssets (`id -> byte length`).
pub fn build_effect_report(
    references: &[EffectReference],
    assets: &BTreeMap<i32, usize>,
) -> EffectReport {
    let mut totals = EffectTotals {
        references: references.len(),
        assets_in_resources: assets.len(),
        ..EffectTotals::default()
    };
    let mut counts: BTreeMap<i32, usize> = BTreeMap::new();
    let mut missing = Vec::new();
    for reference in references {
        if reference.effect_id < 0 {
            totals.no_effect_references += 1;
            continue;
        }
        *counts.entry(reference.effect_id).or_insert(0) += 1;
        if !assets.contains_key(&reference.effect_id) {
            missing.push(reference.clone());
        }
    }
    let effects: Vec<EffectResolution> = counts
        .iter()
        .map(|(id, count)| EffectResolution {
            effect_id: *id,
            enum_name: special_effect_name(*id),
            asset_present: assets.contains_key(id),
            asset_bytes: assets.get(id).copied(),
            referenced_by: *count,
        })
        .collect();
    totals.distinct_effects = effects.len();
    totals.distinct_effects_present = effects.iter().filter(|e| e.asset_present).count();
    totals.effects_not_in_enum = effects.iter().filter(|e| e.enum_name.is_none()).count();
    totals.assets_unreferenced = assets.keys().filter(|id| !counts.contains_key(id)).count();
    EffectReport {
        totals,
        effects,
        missing,
    }
}

/// The fixed `SpecialEffect` names `btl_vfx.GetPlayerCommandSFX` returns without a table row.
pub const FIXED_PLAYER_EFFECTS: &[&str] = &[
    "Steal_Zidane_Dagger",
    "Steal_Zidane_Sword",
    "Steal_Cinna",
    "Steal_Marcus",
    "Steal_Blank",
    "Throw_Racket",
    "Throw_Thief_Sword",
    "Throw_Sword",
    "Throw_Spear",
    "Throw_Claw",
    "Throw_Rod",
    "Throw_Fork",
    "Throw_Dagger",
    "Throw_Disc",
    "Special_Trance_Activate",
    "Special_Trance_End",
];

/// Model and trance model names of each player row.
pub fn player_model_names() -> Vec<(&'static str, &'static str)> {
    names::PLAYERS
        .iter()
        .map(|row| (row.model, row.trance_model))
        .collect()
}

/// `BattleParameters.csv` row `serial`: label, model name and the
/// `MP_IDLE_NORMAL` motion name (`SetPlayerDefMotion`, motion 0).
/// `BattleParameters.csv` row `serial`: its `TranceModelId`
/// (`btl_init.GetModelID(serial, isTrance: true)`).
pub fn player_trance_model(serial: u32) -> Option<&'static str> {
    names::PLAYERS
        .iter()
        .find(|row| row.serial == serial)
        .map(|row| row.trance_model)
}

/// `BattleParameters.csv` row `serial`: its `TranceGlowingColor`, the color
/// `btl_stat.SetStatusVfx` pulses on the model while in Trance.
pub fn player_trance_glow(serial: u32) -> Option<[u8; 3]> {
    names::PLAYERS
        .iter()
        .find(|row| row.serial == serial)
        .map(|row| row.trance_glow)
}

pub fn player_battle_row(serial: u32) -> Option<(&'static str, &'static str, &'static str)> {
    names::PLAYERS
        .iter()
        .find(|row| row.serial == serial)
        .map(|row| (row.label, row.model, row.animations[0]))
}

/// `BattleParameters.csv` row `serial`: its 34 motion names in
/// `BattlePlayerCharacter.PlayerMotionIndex` order (`MP_IDLE_NORMAL` ..
/// `MP_SPECIAL1`).
pub fn player_battle_motions(serial: u32) -> Option<&'static [&'static str; 34]> {
    names::PLAYERS
        .iter()
        .find(|row| row.serial == serial)
        .map(|row| &row.animations)
}

/// `AttackSequence` of each player row (`SpecialEffect` id).
pub fn player_attack_sequences() -> Vec<(u32, &'static str, i32)> {
    names::PLAYERS
        .iter()
        .map(|row| (row.serial, row.label, row.attack_sequence))
        .collect()
}

#[cfg(test)]
mod tests;
