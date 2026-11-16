//! `tantalusctl assets inspect-bundle --path p0data2.bin --battle-model-coverage <out json>`:
//! checks that every model, animation clip and background the 562 battle scenes reference
//! resolves to data, covers the playable characters and weapons, and locates the player
//! ability effect data. The resolution rules and result types are pure and live in
//! `tantalus_assets::pc::battle_models`; this file reads the bundles and the install tables.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use tantalus_assets::pc::battle_models::{
    BackgroundCatalog, ClipIndex, EffectReference, EffectReport, FIXED_PLAYER_EFFECTS, Facts,
    PlayerReport, SceneInput, SceneReport, WEAPON_MODEL_PREFIX, WeaponInput,
    battle_animation_count, build_effect_report, build_player_report, build_scene_report,
    geo_id_by_name, player_attack_sequences, player_model_names, special_effect_id,
};
use tantalus_assets::pc::battle_scene::BattleBundleData;
use tantalus_assets::pc::battle_sequence::SequenceFile;
use tantalus_assets::pc::battle_tables::{BattleTables, extract_battle_tables};
use tantalus_assets::pc::managed_il::Assembly;
use tantalus_assets::pc::model_assembly::{
    AssembleOptions, ContainerEntry, ModelCatalog, ModelDir, ObjectStore, SerializedStore,
    assemble_model, clip_entries,
};
use tantalus_assets::pc::serialized::SerializedFile;
use tantalus_assets::pc::serialized_inventory::StandaloneSerialized;
use tantalus_assets::pc::unity_raw::UnityRawBundle;

const MAX_BUNDLE_BYTES: usize = 600 * 1024 * 1024;
const BACKGROUND_PREFIX: &str = "assets/resources/battlemap/battlemodel/battlemap_all/";

pub struct BattleModelsRequest {
    /// `p0data4.bin` (monster and player models); defaults to a sibling of the battle bundle.
    pub models: Option<PathBuf>,
    /// `p0data5.bin` (animation clips); defaults to a sibling of the battle bundle.
    pub clips: Option<PathBuf>,
    /// `Assembly-CSharp.dll` (weapon and ability tables); defaults to the install's copy.
    pub dll: Option<PathBuf>,
    /// `resources.assets` (`ef<id>` effect TextAssets); defaults to the install's copy.
    pub resources_assets: Option<PathBuf>,
    pub out: PathBuf,
}

