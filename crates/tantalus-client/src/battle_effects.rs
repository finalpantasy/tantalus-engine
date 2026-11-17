//! Stand-in spell effects on the original timing: Fire, Blizzard, Thunder
//! and Cure (also the Potion, which plays Cure), single and multi.
//!
//! The real effects are scripts for the special-effect plugin (PSX MIPS
//! code with their own textures) and are not run. What is data here is
//! when each part happens ([`tantalus_tools::battle_effects`], Memoria's
//! `Sequence.seq`): the effect starts `hit` frames before the machine's
//! `EffectApplied` (the HP change), the damage numbers wait for the
//! `Figure` point and Thunder dims the scene for its `HoldDuration`. The
//! shapes are drawn: flames that burst on the target, ice shards that
//! gather and shatter, a bolt from above, rising green sparkles.
//!
//! Garnet's first summons (Shiva, Ifrit, Ramuh, full and short) draw the
//! same shapes larger over every target. Their effects run far longer than
//! the 40 frames the battle machine gives a command before its HP change,
//! so an effect never starts before its command: it then lands after the
//! HP change, and its numbers wait for its own `Figure` point.
//!
//! The spells' own cameras ([`EffectLibrary`], `PLAY_CAMERA` in the
//! effect's sequence stream) are played by `battle_camera_play`; a summon
//! keeps the scene's camera, as its models are not drawn.
//!
//! Other spells are not drawn.

use bevy::math::Vec3;
use tantalus_tools::battle_effects::{EffectCameraPlay, EffectTiming, effect_camera_plays};
use tantalus_tools::battle_stage::{BATTLE_TICKS_PER_SECOND, BattleStage, StageEvent};

use crate::battle_raster::{BattleCamera, Blend, DrawState, Frame, Texture, ViewVertex};

/// Which stand-in an effect id draws (`SpecialEffect` ids).
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
/// when first asked for: the cameras each effect plays.
#[derive(Debug)]
pub struct EffectLibrary {
    resources: std::path::PathBuf,
    cache: std::sync::Mutex<std::collections::HashMap<i32, std::sync::Arc<Vec<EffectCameraPlay>>>>,
}

