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
//! effect's sequence stream) are played by `battle_camera_play` for every
//! drawn party effect that has any; an enemy attack keeps the scene's
//! attack camera. An enemy plays the effect its attack sequence starts
//! (`SeqExecVfx` / `SeqExecSVfx`, `tantalus_tools::battle_stage::attack_vfx`)
//! and a party member's Attack its `AttackSequence` effect.

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
    /// The request's `monbone[0]`, `monbone[1]` and `arg0`.
    pub args: [i16; 3],
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
    /// The generated texture (`fill_generated`).
    generated: Option<Frame>,
    /// The background capture (`PSXTextureMgr.bgTexture`), 256 x 256, taken
    /// whenever the effect asks for one.
    background: Option<Frame>,
    /// What the effect has done to the units' models so far.
    changes: std::collections::BTreeMap<u16, tantalus_sfx::player::UnitChange>,
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
                // The short effects (Player_Attack 100-118, SFX_Attack 313 and
                // on) have no package: `SFX.PlaySFX` hands the plugin a null
                // buffer and it plays the effect's built-in script.
                let Some((_, object)) = file
                    .text_assets()
                    .ok()?
                    .into_iter()
                    .find(|(asset, _)| *asset == name)
                else {
                    return Some(Arc::new(Vec::new()));
                };
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
    let mut out = Vec::new();
    for (tick, frame) in stage.frames.iter().enumerate() {
        for event in &frame.events {
            out.extend(command_effect(stage, tick, event));
        }
    }
    out
}

/// The effect the `Command` `event` at stage tick `tick` plays (see
/// [`stage_effects`]), known from the start of the command: a live view
/// schedules it (and its cameras) at once.
pub fn command_effect(stage: &BattleStage, tick: usize, event: &StageEvent) -> Option<StageEffect> {
    let StageEvent::Command {
        unit,
        cmd_no,
        sub_no,
        targets,
        short_summon,
        effect_in,
    } = event
    else {
        return None;
    };
    let party = stage
        .units
        .iter()
        .take_while(|unit| matches!(unit.model, StageModel::Player { .. }))
        .count();
    // An enemy plays the effect its scene attack names
    // (`BattleCommandInfo.VfxIndex`); a party member's Attack its
    // character's `AttackSequence` (`btl_vfx.GetPlayerAttackVfx`).
    let own = if *unit >= party {
        stage
            .attack_effects
            .get(usize::try_from(*sub_no).unwrap_or(usize::MAX))
            .copied()
            .flatten()
            .map(|vfx| (i32::from(vfx.effect), vfx.args))
    } else if *cmd_no == tantalus_legacy::battle::queue::cmd::ATTACK {
        match stage.units[*unit].model {
            StageModel::Player { serial, .. } => {
                tantalus_assets::pc::battle_models::player_attack_sequences()
                    .into_iter()
                    .find(|row| row.0 == u32::from(serial))
                    .map(|row| (row.2, [0; 3]))
            }
            _ => None,
        }
    } else {
        None
    };
    if let Some((id, args)) = own {
        let timing = stage.effects.timing_of(id, false)?;
        let mut effect = own_effect(stage, tick, *unit, *effect_in, id, targets, timing);
        effect.args = args;
        return Some(effect);
    }
    if *unit >= party {
        return None;
    }
    // `SysTrans` plays the Trance effect on its unit from the start
    // (`btl_vfx.GetPlayerCommandSFX`).
    if *cmd_no == tantalus_legacy::battle::queue::cmd::SYS_TRANS {
        let entering = matches!(
            stage
                .frames
                .get(tick)
                .and_then(|frame| frame.trance.get(*unit)),
            Some(Some((_, true)))
        );
        let id = if entering {
            tantalus_tools::battle_effects::TRANCE_ACTIVATE
        } else {
            tantalus_tools::battle_effects::TRANCE_END
        };
        return stage.effects.timing.get(&id).map(|timing| StageEffect {
            id,
            caster: *unit,
            targets: vec![*unit],
            item: false,
            args: [0; 3],
            timing: *timing,
            applied: (tick + timing.hit as usize) as f64,
            start: tick as f64,
        });
    }
    let (id, timing) = stage
        .effects
        .effect_of(*cmd_no, *sub_no, targets.len(), *short_summon)?;
    let item = matches!(
        *cmd_no,
        tantalus_legacy::battle::queue::cmd::ITEM
            | tantalus_legacy::battle::queue::cmd::AUTO_POTION
    );
    let applied = effect_applied(stage, tick, *unit, *effect_in);
    Some(StageEffect {
        id,
        caster: *unit,
        targets: targets.clone(),
        item,
        args: [0; 3],
        timing,
        applied: applied as f64,
        start: (applied as f64 - f64::from(timing.hit)).max(tick as f64),
    })
}

