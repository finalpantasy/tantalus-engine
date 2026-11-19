//! The battle view: a battle's [`BattleStage`] drawn over its battle
//! background, seen through the default battle camera, with the party and
//! enemy models at their battle positions, animated from the record
//! ([`crate::battle_motion`]), and the HUD ([`crate::battle_hud`]).
//!
//! [`FightView`] is the play window's: `PlayGame` fights the battle one
//! battle tick (15 a second) at a time while the pad drives the command
//! menu ([`crate::battle_menu`]). [`BattlePlayback`] replays a battle the
//! headless paths already fought. Assets (Memoria sources in each
//! function):
//! - background: `FF9BattleDB.MapModel["BSC_<scene>"]` in `p0data2.bin`
//!   (`HonoluluBattleMain.InitBattleScene`), drawn in the queue order of its
//!   `battlebg.SetDefaultShader` groups;
//! - models: `p0data4.bin` through `model_factory` (`ModelFactory.CreateModel`
//!   with `isBattle`, which hides the `field_model` child); players use their
//!   `BattleParameters.csv` model, enemies `MonAddr[TypeNo].Geo`;
//! - clips: `p0data5.bin`, the `BattleParameters.csv` motions for players
//!   and `mot[]` plus the attack sequences' animations for enemies
//!   (`btl_init.SetMonsterParameter`, `BattleUnit.cs:876`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tantalus_tools::play::pad;

use crate::bundle_store::with_store;
use bevy::math::{Quat, Vec3};
use tantalus_assets::pc::battle_background::{
    BattleBackground, BbgBlend, BbgGroup, assemble_background,
};
use tantalus_assets::pc::battle_models::{
    background_name, geo_id_by_name, player_battle_motions, player_battle_row,
    resolve_animation_id, resolve_animation_name,
};
use tantalus_assets::pc::field_models::model_dirs;
use tantalus_assets::pc::model_assembly::pose::{
    Interpolation, bind_clip, rest_pose, sample_clip, skin_model, transform_point,
};
use tantalus_assets::pc::model_assembly::{
    AssembleOptions, AssembledModel, ModelCatalog, assemble_model, clip_entries,
};
use tantalus_assets::pc::unity_object::animation::AnimationClip;
use tantalus_tools::battle_stage::{BATTLE_TICKS_PER_SECOND, BattleStage, StageFrame, StageModel};

use crate::battle_camera_play::CameraDirector;
use crate::battle_hud::{BattleHud, HudDraw, MenuDraw, PartyRow};
use crate::battle_motion::{ClipKey, Motions, UnitMotion, damage_numbers, unit_motion};
use crate::battle_raster::{BattleCamera, Blend, DrawState, Frame, Texture, ViewVertex};
use tantalus_sfx::player::Bone;

/// Container folder of the battle backgrounds.
const BACKGROUND_PREFIX: &str = "assets/resources/battlemap/battlemodel/battlemap_all/";
/// Unity `MeshTopology.Triangles`.
const TOPOLOGY_TRIANGLES: i32 = 0;
/// Seconds the last frame stays up after the record ends.
pub const HOLD_SECONDS: f64 = 2.0;
/// Clear color behind the background (`Battle Camera` clears to black).
const CLEAR: [u8; 4] = [0, 0, 0, 255];

/// `CharacterId` names.
const CHARACTERS: [&str; 12] = [
    "Zidane", "Vivi", "Dagger", "Steiner", "Freya", "Quina", "Eiko", "Amarant", "Cinna", "Marcus",
    "Blank", "Beatrix",
];

/// What one unit draws with.
#[derive(Debug, Clone, PartialEq, Eq)]
struct UnitLook {
    geo: u32,
    /// A party member's Trance model (`TranceModelId`), which
    /// `btl_vfx.SetTranceModel` swaps in; its clips are the same motions.
    trance_geo: Option<u32>,
    motions: Motions,
}

impl UnitLook {
    /// The model `unit` shows at `seconds`: its Trance model from frame 75
    /// of the `SysTrans` that enters Trance until the one that leaves.
    fn geo_at(&self, stage: &BattleStage, assets: &BattleAssets, unit: usize, seconds: f64) -> u32 {
        match self.trance_geo {
            Some(trance)
                if matches!(assets.models.get(&trance), Some(Some(_)))
                    && stage
                        .shows_trance_model(unit, seconds.max(0.0) * BATTLE_TICKS_PER_SECOND) =>
            {
                trance
            }
            _ => self.geo,
        }
    }
}

fn named_clip(name: &str) -> Option<ClipKey> {
    let clip = resolve_animation_name(name, None, None);
    clip.folder_geo_id.zip(clip.clip_key)
}

fn id_clip(id: u16) -> Option<ClipKey> {
    let clip = resolve_animation_id(u32::from(id), None);
    clip.folder_geo_id.zip(clip.clip_key)
}

fn unit_look(stage: &BattleStage, model: &StageModel) -> Option<UnitLook> {
    match model {
        StageModel::Player { serial, .. } => {
            let (_, model, _) = player_battle_row(u32::from(*serial))?;
            let motions = player_battle_motions(u32::from(*serial))?;
            Some(UnitLook {
                geo: geo_id_by_name(model)?,
                trance_geo: tantalus_assets::pc::battle_models::player_trance_model(u32::from(
                    *serial,
                ))
                .and_then(geo_id_by_name),
                motions: Motions::Player(
                    motions
                        .iter()
                        .map(|name| named_clip(name))
                        // Freya's jump clips (`SFX.BattleCallback` case 12,
                        // models 192 and 585: GEO_MAIN_B0_011 and its Trance).
                        .chain(
                            (model == "GEO_MAIN_B0_011")
                                .then_some(["110", "111", "112"])
                                .into_iter()
                                .flatten()
                                .map(|n| named_clip(&format!("ANH_MAIN_B0_011_{n}"))),
                        )
                        .collect(),
                ),
            })
        }
        StageModel::Enemy {
            geo,
            idle,
            hit,
            death,
        } => Some(UnitLook {
            geo: u32::try_from(*geo).ok()?,
            trance_geo: None,
            motions: Motions::Enemy {
                idle: id_clip(*idle),
                hit: id_clip(*hit),
                death: id_clip(*death),
                attacks: stage
                    .attack_anims
                    .iter()
                    .map(|(ids, moves)| {
                        (ids.iter().filter_map(|id| id_clip(*id)).collect(), *moves)
                    })
                    .collect(),
            },
        }),
        StageModel::None => None,
    }
}

/// Backgrounds, models and clips loaded so far.
pub struct BattleAssets {
    battle_bundle: PathBuf,
    models_bundle: PathBuf,
    clips_bundle: PathBuf,
    backgrounds: HashMap<String, Option<BattleBackground>>,
    /// `BBGINFO.chr_r/g/b` per background (lowercase name): the character
    /// color `btl_stat.SetStatusVfx` adds its glow to.
    chr: HashMap<String, [u8; 3]>,
    models: HashMap<u32, Option<AssembledModel>>,
    clips: HashMap<ClipKey, Option<AnimationClip>>,
    effects: crate::battle_effects::EffectLibrary,
    /// The blinking eyes and other texture animations of each loaded model (`GeoTexAnim`).
    tex_anims: std::sync::Mutex<HashMap<u32, tantalus_assets::pc::geo_tex_anim::TexAnimPlayer>>,
    /// `x64/FF9_Data`, where the status effects' textures are.
    game_data: PathBuf,
    /// The status effects drawn as SPS (`BattleSPSSystem`), read on first use.
    status_sps: std::sync::OnceLock<Vec<Option<tantalus_assets::pc::battle_status_sps::StatusSps>>>,
}

impl BattleAssets {
    /// Runs the models' texture animations to `seconds` into the replay (`GeoTexAnim.HonoUpdate`:
    /// `geoTexAnimService` every second update of a 60 a second battle).
    pub fn advance_tex_anims(&self, seconds: f64) {
        use tantalus_assets::pc::geo_tex_anim::{TexAnimPlayer, material_textures};
        let Ok(mut players) = self.tex_anims.lock() else {
            return;
        };
        let tick = (seconds.max(0.0) * 30.0) as u64;
        for (geo, model) in &self.models {
            let Some(Some(table)) = model.as_ref().map(|model| model.tex_anim.as_ref()) else {
                continue;
            };
            let Some(model) = model else {
                continue;
            };
            players
                .entry(*geo)
                .or_insert_with(|| TexAnimPlayer::new(table.clone()))
                .advance_to(tick, &material_textures(model));
        }
    }

    /// The status effects drawn as SPS, by `SPS.csv` id (`None` where the data is missing).
    pub fn status_sps(&self) -> &[Option<tantalus_assets::pc::battle_status_sps::StatusSps>] {
        self.status_sps.get_or_init(|| {
            tantalus_assets::pc::battle_status_sps::load_status_sps(
                &self.battle_bundle,
                &self.game_data,
            )
            .unwrap_or_else(|error| {
                println!("!! battle status effects: {error}");
                Vec::new()
            })
        })
    }

    /// The special effects' data and the effects playing.
    pub fn effects(&self) -> &crate::battle_effects::EffectLibrary {
        &self.effects
    }

    /// `streaming_assets` holds `p0data2.bin`, `p0data4.bin` and
    /// `p0data5.bin`.
    pub fn new(streaming_assets: &Path) -> Self {
        Self {
            battle_bundle: streaming_assets.join("p0data2.bin"),
            models_bundle: streaming_assets.join("p0data4.bin"),
            clips_bundle: streaming_assets.join("p0data5.bin"),
            backgrounds: HashMap::new(),
            chr: HashMap::new(),
            models: HashMap::new(),
            clips: HashMap::new(),
            tex_anims: std::sync::Mutex::new(HashMap::new()),
            game_data: streaming_assets
                .parent()
                .unwrap_or(streaming_assets)
                .join("x64/FF9_Data"),
            status_sps: std::sync::OnceLock::new(),
            effects: crate::battle_effects::EffectLibrary::new(
                streaming_assets
                    .parent()
                    .unwrap_or(streaming_assets)
                    .join("x64/FF9_Data/resources.assets"),
            ),
        }
    }

    pub fn background(&self, stage: &BattleStage) -> Option<&BattleBackground> {
        self.backgrounds
            .get(&background_name(&stage.scene)?.to_ascii_lowercase())?
            .as_ref()
    }

    /// `BBGINFO.chr_r/g/b` of `stage`'s background (mid gray when its
    /// `INB` is missing).
    pub fn chr(&self, stage: &BattleStage) -> [u8; 3] {
        background_name(&stage.scene)
            .and_then(|name| self.chr.get(&name.to_ascii_lowercase()))
            .copied()
            .unwrap_or([128; 3])
    }

