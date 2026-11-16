//! The battle view: a fought battle's [`BattleStage`] replayed over its
//! battle background, seen through the default battle camera, with the
//! party and enemy models at their battle positions and an HP/ATB readout.
//!
//! The fight itself is already decided when the view opens (`PlayGame`
//! runs it to the end inside one field tick with its stand-in party AI);
//! the view replays the per-tick record at the battle rate, 15 ticks a
//! second. Assets (Memoria sources in each function):
//! - background: `FF9BattleDB.MapModel["BSC_<scene>"]` in `p0data2.bin`
//!   (`HonoluluBattleMain.InitBattleScene`), drawn in the queue order of its
//!   `battlebg.SetDefaultShader` groups;
//! - models: `p0data4.bin` through `model_factory` (`ModelFactory.CreateModel`
//!   with `isBattle`, which hides the `field_model` child); players use their
//!   `BattleParameters.csv` model, enemies `MonAddr[TypeNo].Geo`;
//! - idle clips: `p0data5.bin`, `MP_IDLE_NORMAL` for players and `mot[0]`
//!   for enemies (`btl_init.SetMonsterParameter`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bevy::math::{Quat, Vec3};
use tantalus_assets::pc::battle_background::{
    BattleBackground, BbgBlend, BbgGroup, assemble_background,
};
use tantalus_assets::pc::battle_models::{
    background_name, geo_id_by_name, player_battle_row, resolve_animation_id,
    resolve_animation_name,
};
use tantalus_assets::pc::field_models::model_dirs;
use tantalus_assets::pc::model_assembly::pose::{
    Interpolation, bind_clip, rest_pose, sample_clip, skin_model, transform_point,
};
use tantalus_assets::pc::model_assembly::{
    AssembleOptions, AssembledModel, ModelCatalog, SerializedStore, assemble_model, clip_entries,
};
use tantalus_assets::pc::serialized::SerializedFile;
use tantalus_assets::pc::unity_object::animation::AnimationClip;
use tantalus_assets::pc::unity_raw::UnityRawBundle;
use tantalus_tools::battle_stage::{BattleStage, StageFrame, StageModel};

use crate::battle_raster::{BattleCamera, Blend, DrawState, Frame, Texture, ViewVertex};

const MAX_BUNDLE_BYTES: usize = 600 * 1024 * 1024;
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

/// A clip as `(folder geo id, AnimationDB key)`.
type ClipKey = (u32, u32);

/// What one unit draws with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UnitLook {
    geo: u32,
    idle: Option<ClipKey>,
}

fn unit_look(model: &StageModel) -> Option<UnitLook> {
    match model {
        StageModel::Player { serial, .. } => {
            let (_, model, idle) = player_battle_row(u32::from(*serial))?;
            let clip = resolve_animation_name(idle, None, None);
            Some(UnitLook {
                geo: geo_id_by_name(model)?,
                idle: clip.folder_geo_id.zip(clip.clip_key),
            })
        }
        StageModel::Enemy { geo, idle } => {
            let clip = resolve_animation_id(u32::from(*idle), None);
            Some(UnitLook {
                geo: u32::try_from(*geo).ok()?,
                idle: clip.folder_geo_id.zip(clip.clip_key),
            })
        }
        StageModel::None => None,
    }
}

/// Backgrounds, models and clips loaded so far.
pub struct BattleAssets {
    battle_bundle: PathBuf,
    models_bundle: PathBuf,
    clips_bundle: PathBuf,
    backgrounds: HashMap<String, Option<BattleBackground>>,
    models: HashMap<u32, Option<AssembledModel>>,
    clips: HashMap<ClipKey, Option<AnimationClip>>,
}

impl BattleAssets {
    /// `streaming_assets` holds `p0data2.bin`, `p0data4.bin` and
    /// `p0data5.bin`.
    pub fn new(streaming_assets: &Path) -> Self {
        Self {
            battle_bundle: streaming_assets.join("p0data2.bin"),
            models_bundle: streaming_assets.join("p0data4.bin"),
            clips_bundle: streaming_assets.join("p0data5.bin"),
            backgrounds: HashMap::new(),
            models: HashMap::new(),
            clips: HashMap::new(),
        }
    }