/// Stage tick of `caster`'s `EffectApplied` at or after `tick`, within the
/// command. Before it is recorded (a live battle) or when the command ends
/// without one (Jump, which leaves the field and lands with Spear's), the
/// tick the machine scheduled when the command started (`effect_in`,
/// `Battle::effect_delay`), so a live effect starts once, on time, as
/// `SFX.Play` does after the caster's motions, and is never restarted when
/// the HP change arrives. Only a stage without a schedule takes the default
/// lead.
pub fn effect_applied(
    stage: &BattleStage,
    tick: usize,
    caster: usize,
    effect_in: Option<u32>,
) -> usize {
    for (offset, frame) in stage.frames[tick..].iter().enumerate() {
        let has = |found: fn(&StageEvent, usize) -> bool| {
            frame.events.iter().any(|event| found(event, caster))
        };
        if has(
            |event, caster| matches!(event, StageEvent::Effect { caster: who, .. } if *who == caster),
        ) {
            return tick + offset;
        }
        if has(
            |event, caster| matches!(event, StageEvent::Finished { unit, .. } if *unit == caster),
        ) {
            break;
        }
    }
    tick + effect_in.map_or(crate::battle_motion::EFFECT_TICKS as usize, |ticks| {
        ticks as usize
    })
}

/// An effect a command plays on its own (an enemy attack's, or a party
/// member's Attack).
fn own_effect(
    stage: &BattleStage,
    tick: usize,
    unit: usize,
    effect_in: Option<u32>,
    id: i32,
    targets: &[usize],
    timing: EffectTiming,
) -> StageEffect {
    let applied = effect_applied(stage, tick, unit, effect_in);
    StageEffect {
        id,
        caster: unit,
        targets: targets.to_vec(),
        item: false,
        args: [0; 3],
        timing,
        applied: applied as f64,
        start: (applied as f64 - f64::from(timing.hit)).max(tick as f64),
    }
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
#[derive(Debug, Clone, PartialEq)]
pub struct UnitSpot {
    pub feet: [f32; 3],
    pub height: f32,
    /// Degrees about y.
    pub turn: f32,
    /// The posed model's bones (callback 14).
    pub bones: std::sync::Arc<Vec<Option<tantalus_sfx::player::Bone>>>,
}

pub use tantalus_tools::battle_effects::MAX_FRAMES;

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
            let spot = spots.get(index).cloned().flatten();
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
                position: spot.as_ref().map_or(base, |spot| psx(spot.feet)),
                base,
                // `rot.eulerAngles.y * 11.38`, 4096 to a turn.
                yaw: (spot.as_ref().map_or(unit.turn, |spot| spot.turn) * 4096.0 / 360.0).round()
                    as i16,
                height: spot
                    .as_ref()
                    .map_or(0, |spot| spot.height.round().clamp(0.0, 32767.0) as i16),
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
                bones: spot.map(|spot| spot.bones).unwrap_or_default(),
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

/// Whether any effect of `effects` is on stage at `seconds` and drawn.
pub fn any_playing(effects: &[StageEffect], seconds: f64) -> bool {
    effects.iter().any(|effect| effect.age(seconds) >= 0.0)
}

/// Steps the effects running at `seconds` up to its frame (each effect's
/// plugin is played again from its start when the view goes back), and
/// returns what they did to the units' models, by unit index: summons hide
/// the party, Mini scales, Jump moves (`SFX.BattleCallback`, commands 2, 4,
/// 6, 18 and 25). `spots` are the units' places this frame (`None` for
/// units not drawn); `ground` is the battle background, for the effects
/// that capture it.
pub fn advance_effects(
    camera: &BattleCamera,
    library: &EffectLibrary,
    stage: &BattleStage,
    effects: &[StageEffect],
    spots: &[Option<UnitSpot>],
    ground: Option<&tantalus_assets::pc::battle_background::BattleBackground>,
    seconds: f64,
) -> std::collections::BTreeMap<usize, tantalus_sfx::player::UnitChange> {
    let units = plugin_units(stage, spots);
    let psx_camera = psx_camera(camera);
    let mut playing = library
        .playing
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let mut staged: std::collections::BTreeMap<usize, tantalus_sfx::player::UnitChange> =
        Default::default();
    for effect in effects {
        let age = effect.age(seconds);
        let key = (effect.id, effect.caster, effect.start.to_bits());
        if age < 0.0 {
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
            generated: None,
            background: None,
            changes: Default::default(),
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
                    fill_generated(&mut play.generated, &list);
                    if let Some(params) = list.background_capture {
                        if std::env::var_os("TANTALUS_SFX_BG_OUT").is_some() {
                            eprintln!(
                                "bg capture asked at frame {}: ground {}",
                                play.played + 1,
                                ground.is_some()
                            );
                        }
                        play.background =
                            ground.map(|ground| crate::battle_view::capture_ground(ground, params));
                        // `TANTALUS_SFX_BG_OUT=<png>`: the capture, for a look.
                        if let (Some(path), Some(capture)) =
                            (std::env::var_os("TANTALUS_SFX_BG_OUT"), &play.background)
                        {
                            eprintln!("bg capture frame {} {:?}", play.played + 1, params);
                            let _ =
                                crate::battle_view::save_png(capture, std::path::Path::new(&path));
                        }
                    }
                    play.list = Some(list);
                    play.played += 1;
                    play.changes = run.changes();
                    dump(effect, play.played, &units, &psx_camera, stage.put);
                }
                None => {
                    play.run = None;
                    play.list = None;
                    play.changes.clear();
                }
            }
        }
        for (btl_id, change) in &play.changes {
            let Some(index) = units.iter().position(|unit| unit.btl_id == *btl_id) else {
                continue;
            };
            let entry = staged.entry(index).or_default();
            entry.position = change.position.or(entry.position);
            entry.euler = change.euler.or(entry.euler);
            entry.scale = change.scale.or(entry.scale);
            entry.disappear = entry.disappear.max(change.disappear);
            entry.meshes_hidden |= change.meshes_hidden;
            entry.vanished |= change.vanished;
            entry.fade = change.fade.or(entry.fade);
            entry.appeared |= change.appeared;
            entry.motion = change.motion.clone().or(entry.motion.take());
        }
    }
    staged
}