#[derive(Debug, Serialize)]
pub struct Sources {
    pub battle_bundle: String,
    pub models_bundle: Option<String>,
    pub clips_bundle: Option<String>,
    pub assembly: Option<String>,
    pub assembly_sha256: Option<String>,
    pub resources_assets: Option<String>,
    pub reference: &'static str,
    pub battle_animation_rows: usize,
    pub clip_count: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct PlayerAbilitySequences {
    pub answer: Vec<&'static str>,
    pub effects: Option<EffectReport>,
    /// Attempt to read each `ef<id>` TextAsset with the `.raw17` decoder.
    pub raw17_probe: Option<Raw17Probe>,
    pub player_attack_sequences: Vec<(u32, &'static str, i32)>,
    pub skipped: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Raw17Probe {
    pub assets: usize,
    pub header_parses: usize,
    pub all_streams_terminate: usize,
    pub example_failures: Vec<String>,
}

/// Where a model that is absent from the model bundle shows up in another install container.
#[derive(Debug, Serialize)]
pub struct ElsewhereSearch {
    /// Containers that were opened and read.
    pub searched: Vec<String>,
    /// Containers that could not be opened as serialized bundles, with the reason.
    pub unreadable: Vec<String>,
    /// Geo ids searched for (those with no directory in the model bundle).
    pub geo_ids_searched: usize,
    pub found: Vec<ElsewhereHit>,
}

#[derive(Debug, Serialize)]
pub struct ElsewhereHit {
    pub geo_id: u32,
    pub bundle: String,
    /// `<type>/<geo id>` directories under the standard `assets/resources/models/` prefix.
    pub directories: Vec<String>,
    /// Container paths under any other prefix that have the geo id as a whole path segment
    /// (at most three shown, paths containing `model`); these are not confirmed to be the model.
    pub other_paths: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct BattleModelCoverageReport {
    pub schema: &'static str,
    pub sources: Sources,
    pub scenes: SceneReport,
    pub players: Option<PlayerReport>,
    /// Search of the other `p0data*.bin` containers for models the model bundle lacks.
    pub models_elsewhere: Option<ElsewhereSearch>,
    pub player_ability_sequences: PlayerAbilitySequences,
    pub not_done: Vec<&'static str>,
}

type StoreResult<T> = Result<T, String>;

fn open_store<T>(
    path: &Path,
    run: impl FnOnce(&SerializedStore<'_, '_>) -> StoreResult<T>,
) -> StoreResult<T> {
    let source = path.to_string_lossy().into_owned();
    let bundle = UnityRawBundle::open(path).map_err(|error| error.to_string())?;
    let bytes = bundle
        .read_entry(0, MAX_BUNDLE_BYTES)
        .map_err(|error| error.to_string())?;
    let file = SerializedFile::parse(&bytes, &source).map_err(|error| error.to_string())?;
    let store = SerializedStore::new(&file, &source).map_err(|error| error.to_string())?;
    run(&store)
}

/// Removes backtick-quoted file system paths from a reader message, so the audit file carries
/// no install location.
fn strip_quoted_paths(message: &str) -> String {
    let mut out = String::new();
    let mut rest = message;
    while let Some(start) = rest.find('`') {
        let Some(len) = rest[start + 1..].find('`') else {
            break;
        };
        let quoted = &rest[start + 1..start + 1 + len];
        out.push_str(&rest[..start]);
        if quoted.contains('\\') || quoted.contains(":/") {
            out.push_str("<path>");
        } else {
            out.push('`');
            out.push_str(quoted);
            out.push('`');
        }
        rest = &rest[start + len + 2..];
    }
    out.push_str(rest);
    out
}

pub(crate) fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn sibling(path: &Path, name: &str) -> Option<PathBuf> {
    let candidate = path.parent()?.join(name);
    candidate.is_file().then_some(candidate)
}

pub(crate) fn install_file(bundle: &Path, relative: &str) -> Option<PathBuf> {
    let candidate = bundle.parent()?.parent()?.join(relative);
    candidate.is_file().then_some(candidate)
}

/// Catalog of a bundle plus the directories (by label) that assemble.
struct ModelSide {
    catalog: ModelCatalog,
    assembled: HashMap<String, bool>,
}

fn model_side(
    path: &Path,
    prefix: Option<&str>,
    wanted_geo: &BTreeSet<u32>,
) -> StoreResult<ModelSide> {
    open_store(path, |store| {
        let entries = store.container().map_err(|error| error.to_string())?;
        let catalog = match prefix {
            Some(prefix) => ModelCatalog::from_container_with_prefix(&entries, prefix),
            None => ModelCatalog::from_container(&entries),
        };
        let mut assembled = HashMap::new();
        for geo in wanted_geo {
            for dir in catalog.dirs_for_geo(*geo) {
                let ok = dir.roots.first().is_some_and(|root| {
                    assemble_model(
                        store,
                        &dir.label,
                        *root,
                        AssembleOptions {
                            decode_textures: false,
                        },
                    )
                    .is_ok()
                });
                assembled.insert(dir.label.clone(), ok);
            }
        }
        Ok(ModelSide { catalog, assembled })
    })
}

/// Battle bundle: background catalog with decode checks, and the weapon model catalog.
fn battle_bundle_side(
    path: &Path,
    data: &BattleBundleData,
    wanted_weapon_geo: &BTreeSet<u32>,
) -> StoreResult<(BackgroundCatalog, ModelSide)> {
    open_store(path, |store| {
        let entries: Vec<ContainerEntry> = store.container().map_err(|error| error.to_string())?;
        let mut backgrounds = BackgroundCatalog::from_container(&entries);
        for (key, info) in &data.infos {
            backgrounds.infos.insert(
                key.clone(),
                info.as_ref().cloned().map_err(|error| error.to_string()),
            );
        }
        for entry in &entries {
            let Some(rest) = entry.path.strip_prefix(BACKGROUND_PREFIX) else {
                continue;
            };
            let Some((dir, _)) = rest.split_once('/') else {
                continue;
            };
            let failure: Option<String> = match entry.class_id {
                43 => match store.load(entry.path_id) {
                    Ok(Some(_)) => None,
                    Ok(None) => Some("object not found".to_owned()),
                    Err(error) => Some(error.to_string()),
                },
                28 => match store.texture(entry.path_id) {
                    Ok(Some(_)) => None,
                    Ok(None) => Some("not a Texture2D".to_owned()),
                    Err(error) => Some(error.to_string()),
                },
                _ => continue,
            };
            if let Some(failure) = failure
                && let Some(slot) = backgrounds.dirs.get_mut(&dir.to_ascii_uppercase())
            {
                // Keep the reader's reason but drop the install path it quotes.
                let failure = strip_quoted_paths(&failure);
                let line = format!("{} (class {}): {failure}", entry.path, entry.class_id);
                // A texture the reader cannot decode (unsupported pixel format, mip chain it
                // does not slice) is present data; it is counted apart from missing data.
                if entry.class_id == 28 {
                    slot.undecoded_textures.push(line);
                } else {
                    slot.unreadable.push(line);
                }
            }
        }
        let catalog = ModelCatalog::from_container_with_prefix(&entries, WEAPON_MODEL_PREFIX);
        let mut assembled = HashMap::new();
        for geo in wanted_weapon_geo {
            for dir in catalog.dirs_for_geo(*geo) {
                let ok = dir.roots.first().is_some_and(|root| {
                    assemble_model(
                        store,
                        &dir.label,
                        *root,
                        AssembleOptions {
                            decode_textures: false,
                        },
                    )
                    .is_ok()
                });
                assembled.insert(dir.label.clone(), ok);
            }
        }
        Ok((backgrounds, ModelSide { catalog, assembled }))
    })
}

/// `ef<id>` TextAssets of `resources.assets`: id to byte length, plus the `.raw17` probe.
/// Looks for the given geo ids in every other `p0data*.bin` next to the battle bundle.
fn search_elsewhere(
    battle_path: &Path,
    skip: &[Option<&PathBuf>],
    missing: &BTreeSet<u32>,
) -> ElsewhereSearch {
    let mut out = ElsewhereSearch {
        searched: Vec::new(),
        unreadable: Vec::new(),
        geo_ids_searched: missing.len(),
        found: Vec::new(),
    };
    let Some(dir) = battle_path.parent() else {
        return out;
    };
    let mut names: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            let name = file_name(path);
            name.starts_with("p0data") && name.ends_with(".bin")
        })
        .filter(|path| {
            path != battle_path && !skip.iter().flatten().any(|skipped| *skipped == path)
        })
        .collect();
    names.sort();
    for path in names {
        let name = file_name(&path);
        let result = open_store(&path, |store| {
            let entries = store.container().map_err(|error| error.to_string())?;
            let catalog = ModelCatalog::from_container(&entries);
            let mut by_segment: HashMap<u32, Vec<&str>> = HashMap::new();
            for entry in &entries {
                for segment in entry.path.split('/') {
                    let stem = segment.split('.').next().unwrap_or(segment);
                    if let Ok(id) = stem.parse::<u32>()
                        && missing.contains(&id)
                        && entry.path.to_ascii_lowercase().contains("model")
                    {
                        let paths = by_segment.entry(id).or_default();
                        if paths.len() < 3 && !paths.contains(&entry.path.as_str()) {
                            paths.push(&entry.path);
                        }
                    }
                }
            }
            let mut hits = Vec::new();
            for geo in missing {
                let dirs: Vec<String> = catalog
                    .dirs_for_geo(*geo)
                    .iter()
                    .map(|dir| dir.label.clone())
                    .collect();
                let others: Vec<String> = by_segment
                    .get(geo)
                    .map(|paths| paths.iter().map(|path| path.to_string()).collect())
                    .unwrap_or_default();
                if !dirs.is_empty() || !others.is_empty() {
                    hits.push((*geo, dirs, others));
                }
            }
            Ok(hits)
        });
        match result {
            Ok(hits) => {
                out.searched.push(name.clone());
                for (geo, directories, other_paths) in hits {
                    out.found.push(ElsewhereHit {
                        geo_id: geo,
                        bundle: name.clone(),
                        directories,
                        other_paths,
                    });
                }
            }
            Err(error) => {
                let error = error.split(" for `").next().unwrap_or(&error).to_string();
                out.unreadable.push(format!("{name}: {error}"));
            }
        }
    }
    out
}

fn effect_assets(path: &Path) -> StoreResult<(BTreeMap<i32, usize>, Raw17Probe)> {
    let mut file = StandaloneSerialized::open(path).map_err(|error| error.to_string())?;
    let assets = file.text_assets().map_err(|error| error.to_string())?;
    let mut found = BTreeMap::new();
    let mut probe = Raw17Probe {
        assets: 0,
        header_parses: 0,
        all_streams_terminate: 0,
        example_failures: Vec::new(),
    };
    for (name, object) in assets {
        let Some(digits) = name.strip_prefix("ef") else {
            continue;
        };
        if digits.len() != 3 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let Ok(id) = digits.parse::<i32>() else {
            continue;
        };
        let asset = file
            .read_text_asset(&object)
            .map_err(|error| format!("{name}: {error}"))?;
        found.insert(id, asset.script.len());
        probe.assets += 1;
        let attempt = std::panic::catch_unwind(|| SequenceFile::parse(&asset.script, &name));
        match attempt.unwrap_or_else(|_| {
            Err(tantalus_assets::install::ImportError::invalid(
                &name,
                "decoder panicked",
            ))
        }) {
            Ok(parsed) => {
                probe.header_parses += 1;
                use tantalus_assets::pc::battle_sequence::Terminator;
                if parsed.sequences.values().all(|sequence| {
                    matches!(sequence.terminator, Terminator::End | Terminator::FastEnd)
                }) {
                    probe.all_streams_terminate += 1;
                }
            }
            Err(error) => {
                if probe.example_failures.len() < 3 {
                    probe.example_failures.push(format!("{name}: {error}"));
                }
            }
        }
    }
    Ok((found, probe))
}

pub(crate) fn effect_references(tables: Option<&BattleTables>) -> Vec<EffectReference> {
    let mut references = Vec::new();
    if let Some(tables) = tables {
        for action in &tables.actions {
            references.push(EffectReference {
                source: format!("aa_data[{}].info.vfx_no", action.id),
                effect_id: i32::from(action.info.vfx_no),
            });
            references.push(EffectReference {
                source: format!("aa_data[{}].vfx2", action.id),
                effect_id: i32::from(action.vfx2),
            });
        }
        for item in &tables.item_use {
            references.push(EffectReference {
                source: format!("item_use[{}].info.vfx_no", item.id),
                effect_id: i32::from(item.info.vfx_no),
            });
        }
    }
    for (serial, label, id) in player_attack_sequences() {
        references.push(EffectReference {
            source: format!("player[{serial}] ({label}).attack_sequence"),
            effect_id: id,
        });
    }
    for name in FIXED_PLAYER_EFFECTS {
        if let Some(id) = special_effect_id(name) {
            references.push(EffectReference {
                source: format!("fixed:{name}"),
                effect_id: id,
            });
        }
    }
    references
}

pub fn build_report(
    battle_path: &Path,
    request: &BattleModelsRequest,
) -> StoreResult<BattleModelCoverageReport> {
    let data = BattleBundleData::load(battle_path).map_err(|error| error.to_string())?;

    let models_path = request
        .models
        .clone()
        .or_else(|| sibling(battle_path, "p0data4.bin"));
    let clips_path = request
        .clips
        .clone()
        .or_else(|| sibling(battle_path, "p0data5.bin"));
    let dll_path = request
        .dll
        .clone()
        .or_else(|| install_file(battle_path, "x64/FF9_Data/Managed/Assembly-CSharp.dll"));
    let resources_path = request
        .resources_assets
        .clone()
        .or_else(|| install_file(battle_path, "x64/FF9_Data/resources.assets"));

    let mut skipped = Vec::new();

    let mut assembly_sha = None;
    let tables = match &dll_path {
        Some(dll) => {
            let bytes = std::fs::read(dll)
                .map_err(|error| format!("cannot read {}: {error}", dll.display()))?;
            assembly_sha = Some(format!("{:x}", Sha256::digest(&bytes)));
            let assembly = Assembly::parse(&bytes).map_err(|error| error.to_string())?;
            Some(extract_battle_tables(&assembly).map_err(|error| error.to_string())?)
        }
        None => {
            skipped
                .push("no Assembly-CSharp.dll: weapons and ability effect ids not checked".into());
            None
        }
    };

    let clips = match &clips_path {
        Some(path) => Some(open_store(path, |store| {
            let (clips, _other) = clip_entries(&store.container().map_err(|e| e.to_string())?);
            Ok(clips)
        })?),
        None => {
            skipped.push("no clip bundle: animation clips not checked".into());
            None
        }
    };
    let clip_index = clips.as_deref().map(ClipIndex::new);

    // Geo ids worth assembling: every monster type's geo and every player model.
    let mut wanted_geo: BTreeSet<u32> = BTreeSet::new();
    for scene in data.scenes.values().flatten() {
        for monster in &scene.monster_types {
            if let Ok(geo) = u32::try_from(monster.geo) {
                wanted_geo.insert(geo);
            }
        }
    }
    let player_names = player_model_names();
    for (model, trance) in &player_names {
        wanted_geo.extend(geo_id_by_name(model));
        wanted_geo.extend(geo_id_by_name(trance));
    }
    let models_side = match &models_path {
        Some(path) => Some(model_side(path, None, &wanted_geo)?),
        None => {
            skipped.push("no model bundle: monster and player models not checked".into());
            None
        }
    };

    let models_elsewhere = models_side.as_ref().map(|side| {
        let missing: BTreeSet<u32> = wanted_geo
            .iter()
            .copied()
            .filter(|geo| side.catalog.dirs_for_geo(*geo).is_empty())
            .collect();
        search_elsewhere(
            battle_path,
            &[models_path.as_ref(), clips_path.as_ref()],
            &missing,
        )
    });

    let weapon_inputs: Vec<WeaponInput> = tables
        .as_ref()
        .map(|tables| {
            tables
                .weapons
                .iter()
                .map(|weapon| WeaponInput {
                    id: weapon.id,
                    model_name: weapon.model_name.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    let wanted_weapon_geo: BTreeSet<u32> = weapon_inputs
        .iter()
        .filter_map(|weapon| weapon.model_name.as_deref().and_then(geo_id_by_name))
        .collect();
    let (background_catalog, weapon_side) =
        battle_bundle_side(battle_path, &data, &wanted_weapon_geo)?;

    // Scenes.
    let sequences: BTreeMap<&String, Result<SequenceFile, String>> = data
        .sequence_bytes
        .iter()
        .map(|(key, bytes)| {
            (
                key,
                SequenceFile::parse(bytes, key).map_err(|error| error.to_string()),
            )
        })
        .collect();
    let mut inputs = Vec::new();
    for (key, scene) in &data.scenes {
        let name = key.strip_prefix("EVT_BATTLE_").unwrap_or(key);
        inputs.push(SceneInput {
            name,
            scene: scene.as_ref().map_err(|error| error.to_string()),
            sequence: sequences
                .get(key)
                .map(|parsed| parsed.as_ref().map_err(|error| error.clone())),
        });
    }
    let enemy_assembled = |dir: &ModelDir| -> bool {
        models_side
            .as_ref()
            .and_then(|side| side.assembled.get(&dir.label).copied())
            .unwrap_or(false)
    };
    let facts = Facts {
        catalog: models_side.as_ref().map(|side| &side.catalog),
        assembled: &enemy_assembled,
        clips: clip_index.as_ref(),
        backgrounds: Some(&background_catalog),
    };
    let scenes = build_scene_report(&inputs, &facts);

    let weapon_assembled = |dir: &ModelDir| -> bool {
        weapon_side
            .assembled
            .get(&dir.label)
            .copied()
            .unwrap_or(false)
    };
    let players = build_player_report(
        models_side.as_ref().map(|side| &side.catalog),
        &enemy_assembled,
        Some(&weapon_side.catalog),
        &weapon_assembled,
        clip_index.as_ref(),
        &weapon_inputs,
    );

    // Player ability effects.
    let (assets, probe) = match &resources_path {
        Some(path) => {
            let (assets, probe) = effect_assets(path)?;
            (Some(assets), Some(probe))
        }
        None => {
            skipped.push("no resources.assets: `ef<id>` effect assets not checked".into());
            (None, None)
        }
    };
    let effects = assets
        .as_ref()
        .map(|assets| build_effect_report(&effect_references(tables.as_ref()), assets));

    Ok(BattleModelCoverageReport {
        schema: "tantalus.battle-model-coverage.v1",
        models_elsewhere,
        sources: Sources {
            battle_bundle: file_name(battle_path),
            models_bundle: models_path.as_deref().map(file_name),
            clips_bundle: clips_path.as_deref().map(file_name),
            assembly: dll_path.as_deref().map(file_name),
            assembly_sha256: assembly_sha,
            resources_assets: resources_path.as_deref().map(file_name),
            reference: "Memoria c716e1fa",
            battle_animation_rows: battle_animation_count(),
            clip_count: clip_index.as_ref().map(ClipIndex::len),
        },
        scenes,
        players: Some(players),
        player_ability_sequences: PlayerAbilitySequences {
            answer: vec![
                "Player abilities are not `.raw17` sequences: `btl_vfx.SelectCommandVfx` calls `btlseq.RunSequence` only for enemy commands (EnemyAtk, EnemyCounter, EnemyDying, EnemyReaction).",
                "A player command picks a `SpecialEffect` id in `btl_vfx.GetPlayerCommandSFX`: `AA_DATA.Info.VfxIndex` or `Vfx2` (ability table rows, `FF9BattleDB.aa_data`), `ItemEffect.info.VfxIndex` for items, the character's `AttackSequence` (BattleParameters.csv) for Attack, and fixed ids for Steal, Throw and Trance.",
                "`SFX.SetEffectArgs` loads the TextAsset `SpecialEffects/ef<id:D3>` and hands the bytes to the native `SFX_Play` in `FF9SpecialEffectPlugin.dll`; the install keeps those TextAssets in `x64/FF9_Data/resources.assets` (names `ef000` to `ef510`, not in any `p0data*.bin` container).",
                "The effect format is the native plugin's; no C# reads it, so it is located and counted but not decoded.",
            ],
            effects,
            raw17_probe: probe,
            player_attack_sequences: player_attack_sequences(),
            skipped: skipped.clone(),
        },
        not_done: vec![
            "Background models are static meshes: their meshes and textures are decoded as objects, not assembled (the assembler only reads skinned prefabs).",
            "Texture animation tables (`TAM_B*.tab`) are checked for presence only, not parsed.",
            "Player models, animation names and `AttackSequence` come from the pinned Memoria `BattleParameters.csv` (the reference's defaults); the installed assembly's own player table (`ff9play`, `btl_mot`) is not evaluated.",
            "Weapons: every row of the installed weapon table is checked; which weapon a given equipped item selects is not walked.",
            "`Vfx2` is checked as a reference; the conditions that pick `Vfx2` over `VfxIndex` are not evaluated.",
            "Camera sequences and the effect plugin's own data are not decoded.",
        ],
    })
}

pub fn run_battle_model_coverage(
    battle_path: &Path,
    request: &BattleModelsRequest,
) -> Result<(), String> {
    let report = build_report(battle_path, request)?;
    let text = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    if let Some(parent) = request.out.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(&request.out, format!("{text}\n")).map_err(|error| error.to_string())?;
    let t = &report.scenes.totals;
    println!(
        "{} scenes ({} read, {} fully resolved); enemy models {}/{} distinct, enemy animations {}/{} distinct, sequence animations {}/{} distinct, backgrounds {}/{}; {} missing items; report written to {}",
        t.scenes,
        t.scenes_read,
        t.scenes_fully_resolved,
        t.enemy_models.distinct_resolved,
        t.enemy_models.distinct,
        t.enemy_animations.distinct_resolved,
        t.enemy_animations.distinct,
        t.sequence_animations.distinct_resolved,
        t.sequence_animations.distinct,
        t.backgrounds.distinct_resolved,
        t.backgrounds.distinct,
        report.scenes.missing.len(),
        request.out.display()
    );
    if let Some(players) = &report.players {
        let p = &players.totals;
        println!(
            "players {}: models {}/{}, animations {}/{} distinct; weapons {}/{} with a model",
            p.players,
            p.model_references_resolved,
            p.model_references,
            p.distinct_animations_resolved,
            p.distinct_animations,
            p.weapons_resolved,
            p.weapons_with_model
        );
    }
    if let Some(effects) = &report.player_ability_sequences.effects {
        let e = &effects.totals;
        println!(
            "effects: {} distinct referenced, {} present as ef<id> TextAssets, {} assets in resources.assets",
            e.distinct_effects, e.distinct_effects_present, e.assets_in_resources
        );
    }
    Ok(())
}