    /// Loads what `stage` needs. One line per thing that failed.
    pub fn prepare(&mut self, stage: &BattleStage) -> Vec<String> {
        let mut problems = Vec::new();
        // The background, the models and the clips come from three
        // separate bundles, so they load side by side.
        let background_key = match background_name(&stage.scene) {
            None => {
                problems.push(format!("no MapModel row for BSC_{}", stage.scene));
                None
            }
            Some(name) => Some((name, name.to_ascii_lowercase())),
        }
        .filter(|(_, key)| !self.backgrounds.contains_key(key));
        let looks: Vec<UnitLook> = stage
            .units
            .iter()
            .filter_map(|unit| unit_look(stage, &unit.model))
            .collect();
        // Each member's Trance model too (`BTL_DATA.tranceGo` is built with
        // the battle model).
        let mut want_models: Vec<u32> = looks
            .iter()
            .flat_map(|look| std::iter::once(look.geo).chain(look.trance_geo))
            .filter(|geo| !self.models.contains_key(geo))
            .collect();
        want_models.sort_unstable();
        want_models.dedup();
        let mut want_clips: Vec<ClipKey> = looks
            .iter()
            .flat_map(|look| look.motions.all())
            .filter(|clip| !self.clips.contains_key(clip))
            .collect();
        want_clips.sort_unstable();
        want_clips.dedup();
        let (loaded_background, loaded_models, loaded_clips) = std::thread::scope(|scope| {
            let background = scope.spawn(|| {
                crate::live::bench::time("pl_bg", || {
                    background_key.as_ref().map(|(name, key)| {
                        with_store(&self.battle_bundle, |store| {
                            let entries = store.container().map_err(|error| error.to_string())?;
                            let path = format!("{BACKGROUND_PREFIX}{key}/{key}.prefab");
                            let root = entries
                                .iter()
                                .find(|entry| entry.class_id == 1 && entry.path == path)
                                .or_else(|| {
                                    entries.iter().find(|entry| {
                                        entry.class_id == 1
                                            && entry.path.starts_with(&format!(
                                                "{BACKGROUND_PREFIX}{key}/{key}"
                                            ))
                                    })
                                })
                                .ok_or_else(|| format!("{path} is not in the bundle"))?;
                            let background = assemble_background(store, name, root.path_id)
                                .map_err(|error| error.to_string())?;
                            // `BattleMap/BattleInfo/INB_Bnnn.inb`.
                            let info = format!(
                                "assets/resources/battlemap/battleinfo/{}.inb.bytes",
                                key.replacen("bbg", "inb", 1)
                            );
                            let chr = entries
                                .iter()
                                .find(|entry| entry.path == info)
                                .and_then(|entry| store.text_asset(entry.path_id).ok().flatten())
                                .and_then(|bytes| {
                                    tantalus_assets::pc::battle_scene::BattleInfo::parse(
                                        &bytes, &info,
                                    )
                                    .ok()
                                })
                                .map(|info| [info.chr_r, info.chr_g, info.chr_b]);
                            Ok((background, chr))
                        })
                    })
                })
            });
            let models = scope.spawn(|| {
                crate::live::bench::time("pl_models", || {
                    (!want_models.is_empty()).then(|| {
                        with_store(&self.models_bundle, |store| {
                            let entries = store.container().map_err(|error| error.to_string())?;
                            let catalog = ModelCatalog::from_container(&entries);
                            Ok(want_models
                                .iter()
                                .map(|geo| {
                                    let model = model_dirs(*geo, &catalog)
                                        .into_iter()
                                        .find_map(|dir| {
                                            assemble_model(
                                                store,
                                                &dir.label,
                                                *dir.roots.first()?,
                                                AssembleOptions {
                                                    decode_textures: true,
                                                },
                                            )
                                            .ok()
                                        })
                                        .map(|mut model| {
                                            model.tex_anim =
                                                tantalus_assets::pc::geo_tex_anim::load(
                                                    store, &entries, *geo,
                                                );
                                            model
                                        });
                                    (*geo, model)
                                })
                                .collect::<Vec<_>>())
                        })
                    })
                })
            });
            let clips = scope.spawn(|| {
                crate::live::bench::time("pl_clips", || {
                    (!want_clips.is_empty()).then(|| {
                        with_store(&self.clips_bundle, |store| {
                            let entries = store.container().map_err(|error| error.to_string())?;
                            let (clips, _) = clip_entries(&entries);
                            let by_name: HashMap<(u32, &str), i64> = clips
                                .iter()
                                .map(|clip| ((clip.geo_id, clip.name.as_str()), clip.path_id))
                                .collect();
                            Ok(want_clips
                                .iter()
                                .map(|&(folder, key)| {
                                    let clip = by_name
                                        .get(&(folder, key.to_string().as_str()))
                                        .and_then(|path_id| store.clip(*path_id).ok().flatten());
                                    ((folder, key), clip)
                                })
                                .collect::<Vec<_>>())
                        })
                    })
                })
            });
            (
                background.join().ok().flatten(),
                models.join().ok().flatten(),
                clips.join().ok().flatten(),
            )
        });
        if let (Some((name, key)), Some(loaded)) = (background_key, loaded_background) {
            match loaded {
                Ok((background, chr)) => {
                    if let Some(chr) = chr {
                        self.chr.insert(key.clone(), chr);
                    }
                    problems.extend(
                        background
                            .problems
                            .iter()
                            .take(3)
                            .map(|problem| format!("{name}: {problem}")),
                    );
                    self.backgrounds.insert(key, Some(background));
                }
                Err(error) => {
                    problems.push(format!("{name}: {error}"));
                    self.backgrounds.insert(key, None);
                }
            }
        }
        if let Some(loaded) = loaded_models {
            match loaded {
                Ok(loaded) => {
                    for (geo, model) in &loaded {
                        if model.is_none() {
                            problems.push(format!("model {geo} did not load"));
                        }
                    }
                    self.models.extend(loaded);
                }
                Err(error) => {
                    problems.push(format!("models: {error}"));
                    self.models
                        .extend(want_models.iter().map(|geo| (*geo, None)));
                }
            }
        }
        if let Some(loaded) = loaded_clips {
            match loaded {
                Ok(loaded) => self.clips.extend(loaded),
                Err(error) => {
                    problems.push(format!("clips: {error}"));
                    self.clips
                        .extend(want_clips.iter().map(|clip| (*clip, None)));
                }
            }
        }
        problems
    }
}

/// The camera `stage` opens with: `PatAddr[pattern].Camera`, where 3 and
/// up are random in the game and camera 0 here.
pub fn stage_camera(stage: &BattleStage) -> BattleCamera {
    BattleCamera::preset(if stage.camera < 3 { stage.camera } else { 0 })
}

/// Draws the background groups in `groups`, in their render queue order.
fn draw_background(
    frame: &mut Frame,
    camera: &BattleCamera,
    background: &BattleBackground,
    additive_pass: bool,
) {
    // Queue order: Sky (Background), Minus (AlphaTest), Ground
    // (AlphaTest+1, `_ZWrite` 0), Plus (AlphaTest+3); `_Abr_1` materials
    // (no depth write) after the opaque pass.
    for group in [
        BbgGroup::Sky,
        BbgGroup::Minus,
        BbgGroup::Ground,
        BbgGroup::Plus,
    ] {
        for mesh in background.meshes.iter().filter(|mesh| mesh.group == group) {
            let view: Vec<Vec3> = mesh
                .positions
                .iter()
                .map(|position| camera.to_view(transform_point(&mesh.world, *position)))
                .collect();
            for submesh in &mesh.submeshes {
                if submesh.topology != TOPOLOGY_TRIANGLES {
                    continue;
                }
                let (blend, depth_write) = match submesh.blend {
                    BbgBlend::Opaque => (
                        Blend::Opaque,
                        group == BbgGroup::Plus || group == BbgGroup::Minus,
                    ),
                    BbgBlend::Additive => (Blend::Screen, false),
                    BbgBlend::Multiply => (Blend::Multiply, false),
                };
                if (blend != Blend::Opaque) != additive_pass {
                    continue;
                }
                let state = DrawState {
                    blend,
                    depth_test: group != BbgGroup::Sky,
                    depth_write,
                    cull_back: true,
                };
                let texture = submesh
                    .texture
                    .and_then(|index| background.textures.get(index))
                    .map(|texture| Texture {
                        width: texture.width,
                        height: texture.height,
                        rgba: &texture.rgba,
                        clamp: true,
                    });
                let first = submesh.first_index as usize;
                let end = first + submesh.index_count as usize;
                let Some(indices) = mesh.indices.get(first..end) else {
                    continue;
                };
                for corner in indices.chunks_exact(3) {
                    let vertex = |at: u32| {
                        Some(ViewVertex {
                            position: *view.get(at as usize)?,
                            uv: mesh.uvs.get(at as usize).copied().unwrap_or_default(),
                        })
                    };
                    if let (Some(a), Some(b), Some(c)) =
                        (vertex(corner[0]), vertex(corner[1]), vertex(corner[2]))
                    {
                        frame.draw(camera, [a, b, c], texture, state);
                    }
                }
            }
        }
    }
}

/// `PSXTextureMgr.CaptureBG`: the battle ground (the first ground mesh)
/// seen from the camera `params` (host command 120, `bgParam`) describe,
/// into a 256 x 256 picture (`bgTexture`). The camera is `worldToCamera =
/// TRS((-p4, p5, -(10496 + p6)), Euler(-90, 0, 0))`, so a point at depth `y +
/// 10496 + p6` lands at `512 * (x - p4, z + p5) / depth`, in a window with
/// its left edge at `-(p2 / 2) - p0 % 64` and its top at `p3 / 2 + p1 % 256`
/// (`PerspectiveOffCenter(l, l + 256, t - 256, t, 512, 65536)`). Rows run
/// top down; pixels the ground does not cover stay clear (alpha 0).
pub fn capture_ground(background: &BattleBackground, params: [i32; 7]) -> Frame {
    // The window, in 320 x 240 projection pixels (`PSXTextureMgr.CaptureBG`).
    let left = -((params[2] >> 1) as f32) - (params[0] % 64) as f32;
    let top = (params[3] >> 1) as f32 + (params[1] % 256) as f32;
    let depth = 10496.0 + params[6] as f32;
    let mut frame = Frame::new(256, 256, [7, 7, 7, 0]);
    let Some(mesh) = background
        .meshes
        .iter()
        .find(|mesh| mesh.group == BbgGroup::Ground)
    else {
        return Frame::new(256, 256, [0, 0, 0, 0]);
    };
    // `Frame::project` puts a view point at `160 + near * x / d` and `120 -
    // near * y / d` of 320 x 220, then scales to the frame: shear the view
    // so that it lands where the capture's window says instead.
    let (kx, ky) = (256.0 / 320.0, 256.0 / 220.0);
    let camera = BattleCamera {
        view: bevy::math::Mat4::IDENTITY,
        near: 512.0,
    };
    let state = DrawState {
        blend: Blend::Opaque,
        depth_test: true,
        depth_write: true,
        cull_back: false,
    };
    let view: Vec<Vec3> = mesh
        .positions
        .iter()
        .map(|position| {
            let world = transform_point(&mesh.world, *position);
            let d = world[1] + depth;
            let (x, y) = (world[0] - params[4] as f32, world[2] + params[5] as f32);
            Vec3::new(
                x / kx - ((left / kx + 160.0) / 512.0) * d,
                (120.0 - top / ky) * d / 512.0 + y / ky,
                -d,
            )
        })
        .collect();
    for submesh in &mesh.submeshes {
        if submesh.topology != TOPOLOGY_TRIANGLES {
            continue;
        }
        let texture = submesh
            .texture
            .and_then(|index| background.textures.get(index))
            .map(|texture| Texture {
                width: texture.width,
                height: texture.height,
                rgba: &texture.rgba,
                clamp: true,
            });
        let first = submesh.first_index as usize;
        let end = first + submesh.index_count as usize;
        let Some(indices) = mesh.indices.get(first..end) else {
            continue;
        };
        for corner in indices.chunks_exact(3) {
            let vertex = |at: u32| {
                Some(ViewVertex {
                    position: *view.get(at as usize)?,
                    uv: mesh.uvs.get(at as usize).copied().unwrap_or_default(),
                })
            };
            if let (Some(a), Some(b), Some(c)) =
                (vertex(corner[0]), vertex(corner[1]), vertex(corner[2]))
            {
                frame.draw(&camera, [a, b, c], texture, state);
            }
        }
    }
    frame
}