/// Draws the last frame [`advance_effects`] stepped of each effect running
/// at `seconds`, over `frame`.
pub fn show_effects(
    frame: &mut Frame,
    library: &EffectLibrary,
    effects: &[StageEffect],
    seconds: f64,
) {
    let playing = library
        .playing
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    for effect in effects {
        let key = (effect.id, effect.caster, effect.start.to_bits());
        if effect.age(seconds) < 0.0 {
            continue;
        }
        if let Some(play) = playing.get(&key)
            && let Some(list) = &play.list
        {
            draw_list(
                frame,
                list,
                play.generated.as_ref(),
                play.background.as_ref(),
            );
        }
    }
}

/// Draws the effects running at `seconds` ([`advance_effects`], then
/// [`show_effects`]), the units drawn as they are.
pub fn draw_effects(
    frame: &mut Frame,
    camera: &BattleCamera,
    library: &EffectLibrary,
    stage: &BattleStage,
    effects: &[StageEffect],
    spots: &[Option<UnitSpot>],
    seconds: f64,
) {
    advance_effects(camera, library, stage, effects, spots, None, seconds);
    show_effects(frame, library, effects, seconds);
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
        args: effect.args,
        put,
    };
    let package = library.package(effect.id)?;
    library.runtime()?.play(effect.id, &package, &setup)
}

fn psx_blend(blend: tantalus_sfx::render::Blend) -> Blend {
    use tantalus_sfx::render::Blend as PsxBlend;
    match blend {
        PsxBlend::Opaque => Blend::Opaque,
        PsxBlend::Add => Blend::Add,
        PsxBlend::Subtract => Blend::Subtract,
    }
}

