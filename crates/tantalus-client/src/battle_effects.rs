//! The party's spell and item effects, played by the special-effect
//! plugin ([`tantalus_sfx`], the translated `FF9SpecialEffectPlugin`) on
//! the original timing.
//!
//! When each part happens is the plugin's own
//! ([`tantalus_tools::battle_effects::EffectTable::load_plugin_timing`]): the
//! machine's `EffectApplied` (the HP change) falls on the effect's first
//! Effect point, the damage numbers on its `Figure` point, and Thunder
//! dims the scene for its `HoldDuration` (`Sequence.seq`).
//!
//! What is drawn is the plugin's: each effect gets a fresh plugin
//! (`SFX_Play` with the battle's units as `SFX.InitBattleParty` gives
//! them), steps one battle frame at a time (`SFX.UpdatePlugin` runs once
//! per `BattleSubSystem`) under the view's camera, and its packets are
//! drawn as `SFXRender` batches them, over the scene.
//!
//! The spells' own cameras ([`EffectLibrary`], `PLAY_CAMERA` in the
//! effect's sequence stream) are played by `battle_camera_play` for the
//! effects in [`EffectKind`]; a summon keeps the scene's camera.
//!
//! Effects that do not match the player's DLL are not drawn ([`is_drawn`]):
//! 212 to 224, which the DLL itself does not play the same way twice, and
//! eleven sword skills still to fix.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use tantalus_assets::pc::battle_camera::PsxCamera;
use tantalus_sfx::player::{Setup, Unit};
use tantalus_tools::battle_effects::{EffectCameraPlay, EffectTiming, effect_camera_plays};
use tantalus_tools::battle_stage::{BATTLE_TICKS_PER_SECOND, BattleStage, StageEvent, StageModel};

use crate::battle_raster::{BattleCamera, Blend, Frame, ScreenCorner, Texture};

/// The effects whose own cameras play (`SpecialEffect` ids): the first
/// spells and summons, until the rest are checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectKind {
    Fire,
    Blizzard,
    Thunder,
    Cure,
    /// `Shiva__Full` 38 / `__Short` 407.
    Shiva,
    /// `Ifrit__Full` 276 / `__Short` 445.
    Ifrit,
    /// `Ramuh__Full` 186 / `__Short` 415.
    Ramuh,
    /// `Special_Trance_Activate` 257 / `Special_Trance_End` 489.
    Trance,
}

impl EffectKind {
    /// `Fire__Single` 0 / `__Multi` 70, `Blizzard` 6 / 62, `Thunder` 3 /
    /// 66, `Cure` 8 / 9 (the Potion item uses 8).
    pub fn of(id: i32) -> Option<Self> {
        match id {
            0 | 70 => Some(Self::Fire),
            6 | 62 => Some(Self::Blizzard),
            3 | 66 => Some(Self::Thunder),
            8 | 9 => Some(Self::Cure),
            38 | 407 => Some(Self::Shiva),
            276 | 445 => Some(Self::Ifrit),
            186 | 415 => Some(Self::Ramuh),
            257 | 489 => Some(Self::Trance),
            _ => None,
        }
    }

    pub fn is_summon(self) -> bool {
        matches!(self, Self::Shiva | Self::Ifrit | Self::Ramuh)
    }
}

/// One party command's effect on the stage.
#[derive(Debug, Clone, PartialEq)]
pub struct StageEffect {
    pub id: i32,
    pub caster: usize,
    pub targets: Vec<usize>,
    /// An `Item` or `AutoPotion` command (`BTL_VFX_REQ.flgs` bit 2).
    pub item: bool,
    pub timing: EffectTiming,
    /// Stage tick of the machine's `EffectApplied`.
    pub applied: f64,
    /// Stage tick of `PlaySFX`: `applied` less `hit`, not before the
    /// command.
    pub start: f64,
}

impl StageEffect {
    /// Battle frames since `PlaySFX` at `seconds`.
    pub fn age(&self, seconds: f64) -> f64 {
        seconds * BATTLE_TICKS_PER_SECOND - self.start
    }
}