    pub fn background(&self, stage: &BattleStage) -> Option<&BattleBackground> {
        self.backgrounds
            .get(&background_name(&stage.scene)?.to_ascii_lowercase())?
            .as_ref()
    }

    /// Loads what `stage` needs. One line per thing that failed.
    pub fn prepare(&mut self, stage: &BattleStage) -> Vec<String> {
        let mut problems = Vec::new();
        match background_name(&stage.scene) {
            None => problems.push(format!("no MapModel row for BSC_{}", stage.scene)),
            Some(name) => {
                let key = name.to_ascii_lowercase();
                if !self.backgrounds.contains_key(&key) {
                    let loaded = with_store(&self.battle_bundle, |store| {
                        let entries = store.container().map_err(|error| error.to_string())?;
                        let path = format!("{BACKGROUND_PREFIX}{key}/{key}.prefab");
                        let root = entries
                            .iter()
                            .find(|entry| entry.class_id == 1 && entry.path == path)
                            .or_else(|| {
                                entries.iter().find(|entry| {
                                    entry.class_id == 1
                                        && entry
                                            .path
                                            .starts_with(&format!("{BACKGROUND_PREFIX}{key}/{key}"))
                                })
                            })
                            .ok_or_else(|| format!("{path} is not in the bundle"))?;
                        assemble_background(store, name, root.path_id)
                            .map_err(|error| error.to_string())
                    });
                    match loaded {
                        Ok(background) => {
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
            }
        }
        let looks: Vec<UnitLook> = stage
            .units
            .iter()
            .filter_map(|unit| unit_look(&unit.model))
            .collect();
        let want_models: Vec<u32> = looks
            .iter()
            .map(|look| look.geo)
            .filter(|geo| !self.models.contains_key(geo))
            .collect();
        if !want_models.is_empty() {
            let loaded = with_store(&self.models_bundle, |store| {
                let entries = store.container().map_err(|error| error.to_string())?;
                let catalog = ModelCatalog::from_container(&entries);
                Ok(want_models
                    .iter()
                    .map(|geo| {
                        let model = model_dirs(*geo, &catalog).into_iter().find_map(|dir| {
                            assemble_model(
                                store,
                                &dir.label,
                                *dir.roots.first()?,
                                AssembleOptions {
                                    decode_textures: true,
                                },
                            )
                            .ok()
                        });
                        (*geo, model)
                    })
                    .collect::<Vec<_>>())
            });
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
        let want_clips: Vec<ClipKey> = looks
            .iter()
            .filter_map(|look| look.idle)
            .filter(|clip| !self.clips.contains_key(clip))
            .collect();
        if !want_clips.is_empty() {
            let loaded = with_store(&self.clips_bundle, |store| {
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
            });
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

fn with_store<T>(
    path: &Path,
    run: impl FnOnce(&SerializedStore<'_, '_>) -> Result<T, String>,
) -> Result<T, String> {
    let source = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let bundle = UnityRawBundle::open(path).map_err(|error| error.to_string())?;
    let bytes = bundle
        .read_entry(0, MAX_BUNDLE_BYTES)
        .map_err(|error| error.to_string())?;
    let file = SerializedFile::parse(&bytes, &source).map_err(|error| error.to_string())?;
    let store = SerializedStore::new(&file, &source).map_err(|error| error.to_string())?;
    run(&store)
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

/// `ModelFactory.CreateModel(isBattle)` turns off every renderer under the
/// `field_model` child.
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

/// Draws one unit's model, posed `time` seconds into its idle.
fn draw_unit(
    frame: &mut Frame,
    camera: &BattleCamera,
    assets: &BattleAssets,
    look: UnitLook,
    position: [f32; 3],
    turn: f32,
    time: f32,
) -> bool {
    let Some(Some(model)) = assets.models.get(&look.geo) else {
        return false;
    };
    let clip = look.idle.and_then(|clip| assets.clips.get(&clip)?.as_ref());
    let poses = match clip {
        Some(clip) => {
            let length = clip_length(clip);
            let time = if length > 0.0 {
                time.rem_euclid(length)
            } else {
                0.0
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
    // about the up axis.
    let rotation =
        Quat::from_rotation_y(turn.to_radians()) * Quat::from_rotation_z(std::f32::consts::PI);
    let origin = Vec3::from_array(position);
    let state = DrawState {
        blend: Blend::Opaque,
        depth_test: true,
        depth_write: true,
        cull_back: true,
    };
    for (part, vertices) in model.parts.iter().zip(skin_model(model, &poses)) {
        if under_field_model(model, part.node) {
            continue;
        }
        let view: Vec<Vec3> = vertices
            .iter()
            .map(|vertex| {
                let world = origin + rotation * Vec3::from_array(*vertex);
                camera.to_view(world.to_array())
            })
            .collect();
        for submesh in &part.submeshes {
            if submesh.topology != TOPOLOGY_TRIANGLES {
                continue;
            }
            let texture = submesh.material.as_ref().and_then(|material| {
                material.textures.iter().find_map(|texture| {
                    let rgba = texture.rgba.as_deref()?;
                    let width = usize::try_from(texture.width).ok()?;
                    let height = usize::try_from(texture.height).ok()?;
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
    true
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

/// A unit's readout name.
fn unit_name(model: &StageModel, enemy: usize) -> String {
    match model {
        StageModel::Player { character, .. } => CHARACTERS
            .get(usize::from(*character))
            .map_or_else(|| format!("Player {character}"), |name| (*name).to_owned()),
        _ => format!("Enemy {}", enemy + 1),
    }
}

/// The background drawn once (it and the camera do not move yet): color
/// and depth after the opaque pass.
pub struct BattleScene {
    pub camera: BattleCamera,
    base: Frame,
    /// How many units have a model drawn.
    pub models: usize,
    pub background_triangles: usize,
}

impl BattleScene {
    pub fn new(assets: &BattleAssets, stage: &BattleStage, width: u32, height: u32) -> Self {
        let camera = stage_camera(stage);
        let mut base = Frame::new(width, height, CLEAR);
        let mut background_triangles = 0;
        if let Some(background) = assets.background(stage) {
            draw_background(&mut base, &camera, background, false);
            background_triangles = background.triangle_count();
        }
        let models = stage
            .units
            .iter()
            .filter_map(|unit| unit_look(&unit.model))
            .filter(|look| matches!(assets.models.get(&look.geo), Some(Some(_))))
            .count();
        Self {
            camera,
            base,
            models,
            background_triangles,
        }
    }

    /// The frame `seconds` into the replay.
    pub fn render(&self, assets: &BattleAssets, stage: &BattleStage, seconds: f64) -> Frame {
        let mut frame = self.base.clone();
        let record = stage.frame_at(seconds);
        for (index, unit) in stage.units.iter().enumerate() {
            let present = record.is_none_or(|record| {
                record.present.get(index).copied().unwrap_or(true)
                    && !(index >= player_count(stage)
                        && record.hp.get(index).copied().unwrap_or(1) == 0)
            });
            if !present {
                continue;
            }
            if let Some(look) = unit_look(&unit.model) {
                draw_unit(
                    &mut frame,
                    &self.camera,
                    assets,
                    look,
                    unit.position,
                    unit.turn,
                    seconds as f32,
                );
            }
        }
        if let Some(background) = assets.background(stage) {
            draw_background(&mut frame, &self.camera, background, true);
        }
        readout(&mut frame, stage, record, seconds);
        frame
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
            unit_name(&unit.model, index - players),
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

/// `frame` as a Bevy image (sRGB RGBA).
pub fn frame_image(frame: &Frame) -> bevy::image::Image {
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    bevy::image::Image::new(
        Extent3d {
            width: frame.width,
            height: frame.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        frame.rgba.clone(),
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::default(),
    )
}

/// Saves `frame` as a PNG.
pub fn save_png(frame: &Frame, output: &Path) -> Result<(), String> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    frame_image(frame)
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
pub const LIVE_SIZE: (u32, u32) = (640, 480);
/// Replay speed while confirm is held.
pub const FAST_FORWARD: f64 = 4.0;

/// The battle view in the play window: its own camera (drawn after the
/// field's, clearing it) showing one sprite with the CPU frame.
pub struct LiveBattle {
    pub playback: BattlePlayback,
    image: bevy::asset::Handle<bevy::image::Image>,
    camera: bevy::ecs::entity::Entity,
    sprite: bevy::ecs::entity::Entity,
}

impl LiveBattle {
    /// Loads what `stage` needs and opens the view.
    pub fn start(
        world: &mut bevy::ecs::world::World,
        assets: &mut BattleAssets,
        stage: BattleStage,
    ) -> Self {
        use bevy::camera::visibility::RenderLayers;
        use bevy::camera::{RenderTarget, ScalingMode};
        use bevy::prelude::*;
        for problem in assets.prepare(&stage) {
            println!("!! battle assets: {problem}");
        }
        let playback = BattlePlayback::new(assets, stage, LIVE_SIZE.0, LIVE_SIZE.1);
        let image = world
            .resource_mut::<Assets<Image>>()
            .add(frame_image(&playback.render(assets)));
        // Draw where the field camera draws (the window, or an offscreen
        // image in a capture).
        let mut targets = world.query_filtered::<&RenderTarget, With<Camera2d>>();
        let target = targets.iter(world).next().cloned();
        let (width, height) = crate::window::FIELD_FRAME;
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
            playback,
            image,
            camera,
            sprite,
        }
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
        use bevy::prelude::*;
        let speed = if held & tantalus_tools::play::pad::CONFIRM != 0 {
            FAST_FORWARD
        } else {
            1.0
        };
        let done = self.playback.advance(delta, speed);
        let frame = self.playback.render(assets);
        if let Some(mut image) = world.resource_mut::<Assets<Image>>().get_mut(&self.image) {
            image.data = Some(frame.rgba);
        }
        // Other code moves every 2D camera; keep the sprite in front of ours.
        let at = world
            .get::<Transform>(self.camera)
            .map(|transform| transform.translation.truncate());
        if let (Some(at), Some(mut transform)) = (at, world.get_mut::<Transform>(self.sprite)) {
            transform.translation.x = at.x;
            transform.translation.y = at.y;
        }
        done
    }

    /// Closes the view.
    pub fn finish(self, world: &mut bevy::ecs::world::World) {
        for entity in [self.sprite, self.camera] {
            if let Ok(entity) = world.get_entity_mut(entity) {
                entity.despawn();
            }
        }
        world
            .resource_mut::<bevy::asset::Assets<bevy::image::Image>>()
            .remove(&self.image);
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

/// One play-window frame of the battle view, when one is open: advances
/// it, closes it at its end. True while the view is (still) up, so the
/// caller leaves the field paused.
pub fn run_live_battle(
    world: &mut bevy::ecs::world::World,
    battle: &mut Option<LiveBattle>,
    assets: &BattleAssets,
    delta: f64,
    held: i32,
) -> bool {
    use bevy::prelude::*;
    use bevy::window::PrimaryWindow;
    let Some(mut open) = battle.take() else {
        return false;
    };
    if open.tick(world, assets, delta, held) {
        open.finish(world);
        return false;
    }
    let title = open.title();
    let mut windows = world.query_filtered::<&mut Window, With<PrimaryWindow>>();
    if let Ok(mut window) = windows.single_mut(world)
        && window.title != title
    {
        window.title = title;
    }
    *battle = Some(open);
    true
}

#[cfg(test)]
mod tests {
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
                },
                StageUnit {
                    model: StageModel::Enemy { geo: -1, idle: 0 },
                    position: [0.0, 0.0, 1000.0],
                    turn: 0.0,
                    max_hp: 50,
                    max_mp: 0,
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