/// A VRAM page of the list as a texture.
fn page_texture<'a>(
    list: &'a tantalus_sfx::render::RenderList,
    batch: &tantalus_sfx::render::Batch,
) -> Option<Texture<'a>> {
    batch
        .page
        .and_then(|page| list.pages.get(page))
        .map(|page| Texture {
            width: 256,
            height: 256,
            rgba: &page.rgba,
            clamp: true,
        })
}

/// The generated texture (`PSXTextureMgr.genTexture`, 512 x 256, VRAM from
/// (384, 256) on) after `list`'s `DR_AREA` pairs: its batches draw into
/// `generated`, which is created black by the first begin
/// (`SFXRenderTextureBegin`) and keeps what it has from frame to frame.
/// Called once per plugin frame, in order.
pub fn fill_generated(generated: &mut Option<Frame>, list: &tantalus_sfx::render::RenderList) {
    use tantalus_sfx::render::{GEN_H, GEN_W};
    for batch in &list.batches {
        if batch.generated_begin && generated.is_none() {
            *generated = Some(Frame::new(GEN_W as u32, GEN_H as u32, [0, 0, 0, 255]));
        }
        if !batch.generated_target || batch.screen {
            continue;
        }
        let Some(target) = generated.as_mut() else {
            continue;
        };
        let blend = psx_blend(batch.blend);
        // The texture the batch itself reads may be the one it draws to.
        let snapshot = batch.generated.then(|| target.rgba.clone());
        let texture = match &snapshot {
            Some(rgba) => Some(Texture {
                width: GEN_W,
                height: GEN_H,
                rgba,
                clamp: true,
            }),
            None => page_texture(list, batch),
        };
        let (tw, th) = if batch.generated {
            (GEN_W as f32, GEN_H as f32)
        } else {
            (256.0, 256.0)
        };
        for triangle in &batch.triangles {
            let corners = triangle.map(|corner| ScreenCorner {
                xy: corner.xy,
                uv: [corner.uv[0] / tw, 1.0 - corner.uv[1] / th],
                color: corner.color,
            });
            target.draw_screen(corners, texture, blend);
        }
    }
}