/// `ModelFactory.CreateModel(isBattle)` turns off every renderer under the
/// `field_model` child.
/// The model's `mesh<N>` object a node is under (`BTL_DATA.SetIsEnabledMeshRenderer`).
fn mesh_of(model: &AssembledModel, node: usize) -> Option<u32> {
    let mut at = Some(node);
    while let Some(index) = at {
        let node = model.nodes.get(index)?;
        if let Some(number) = node
            .name
            .strip_prefix("mesh")
            .and_then(|number| number.parse::<u32>().ok())
        {
            return Some(number);
        }
        at = node.parent;
    }
    None
}

fn under_field_model(model: &AssembledModel, node: usize) -> bool {
    let mut at = Some(node);
    while let Some(index) = at {
        let Some(node) = model.nodes.get(index) else {
            return false;
        };
        if node.name == "field_model" {
            return true;
        }
        at = node.parent;
    }
    false
}

/// How an effect changes the way a unit is drawn (the plugin's host
/// commands, `tantalus_sfx::player::UnitChange`).
#[derive(Clone, Copy)]
struct Staging<'a> {
    /// False to pose the unit (its height, bones and place) without
    /// drawing it.
    draw: bool,
    change: Option<&'a tantalus_sfx::player::UnitChange>,
    /// The background's character color (`BBGINFO.chr_r/g/b`), which a fade adds to.
    chr: [u8; 3],
    /// `meshflags`: bit `i` hides the model's `mesh<i>` subtree
    /// (`geo.geoMeshHide`).
    hidden_meshes: u16,
}

impl Staging<'_> {
    const PLAIN: Staging<'static> = Staging {
        draw: true,
        change: None,
        chr: [128; 3],
        hidden_meshes: 0,
    };
}

/// Draws one unit's model posed as `motion` says. Returns the height of
/// its highest vertex above `position` (for the cursor and numbers).
#[allow(clippy::too_many_arguments)]
fn draw_unit(
    frame: &mut Frame,
    camera: &BattleCamera,
    assets: &BattleAssets,
    (geo, bone): (u32, Option<u8>),
    motion: &UnitMotion,
    (position, turn): ([f32; 3], f32),
    glow: Option<[u8; 3]>,
    staging: Staging<'_>,
) -> Option<(f32, Option<Vec3>, Vec<Option<Bone>>)> {
    let Some(Some(model)) = assets.models.get(&geo) else {
        return None;
    };
    let clip = motion
        .clip
        .and_then(|clip| assets.clips.get(&clip)?.as_ref());
    let poses = match clip {
        Some(clip) => {
            let length = clip_length(clip);
            let time = if length <= 0.0 {
                0.0
            } else if motion.looping {
                motion.time.rem_euclid(length)
            } else {
                motion.time.clamp(0.0, length)
            };
            sample_clip(
                model,
                clip,
                &bind_clip(model, clip),
                time,
                Interpolation::Hermite,
            )
        }
        None => rest_pose(model),
    };
    // `Euler(0, turn, 180)`: the model's y-down space turned upright, then
    // about the up axis. An effect that set the angles (`Set Angles`) gives
    // its own Euler angles.
    let rotation = match staging.change.and_then(|change| change.euler) {
        Some([x, y, z]) => Quat::from_euler(
            bevy::math::EulerRot::YXZ,
            y.to_radians(),
            x.to_radians(),
            z.to_radians(),
        ),
        None => {
            Quat::from_rotation_y(turn.to_radians()) * Quat::from_rotation_z(std::f32::consts::PI)
        }
    };
    // An effect that set the position (`Set Position`, PSX space y down)
    // puts the model there, the unit's own motion aside.
    let origin = match staging.change.and_then(|change| change.position) {
        Some([x, y, z]) => Vec3::new(f32::from(x), -f32::from(y), f32::from(z)),
        None => Vec3::from_array(position) + Vec3::from_array(motion.offset),
    };
    // `localScale` (`Set Size`).
    let scale = staging
        .change
        .and_then(|change| change.scale)
        .map_or(Vec3::ONE, Vec3::from_array);
    let world = tantalus_assets::pc::model_assembly::pose::world_matrices(&model.nodes, &poses);
    // `GetChildByName("bone" + tar_bone)`: that bone's place in the pose.
    let bone_at = bone.and_then(|bone| {
        let name = format!("bone{bone:03}");
        let index = model.nodes.iter().position(|node| node.name == name)?;
        // Row-major: the translation is the last column.
        let m = world.get(index)?;
        let local = Vec3::new(m[3], m[7], m[11]);
        Some(origin + rotation * (scale * local))
    });
    // Every `bone<NNN>` object's world matrix, for the effects' bone queries
    // (`SFXData.BattleCallback` case 14: `localToWorldMatrix`).
    let turn_matrix = bevy::math::Mat3::from_quat(rotation);
    let mut bones: Vec<Option<Bone>> = Vec::new();
    for (node, m) in model.nodes.iter().zip(&world) {
        let Some(number) = node
            .name
            .strip_prefix("bone")
            .and_then(|number| number.parse::<usize>().ok())
        else {
            continue;
        };
        let local = bevy::math::Mat3::from_cols_array(&[
            m[0], m[4], m[8], m[1], m[5], m[9], m[2], m[6], m[10],
        ]);
        let turned = turn_matrix * local;
        let place = origin + rotation * (scale * Vec3::new(m[3], m[7], m[11]));
        let column = |c: usize| turned.col(c).to_array();
        let (c0, c1, c2) = (column(0), column(1), column(2));
        if bones.len() <= number {
            bones.resize(number + 1, None);
        }
        bones[number] = Some(Bone::from_world([
            [c0[0], c1[0], c2[0], place.x],
            [c0[1], c1[1], c2[1], place.y],
            [c0[2], c1[2], c2[2], place.z],
            [0.0, 0.0, 0.0, 1.0],
        ]));
    }
    // `Show/Hide mesh` (25): the semi-transparent fade darkens the model
    // (`GeoAddColor2DrawPacket`) and, low enough, adds it to the scene.
    // `btl_stat.GeoAddColor2DrawPacket(go, fade - 128, ..)`: the background's character color
    // plus the fade above 128 (each channel at least 0, at most 255); with every channel 0 the
    // renderers are disabled, so the unit is not drawn.
    let fade = staging.change.and_then(|change| change.fade);
    let glow = match fade {
        Some((value, _)) => Some(
            staging
                .chr
                .map(|chr| (i32::from(chr) + (value - 128).max(0)).clamp(0, 255) as u8),
        ),
        None => glow,
    };
    let faded_out = fade.is_some() && glow == Some([0; 3]);
    let mut top = 0.0f32;
    let players = assets.tex_anims.lock().ok();
    let player = players.as_ref().and_then(|players| players.get(&geo));
    // The glowing model's textures, tinted once per texture.
    let mut tinted: HashMap<usize, Vec<u8>> = HashMap::new();
    if let Some(color) = glow {
        for texture in model
            .parts
            .iter()
            .flat_map(|part| &part.submeshes)
            .filter_map(|submesh| submesh.material.as_ref())
            .flat_map(|material| &material.textures)
        {
            if let Some(rgba) = texture.rgba.as_deref() {
                tinted
                    .entry(rgba.as_ptr() as usize)
                    .or_insert_with(|| crate::trance_glow::tint(rgba, color));
            }
        }
    }
    let state = DrawState {
        blend: if fade.is_some_and(|(_, translucent)| translucent) {
            Blend::Add
        } else {
            Blend::Opaque
        },
        depth_test: true,
        depth_write: true,
        cull_back: true,
    };
    for (part_index, (part, vertices)) in model
        .parts
        .iter()
        .zip(skin_model(model, &poses))
        .enumerate()
    {
        if under_field_model(model, part.node) {
            continue;
        }
        let view: Vec<Vec3> = vertices
            .iter()
            .map(|vertex| {
                let world = origin + rotation * (scale * Vec3::from_array(*vertex));
                top = top.max(world.y - origin.y);
                camera.to_view(world.to_array())
            })
            .collect();
        if !staging.draw || faded_out {
            continue;
        }
        if staging.hidden_meshes != 0
            && mesh_of(model, part.node)
                .is_some_and(|mesh| mesh < 16 && staging.hidden_meshes & (1 << mesh) != 0)
        {
            continue;
        }
        for (submesh_index, submesh) in part.submeshes.iter().enumerate() {
            if submesh.topology != TOPOLOGY_TRIANGLES {
                continue;
            }
            let mut texture = submesh.material.as_ref().and_then(|material| {
                material.textures.iter().find_map(|texture| {
                    let rgba = texture.rgba.as_deref()?;
                    let width = usize::try_from(texture.width).ok()?;
                    let height = usize::try_from(texture.height).ok()?;
                    let rgba = match glow {
                        Some(_) => tinted.get(&(rgba.as_ptr() as usize))?.as_slice(),
                        None => rgba,
                    };
                    (width > 0 && height > 0 && rgba.len() >= width * height * 4).then_some(
                        Texture {
                            width,
                            height,
                            rgba,
                            clamp: false,
                        },
                    )
                })
            });
            // `GeoTexAnim`: the renderer's own material (its first) shows the redrawn texture.
            if let (0, Some(texture), Some(canvas)) = (
                submesh_index,
                texture.as_mut(),
                player.and_then(|player| player.canvas(part_index)),
            ) && glow.is_none()
                && canvas.len() >= texture.width * texture.height * 4
            {
                texture.rgba = canvas;
            }
            let first = submesh.first_index as usize;
            let end = first + submesh.index_count as usize;
            let Some(indices) = part.indices.get(first..end) else {
                continue;
            };
            for corner in indices.chunks_exact(3) {
                let vertex = |at: u32| {
                    Some(ViewVertex {
                        position: *view.get(at as usize)?,
                        uv: part.uvs.get(at as usize).copied().unwrap_or_default(),
                    })
                };
                if let (Some(a), Some(b), Some(c)) =
                    (vertex(corner[0]), vertex(corner[1]), vertex(corner[2]))
                {
                    frame.draw(camera, [a, b, c], texture, state);
                }
            }
        }
    }
    Some((top, bone_at, bones))
}