/// The effects' data (`ef<id>` TextAssets of `resources.assets`), read
/// when first asked for: the cameras each effect plays, and the effects
/// playing now.
#[derive(Debug)]
pub struct EffectLibrary {
    resources: std::path::PathBuf,
    cache: std::sync::Mutex<std::collections::HashMap<i32, std::sync::Arc<EffectCameras>>>,
    /// The plugin's per-effect flag words, read once.
    flags: std::sync::OnceLock<Vec<u16>>,
    /// The plugin's data from the player's DLL, read once.
    runtime: std::sync::OnceLock<Option<tantalus_sfx::SfxRuntime>>,
    packages: Mutex<HashMap<i32, Option<Arc<Vec<u8>>>>>,
    playing: Mutex<HashMap<PlayKey, Playing>>,
}

/// An effect on the stage: its id, caster and start tick.
type PlayKey = (i32, usize, u64);

/// One effect's plugin, `played` frames in, and its last frame.
#[derive(Debug)]
struct Playing {
    run: Option<tantalus_sfx::player::EffectRun>,
    played: u32,
    list: Option<tantalus_sfx::render::RenderList>,
}

/// An effect's own cameras.
#[derive(Debug, Clone, Default)]
pub struct EffectCameras {
    pub plays: Vec<EffectCameraPlay>,
    /// Its header forces them on with Battle Camera: Fixed
    /// (`RecomputeGate323268`, header byte 28 bit `0x8`).
    pub forced: bool,
}

impl EffectLibrary {
    /// `resources` is `x64/FF9_Data/resources.assets`.
    pub fn new(resources: std::path::PathBuf) -> Self {
        Self {
            resources,
            cache: std::sync::Mutex::new(std::collections::HashMap::new()),
            flags: std::sync::OnceLock::new(),
            runtime: std::sync::OnceLock::new(),
            packages: Mutex::new(HashMap::new()),
            playing: Mutex::new(HashMap::new()),
        }
    }

    fn dll(&self) -> std::path::PathBuf {
        self.resources
            .with_file_name("Plugins")
            .join("FF9SpecialEffectPlugin.dll")
    }

    fn runtime(&self) -> Option<&tantalus_sfx::SfxRuntime> {
        self.runtime
            .get_or_init(|| {
                std::fs::read(self.dll())
                    .ok()
                    .and_then(tantalus_sfx::SfxRuntime::new)
            })
            .as_ref()
    }

    /// Effect `id`'s package (`None` when it cannot be read).
    pub fn package(&self, id: i32) -> Option<Arc<Vec<u8>>> {
        let mut packages = self.packages.lock().unwrap_or_else(PoisonError::into_inner);
        packages
            .entry(id)
            .or_insert_with(|| {
                use tantalus_assets::pc::serialized_inventory::StandaloneSerialized;
                let name = format!("ef{id:03}");
                let mut file = StandaloneSerialized::open(&self.resources).ok()?;
                let (_, object) = file
                    .text_assets()
                    .ok()?
                    .into_iter()
                    .find(|(asset, _)| *asset == name)?;
                Some(Arc::new(file.read_text_asset(&object).ok()?.script))
            })
            .clone()
    }

    /// Effect `id`'s flag word from `Plugins/FF9SpecialEffectPlugin.dll`
    /// beside `resources.assets` (0 when it cannot be read).
    pub fn flags(&self, id: i32) -> u16 {
        let flags = self.flags.get_or_init(|| {
            std::fs::read(self.dll())
                .ok()
                .and_then(|bytes| tantalus_assets::pc::special_effect::plugin_effect_flags(&bytes))
                .unwrap_or_default()
        });
        usize::try_from(id)
            .ok()
            .and_then(|id| flags.get(id))
            .copied()
            .unwrap_or(0)
    }

    /// The cameras effect `id` plays (empty when its data is missing).
    pub fn cameras(&self, id: i32) -> std::sync::Arc<EffectCameras> {
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache
            .entry(id)
            .or_insert_with(|| std::sync::Arc::new(self.read(id).unwrap_or_default()))
            .clone()
    }

    fn read(&self, id: i32) -> Option<EffectCameras> {
        use tantalus_assets::pc::special_effect::EffectFile;
        let bytes = self.package(id)?;
        let effect = EffectFile::parse(&bytes, &format!("ef{id:03}")).ok()?;
        Some(EffectCameras {
            plays: effect_camera_plays(&effect, &bytes),
            forced: self.flags(id) & 8 != 0,
        })
    }
}