/// One frame's packets, in `SFXRender`'s order, over the frame (PSX screen
/// 320 x 220 around 160, 120, as [`Frame::project`] places points). The
/// batches that draw into the generated texture were drawn by
/// [`fill_generated`]; the ones that read it read `generated`.
pub fn draw_list(
    frame: &mut Frame,
    list: &tantalus_sfx::render::RenderList,
    generated: Option<&Frame>,
    background: Option<&Frame>,
) {
    use tantalus_sfx::render::{GEN_H, GEN_W};
    let (sx, sy) = (
        frame.width as f32 / frame.psx_w,
        frame.height as f32 / crate::battle_raster::PSX_HEIGHT,
    );
    // A widescreen frame is wider than the 320 the packets were made for:
    // `SFXRender.CalculateWidescreenOffsetX` centers them, and what covers
    // the whole width (`FixWidescreenFace`'s full-width rectangles, the
    // screen-sampling faces) stretches over it instead.
    let psx_w = frame.psx_w;
    let wide = psx_w / crate::battle_raster::PSX_WIDTH;
    let offset = (psx_w - crate::battle_raster::PSX_WIDTH) / 2.0;
    // The screen the scene left, for the batches that sample it
    // (`PSXTextureMgr.blurTexture`, captured before the effects draw): taken
    // once, from the frame as it is now.
    let screen = list
        .batches
        .iter()
        .any(|batch| batch.screen)
        .then(|| frame.capture())
        .flatten();
    let mut screen = screen;
    for batch in &list.batches {
        if batch.capture {
            // `SFXScreenShot.Render`: the screen as the batches before it
            // left it.
            screen = frame.capture();
            continue;
        }
        if batch.generated_target || batch.generated_begin {
            continue;
        }
        let blend = psx_blend(batch.blend);
        let texture = if batch.generated {
            let Some(generated) = generated else {
                continue;
            };
            Some(Texture {
                width: GEN_W,
                height: GEN_H,
                rgba: &generated.rgba,
                clamp: true,
            })
        } else if batch.background {
            // No ground mesh: the capture is clear, and nothing shows.
            let Some(background) = background else {
                continue;
            };
            Some(Texture {
                width: background.width as usize,
                height: background.height as usize,
                rgba: &background.rgba,
                clamp: true,
            })
        } else {
            page_texture(list, batch)
        };
        if batch.screen && screen.is_none() {
            continue;
        }
        // Page texels run 0..256; the screen's are PSX pixels of the 320 x
        // 220 frame (the packets' own coordinates), so a texel is the pixel
        // the same packet corner covers.
        let (tw, th) = if batch.screen {
            (
                crate::battle_raster::PSX_WIDTH,
                crate::battle_raster::PSX_HEIGHT,
            )
        } else if batch.generated {
            (GEN_W as f32, GEN_H as f32)
        } else {
            (256.0, 256.0)
        };
        for triangle in &batch.triangles {
            let full_width = triangle.iter().any(|corner| corner.xy[0] <= 0.0)
                && triangle
                    .iter()
                    .any(|corner| corner.xy[0] >= crate::battle_raster::PSX_WIDTH);
            let place = |x: f32| {
                if batch.screen {
                    x * wide
                } else if full_width {
                    if x <= 0.0 {
                        0.0
                    } else if x >= crate::battle_raster::PSX_WIDTH {
                        psx_w
                    } else {
                        x + offset
                    }
                } else {
                    x + offset
                }
            };
            let corners = triangle.map(|corner| ScreenCorner {
                xy: [place(corner.xy[0]) * sx, corner.xy[1] * sy],
                // Page rows run top-down; a `Texture`'s `v = 0` is its last.
                uv: [corner.uv[0] / tw, 1.0 - corner.uv[1] / th],
                color: corner.color,
            });
            match screen.as_ref().filter(|_| batch.screen) {
                Some(screen) => frame.draw_captured(corners, screen, blend),
                None => frame.draw_screen(corners, texture, blend),
            }
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
            effect_in: None,
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
            args: [0; 3],
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

    #[test]
    fn the_generated_texture_is_drawn_once_and_read_later() {
        use tantalus_sfx::render::{Batch, Blend as PsxBlend, Corner, RenderList};
        let corner = |x: f32, y: f32| Corner {
            xy: [x, y],
            uv: [x, y],
            color: [1.0, 0.0, 0.0],
        };
        let batch = |begin: bool, target: bool, generated: bool| Batch {
            blend: PsxBlend::Opaque,
            page: None,
            screen: false,
            capture: false,
            generated_begin: begin,
            generated_target: target,
            generated,
            background: false,
            triangles: if begin {
                Vec::new()
            } else {
                vec![[corner(8.0, 8.0), corner(40.0, 8.0), corner(8.0, 40.0)]]
            },
        };
        let list = RenderList {
            batches: vec![batch(true, false, false), batch(false, true, false)],
            pages: Vec::new(),
            background_capture: None,
        };
        let mut generated = None;
        fill_generated(&mut generated, &list);
        let texture = generated.as_ref().expect("created by the begin");
        let at = |x: usize, y: usize| texture.rgba[(y * 512 + x) * 4..][..4].to_vec();
        assert_eq!(at(12, 12), [255, 0, 0, 255]);
        assert_eq!(at(300, 200), [0, 0, 0, 255]);
        // The screen draw skips the batch that went into the texture.
        let mut frame = Frame::new(320, 220, [0, 0, 0, 255]);
        draw_list(&mut frame, &list, generated.as_ref(), None);
        assert!(frame.rgba.chunks(4).all(|pixel| pixel[0] == 0));
        // A batch that reads it shows it, 0..256 of its 512 texels wide.
        let read = RenderList {
            batches: vec![Batch {
                triangles: vec![[corner(8.0, 8.0), corner(40.0, 8.0), corner(8.0, 40.0)]],
                ..batch(false, false, true)
            }],
            pages: Vec::new(),
            background_capture: None,
        };
        draw_list(&mut frame, &read, generated.as_ref(), None);
        assert!(frame.rgba.chunks(4).any(|pixel| pixel[0] == 255));
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
                    bones: Default::default(),
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

    /// A contact sheet of one effect played in the battle view: effect
    /// `TANTALUS_SFX_EFFECT` on a caster and a target, frames
    /// `TANTALUS_SFX_FRAMES` (comma separated), over `TANTALUS_SFX_BG` (a
    /// PNG of a battle, or a test pattern), to `TANTALUS_SFX_OUT`.
    #[test]
    #[ignore = "needs the game data; writes a PNG"]
    fn an_effect_contact_sheet() {
        let var = |name: &str| std::env::var(name).unwrap_or_default();
        let id: i32 = var("TANTALUS_SFX_EFFECT").parse().unwrap_or(0);
        let frames: Vec<u32> = var("TANTALUS_SFX_FRAMES")
            .split(',')
            .filter_map(|frame| frame.parse().ok())
            .collect();
        let library = EffectLibrary::new(install().join("x64/FF9_Data/resources.assets"));
        let mut stage = fire_stage();
        stage.units[1] = unit(StageModel::Enemy {
            geo: 0,
            idle: 0,
            hit: 0,
            death: 0,
        });
        stage.units[0].position = [0.0, 0.0, 1000.0];
        stage.units[1].position = [0.0, 0.0, -1000.0];
        let effect = StageEffect {
            id,
            caster: 0,
            targets: vec![1],
            item: false,
            args: [0; 3],
            timing: EffectTiming::default(),
            applied: 0.0,
            start: 0.0,
        };
        let camera = BattleCamera::preset(0);
        let spots: Vec<_> = stage
            .units
            .iter()
            .map(|unit| {
                Some(UnitSpot {
                    feet: unit.position,
                    height: 600.0,
                    turn: 0.0,
                    bones: Default::default(),
                })
            })
            .collect();
        let (w, h) = (480u32, 330u32);
        let background = match image::open(var("TANTALUS_SFX_BG")) {
            Ok(png) => image::imageops::resize(
                &png.to_rgba8(),
                w,
                h,
                image::imageops::FilterType::Triangle,
            ),
            Err(_) => image::RgbaImage::from_fn(w, h, |x, y| {
                let tile = ((x / 24) + (y / 24)) % 2 == 0;
                let base = if tile { 150 } else { 90 };
                image::Rgba([
                    (x * 255 / w) as u8 / 2 + base / 2,
                    base,
                    (y * 255 / h) as u8,
                    255,
                ])
            }),
        };
        let columns = 3u32;
        let rows = (frames.len() as u32).div_ceil(columns).max(1);
        let mut sheet = image::RgbaImage::new(w * columns, h * rows);
        for (index, shot) in frames.iter().enumerate() {
            let mut frame = Frame::new(w, h, [0, 0, 0, 255]);
            frame.rgba.copy_from_slice(background.as_raw());
            // The plugin steps one frame per call, so ask for each shot
            // after the ones before it (the view plays a frame ahead).
            draw_effects(
                &mut frame,
                &camera,
                &library,
                &stage,
                std::slice::from_ref(&effect),
                &spots,
                f64::from(*shot) / BATTLE_TICKS_PER_SECOND,
            );
            let (cx, cy) = ((index as u32 % columns) * w, (index as u32 / columns) * h);
            for y in 0..h {
                for x in 0..w {
                    let at = ((y * w + x) * 4) as usize;
                    let p = &frame.rgba[at..at + 4];
                    sheet.put_pixel(cx + x, cy + y, image::Rgba([p[0], p[1], p[2], 255]));
                }
            }
        }
        sheet.save(var("TANTALUS_SFX_OUT")).expect("png");
        // The generated texture as the last frame left it.
        let playing = library.playing.lock().unwrap();
        if let Some(generated) = playing.values().find_map(|play| play.generated.as_ref()) {
            let png = image::RgbaImage::from_raw(
                generated.width,
                generated.height,
                generated.rgba.clone(),
            )
            .expect("generated texture");
            png.save(var("TANTALUS_SFX_OUT").replace(".png", "-generated.png"))
                .expect("png");
        }
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
                        bones: Default::default(),
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
            args: [0; 3],
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
                        draw_list(&mut frame, list, None, None);
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