/// The last key time over every curve.
fn clip_length(clip: &AnimationClip) -> f32 {
    let rotation = clip
        .rotation_curves
        .iter()
        .filter_map(|c| c.keys.last().map(|k| k.time));
    let position = clip
        .position_curves
        .iter()
        .filter_map(|c| c.keys.last().map(|k| k.time));
    let scale = clip
        .scale_curves
        .iter()
        .filter_map(|c| c.keys.last().map(|k| k.time));
    rotation.chain(position).chain(scale).fold(0.0, f32::max)
}

/// A unit's name: its battle-text name, or [`unit_name`].
fn display_name(unit: &tantalus_tools::battle_stage::StageUnit, enemy: usize) -> String {
    unit.name
        .clone()
        .unwrap_or_else(|| unit_name(&unit.model, enemy))
}

/// A unit's readout name.
fn unit_name(model: &StageModel, enemy: usize) -> String {
    match model {
        StageModel::Player { character, .. } => CHARACTERS
            .get(usize::from(*character))
            .map_or_else(|| format!("Player {character}"), |name| (*name).to_owned()),
        _ => format!("Enemy {}", enemy + 1),
    }
}

/// The background drawn once for the fixed camera (color and depth after
/// the opaque pass), and the scene's scripted cameras when it has them.
pub struct BattleScene {
    pub camera: BattleCamera,
    base: Frame,
    /// The scripted cameras ([`CameraDirector`]); `None` keeps `camera`.
    director: std::sync::Mutex<Option<CameraDirector>>,
    /// Model height of each unit (camera anchor heights).
    heights: Vec<f32>,
    looks: Vec<Option<UnitLook>>,
    /// How many units have a model drawn.
    pub models: usize,
    pub background_triangles: usize,
}

/// What the HUD adds over a live battle.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Overlay {
    pub menu: Option<MenuDraw>,
    /// Party member whose menu is open.
    pub asked: Option<usize>,
    /// Units under the target cursor.
    pub target: Vec<usize>,
    pub banner: Option<String>,
    /// The result screen ([`results_lines`]).
    pub results: Option<(String, Vec<crate::battle_hud::ListLine>)>,
}

impl BattleScene {
    pub fn new(assets: &BattleAssets, stage: &BattleStage, width: u32, height: u32) -> Self {
        let camera = stage_camera(stage);
        let mut base = Frame::for_view(width, height, CLEAR);
        let mut background_triangles = 0;
        if let Some(background) = assets.background(stage) {
            draw_background(&mut base, &camera, background, false);
            background_triangles = background.triangle_count();
        }
        let looks: Vec<Option<UnitLook>> = stage
            .units
            .iter()
            .map(|unit| unit_look(stage, &unit.model))
            .collect();
        let models = looks
            .iter()
            .flatten()
            .filter(|look| matches!(assets.models.get(&look.geo), Some(Some(_))))
            .count();
        // Each model's height at the start of its idle, for the camera's
        // anchor points.
        let length = |clip: ClipKey| {
            assets
                .clips
                .get(&clip)
                .and_then(Option::as_ref)
                .map_or(0.0, clip_length)
        };
        let heights: Vec<f32> = stage
            .units
            .iter()
            .enumerate()
            .map(|(index, unit)| {
                let Some(Some(look)) = looks.get(index) else {
                    return 0.0;
                };
                let motion = unit_motion(stage, index, 0.0, &look.motions, &length);
                let mut probe = Frame::new(1, 1, CLEAR);
                draw_unit(
                    &mut probe,
                    &camera,
                    assets,
                    (look.geo, None),
                    &motion,
                    (unit.position, unit.turn),
                    None,
                    Staging::PLAIN,
                )
                .map_or(0.0, |(top, _, _)| top)
            })
            .collect();
        let director = CameraDirector::new(stage, &heights, Some(&assets.effects));
        Self {
            camera,
            base,
            director: std::sync::Mutex::new(director),
            heights,
            looks,
            models,
            background_triangles,
        }
    }

    /// Battle ticks the opening camera plays before the battle starts (0 without a camera
    /// script). See [`CameraDirector::opening_ticks`].
    pub fn opening_ticks(&self) -> usize {
        let director = self
            .director
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        director
            .as_ref()
            .and_then(|director| director.opening_ticks(OPENING_LIMIT))
            .unwrap_or(0)
    }

    /// Shows the battle through the fixed camera only.
    pub fn fix_camera(&mut self) {
        self.director = std::sync::Mutex::new(None);
    }

    /// The camera at `seconds` into `stage`.
    pub fn camera_at(
        &self,
        assets: &BattleAssets,
        stage: &BattleStage,
        seconds: f64,
    ) -> BattleCamera {
        let tick = (seconds.max(0.0) * BATTLE_TICKS_PER_SECOND) as usize;
        let mut director = self
            .director
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(running) = director.as_mut() else {
            return self.camera;
        };
        if tick + 1 < running.ticks() {
            *running = match CameraDirector::new(stage, &self.heights, Some(&assets.effects)) {
                Some(fresh) => fresh,
                None => return self.camera,
            };
        }
        let length = |clip: ClipKey| {
            assets
                .clips
                .get(&clip)
                .and_then(Option::as_ref)
                .map_or(0.0, clip_length)
        };
        // Where each unit stands and faces at a tick (steps forward, runs,
        // turns, jumps).
        let offsets = |tick: usize| -> Vec<([f32; 3], Option<f32>)> {
            let seconds = tick as f64 / BATTLE_TICKS_PER_SECOND;
            self.looks
                .iter()
                .enumerate()
                .map(|(index, look)| match look {
                    Some(look) => {
                        let motion = unit_motion(stage, index, seconds, &look.motions, &length);
                        (motion.offset, motion.facing)
                    }
                    None => ([0.0; 3], None),
                })
                .collect()
        };
        running
            .camera_at(stage, tick, &offsets, Some(&assets.effects))
            .unwrap_or(self.camera)
    }

    /// The frame `seconds` into the replay, with the plain readout.
    pub fn render(&self, assets: &BattleAssets, stage: &BattleStage, seconds: f64) -> Frame {
        self.render_with(assets, stage, seconds, None, &Overlay::default())
    }

    /// The frame `seconds` into `stage`, with the game-font HUD when `hud`
    /// loaded (the plain readout otherwise).
    pub fn render_with(
        &self,
        assets: &BattleAssets,
        stage: &BattleStage,
        seconds: f64,
        hud: Option<&BattleHud>,
        overlay: &Overlay,
    ) -> Frame {
        use crate::live::bench::time;
        assets.advance_tex_anims(seconds);
        let camera = self.camera_at(assets, stage, seconds);
        let mut frame = time("b_base", || {
            if camera == self.camera {
                self.base.clone()
            } else {
                let mut frame = Frame::for_view(self.base.width, self.base.height, CLEAR);
                if let Some(background) = assets.background(stage) {
                    draw_background(&mut frame, &camera, background, false);
                }
                frame
            }
        });
        let record = stage.frame_at(seconds);
        let length = |clip: ClipKey| {
            assets
                .clips
                .get(&clip)
                .and_then(Option::as_ref)
                .map_or(0.0, clip_length)
        };
        let effects = crate::battle_effects::stage_effects(stage);
        time("b_dim", || {
            crate::battle_effects::dim(
                &mut frame,
                crate::battle_effects::scene_intensity(&effects, seconds),
            )
        });
        // Where each unit's head is on screen, and where it stands. The
        // closure poses every unit (and draws it when asked), as the
        // effects playing have staged it.
        let place_units = |frame: &mut Frame,
                           draw: bool,
                           staged: &std::collections::BTreeMap<
            usize,
            tantalus_sfx::player::UnitChange,
        >| {
            let mut heads: Vec<Option<[f32; 2]>> = vec![None; stage.units.len()];
            let mut spots: Vec<Option<crate::battle_effects::UnitSpot>> =
                vec![None; stage.units.len()];
            for (index, unit) in stage.units.iter().enumerate() {
                let present =
                    record.is_none_or(|record| record.present.get(index).copied().unwrap_or(true));
                let Some(Some(look)) = self.looks.get(index) else {
                    continue;
                };
                if !present {
                    continue;
                }
                let mut motion = unit_motion(stage, index, seconds, &look.motions, &length);
                let change = staged.get(&index);
                // A party member plays the motions its effect sets (`SFX.BattleCallback`
                // case 12: the plugin steps forward, chants and casts, as the game does).
                // Jump and Spear: the jumper shows while Jump plays (she leaves at the
                // command's end, `JumpStatusScript.OnFinishCommand`) and once Spear
                // brings her back (`Set Hidden` off, its frame 58) to fall; Spear sets
                // her animation before that, while she is still away.
                let effect_motion = change
                    .and_then(|change| change.motion.as_ref())
                    .filter(|_| matches!(look.motions, crate::battle_motion::Motions::Player(_)));
                let leaping = effect_motion.is_some_and(|motion| motion.effect == 393);
                if motion.hidden && !leaping && !change.is_some_and(|change| change.appeared) {
                    continue;
                }
                if let Some(set) = effect_motion {
                    motion = crate::battle_motion::UnitMotion {
                        clip: crate::battle_motion::effect_clip(
                            &look.motions,
                            set.effect,
                            usize::from(set.index),
                        ),
                        time: set.frames() as f32
                            / tantalus_tools::battle_stage::BATTLE_TICKS_PER_SECOND as f32,
                        looping: true,
                        hidden: false,
                        ..motion
                    };
                }
                // `btl_stat.SetStatusVfx`: a living member in Trance pulses.
                let glow = match (&unit.model, record) {
                    (StageModel::Player { serial, .. }, Some(record))
                        if std::env::var_os("TANTALUS_NO_GLOW").is_none()
                            && matches!(record.trance.get(index), Some(Some((_, true))))
                            && record.hp.get(index).is_some_and(|hp| *hp > 0) =>
                    {
                        tantalus_assets::pc::battle_models::player_trance_glow(u32::from(*serial))
                            .map(|glow| {
                                crate::trance_glow::glow_color(
                                    assets.chr(stage),
                                    glow,
                                    crate::trance_glow::battle_counter(seconds),
                                )
                            })
                    }
                    _ => None,
                };
                // Vanish (`HideMesh(mesh_banish, true)`): the unit's `Mesh[1]` mask hides some
                // of its meshes; the full mask of a party member hides it all. The weapon
                // stays, which the view does not draw apart from the model.
                let banish = stage.banish_meshes.get(index).copied().unwrap_or(u16::MAX);
                let vanished = change.is_some_and(|change| change.vanished);
                let hidden = change.is_some_and(|change| {
                    change.disappear != 0
                        || change.meshes_hidden
                        || (change.vanished && banish == u16::MAX)
                });
                if let Some((top, bone, bones)) = time("b_unit", || {
                    draw_unit(
                        frame,
                        &camera,
                        assets,
                        (
                            look.geo_at(stage, assets, index, seconds),
                            stage.tar_bones.get(index).copied().flatten(),
                        ),
                        &motion,
                        (unit.position, motion.facing.unwrap_or(unit.turn)),
                        glow,
                        Staging {
                            draw: draw && !hidden,
                            change,
                            chr: assets.chr(stage),
                            hidden_meshes: if vanished { banish } else { 0 },
                        },
                    )
                }) {
                    let feet = match change.and_then(|change| change.position) {
                        Some([x, y, z]) => Vec3::new(f32::from(x), -f32::from(y), f32::from(z)),
                        None => Vec3::from_array(unit.position) + Vec3::from_array(motion.offset),
                    };
                    // An enemy's numbers and cursor sit on its `tar_bone`.
                    let at = bone.unwrap_or(feet + Vec3::Y * top);
                    heads[index] = frame.project(&camera, camera.to_view(at.to_array()));
                    spots[index] = Some(crate::battle_effects::UnitSpot {
                        feet: feet.to_array(),
                        height: top,
                        turn: motion.facing.unwrap_or(unit.turn),
                        bones: std::sync::Arc::new(bones),
                    });
                }
            }
            (heads, spots)
        };
        // Effects see the units where they stand, then change how they are
        // drawn (summons hide the party, Mini shrinks, Jump moves).
        let staged = if crate::battle_effects::any_playing(&effects, seconds) {
            let (_, spots) = place_units(&mut frame, false, &Default::default());
            time("b_effects", || {
                crate::battle_effects::advance_effects(
                    &camera,
                    &assets.effects,
                    stage,
                    &effects,
                    &spots,
                    assets.background(stage),
                    seconds,
                )
            })
        } else {
            Default::default()
        };
        let (heads, spots) = place_units(&mut frame, true, &staged);
        time("b_front", || {
            if let Some(background) = assets.background(stage) {
                draw_background(&mut frame, &camera, background, true);
            }
        });
        // `btl2d.StatusUpdateVisuals`: each afflicted unit's status SPS at its icon position.
        if let Some(record) = record
            && std::env::var_os("TANTALUS_NO_STATUS_SPS").is_none()
        {
            let afflicted: Vec<crate::battle_status::Afflicted<'_>> = spots
                .iter()
                .enumerate()
                .filter_map(|(index, spot)| {
                    let spot = spot.as_ref()?;
                    let status = record.status.get(index).copied().unwrap_or(0);
                    (status != 0).then_some(())?;
                    Some(crate::battle_status::Afflicted {
                        status,
                        icons: stage.status_icons.get(index).copied().flatten()?,
                        bones: &spot.bones,
                        turn: spot.turn,
                    })
                })
                .collect();
            if !afflicted.is_empty() {
                let tick = (seconds.max(0.0) * BATTLE_TICKS_PER_SECOND) as u64;
                time("b_status", || {
                    crate::battle_status::draw(
                        &mut frame,
                        &camera,
                        assets.status_sps(),
                        &afflicted,
                        tick,
                    );
                });
            }
        }
        time("b_effects", || {
            crate::battle_effects::show_effects(&mut frame, &assets.effects, &effects, seconds)
        });
        match hud {
            Some(hud) => {
                time("b_hud", || {
                    let draw = hud_draw(stage, &effects, seconds, overlay, &heads);
                    hud.draw(&mut frame, &draw);
                });
            }
            None => readout(&mut frame, stage, record, seconds),
        }
        frame
    }
}