/// Every party command with a known effect, in stage order, on the
/// plugin's own timing (`EffectTable::load_plugin_timing`: the HP change
/// and numbers wait for its `Effect` and `Figure` points, callbacks 23 and
/// 24, as in the Steam game, where `ExecVfxCommand` runs from the plugin;
/// `Sequence.seq` stays for effects the plugin cannot play).
pub fn stage_effects(stage: &BattleStage) -> Vec<StageEffect> {
    let party = stage
        .units
        .iter()
        .take_while(|unit| matches!(unit.model, StageModel::Player { .. }))
        .count();
    let mut out = Vec::new();
    for (tick, frame) in stage.frames.iter().enumerate() {
        for event in &frame.events {
            let StageEvent::Command {
                unit,
                cmd_no,
                sub_no,
                targets,
                short_summon,
            } = event
            else {
                continue;
            };
            if *unit >= party {
                continue;
            }
            // `SysTrans` plays the Trance effect on its unit from the start
            // (`btl_vfx.GetPlayerCommandSFX`).
            if *cmd_no == tantalus_legacy::battle::queue::cmd::SYS_TRANS {
                let entering = matches!(frame.trance.get(*unit), Some(Some((_, true))));
                let id = if entering {
                    tantalus_tools::battle_effects::TRANCE_ACTIVATE
                } else {
                    tantalus_tools::battle_effects::TRANCE_END
                };
                if let Some(timing) = stage.effects.timing.get(&id) {
                    out.push(StageEffect {
                        id,
                        caster: *unit,
                        targets: vec![*unit],
                        item: false,
                        timing: *timing,
                        applied: (tick + timing.hit as usize) as f64,
                        start: tick as f64,
                    });
                }
                continue;
            }
            let Some((id, timing)) =
                stage
                    .effects
                    .effect_of(*cmd_no, *sub_no, targets.len(), *short_summon)
            else {
                continue;
            };
            let item = matches!(
                *cmd_no,
                tantalus_legacy::battle::queue::cmd::ITEM
                    | tantalus_legacy::battle::queue::cmd::AUTO_POTION
            );
            let applied = stage.frames[tick..]
                .iter()
                .enumerate()
                .find_map(|(offset, frame)| {
                    frame
                        .events
                        .iter()
                        .any(|event| matches!(event, StageEvent::Effect { caster, .. } if caster == unit))
                        .then_some(tick + offset)
                })
                .unwrap_or(tick + crate::battle_motion::EFFECT_TICKS as usize);
            out.push(StageEffect {
                id,
                caster: *unit,
                targets: targets.clone(),
                item,
                timing,
                applied: applied as f64,
                start: (applied as f64 - f64::from(timing.hit)).max(tick as f64),
            });
        }
    }
    out
}

/// Seconds a damage number from `caster`'s `EffectApplied` at stage tick
/// `tick` waits for its effect's `Figure` point (0 without an effect).
pub fn figure_delay(effects: &[StageEffect], caster: usize, tick: usize) -> f64 {
    effects
        .iter()
        .find(|effect| effect.caster == caster && (effect.applied - tick as f64).abs() < 0.5)
        .map_or(0.0, |effect| {
            ((effect.start + f64::from(effect.timing.figure) - effect.applied)
                / BATTLE_TICKS_PER_SECOND)
                .max(0.0)
        })
}

/// How bright the scene is at `seconds` (`SetBackgroundIntensity`): 1
/// unless an effect dims it.
pub fn scene_intensity(effects: &[StageEffect], seconds: f64) -> f32 {
    let mut intensity = 1.0f32;
    for effect in effects {
        let Some((low, fade, hold)) = effect.timing.dim else {
            continue;
        };
        let age = effect.age(seconds) as f32;
        let (fade, hold) = (fade.max(1) as f32, hold as f32);
        // Fades in over `fade`, holds, then back over 8 frames (the
        // closing `SetBackgroundIntensity: Intensity=1 ; Time=8`).
        let amount = if age < 0.0 {
            0.0
        } else if age < fade {
            age / fade
        } else if age < hold {
            1.0
        } else if age < hold + 8.0 {
            1.0 - (age - hold) / 8.0
        } else {
            0.0
        };
        intensity = intensity.min(1.0 - (1.0 - low) * amount);
    }
    intensity
}