impl EffectLibrary {
    /// `resources` is `x64/FF9_Data/resources.assets`.
    pub fn new(resources: std::path::PathBuf) -> Self {
        Self {
            resources,
            cache: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// The cameras effect `id` plays (empty when its data is missing).
    pub fn cameras(&self, id: i32) -> std::sync::Arc<Vec<EffectCameraPlay>> {
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache
            .entry(id)
            .or_insert_with(|| std::sync::Arc::new(self.read(id).unwrap_or_default()))
            .clone()
    }

    fn read(&self, id: i32) -> Option<Vec<EffectCameraPlay>> {
        use tantalus_assets::pc::serialized_inventory::StandaloneSerialized;
        use tantalus_assets::pc::special_effect::EffectFile;
        let name = format!("ef{id:03}");
        let mut file = StandaloneSerialized::open(&self.resources).ok()?;
        let (_, object) = file
            .text_assets()
            .ok()?
            .into_iter()
            .find(|(asset, _)| *asset == name)?;
        let bytes = file.read_text_asset(&object).ok()?.script;
        let effect = EffectFile::parse(&bytes, &name).ok()?;
        Some(effect_camera_plays(&effect, &bytes))
    }
}

/// Every party command with a known effect, in stage order.
pub fn stage_effects(stage: &BattleStage) -> Vec<StageEffect> {
    let party = stage
        .units
        .iter()
        .take_while(|unit| {
            matches!(
                unit.model,
                tantalus_tools::battle_stage::StageModel::Player { .. }
            )
        })
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
            let Some((id, timing)) =
                stage
                    .effects
                    .effect_of(*cmd_no, *sub_no, targets.len(), *short_summon)
            else {
                continue;
            };
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

/// Where a unit is now: its feet and height (world, y up).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitSpot {
    pub feet: [f32; 3],
    pub height: f32,
}

/// Draws the effects running at `seconds`. `spots` are the units' places
/// this frame (`None` for units not drawn).
pub fn draw_effects(
    frame: &mut Frame,
    camera: &BattleCamera,
    effects: &[StageEffect],
    spots: &[Option<UnitSpot>],
    seconds: f64,
) {
    for effect in effects {
        let Some(kind) = EffectKind::of(effect.id) else {
            continue;
        };
        let age = effect.age(seconds) as f32;
        let length = effect.timing.length.max(effect.timing.hit + 1) as f32;
        if !(0.0..length).contains(&age) {
            continue;
        }
        let hit = effect.timing.hit as f32;
        for (order, target) in effect.targets.iter().enumerate() {
            let Some(spot) = spots.get(*target).copied().flatten() else {
                continue;
            };
            let mut sprites = Vec::new();
            let seed = (*target as u32).wrapping_mul(2_654_435_761) ^ (order as u32);
            // A summon: the same shapes, larger.
            let big = UnitSpot {
                feet: spot.feet,
                height: spot.height.max(400.0) * 2.0,
            };
            match kind {
                EffectKind::Fire => fire(&mut sprites, spot, age, hit, length, seed),
                EffectKind::Blizzard => blizzard(&mut sprites, spot, age, hit, length, seed),
                EffectKind::Thunder => thunder(&mut sprites, spot, age, hit, length, seed),
                EffectKind::Cure => cure(&mut sprites, spot, age, hit, length, seed),
                EffectKind::Shiva => blizzard(&mut sprites, big, age, hit, length, seed),
                EffectKind::Ifrit => fire(&mut sprites, big, age, hit, length, seed),
                EffectKind::Ramuh => thunder(&mut sprites, big, age, hit, length, seed),
            }
            for sprite in sprites {
                draw_sprite(frame, camera, &sprite);
            }
        }
    }
}

/// A camera-facing quad.
#[derive(Debug, Clone, Copy)]
struct Sprite {
    center: Vec3,
    /// Half width and half height (world units).
    size: [f32; 2],
    /// Rotation about the view axis (radians).
    roll: f32,
    shape: Shape,
    /// Peak color (0 to 1), already scaled by its fade.
    color: [f32; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// Soft round glow.
    Glow,
    /// Four-pointed star.
    Star,
    /// Long thin diamond.
    Shard,
}

const TEXTURE_SIZE: usize = 32;

/// The shape's coverage at texel `(x, y)`, 0 to 1.
fn coverage(shape: Shape, x: usize, y: usize) -> f32 {
    let half = TEXTURE_SIZE as f32 / 2.0;
    let u = (x as f32 + 0.5 - half) / half;
    let v = (y as f32 + 0.5 - half) / half;
    match shape {
        Shape::Glow => (1.0 - (u * u + v * v).sqrt()).max(0.0).powf(1.5),
        Shape::Star => {
            let core = (1.0 - (u * u + v * v).sqrt() * 2.0).max(0.0);
            let arms = (1.0 - u.abs() * 8.0).max(0.0) * (1.0 - v.abs())
                + (1.0 - v.abs() * 8.0).max(0.0) * (1.0 - u.abs());
            (core + arms).min(1.0)
        }
        Shape::Shard => (1.0 - u.abs() * 3.0 - v.abs()).clamp(0.0, 0.5) * 2.0,
    }
}

fn draw_sprite(frame: &mut Frame, camera: &BattleCamera, sprite: &Sprite) {
    let peak = sprite.color.map(|channel| channel.clamp(0.0, 1.0));
    if peak.iter().all(|channel| *channel < 0.01) {
        return;
    }
    let mut rgba = vec![0u8; TEXTURE_SIZE * TEXTURE_SIZE * 4];
    for y in 0..TEXTURE_SIZE {
        for x in 0..TEXTURE_SIZE {
            let amount = coverage(sprite.shape, x, y);
            let at = (y * TEXTURE_SIZE + x) * 4;
            for channel in 0..3 {
                rgba[at + channel] = (peak[channel] * amount * 255.0) as u8;
            }
            rgba[at + 3] = 255;
        }
    }
    let texture = Texture {
        width: TEXTURE_SIZE,
        height: TEXTURE_SIZE,
        rgba: &rgba,
        clamp: true,
    };
    let center = camera.to_view(sprite.center.to_array());
    let (sin, cos) = sprite.roll.sin_cos();
    let right = Vec3::new(cos, sin, 0.0) * sprite.size[0];
    let up = Vec3::new(-sin, cos, 0.0) * sprite.size[1];
    let corner = |sx: f32, sy: f32, uv: [f32; 2]| ViewVertex {
        position: center + right * sx + up * sy,
        uv,
    };
    let quad = [
        corner(-1.0, -1.0, [0.0, 0.0]),
        corner(1.0, -1.0, [1.0, 0.0]),
        corner(1.0, 1.0, [1.0, 1.0]),
        corner(-1.0, 1.0, [0.0, 1.0]),
    ];
    let state = DrawState {
        blend: Blend::Screen,
        depth_test: true,
        depth_write: false,
        cull_back: false,
    };
    frame.draw(camera, [quad[0], quad[1], quad[2]], Some(texture), state);
    frame.draw(camera, [quad[0], quad[2], quad[3]], Some(texture), state);
}

/// A small deterministic hash to scatter the sprites.
fn scatter(seed: u32, index: u32) -> f32 {
    let mut value = seed ^ index.wrapping_mul(0x9E37_79B9);
    value ^= value >> 15;
    value = value.wrapping_mul(0x2C1B_3C6D);
    value ^= value >> 12;
    value = value.wrapping_mul(0x297A_2D39);
    value ^= value >> 15;
    (value & 0xFFFF) as f32 / 65535.0
}

/// 0 to 1 over `[from, to]`, clamped.
fn ramp(value: f32, from: f32, to: f32) -> f32 {
    ((value - from) / (to - from).max(1e-3)).clamp(0.0, 1.0)
}

fn scale(color: [f32; 3], amount: f32) -> [f32; 3] {
    color.map(|channel| channel * amount)
}

/// Sparks gather on the target, then flames burst up and out and fade.
fn fire(out: &mut Vec<Sprite>, spot: UnitSpot, age: f32, hit: f32, length: f32, seed: u32) {
    let middle = Vec3::from_array(spot.feet) + Vec3::Y * spot.height * 0.45;
    let radius = (spot.height * 0.6).max(180.0);
    if age < hit {
        let gather = ramp(age, 0.0, hit);
        for index in 0..6 {
            let angle = scatter(seed, index) * std::f32::consts::TAU;
            let away = radius * 1.4 * (1.0 - gather);
            out.push(Sprite {
                center: middle + Vec3::new(angle.cos() * away, 0.0, angle.sin() * away),
                size: [60.0; 2],
                roll: 0.0,
                shape: Shape::Glow,
                color: scale([1.0, 0.55, 0.15], 0.6 + 0.4 * gather),
            });
        }
        return;
    }
    let burst = age - hit;
    let fade = 1.0 - ramp(age, hit + (length - hit) * 0.4, length);
    let grow = ramp(burst, 0.0, 6.0);
    out.push(Sprite {
        center: middle,
        size: [radius * (0.6 + 0.8 * grow); 2],
        roll: 0.0,
        shape: Shape::Glow,
        color: scale(
            [1.0, 0.75, 0.35],
            fade * (1.0 - 0.5 * ramp(burst, 4.0, 14.0)),
        ),
    });
    for index in 0..14 {
        let angle = scatter(seed, index) * std::f32::consts::TAU;
        let speed = 0.5 + scatter(seed, index + 40);
        let spread = radius * grow * speed;
        let rise = burst * (12.0 + 18.0 * scatter(seed, index + 80));
        let size = radius
            * (0.35 + 0.25 * scatter(seed, index + 120))
            * (1.0 - 0.4 * ramp(burst, 0.0, length - hit));
        out.push(Sprite {
            center: middle
                + Vec3::new(
                    angle.cos() * spread,
                    rise - radius * 0.3,
                    angle.sin() * spread,
                ),
            size: [size, size * 1.3],
            roll: 0.0,
            shape: Shape::Glow,
            color: scale([1.0, 0.35 + 0.3 * scatter(seed, index + 160), 0.08], fade),
        });
    }
}

/// Shards form in a ring above the target and close in, then shatter out
/// with a white flash.
fn blizzard(out: &mut Vec<Sprite>, spot: UnitSpot, age: f32, hit: f32, length: f32, seed: u32) {
    let middle = Vec3::from_array(spot.feet) + Vec3::Y * spot.height * 0.5;
    let radius = (spot.height * 0.7).max(200.0);
    let count = 10;
    if age < hit {
        let form = ramp(age, 0.0, hit * 0.4);
        let close = ramp(age, hit * 0.4, hit);
        for index in 0..count {
            let angle = index as f32 / count as f32 * std::f32::consts::TAU + age * 0.05;
            let away = radius * (1.5 - 1.1 * close);
            let lift = radius * 0.8 * (1.0 - close) + (scatter(seed, index) - 0.5) * radius * 0.6;
            out.push(Sprite {
                center: middle + Vec3::new(angle.cos() * away, lift, angle.sin() * away),
                size: [45.0 * form, 130.0 * form],
                roll: angle,
                shape: Shape::Shard,
                color: scale([0.6, 0.85, 1.0], 0.5 + 0.5 * close),
            });
        }
        return;
    }
    let burst = age - hit;
    let fade = 1.0 - ramp(burst, 0.0, length - hit);
    out.push(Sprite {
        center: middle,
        size: [radius * (1.0 + 0.3 * ramp(burst, 0.0, 4.0)); 2],
        roll: 0.0,
        shape: Shape::Glow,
        color: scale([0.85, 0.95, 1.0], 1.0 - ramp(burst, 0.0, 8.0)),
    });
    for index in 0..count * 2 {
        let angle = scatter(seed, index + 200) * std::f32::consts::TAU;
        let away = radius * 0.4 + burst * (25.0 + 20.0 * scatter(seed, index + 240));
        let lift = (scatter(seed, index + 280) - 0.4) * radius - burst * burst * 0.8;
        out.push(Sprite {
            center: middle + Vec3::new(angle.cos() * away, lift, angle.sin() * away),
            size: [30.0, 90.0],
            roll: angle + burst * 0.3,
            shape: Shape::Shard,
            color: scale([0.6, 0.85, 1.0], fade),
        });
    }
}

/// A charge above the target, then a jagged bolt down onto it and a flash.
fn thunder(out: &mut Vec<Sprite>, spot: UnitSpot, age: f32, hit: f32, length: f32, seed: u32) {
    let feet = Vec3::from_array(spot.feet);
    let middle = feet + Vec3::Y * spot.height * 0.5;
    let sky = feet + Vec3::Y * (spot.height + 1600.0);
    if age < hit {
        let charge = ramp(age, 0.0, hit);
        out.push(Sprite {
            center: sky - Vec3::Y * 600.0,
            size: [200.0 * charge; 2],
            roll: 0.0,
            shape: Shape::Star,
            color: scale([0.85, 0.85, 1.0], charge),
        });
        return;
    }
    let burst = age - hit;
    let fade = 1.0 - ramp(burst, 0.0, length - hit);
    // Strikes: on for two frames out of three for the first 9 frames.
    let striking = burst < 9.0 && (burst as u32) % 3 != 2;
    if striking {
        let steps = 9;
        let mut previous = sky;
        let flicker = burst as u32;
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            let jitter = if step == steps { 0.0 } else { 160.0 };
            let point = sky.lerp(middle, t)
                + Vec3::new(
                    (scatter(seed ^ flicker, step) - 0.5) * jitter * 2.0,
                    0.0,
                    (scatter(seed ^ flicker, step + 30) - 0.5) * jitter,
                );
            let mid = (previous + point) * 0.5;
            let along = point - previous;
            for (width, color) in [(70.0, [0.35, 0.4, 1.0]), (28.0, [1.0, 1.0, 1.0])] {
                out.push(Sprite {
                    center: mid,
                    size: [width, along.length() * 0.6],
                    roll: -along.x.atan2(along.y),
                    shape: Shape::Glow,
                    color,
                });
            }
            previous = point;
        }
    }
    out.push(Sprite {
        center: middle,
        size: [spot.height.max(300.0) * (0.8 + 0.4 * ramp(burst, 0.0, 6.0)); 2],
        roll: 0.0,
        shape: Shape::Star,
        color: scale(
            [0.8, 0.85, 1.0],
            fade * (1.0 - 0.6 * ramp(burst, 2.0, 10.0)),
        ),
    });
}

/// Green-white sparkles rise around the target inside a soft glow.
fn cure(out: &mut Vec<Sprite>, spot: UnitSpot, age: f32, hit: f32, length: f32, seed: u32) {
    let feet = Vec3::from_array(spot.feet);
    let radius = (spot.height * 0.5).max(160.0);
    let fade_in = ramp(age, 0.0, hit.max(4.0));
    let fade = fade_in * (1.0 - ramp(age, length * 0.6, length));
    out.push(Sprite {
        center: feet + Vec3::Y * spot.height * 0.5,
        size: [radius * 1.4, spot.height.max(300.0) * 0.8],
        roll: 0.0,
        shape: Shape::Glow,
        color: scale([0.35, 0.8, 0.45], fade * 0.8),
    });
    for index in 0..16 {
        let angle = scatter(seed, index) * std::f32::consts::TAU + age * 0.12;
        let cycle = (age / 24.0 + scatter(seed, index + 50)).fract();
        let away = radius * (0.5 + 0.5 * scatter(seed, index + 90));
        out.push(Sprite {
            center: feet
                + Vec3::new(
                    angle.cos() * away,
                    cycle * spot.height.max(300.0) * 1.3,
                    angle.sin() * away,
                ),
            size: [70.0; 2],
            roll: 0.0,
            shape: Shape::Star,
            color: scale([0.75, 1.0, 0.8], fade * (1.0 - cycle)),
        });
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
    fn a_fire_burst_lights_the_target() {
        let stage = fire_stage();
        let effects = stage_effects(&stage);
        let camera = BattleCamera::preset(0);
        let spots = vec![
            None,
            Some(UnitSpot {
                feet: [0.0, 0.0, 400.0],
                height: 600.0,
            }),
        ];
        let blank = Frame::new(160, 110, [0, 0, 0, 255]);
        let mut frame = blank.clone();
        draw_effects(
            &mut frame,
            &camera,
            &effects,
            &spots,
            44.0 / BATTLE_TICKS_PER_SECOND,
        );
        let lit = |frame: &Frame| frame.rgba.chunks(4).filter(|pixel| pixel[0] > 40).count();
        assert!(lit(&frame) > 100, "lit {}", lit(&frame));
        // Before the cast nothing is drawn.
        let mut early = blank.clone();
        draw_effects(&mut early, &camera, &effects, &spots, 1.0);
        assert_eq!(lit(&early), 0);
    }
}