/// The HUD contents at `seconds`. `heads` are units' head points on the
/// frame (`Frame::project`).
fn hud_draw(
    stage: &BattleStage,
    effects: &[crate::battle_effects::StageEffect],
    seconds: f64,
    overlay: &Overlay,
    heads: &[Option<[f32; 2]>],
) -> HudDraw {
    let record = stage.frame_at(seconds);
    let players = player_count(stage);
    let hp = |index: usize| record.and_then(|r| r.hp.get(index).copied());
    let party = stage
        .units
        .iter()
        .enumerate()
        .take(players)
        .map(|(index, unit)| {
            let atb =
                record
                    .and_then(|r| r.atb.get(index).copied())
                    .map_or(0.0, |(current, max)| {
                        if max > 0 {
                            f32::from(current.max(0)) / f32::from(max)
                        } else {
                            0.0
                        }
                    });
            PartyRow {
                name: unit_name(&unit.model, 0),
                hp: (hp(index).unwrap_or(unit.max_hp), unit.max_hp),
                mp: (
                    record
                        .and_then(|r| r.mp.get(index).copied())
                        .unwrap_or(unit.max_mp),
                    unit.max_mp,
                ),
                atb,
                trance: record
                    .and_then(|r| r.trance.get(index).copied().flatten())
                    .map(|(gauge, active)| (f32::from(gauge) / 255.0, active)),
                asked: overlay.asked == Some(index),
            }
        })
        .collect();
    let enemies = stage
        .units
        .iter()
        .enumerate()
        .skip(players)
        .filter(|(index, unit)| {
            !(matches!(unit.model, StageModel::None) && unit.max_hp == 0)
                && hp(*index).unwrap_or(1) > 0
                && record.is_none_or(|r| r.present.get(*index).copied().unwrap_or(true))
        })
        .map(|(index, unit)| display_name(unit, index - players))
        .collect();
    let numbers = damage_numbers(stage, effects, seconds)
        .into_iter()
        .filter_map(|(target, amount, since)| {
            Some((heads.get(target).copied().flatten()?, amount, since))
        })
        .collect();
    let banner = overlay
        .banner
        .clone()
        .filter(|_| overlay.results.is_none())
        .or_else(|| {
            if overlay.results.is_some() {
                return None;
            }
            (seconds >= stage.duration() && !stage.frames.is_empty())
                .then(|| stage.outcome.clone())
                .flatten()
                .map(|outcome| match outcome.as_str() {
                    "victory" => "Victory".to_owned(),
                    "defeat" => "Defeat".to_owned(),
                    other => other.to_owned(),
                })
        });
    HudDraw {
        party,
        enemies,
        menu: overlay.menu.clone(),
        target: overlay
            .target
            .iter()
            .filter_map(|&target| heads.get(target).copied().flatten())
            .collect(),
        numbers,
        banner,
        results: overlay.results.clone(),
    }
}

fn player_count(stage: &BattleStage) -> usize {
    stage
        .units
        .iter()
        .take_while(|unit| matches!(unit.model, StageModel::Player { .. }))
        .count()
}

/// The HP/ATB text: enemies top left, party bottom, battle line on top.
fn readout(frame: &mut Frame, stage: &BattleStage, record: Option<&StageFrame>, seconds: f64) {
    let scale = (frame.height as i32 / 240).max(1);
    let line = 9 * scale;
    let white = [240, 240, 240];
    let mut header = format!(
        "Battle {} {} t {:.1}s",
        stage.battle,
        stage.scene,
        seconds.min(stage.duration())
    );
    if seconds >= stage.duration() {
        header.push_str(&format!(
            "  {}",
            stage.outcome.as_deref().unwrap_or("no result")
        ));
    }
    frame.rect(0, 0, frame.width as i32, line + 2 * scale, [0, 0, 0], 0.5);
    frame.text(2 * scale, scale, &header, scale, white);
    let players = player_count(stage);
    let hp = |index: usize| record.and_then(|r| r.hp.get(index).copied());
    let atb = |index: usize| record.and_then(|r| r.atb.get(index).copied());
    let mut enemy_row = 0;
    for (index, unit) in stage.units.iter().enumerate().skip(players) {
        if matches!(unit.model, StageModel::None) && unit.max_hp == 0 {
            continue;
        }
        let y = line * (2 + enemy_row);
        let text = format!(
            "{} HP {}/{}",
            display_name(unit, index - players),
            hp(index).unwrap_or(unit.max_hp),
            unit.max_hp
        );
        frame.rect(
            0,
            y - scale,
            (text.len() as i32 + 1) * 6 * scale,
            line,
            [0, 0, 0],
            0.4,
        );
        frame.text(2 * scale, y, &text, scale, [255, 210, 200]);
        enemy_row += 1;
    }
    let rows = players as i32;
    let top = frame.height as i32 - line * rows - 2 * scale;
    frame.rect(
        0,
        top - scale,
        frame.width as i32,
        line * rows + 3 * scale,
        [0, 0, 30],
        0.6,
    );
    for (index, unit) in stage.units.iter().enumerate().take(players) {
        let y = top + line * index as i32;
        let now = hp(index).unwrap_or(unit.max_hp);
        let color = if now == 0 {
            [255, 90, 90]
        } else if now * 4 <= unit.max_hp {
            [255, 220, 80]
        } else {
            white
        };
        let mp = record
            .and_then(|r| r.mp.get(index).copied())
            .unwrap_or(unit.max_mp);
        let text = format!(
            "{:<8} HP {:>4}/{:<4} MP {:>3}/{:<3}",
            unit_name(&unit.model, 0),
            now,
            unit.max_hp,
            mp,
            unit.max_mp
        );
        let width = frame.text(4 * scale, y, &text, scale, color);
        // ATB gauge.
        let bar_x = 4 * scale + width + 4 * scale;
        let bar_width = (frame.width as i32 - bar_x - 4 * scale).clamp(0, 60 * scale);
        if let Some((current, max)) = atb(index)
            && max > 0
            && bar_width > 0
        {
            let filled = bar_width * i32::from(current.clamp(0, max)) / i32::from(max);
            frame.rect(bar_x, y, bar_width, 6 * scale, [40, 40, 60], 1.0);
            let ready = current >= max;
            let fill = if ready {
                [255, 230, 90]
            } else {
                [90, 180, 255]
            };
            frame.rect(bar_x, y, filled, 6 * scale, fill, 1.0);
        }
        // Trance gauge under the ATB (`BattleHUD` `TranceSlider`): pink,
        // full and bright in Trance.
        if let Some(Some((trance, active))) = record.and_then(|r| r.trance.get(index).copied())
            && bar_width > 0
        {
            let filled = bar_width * i32::from(trance) / 255;
            let fill = if active {
                [255, 150, 230]
            } else {
                [220, 90, 170]
            };
            frame.rect(
                bar_x,
                y + 7 * scale,
                bar_width,
                2 * scale,
                [40, 20, 40],
                1.0,
            );
            frame.rect(bar_x, y + 7 * scale, filled, 2 * scale, fill, 1.0);
        }
    }
}