/// Darkens the whole frame to `intensity`.
pub fn dim(frame: &mut Frame, intensity: f32) {
    if intensity >= 1.0 {
        return;
    }
    let (width, height) = (frame.width as i32, frame.height as i32);
    frame.rect(0, 0, width, height, [0, 0, 0], 1.0 - intensity.max(0.0));
}

/// Where a unit is now: its feet, height and facing (world, y up).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitSpot {
    pub feet: [f32; 3],
    pub height: f32,
    /// Degrees about y.
    pub turn: f32,
}

pub use tantalus_tools::battle_effects::{MAX_FRAMES, is_drawn};

/// PSX battle space (y down) from the view's world (y up).
fn psx(point: [f32; 3]) -> [i16; 3] {
    let clamp = |value: f32| value.round().clamp(-32768.0, 32767.0) as i16;
    [clamp(point[0]), clamp(-point[1]), clamp(point[2])]
}

/// The plugin's units (`btl_list` order: the party, then the enemies,
/// eight at most) from the stage and where they are now.
pub fn plugin_units(stage: &BattleStage, spots: &[Option<UnitSpot>]) -> Vec<Unit> {
    let party = stage
        .units
        .iter()
        .take_while(|unit| matches!(unit.model, StageModel::Player { .. }))
        .count();
    stage
        .units
        .iter()
        .enumerate()
        .take(8)
        .map(|(index, unit)| {
            let player = index < party;
            let (slot, serial) = match unit.model {
                StageModel::Player { character, serial } => (character, serial),
                _ => (index.saturating_sub(party) as u8, 0),
            };
            let spot = spots.get(index).copied().flatten();
            let base = psx(unit.position);
            Unit {
                btl_id: if player {
                    1 << index
                } else {
                    0x10 << index.saturating_sub(party).min(11)
                },
                player,
                slot,
                serial,
                position: spot.map_or(base, |spot| psx(spot.feet)),
                base,
                // `rot.eulerAngles.y * 11.38`, 4096 to a turn.
                yaw: (spot.map_or(unit.turn, |spot| spot.turn) * 4096.0 / 360.0).round() as i16,
                height: spot.map_or(0, |spot| spot.height.round().clamp(0.0, 32767.0) as i16),
                // `SFX.InitBtlData`: `radius_collision` (256 for a player),
                // and an enemy's `geo_radius` the first enemy's
                // `radius_effect`.
                radius: if player {
                    256
                } else {
                    stage.radii.get(index).copied().unwrap_or(0.0) as i32
                },
                geo_radius: if player { 256 } else { stage.enemy_radius },
                tar_bone: stage.tar_bones.get(index).copied().flatten().unwrap_or(0),
            }
        })
        .collect()
}

/// The PSX camera the view's camera is (`battle_camera_play::unity_camera`
/// undone): what the plugin projects with.
pub fn psx_camera(camera: &BattleCamera) -> PsxCamera {
    let v = |row: usize, column: usize| camera.view.row(row)[column];
    let r = |value: f32| (value * 4096.0).round() as i32;
    let t = |value: f32| value.round() as i32;
    PsxCamera {
        rotation: [
            [r(v(0, 0)), r(-v(0, 1)), r(v(0, 2))],
            [r(-v(1, 0)), r(v(1, 1)), r(-v(1, 2))],
            [r(-v(2, 0)), r(v(2, 1)), r(-v(2, 2))],
        ],
        translation: [t(v(0, 3)), t(-v(1, 3)), t(-v(2, 3))],
        h: camera.near.round() as i32,
    }
}