/// A replay in progress.
pub struct BattlePlayback {
    pub stage: BattleStage,
    pub scene: BattleScene,
    pub seconds: f64,
}

impl BattlePlayback {
    pub fn new(assets: &BattleAssets, stage: BattleStage, width: u32, height: u32) -> Self {
        let scene = BattleScene::new(assets, &stage, width, height);
        Self {
            stage,
            scene,
            seconds: 0.0,
        }
    }

    /// Moves the replay on by `delta` seconds (`speed` times faster);
    /// true once it has ended and held its last frame.
    pub fn advance(&mut self, delta: f64, speed: f64) -> bool {
        self.seconds += delta * speed;
        self.seconds >= self.stage.duration() + HOLD_SECONDS
    }

    pub fn render(&self, assets: &BattleAssets) -> Frame {
        self.scene.render(assets, &self.stage, self.seconds)
    }
}

/// `frame` as a Bevy image (sRGB RGBA). A GPU frame's image starts black;
/// [`show_frame`] draws into it. Either can be drawn into through a plain
/// RGBA8 view ([`crate::frame_gpu`]).
pub fn frame_image(frame: &Frame) -> bevy::image::Image {
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
    let data = if frame.recording().is_some() {
        vec![0; (frame.width * frame.height * 4) as usize]
    } else {
        frame.rgba.clone()
    };
    let mut image = bevy::image::Image::new(
        Extent3d {
            width: frame.width,
            height: frame.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::default(),
    );
    image.texture_descriptor.usage |= TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC;
    image.texture_descriptor.view_formats = &[TextureFormat::Rgba8Unorm];
    image
}

/// Puts `frame` in `image` (made by [`frame_image`]): a GPU frame is drawn
/// on the next render, a CPU frame's pixels are copied in.
pub fn show_frame(
    world: &mut bevy::ecs::world::World,
    image: &bevy::asset::Handle<bevy::image::Image>,
    frame: Frame,
) {
    use bevy::prelude::*;
    if frame.recording().is_some() {
        if let Some(recording) = frame.into_recording() {
            crate::frame_gpu::submit(world, image, recording);
        }
    } else if let Some(mut target) = world.resource_mut::<Assets<Image>>().get_mut(image) {
        target.data = Some(frame.rgba);
    }
}

/// The frame's pixels: a GPU frame drawn on the headless device.
pub fn frame_pixels(frame: &Frame) -> Result<std::borrow::Cow<'_, [u8]>, String> {
    match frame.recording() {
        Some(recording) => crate::frame_gpu::read_back(recording).map(std::borrow::Cow::Owned),
        None => Ok(std::borrow::Cow::Borrowed(&frame.rgba)),
    }
}

/// Saves `frame` as a PNG.
pub fn save_png(frame: &Frame, output: &Path) -> Result<(), String> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    let pixels = frame_pixels(frame)?.into_owned();
    let mut image = frame_image(frame);
    image.data = Some(pixels);
    image
        .try_into_dynamic()
        .map_err(|error| format!("cannot convert the frame: {error}"))?
        .to_rgba8()
        .save(output)
        .map_err(|error| format!("cannot write {}: {error}", output.display()))
}

/// Render layer of the battle view's camera and sprite, apart from the
/// field's.
pub const BATTLE_LAYER: usize = 7;
/// Size of the CPU battle frame in the window.
/// The classic (4:3) size; see [`crate::widescreen::live_size`] for the one in use.
pub const LIVE_SIZE: (u32, u32) = (640, 480);
/// Replay speed while confirm is held.
pub const FAST_FORWARD: f64 = 4.0;

/// The battle's place in the play window: its own camera (drawn after the
/// field's, clearing it) showing one sprite with the CPU frame.
pub struct BattleWindow {
    image: bevy::asset::Handle<bevy::image::Image>,
    camera: bevy::ecs::entity::Entity,
    sprite: bevy::ecs::entity::Entity,
}

impl BattleWindow {
    pub fn open(world: &mut bevy::ecs::world::World, frame: &Frame) -> Self {
        use bevy::camera::visibility::RenderLayers;
        use bevy::camera::{RenderTarget, ScalingMode};
        use bevy::prelude::*;
        let image = world
            .resource_mut::<Assets<Image>>()
            .add(frame_image(frame));
        if frame.recording().is_some() {
            show_frame(world, &image, frame.clone());
        }
        // Draw where the field camera draws (the window, or an offscreen
        // image in a capture).
        let mut targets = world.query_filtered::<&RenderTarget, With<Camera2d>>();
        let target = targets.iter(world).next().cloned();
        let (width, height) = (
            crate::widescreen::FRAME_HEIGHT * frame.width as f32 / frame.height as f32,
            crate::widescreen::FRAME_HEIGHT,
        );
        let camera = world
            .spawn((
                Camera2d,
                Camera {
                    order: 10,
                    clear_color: ClearColorConfig::Custom(Color::BLACK),
                    ..default()
                },
                Projection::Orthographic(OrthographicProjection {
                    scaling_mode: ScalingMode::AutoMin {
                        min_width: width,
                        min_height: height,
                    },
                    ..OrthographicProjection::default_2d()
                }),
                RenderLayers::layer(BATTLE_LAYER),
            ))
            .id();
        if let Some(target) = target {
            world.entity_mut(camera).insert(target);
        }
        crate::window::fit_frame_camera(world, camera, (frame.width, frame.height));
        let sprite = world
            .spawn((
                Sprite {
                    image: image.clone(),
                    custom_size: Some(Vec2::new(width, height)),
                    ..default()
                },
                Transform::default(),
                RenderLayers::layer(BATTLE_LAYER),
            ))
            .id();
        Self {
            image,
            camera,
            sprite,
        }
    }

    /// Shows `frame`.
    pub fn show(&self, world: &mut bevy::ecs::world::World, frame: Frame) {
        use bevy::prelude::*;
        crate::window::fit_frame_camera(world, self.camera, (frame.width, frame.height));
        show_frame(world, &self.image, frame);
        // Other code moves every 2D camera; keep the sprite in front of ours.
        let at = world
            .get::<Transform>(self.camera)
            .map(|transform| transform.translation.truncate());
        if let (Some(at), Some(mut transform)) = (at, world.get_mut::<Transform>(self.sprite)) {
            transform.translation.x = at.x;
            transform.translation.y = at.y;
        }
    }

    pub fn close(self, world: &mut bevy::ecs::world::World) {
        for entity in [self.sprite, self.camera] {
            if let Ok(entity) = world.get_entity_mut(entity) {
                entity.despawn();
            }
        }
        world
            .resource_mut::<bevy::asset::Assets<bevy::image::Image>>()
            .remove(&self.image);
    }
}

/// A recorded battle replayed in the play window.
pub struct LiveBattle {
    pub playback: BattlePlayback,
    window: BattleWindow,
}

impl LiveBattle {
    /// Loads what `stage` needs and opens the view.
    pub fn start(
        world: &mut bevy::ecs::world::World,
        assets: &mut BattleAssets,
        stage: BattleStage,
    ) -> Self {
        for problem in assets.prepare(&stage) {
            println!("!! battle assets: {problem}");
        }
        crate::widescreen::latch_live_size();
        let size = crate::widescreen::live_size();
        let playback = BattlePlayback::new(assets, stage, size.0, size.1);
        let window = BattleWindow::open(world, &playback.render(assets));
        Self { playback, window }
    }

    /// Moves the replay on and redraws; true once it is over. Holding
    /// confirm plays it `FAST_FORWARD` times faster.
    pub fn tick(
        &mut self,
        world: &mut bevy::ecs::world::World,
        assets: &BattleAssets,
        delta: f64,
        held: i32,
    ) -> bool {
        let speed = if held & tantalus_tools::play::pad::CONFIRM != 0 {
            FAST_FORWARD
        } else {
            1.0
        };
        let done = self.playback.advance(delta, speed);
        self.window.show(world, self.playback.render(assets));
        done
    }

    /// Closes the view.
    pub fn finish(self, world: &mut bevy::ecs::world::World) {
        self.window.close(world);
    }

    /// The window title line.
    pub fn title(&self) -> String {
        let stage = &self.playback.stage;
        format!(
            "Tantalus Engine: battle {} {} ({:.0}/{:.0}s, hold confirm to speed up)",
            stage.battle,
            stage.scene,
            self.playback.seconds.min(stage.duration()),
            stage.duration()
        )
    }
}

/// Seconds per battle tick.
const TICK_SECONDS: f64 = 1.0 / tantalus_tools::battle_stage::BATTLE_TICKS_PER_SECOND;

/// A battle fought in real time: `PlayGame` fights it one battle tick per
/// fifteenth of a second while the pad drives the command menu.
pub struct FightView {
    pub scene: BattleScene,
    pub menu: crate::battle_menu::BattleMenu,
    /// A chosen command waiting for the next battle tick.
    pending: Option<tantalus_game::battle::BattleCommand>,
    /// Seconds since the last battle tick.
    since_tick: f64,
    /// Pad bits held last frame.
    previous: i32,
    /// The finished battle and seconds since, shown with its result
    /// screen until Confirm (at least `HOLD_SECONDS`).
    ended: Option<(BattleStage, f64)>,
    /// What the battle paid, from [`tantalus_tools::play::PlayGame::last_results`].
    pub results: Option<tantalus_tools::play::BattleResults>,
    /// Seconds L1 and R1 have been held (`BattleHUD._runCounter`).
    run_held: f64,
    /// The result screen's page: 0 the EXP and AP, 1 the gil and items
    /// (`BattleResultUI.DisplayEXPAndAPInfo`, `DisplayGilAndItemInfo`).
    result_page: usize,
    window: Option<BattleWindow>,
    pub battle: i32,
    pub scene_name: String,
    /// Stage frames the opening camera plays first: until then the battle is in
    /// `PHASE_ENTER` (no menu, no ATB). Without it a command started during the opening and
    /// its camera took the half-run opening track, which looked at nothing.
    opening: usize,
}

/// The longest opening camera waited for, in battle ticks.
const OPENING_LIMIT: usize = 900;

impl FightView {
    /// Opens the view of the battle `game` is fighting. `window` is false
    /// for a capture that only renders frames.
    pub fn start(
        world: Option<&mut bevy::ecs::world::World>,
        assets: &mut BattleAssets,
        hud: Option<&BattleHud>,
        game: &tantalus_tools::play::PlayGame,
    ) -> Option<Self> {
        let fight = game.battle()?;
        let stage = fight.stage();
        for problem in assets.prepare(stage) {
            println!("!! battle assets: {problem}");
        }
        crate::widescreen::latch_live_size();
        let size = crate::widescreen::live_size();
        let scene = BattleScene::new(assets, stage, size.0, size.1);
        let opening = scene.opening_ticks();
        let mut view = Self {
            opening,
            scene,
            menu: crate::battle_menu::BattleMenu::default(),
            pending: None,
            since_tick: 0.0,
            previous: 0,
            ended: None,
            results: None,
            run_held: 0.0,
            result_page: 0,
            window: None,
            battle: stage.battle,
            scene_name: stage.scene.clone(),
        };
        if let Some(world) = world {
            let frame = view.render(assets, hud, game);
            view.window = Some(BattleWindow::open(world, &frame));
        }
        Some(view)
    }