/// Draws the effects running at `seconds`. `spots` are the units' places
/// this frame (`None` for units not drawn).
///
/// Each effect's plugin is stepped up to the frame `seconds` is in, and
/// played again from its start when the view goes back.
pub fn draw_effects(
    frame: &mut Frame,
    camera: &BattleCamera,
    library: &EffectLibrary,
    stage: &BattleStage,
    effects: &[StageEffect],
    spots: &[Option<UnitSpot>],
    seconds: f64,
) {
    let units = plugin_units(stage, spots);
    let psx_camera = psx_camera(camera);
    let mut playing = library
        .playing
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    for effect in effects {
        let age = effect.age(seconds);
        let key = (effect.id, effect.caster, effect.start.to_bits());
        if age < 0.0 || !is_drawn(effect.id) {
            playing.remove(&key);
            continue;
        }
        let target = (age.floor() as u32).saturating_add(1).min(MAX_FRAMES);
        if playing.get(&key).is_some_and(|play| play.played > target) {
            playing.remove(&key);
        }
        let play = playing.entry(key).or_insert_with(|| Playing {
            run: start(library, effect, &units, stage.put),
            played: 0,
            list: None,
        });
        if play.played < target
            && let Some(run) = play.run.as_mut()
        {
            run.set_units(&units);
        }
        while play.played < target {
            let Some(run) = play.run.as_mut() else {
                play.list = None;
                break;
            };
            match run.step(&psx_camera) {
                Some(list) => {
                    play.list = Some(list);
                    play.played += 1;
                    dump(effect, play.played, &units, &psx_camera, stage.put);
                }
                None => {
                    play.run = None;
                    play.list = None;
                }
            }
        }
        if let Some(list) = &play.list {
            draw_list(frame, list);
        }
    }
}

/// `TANTALUS_SFX_DUMP=<file>`: appends each played frame's input (the
/// effect, the units and the camera) as a JSON line, for the DLL
/// comparison (`tests::the_battle_frames_match_the_dll`).
fn dump(effect: &StageEffect, frame: u32, units: &[Unit], camera: &PsxCamera, put: [i16; 2]) {
    static FILE: std::sync::OnceLock<Option<Mutex<std::fs::File>>> = std::sync::OnceLock::new();
    let Some(file) = FILE.get_or_init(|| {
        let path = std::env::var_os("TANTALUS_SFX_DUMP")?;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
            .map(Mutex::new)
    }) else {
        return;
    };
    let units: Vec<_> = units
        .iter()
        .map(|unit| {
            serde_json::json!([
                unit.btl_id,
                unit.player,
                unit.slot,
                unit.serial,
                unit.position,
                unit.base,
                unit.yaw,
                unit.height,
                unit.radius,
                unit.tar_bone,
                unit.geo_radius
            ])
        })
        .collect();
    let line = serde_json::json!({
        "id": effect.id,
        "caster": effect.caster,
        "targets": effect.targets,
        "item": effect.item,
        "start": effect.start,
        "frame": frame,
        "put": put,
        "units": units,
        "camera": [camera.rotation, camera.translation, camera.h],
    });
    use std::io::Write;
    let mut file = file.lock().unwrap_or_else(PoisonError::into_inner);
    let _ = writeln!(file, "{line}");
}

/// A fresh plugin playing `effect` (`None` without the plugin or its
/// package, or when it faults).
fn start(
    library: &EffectLibrary,
    effect: &StageEffect,
    units: &[Unit],
    put: [i16; 2],
) -> Option<tantalus_sfx::player::EffectRun> {
    if effect.caster >= units.len() {
        return None;
    }
    let setup = Setup {
        units: units.to_vec(),
        caster: effect.caster,
        targets: effect
            .targets
            .iter()
            .copied()
            .filter(|target| *target < units.len())
            .collect(),
        item: effect.item,
        put,
    };
    let package = library.package(effect.id)?;
    library.runtime()?.play(effect.id, &package, &setup)
}