    /// Shows the battle through default camera `camera` instead of the
    /// pattern's.
    pub fn set_camera(
        &mut self,
        assets: &BattleAssets,
        game: &tantalus_tools::play::PlayGame,
        camera: u8,
    ) {
        if let Some(fight) = game.battle() {
            let mut stage = fight.stage().clone();
            stage.camera = camera;
            let size = crate::widescreen::live_size();
            self.scene = BattleScene::new(assets, &stage, size.0, size.1);
            self.scene.fix_camera();
        }
    }

    /// Battle seconds shown now.
    fn seconds(&self, stage: &BattleStage) -> f64 {
        stage.frames.len().saturating_sub(1) as f64 * TICK_SECONDS + self.since_tick
    }

    /// One frame: pad input to the menu, battle ticks for `delta` seconds.
    /// Returns the play events of the ticks and whether the view is done
    /// (the battle over and its result shown).
    pub fn update(
        &mut self,
        game: &mut tantalus_tools::play::PlayGame,
        delta: f64,
        held: i32,
    ) -> (Vec<tantalus_tools::play::PlayEvent>, bool) {
        let pressed = held & !self.previous;
        self.previous = held;
        let mut events = Vec::new();
        if let Some((_, shown)) = &mut self.ended {
            *shown += delta;
            // The result screen waits for Confirm; an escape or other end
            // closes by itself.
            let waits = self
                .results
                .as_ref()
                .is_some_and(|results| matches!(results.outcome.as_str(), "victory" | "defeat"));
            let confirmed = *shown >= HOLD_SECONDS && pressed & pad::CONFIRM != 0;
            let victory = self
                .results
                .as_ref()
                .is_some_and(|results| results.outcome == "victory");
            // Confirm on the EXP and AP page turns to the gil and items page
            // (`BattleResultUI` `EndEXPAndAP` -> `StartGilAndItem`).
            if confirmed && victory && self.result_page == 0 {
                self.result_page = 1;
                return (events, false);
            }
            let done = *shown >= HOLD_SECONDS && (!waits || confirmed);
            if done
                && self
                    .results
                    .as_ref()
                    .is_some_and(|results| results.outcome == "defeat")
            {
                // Game over: back to the saved games
                // (`FF9StateSystem` game over -> title -> Continue).
                game.open_load_screen();
            }
            if done {
                // `BattleResultUI.Hide` -> `battle.ff9ShutdownStateBattleResult`.
                game.battle_result_closed();
            }
            return (events, done);
        }
        let (remember, wait) = game.settings().map_or((false, false), |settings| {
            (settings.cursor != 0, settings.atb == 1)
        });
        self.menu.remember = remember;
        let opening = game
            .battle()
            .is_some_and(|fight| fight.stage().frames.len() < self.opening);
        if let Some(fight) = game.battle()
            && self.pending.is_none()
            && !opening
        {
            let before = menu_snapshot(&self.menu);
            self.pending = self.menu.update(fight, pressed);
            let after = menu_snapshot(&self.menu);
            let sent = self.pending.is_some();
            game.battle_menu_sound(before, after, pressed, sent);
        }
        let stalled = opening || (wait && self.menu.wait_stalls_atb());
        if let Some(fight) = game.battle_mut() {
            fight.set_atb_wait_stall(stalled);
        }
        // Holding L1 and R1 runs: a flee roll each second held
        // (`BattleHUD.Update`, `btl_sys.CheckEscape(true)`).
        if held & pad::L1 != 0 && held & pad::R1 != 0 {
            self.run_held += delta;
            if self.run_held > 1.0 {
                self.run_held = 0.0;
                if let Some(fight) = game.battle_mut() {
                    fight.try_flee();
                }
            }
        } else {
            self.run_held = 0.0;
        }
        self.since_tick += delta;
        while self.since_tick >= TICK_SECONDS && self.ended.is_none() {
            self.since_tick -= TICK_SECONDS;
            events.extend(game.battle_tick(self.pending.take()));
            if game.battle().is_none() {
                let stage = game.last_battle.take().unwrap_or_default();
                self.results = game.last_results.take();
                self.ended = Some((stage, 0.0));
            }
        }
        (events, false)
    }

    /// The frame now.
    pub fn render(
        &self,
        assets: &BattleAssets,
        hud: Option<&BattleHud>,
        game: &tantalus_tools::play::PlayGame,
    ) -> Frame {
        let item_name = |item| game.item_name(item).map(str::to_owned);
        if let Some((stage, shown)) = &self.ended {
            let seconds = stage.duration() + shown;
            // The outcome banner first, then the result screen.
            let overlay = Overlay {
                results: self
                    .results
                    .as_ref()
                    .filter(|_| *shown >= 1.0)
                    .map(|results| {
                        let world = game.world();
                        // (total EXP, EXP still to the next level) after the battle
                        // (`BattleResultUI.DisplayCharacterInfo`).
                        let progress = |character: i32, level: usize| {
                            let exp = world.character_exp.get(&character).copied().unwrap_or(0);
                            let next = world
                                .level_experience
                                .as_ref()
                                .and_then(|table| table.get(level).copied())
                                .map(|to_level| to_level.saturating_sub(exp));
                            (exp, next)
                        };
                        results_lines(
                            results,
                            ResultPage {
                                page: self.result_page,
                                gil_now: world.gil,
                                progress: &progress,
                            },
                            &|character| tantalus_tools::play::character_name(character).to_owned(),
                            &item_name,
                        )
                    }),
                ..Overlay::default()
            };
            return self
                .scene
                .render_with(assets, stage, seconds, hud, &overlay);
        }
        let Some(fight) = game.battle() else {
            let size = crate::widescreen::live_size();
            return Frame::for_view(size.0, size.1, CLEAR);
        };
        let stage = fight.stage();
        let seconds = self.seconds(stage);
        // `PHASE_ENTER`: the opening camera plays without the command menu.
        let mut overlay = if stage.frames.len() < self.opening {
            Overlay::default()
        } else {
            menu_overlay(fight, &self.menu, &item_name)
        };
        overlay.banner = steal_message(stage, seconds, &item_name);
        self.scene
            .render_with(assets, stage, seconds, hud, &overlay)
    }

    /// Rebuilds the scene and the window at the window's new aspect when it
    /// changed since the battle began (`FieldMap.OnWidescreenSupportChanged`
    /// follows the screen size live).
    pub fn resize_if_stale(
        &mut self,
        world: &mut bevy::ecs::world::World,
        assets: &BattleAssets,
        hud: Option<&BattleHud>,
        game: &tantalus_tools::play::PlayGame,
    ) {
        if !crate::widescreen::live_size_stale() {
            return;
        }
        let Some(fight) = game.battle() else {
            return;
        };
        crate::widescreen::latch_live_size();
        let size = crate::widescreen::live_size();
        self.scene = BattleScene::new(assets, fight.stage(), size.0, size.1);
        if let Some(old) = self.window.take() {
            old.close(world);
            let frame = self.render(assets, hud, game);
            self.window = Some(BattleWindow::open(world, &frame));
        }
    }

    /// Draws the frame into the window.
    pub fn show(
        &self,
        world: &mut bevy::ecs::world::World,
        assets: &BattleAssets,
        hud: Option<&BattleHud>,
        game: &tantalus_tools::play::PlayGame,
    ) {
        if let Some(window) = &self.window {
            let frame = crate::live::bench::time("b_render", || self.render(assets, hud, game));
            crate::live::bench::time("b_upload", || window.show(world, frame));
        }
    }

    /// Closes the view.
    pub fn finish(self, world: &mut bevy::ecs::world::World) {
        if let Some(window) = self.window {
            window.close(world);
        }
    }

    /// The window title line.
    pub fn title(&self) -> String {
        format!(
            "Tantalus Engine: battle {} {} (arrows, Enter, Backspace)",
            self.battle, self.scene_name
        )
    }
}

/// The menu as the HUD draws it.
/// The command menu's place, for [`tantalus_tools::battle_sound::menu_cue`].
fn menu_snapshot(
    menu: &crate::battle_menu::BattleMenu,
) -> tantalus_tools::battle_sound::MenuSnapshot {
    let (depth, cursor) = match menu.level {
        crate::battle_menu::MenuLevel::Commands => (0, menu.command),
        crate::battle_menu::MenuLevel::List { .. } => (1, menu.list),
        crate::battle_menu::MenuLevel::Target { .. } => (2, menu.target),
    };
    tantalus_tools::battle_sound::MenuSnapshot {
        unit: menu.unit,
        depth,
        cursor,
    }
}

pub fn menu_overlay(
    fight: &tantalus_tools::field_battle::LiveFight,
    menu: &crate::battle_menu::BattleMenu,
    item_name: &dyn Fn(i32) -> Option<String>,
) -> Overlay {
    use crate::battle_hud::ListLine;
    use crate::battle_menu::{MenuLevel, list_of, row_enabled, usable};
    use tantalus_tools::field_battle::CommandKind;
    let Some(unit) = menu.unit else {
        return Overlay::default();
    };
    let Some(entries) = fight.menu(unit) else {
        return Overlay::default();
    };
    let line = |entry: &tantalus_tools::field_battle::MenuEntry| {
        let (text, right) = match entry.item {
            Some(item) => (
                item_name(item).unwrap_or_else(|| format!("Item {item}")),
                fight.item_count(item).to_string(),
            ),
            None => (
                entry.label.clone(),
                if entry.mp > 0 {
                    entry.mp.to_string()
                } else {
                    String::new()
                },
            ),
        };
        ListLine {
            text,
            right,
            enabled: usable(fight, unit, entry),
        }
    };
    let row = |index: usize, text: String| ListLine {
        text,
        right: String::new(),
        enabled: row_enabled(entries, index),
    };
    let title = |index: usize| {
        entries
            .commands
            .get(index)
            .map(|command| command.title.clone())
            .unwrap_or_default()
    };
    let commands = vec![
        row(0, entries.attack.label.clone()),
        row(1, title(0)),
        row(2, title(1)),
        row(3, "Item".into()),
        row(4, entries.defend.label.clone()),
        row(5, entries.change.label.clone()),
    ];
    let open_list = |kind| {
        (
            list_of(entries, kind).iter().map(line).collect::<Vec<_>>(),
            menu.list,
        )
    };
    let targeted = menu.targeted(fight);
    let (list, targeting, help) = match &menu.level {
        MenuLevel::Commands => {
            // The help line names a direct command's effect.
            let help = match menu.command {
                1 | 2 => match entries.commands.get(menu.command - 1).map(|c| &c.kind) {
                    Some(CommandKind::Unavailable) => Some("Not available yet".to_owned()),
                    _ => None,
                },
                _ => None,
            };
            (None, false, help)
        }
        MenuLevel::List(kind) => {
            let list = open_list(*kind);
            let help = list.0.get(list.1).map(|line| line.text.clone());
            (Some(list), false, help)
        }
        MenuLevel::Target { back, all, .. } => {
            let list = match back.as_ref() {
                MenuLevel::List(kind) => Some(open_list(*kind)),
                _ => None,
            };
            let stage = fight.stage();
            let players = player_count(stage);
            let help = if *all && targeted.len() > 1 {
                Some(if targeted.iter().all(|&target| target >= players) {
                    "All enemies".to_owned()
                } else {
                    "All allies".to_owned()
                })
            } else {
                targeted.first().and_then(|&target| {
                    stage
                        .units
                        .get(target)
                        .map(|unit| display_name(unit, target.saturating_sub(players)))
                })
            };
            (list, true, help)
        }
    };
    Overlay {
        menu: Some(MenuDraw {
            commands,
            command_cursor: menu.command,
            list,
            targeting,
            help,
        }),
        asked: Some(unit),
        target: targeted,
        banner: None,
        results: None,
    }
}