/// One frame's packets, in `SFXRender`'s order, over the frame (PSX screen
/// 320 x 220 around 160, 120, as [`Frame::project`] places points).
pub fn draw_list(frame: &mut Frame, list: &tantalus_sfx::render::RenderList) {
    use tantalus_sfx::render::Blend as PsxBlend;
    let (sx, sy) = (
        frame.width as f32 / crate::battle_raster::PSX_WIDTH,
        frame.height as f32 / crate::battle_raster::PSX_HEIGHT,
    );
    for batch in &list.batches {
        let blend = match batch.blend {
            PsxBlend::Opaque => Blend::Opaque,
            PsxBlend::Add => Blend::Add,
            PsxBlend::Subtract => Blend::Subtract,
        };
        let texture = batch
            .page
            .and_then(|page| list.pages.get(page))
            .map(|page| Texture {
                width: 256,
                height: 256,
                rgba: &page.rgba,
                clamp: true,
            });
        for triangle in &batch.triangles {
            let corners = triangle.map(|corner| ScreenCorner {
                xy: [corner.xy[0] * sx, corner.xy[1] * sy],
                // Page rows run top-down; a `Texture`'s `v = 0` is its last.
                uv: [corner.uv[0] / 256.0, 1.0 - corner.uv[1] / 256.0],
                color: corner.color,
            });
            frame.draw_screen(corners, texture, blend);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tantalus_tools::battle_effects::EffectTable;
    use tantalus_tools::battle_stage::{StageFrame, StageModel, StageUnit};

    fn unit(model: StageModel) -> StageUnit {
        StageUnit {
            model,
            position: [0.0; 3],
            turn: 0.0,
            max_hp: 10,
            max_mp: 0,
            type_no: None,
            name: None,
        }
    }

    /// A Fire (hit 5, figure 17, length 49) cast at tick 2, applied at 42.
    fn fire_stage() -> BattleStage {
        let mut table = EffectTable::default();
        table.abilities.insert(25, (70, 0, true));
        table.timing.insert(
            0,
            EffectTiming {
                hit: 5,
                figure: 17,
                length: 49,
                dim: None,
            },
        );
        let mut stage = BattleStage {
            units: vec![
                unit(StageModel::Player {
                    character: 0,
                    serial: 0,
                }),
                unit(StageModel::None),
            ],
            effects: std::sync::Arc::new(table),
            ..BattleStage::default()
        };
        stage.frames = vec![StageFrame::default(); 60];
        stage.frames[2].events.push(StageEvent::Command {
            unit: 0,
            cmd_no: 22,
            sub_no: 25,
            targets: vec![1],
            short_summon: false,
        });
        stage.frames[42].events.push(StageEvent::Effect {
            caster: 0,
            target: 1,
            damage: 30,
        });
        stage
    }

    #[test]
    fn an_effect_starts_its_hit_frames_before_the_hp_change() {
        let stage = fire_stage();
        let effects = stage_effects(&stage);
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].id, 0);
        assert_eq!(effects[0].start, 37.0);
        assert_eq!(effects[0].age(42.0 / BATTLE_TICKS_PER_SECOND), 5.0);
        // The numbers wait 12 frames for the Figure point.
        assert_eq!(
            figure_delay(&effects, 0, 42),
            12.0 / BATTLE_TICKS_PER_SECOND
        );
        assert_eq!(figure_delay(&effects, 0, 30), 0.0);
    }

    #[test]
    fn thunder_dims_the_scene_and_brings_it_back() {
        let effect = StageEffect {
            id: 3,
            caster: 0,
            targets: vec![1],
            item: false,
            timing: EffectTiming {
                hit: 11,
                figure: 25,
                length: 41,
                dim: Some((0.5, 4, 41)),
            },
            applied: 11.0,
            start: 0.0,
        };
        let at = |frame: f64| {
            scene_intensity(
                std::slice::from_ref(&effect),
                frame / BATTLE_TICKS_PER_SECOND,
            )
        };
        assert_eq!(at(-1.0), 1.0);
        assert_eq!(at(2.0), 0.75);
        assert_eq!(at(20.0), 0.5);
        assert_eq!(at(45.0), 0.75);
        assert_eq!(at(60.0), 1.0);
    }

    #[test]
    fn the_psx_camera_round_trips() {
        for preset in 0..3 {
            let camera = BattleCamera::preset(preset);
            let back = crate::battle_camera_play::unity_camera(&psx_camera(&camera));
            let (a, b) = (camera.view.to_cols_array(), back.view.to_cols_array());
            for (a, b) in a.iter().zip(b) {
                assert!((a - b).abs() < 1.0, "{a} {b}");
            }
            assert_eq!(back.near, camera.near);
        }
    }

    /// The player's game (`TANTALUS_FF9`, or the Steam install).
    fn install() -> std::path::PathBuf {
        std::env::var_os("TANTALUS_FF9").map_or_else(
            || std::path::PathBuf::from("E:/SteamLibrary/steamapps/common/FINAL FANTASY IX"),
            std::path::PathBuf::from,
        )
    }

    #[test]
    fn the_plugin_draws_fire_on_the_target() {
        let resources = install().join("x64/FF9_Data/resources.assets");
        if !resources.exists() {
            eprintln!("skipped: no game data");
            return;
        }
        let library = EffectLibrary::new(resources);
        let mut stage = fire_stage();
        stage.units[0].position = [0.0, 0.0, 1000.0];
        stage.units[1] = unit(StageModel::Enemy {
            geo: 0,
            idle: 0,
            hit: 0,
            death: 0,
        });
        stage.units[1].position = [0.0, 0.0, -1000.0];
        let effects = stage_effects(&stage);
        let camera = BattleCamera::preset(0);
        let spots: Vec<_> = stage
            .units
            .iter()
            .map(|unit| {
                Some(UnitSpot {
                    feet: unit.position,
                    height: 600.0,
                    turn: 0.0,
                })
            })
            .collect();
        let blank = Frame::new(320, 220, [0, 0, 0, 255]);
        let lit = |frame: &Frame| frame.rgba.chunks(4).filter(|pixel| pixel[0] > 40).count();
        let at = |frame: f64| (effects[0].start + frame) / BATTLE_TICKS_PER_SECOND;
        let draw = |seconds: f64| {
            let mut frame = blank.clone();
            draw_effects(
                &mut frame, &camera, &library, &stage, &effects, &spots, seconds,
            );
            frame
        };
        // Before the cast nothing is drawn.
        assert_eq!(lit(&draw(at(-1.0))), 0);
        let most = (0..120)
            .step_by(4)
            .map(|frame| lit(&draw(at(f64::from(frame)))))
            .max()
            .unwrap_or(0);
        assert!(most > 500, "lit {most}");
    }

    /// A battle's effect frames (`TANTALUS_SFX_DUMP`, written by a
    /// capture) replayed on the player's own DLL and on the plugin, with
    /// the same units and camera each frame: the render lists must match,
    /// and the frames in `TANTALUS_SFX_FRAMES` are drawn side by side
    /// (plugin left, DLL right) into `TANTALUS_SFX_OUT`.
    #[cfg(feature = "dll-oracle")]
    #[test]
    #[ignore = "needs the player's FF9SpecialEffectPlugin.dll and a dump"]
    fn the_battle_frames_match_the_dll() {
        use tantalus_sfx::oracle::{INIT_PARAM_SIZE, Oracle, REQUEST_SIZE};
        let var = |name: &str| std::env::var(name).unwrap_or_default();
        let wanted: i32 = var("TANTALUS_SFX_EFFECT").parse().unwrap_or(0);
        let shots: Vec<u32> = var("TANTALUS_SFX_FRAMES")
            .split(',')
            .filter_map(|frame| frame.parse().ok())
            .collect();
        let out = std::path::PathBuf::from(var("TANTALUS_SFX_OUT"));
        let lines: Vec<serde_json::Value> = std::fs::read_to_string(var("TANTALUS_SFX_DUMP"))
            .expect("dump")
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .filter(|line: &serde_json::Value| line["id"] == wanted)
            .collect();
        assert!(!lines.is_empty(), "no frames of effect {wanted}");
        // The first play of the effect only.
        let start = lines[0]["start"].clone();
        let lines: Vec<_> = lines
            .into_iter()
            .filter(|line| line["start"] == start)
            .collect();
        let int = |value: &serde_json::Value| value.as_i64().unwrap_or(0);
        let units = |line: &serde_json::Value| -> Vec<Unit> {
            line["units"]
                .as_array()
                .expect("units")
                .iter()
                .map(|unit| {
                    let v3 = |value: &serde_json::Value| {
                        [0, 1, 2].map(|index| int(&value[index]) as i16)
                    };
                    Unit {
                        btl_id: int(&unit[0]) as u16,
                        player: unit[1].as_bool().unwrap_or(false),
                        slot: int(&unit[2]) as u8,
                        serial: int(&unit[3]) as u8,
                        position: v3(&unit[4]),
                        base: v3(&unit[5]),
                        yaw: int(&unit[6]) as i16,
                        height: int(&unit[7]) as i16,
                        radius: int(&unit[8]) as i32,
                        geo_radius: int(&unit[10]) as i32,
                        tar_bone: int(&unit[9]) as u8,
                    }
                })
                .collect()
        };
        let camera = |line: &serde_json::Value| {
            let camera = &line["camera"];
            PsxCamera {
                rotation: [0, 1, 2]
                    .map(|row| [0, 1, 2].map(|column| int(&camera[0][row][column]) as i32)),
                translation: [0, 1, 2].map(|index| int(&camera[1][index]) as i32),
                h: int(&camera[2]) as i32,
            }
        };
        let first = &lines[0];
        let setup = Setup {
            units: units(first),
            caster: int(&first["caster"]) as usize,
            targets: first["targets"]
                .as_array()
                .expect("targets")
                .iter()
                .map(|target| int(target) as usize)
                .collect(),
            item: first["item"].as_bool().unwrap_or(false),
            put: [int(&first["put"][0]) as i16, int(&first["put"][1]) as i16],
        };
        let library = EffectLibrary::new(install().join("x64/FF9_Data/resources.assets"));
        let package = library.package(wanted).expect("package");
        let mut ours = library
            .runtime()
            .expect("plugin")
            .play(wanted, &package, &setup)
            .expect("start");
        let mut theirs = Oracle::load(&library.dll()).expect("dll");
        theirs.use_battle_host(&setup.units);
        let param: [u8; INIT_PARAM_SIZE] = tantalus_sfx::player::init_param(&setup)
            .try_into()
            .expect("param");
        let request: [u8; REQUEST_SIZE] = tantalus_sfx::player::request(&setup)
            .try_into()
            .expect("request");
        theirs.init_battle(&param).expect("init");
        theirs.play(wanted, &package, &request).expect("play");
        let mut same = 0;
        for line in &lines {
            let frame = int(&line["frame"]) as u32;
            let (units, camera) = (units(line), camera(line));
            ours.set_units(&units);
            theirs.set_units(&units);
            theirs.set_camera(&camera);
            let mine = ours.step(&camera);
            let dll = theirs
                .frame()
                .expect("frame")
                .and_then(|prims| theirs.render(wanted, &prims));
            if mine != dll {
                let describe = |list: &Option<tantalus_sfx::render::RenderList>| {
                    list.as_ref().map(|list| {
                        list.batches
                            .iter()
                            .map(|batch| {
                                let first = batch.triangles.first().map(|t| t[0]);
                                format!(
                                    "{:?}/{:?}/{} {:?}",
                                    batch.blend,
                                    batch.page,
                                    batch.triangles.len(),
                                    first
                                )
                            })
                            .collect::<Vec<_>>()
                    })
                };
                if let (Some(a), Some(b)) = (&mine, &dll) {
                    for (index, (x, y)) in a.pages.iter().zip(&b.pages).enumerate() {
                        let differ = x.rgba.iter().zip(&y.rgba).filter(|(p, q)| p != q).count();
                        let first = x.rgba.iter().zip(&y.rgba).position(|(p, q)| p != q);
                        println!("page {index}: {differ} bytes differ, first {first:?}");
                    }
                    for (x, y) in a.batches.iter().zip(&b.batches) {
                        let at = x
                            .triangles
                            .iter()
                            .zip(&y.triangles)
                            .position(|(p, q)| p != q);
                        if let Some(at) = at {
                            println!("tri {at}: {:?} vs {:?}", x.triangles[at], y.triangles[at]);
                        }
                    }
                }
                panic!(
                    "frame {frame}: the render lists differ
port {:#?}
dll  {:#?}",
                    describe(&mine),
                    describe(&dll)
                );
            }
            same += 1;
            if std::env::var_os("TANTALUS_SFX_VERBOSE").is_some() {
                println!(
                    "frame {frame}: {} triangles",
                    mine.as_ref().map_or(0, |list| list.triangles())
                );
            }
            if shots.contains(&frame) {
                let draw = |list: &Option<tantalus_sfx::render::RenderList>| {
                    let mut frame = Frame::new(640, 480, [0, 0, 0, 255]);
                    if let Some(list) = list {
                        draw_list(&mut frame, list);
                    }
                    frame
                };
                let (left, right) = (draw(&mine), draw(&dll));
                let mut sheet = image::RgbaImage::new(1280, 480);
                for (offset, frame) in [(0, &left), (640, &right)] {
                    for y in 0..480u32 {
                        for x in 0..640u32 {
                            let at = ((y * 640 + x) * 4) as usize;
                            let pixel = &frame.rgba[at..at + 4];
                            sheet.put_pixel(
                                x + offset,
                                y,
                                image::Rgba([pixel[0], pixel[1], pixel[2], 255]),
                            );
                        }
                    }
                }
                sheet
                    .save(out.join(format!("ef{wanted:03}-frame{frame:03}-port-vs-dll.png")))
                    .expect("png");
            }
        }
        println!("effect {wanted}: {same} frames match the DLL");
    }
}