/// The latest Steal or Eat message within two seconds of `seconds`
/// (`BattleMesages.Stole` / `CouldNotStealAnything`).
pub fn steal_message(
    stage: &BattleStage,
    seconds: f64,
    item_name: &dyn Fn(i32) -> Option<String>,
) -> Option<String> {
    use tantalus_tools::battle_stage::StageEvent;
    let now = (seconds.max(0.0) * BATTLE_TICKS_PER_SECOND) as usize;
    let first = now.saturating_sub((2.0 * BATTLE_TICKS_PER_SECOND) as usize);
    stage
        .frames
        .get(first..=now.min(stage.frames.len().saturating_sub(1)))?
        .iter()
        .rev()
        .flat_map(|frame| frame.events.iter().rev())
        .find_map(|event| match event {
            StageEvent::Stole { item, .. } => Some(match item {
                Some(item) => format!(
                    "Stole {}!",
                    item_name(*item as i32).unwrap_or_else(|| format!("item {item}"))
                ),
                None => "Couldn't steal anything.".to_owned(),
            }),
            StageEvent::Ate { result, .. } => Some(eat_message(*result).to_owned()),
            _ => None,
        })
}

/// The banner for an Eat or Cook result ([`StageEvent::Ate`]'s `result`).
pub fn eat_message(result: u8) -> &'static str {
    match result {
        0 => "Can't eat that.",
        1 => "Too strong to eat!",
        2 => "Tastes bad!",
        _ => "Learned a new ability!",
    }
}

/// The result screen state [`results_lines`] draws from.
pub struct ResultPage<'a> {
    /// 0: EXP and AP; 1: gil and items.
    pub page: usize,
    /// The party's gil after the battle.
    pub gil_now: u32,
    /// `(character, level)` -> (total EXP, EXP to the next level, `None` at the top level).
    pub progress: &'a dyn Fn(i32, usize) -> (u64, Option<u64>),
}

/// The result screen's info line and lines for `results`, one page of
/// `BattleResultUI`: page 0 (`DisplayEXPAndAPInfo`, `DisplayCharacterInfo`)
/// the EXP and AP won, then each member's level, total EXP and EXP to the
/// next level with "Level Up!" under a member who rose; page 1
/// (`DisplayGilAndItemInfo`) the gil won, the party's gil, the items won and
/// stolen and a card, or "No item received.". The captions are the English
/// `Localization` rows (`BattleResultInfoEXPAP`, `BattleResultInfoGilItem`,
/// `EXP`, `NoItemGet`). A defeat is the game over screen. Lines with
/// `enabled` false are drawn yellow.
pub fn results_lines(
    results: &tantalus_tools::play::BattleResults,
    page: ResultPage<'_>,
    name: &dyn Fn(i32) -> String,
    item_name: &dyn Fn(i32) -> Option<String>,
) -> (String, Vec<crate::battle_hud::ListLine>) {
    use crate::battle_hud::ListLine;
    let plain = |text: String, right: String| ListLine {
        text,
        right,
        enabled: true,
    };
    let item = |id: u32| item_name(id as i32).unwrap_or_else(|| format!("Item {id}"));
    match results.outcome.as_str() {
        "victory" if page.page == 0 => {
            let exp = results.exp.iter().map(|e| e.1).max().unwrap_or(0);
            let mut lines = vec![
                plain("EXP".into(), exp.to_string()),
                plain("AP".into(), results.ap.to_string()),
            ];
            for &(character, _, from, to) in &results.exp {
                let (total, next) = (page.progress)(character, to);
                let next = next.map_or_else(|| "-".to_owned(), |n| n.to_string());
                lines.push(plain(
                    format!("{}  Lv {to}", name(character)),
                    format!("EXP {total}  Next Lv {next}"),
                ));
                if to > from {
                    lines.push(ListLine {
                        text: "  Level Up!".into(),
                        right: String::new(),
                        enabled: false,
                    });
                }
            }
            ("Received EXP and AP!".to_owned(), lines)
        }
        "victory" => {
            let mut lines = vec![
                plain("Gil".into(), results.gil.to_string()),
                plain("Current Gil".into(), page.gil_now.to_string()),
            ];
            for &(id, count) in &results.items {
                lines.push(plain(item(id), count.to_string()));
            }
            for &id in &results.stolen {
                lines.push(plain(format!("{} (stolen)", item(id)), "1".into()));
            }
            if let Some(card) = results.card {
                lines.push(plain(format!("Card {card}"), "1".into()));
            }
            if lines.len() == 2 {
                lines.push(plain("No item received.".into(), String::new()));
            }
            ("Received Item(s) and Gil".to_owned(), lines)
        }
        "defeat" => (
            "GAME OVER".to_owned(),
            vec![plain("Confirm: load a saved game".into(), String::new())],
        ),
        "escape" | "party_flee" => ("Escaped".to_owned(), Vec::new()),
        other => (other.to_owned(), Vec::new()),
    }
}

/// One play-window frame of a replay, when one is open: advances it,
/// closes it at its end. True while the view is (still) up, so the caller
/// leaves the field paused.
pub fn run_live_battle(
    world: &mut bevy::ecs::world::World,
    battle: &mut Option<LiveBattle>,
    assets: &BattleAssets,
    delta: f64,
    held: i32,
) -> bool {
    let Some(mut open) = battle.take() else {
        return false;
    };
    if open.tick(world, assets, delta, held) {
        open.finish(world);
        return false;
    }
    set_title(world, open.title());
    *battle = Some(open);
    true
}

/// Sets the window title.
pub fn set_title(world: &mut bevy::ecs::world::World, title: String) {
    use bevy::prelude::*;
    use bevy::window::PrimaryWindow;
    let mut windows = world.query_filtered::<&mut Window, With<PrimaryWindow>>();
    if let Ok(mut window) = windows.single_mut(world)
        && window.title != title
    {
        window.title = title;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn results_lines_show_level_ups_and_game_over() {
        let results = tantalus_tools::play::BattleResults {
            outcome: "victory".into(),
            gil: 30,
            ap: 2,
            exp: vec![(0, 50, 2, 3), (1, 50, 2, 2)],
            items: vec![(236, 1)],
            stolen: vec![88],
            card: None,
        };
        let progress = |character: i32, level: usize| (100 + character as u64, Some(level as u64));
        let page = |page| ResultPage {
            page,
            gil_now: 500,
            progress: &progress,
        };
        let name = |id| format!("C{id}");
        let item = |item| Some(format!("I{item}"));
        let (title, lines) = results_lines(&results, page(0), &name, &item);
        assert_eq!(title, "Received EXP and AP!");
        let rows: Vec<(&str, &str)> = lines
            .iter()
            .map(|line| (line.text.as_str(), line.right.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                ("EXP", "50"),
                ("AP", "2"),
                ("C0  Lv 3", "EXP 100  Next Lv 3"),
                ("  Level Up!", ""),
                ("C1  Lv 2", "EXP 101  Next Lv 2"),
            ]
        );
        let (title, lines) = results_lines(&results, page(1), &name, &item);
        assert_eq!(title, "Received Item(s) and Gil");
        let texts: Vec<&str> = lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(texts, ["Gil", "Current Gil", "I236", "I88 (stolen)"]);
        let empty = tantalus_tools::play::BattleResults {
            outcome: "victory".into(),
            ..Default::default()
        };
        let (_, lines) = results_lines(&empty, page(1), &name, &item);
        assert_eq!(lines[2].text, "No item received.");
        let lost = tantalus_tools::play::BattleResults {
            outcome: "defeat".into(),
            ..Default::default()
        };
        assert_eq!(
            results_lines(&lost, page(0), &|_| String::new(), &|_| None).0,
            "GAME OVER"
        );
    }

    use super::*;
    use tantalus_tools::battle_stage::StageUnit;

    fn stage() -> BattleStage {
        let mut stage = BattleStage {
            battle: 1,
            scene: "EF_E006".into(),
            units: vec![
                StageUnit {
                    model: StageModel::Player {
                        character: 0,
                        serial: 0,
                    },
                    position: [0.0, 0.0, -1560.0],
                    turn: 180.0,
                    max_hp: 100,
                    max_mp: 10,
                    type_no: None,
                    name: None,
                },
                StageUnit {
                    model: StageModel::Enemy {
                        geo: -1,
                        idle: 0,
                        hit: 0,
                        death: 0,
                    },
                    position: [0.0, 0.0, 1000.0],
                    turn: 0.0,
                    max_hp: 50,
                    max_mp: 0,
                    type_no: Some(0),
                    name: None,
                },
            ],
            ..BattleStage::default()
        };
        stage.record(0, &[], &[], &[], "x");
        stage
    }

    #[test]
    fn a_replay_ends_after_its_record_and_the_hold() {
        let assets = BattleAssets::new(Path::new("missing"));
        let mut playback = BattlePlayback::new(&assets, stage(), 64, 48);
        assert!(!playback.advance(1.0, 1.0));
        assert!(playback.advance(HOLD_SECONDS, 1.0));
    }

    #[test]
    fn the_readout_draws_without_assets() {
        let assets = BattleAssets::new(Path::new("missing"));
        let stage = stage();
        let scene = BattleScene::new(&assets, &stage, 320, 240);
        let frame = scene.render(&assets, &stage, 0.0);
        assert!(frame.rgba.chunks(4).any(|pixel| pixel[0] > 200));
        assert_eq!(unit_name(&stage.units[0].model, 0), "Zidane");
        assert_eq!(unit_name(&stage.units[1].model, 0), "Enemy 1");
    }
}
