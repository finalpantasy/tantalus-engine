//! Plays the scene's battle cameras over a stage record: the opening,
//! the default camera, the reaction camera of each command and an enemy's
//! own camera (`tantalus_assets::pc::battle_camera`, the special-effect
//! plugin's camera script), one script frame per battle tick.
//!
//! When the host starts which camera:
//! - `SFX.StartPlungeCamera` at the start: set 0, its variant
//!   `CameraNo` (`PatAddr[PatNum].Camera`; 3 and up are random, 0 here).
//! - At an effect's start (`SeqStateUpdateStart` -> `SfxStateReset`,
//!   `SfxTriggerReaction`): a party member's command takes set 3 when it
//!   aims at the party and 4 otherwise, once its effect plays (after an
//!   attacker's run), unless the effect's flag word says otherwise
//!   ([`reaction_camera`]); an enemy's takes its own set (5 + slot) with the
//!   command.
//! - `SeqExecRunCamera` (sequence code 0x20): `SetCameraTarget` and
//!   `SetEnemyCamera` at the op's frame.
//! - At the effect's end (`SfxBeginBlock`): back to the default set 1.
//!   Here that is the command's `CommandFinished`.
//! - `battle.cs:414` on victory: `SetCamera(2)` with 150.
//!
//! Spells and summons start the cameras inside their own effect data
//! instead of sets 3 and 4; those are not read yet, so a spell drawn by
//! `battle_effects` leaves the camera where it is.

use bevy::math::Mat4;
use tantalus_assets::pc::battle_camera::{CameraActor, CameraScript, CameraWorld, PsxCamera};
use tantalus_tools::battle_stage::{BattleStage, StageEvent, StageModel};

use crate::battle_raster::BattleCamera;

/// An effect's camera: tick, caster, target, the camera sets to pick from
/// (one unless `IsAlternate`), tick the effect ends.
type EffectPlay = (usize, usize, Option<usize>, Vec<Vec<u8>>, usize);

/// The camera state after the ticks run so far.
#[derive(Debug, Clone)]
pub struct CameraDirector {
    script: CameraScript,
    world: CameraWorld,
    /// Stage frames already run.
    ticks: usize,
    camera: Option<PsxCamera>,
    /// Enemy cameras waiting for their op's frame: tick, caster, targets,
    /// camera number.
    pending: Vec<(usize, usize, Vec<usize>, u8)>,
    /// An enemy command whose camera is `RunCamera` 0x12: its unit,
    /// targets and `CameraNo`, until the machine says whether it plays.
    alternate: Option<(usize, Vec<usize>, u8)>,
    /// A party command's reaction camera waiting for its effect to play:
    /// tick, camera, caster, target.
    reaction: Option<(usize, Reaction, usize, Option<usize>)>,
    /// The effects' own cameras (`PLAY_CAMERA`), in tick order: tick,
    /// caster, target, camera set bytes, tick the effect ends.
    effect_plays: Vec<EffectPlay>,
    /// Ticks a drawn effect starts (`SFX_Play`): whether `SfxStateReset` draws the side bit
    /// and its target centre ([`CameraScript::effect_center`]).
    effect_starts: Vec<(usize, bool, [i16; 3])>,
    /// The effects whose own cameras play ([`Self::effect_plays`]).
    own_camera: std::collections::HashSet<i32>,
    /// Party commands (unit, stage tick) whose effect is scheduled.
    scheduled: std::collections::HashSet<(usize, usize)>,
    /// Tick the playing effect camera's effect ends.
    effect_end: Option<usize>,
    /// The unit whose command holds the camera.
    busy: Option<usize>,
    victory: bool,
    party: usize,
}

impl CameraDirector {
    /// `heights` are the units' model heights (battle units); `library`
    /// gives the drawn effects' own cameras. `None` when the scene has no
    /// camera block.
    pub fn new(
        stage: &BattleStage,
        heights: &[f32],
        library: Option<&crate::battle_effects::EffectLibrary>,
    ) -> Option<Self> {
        let block = stage.camera_block.clone()?;
        let party = stage
            .units
            .iter()
            .take_while(|unit| matches!(unit.model, StageModel::Player { .. }))
            .count();
        let actors = stage
            .units
            .iter()
            .enumerate()
            .map(|(index, unit)| CameraActor {
                position: [
                    unit.position[0] as i16,
                    -unit.position[1] as i16,
                    unit.position[2] as i16,
                ],
                base: [
                    unit.position[0] as i16,
                    -unit.position[1] as i16,
                    unit.position[2] as i16,
                ],
                yaw: yaw(unit.turn),
                player: index < party,
                // `SFX.SetupSfxInit`: a player's `geo_radius` is its
                // `radius_collision` (256) and `geo_height` twice that; an
                // enemy's radius is the first enemy's `radius_effect`.
                radius: if index < party {
                    256
                } else {
                    stage.enemy_radius
                },
                height: if index < party {
                    512
                } else {
                    heights.get(index).map_or(0, |h| *h as i32)
                },
            })
            .collect();
        let world = CameraWorld {
            actors,
            put: stage.put,
            start_type: 0,
        };
        let mut script = CameraScript::new(block, stage.battle as u32);
        script.context.camera_no = if stage.camera < 3 { stage.camera } else { 0 };
        script.start_plunge(0);
        let mut director = Some(Self {
            script,
            world,
            ticks: 0,
            camera: None,
            pending: Vec::new(),
            alternate: None,
            reaction: None,
            effect_plays: Vec::new(),
            effect_starts: Vec::new(),
            own_camera: std::collections::HashSet::new(),
            scheduled: std::collections::HashSet::new(),
            effect_end: None,
            busy: None,
            victory: false,
            party,
        });
        if let Some(director) = director.as_mut() {
            for (tick, frame) in stage.frames.iter().enumerate() {
                for event in &frame.events {
                    director.schedule(stage, tick, event, library);
                }
            }
        }
        director
    }

    /// Schedules the effect cameras of the party `Command` `event` at stage
    /// tick `tick`, once. The effect's start is known when the command
    /// starts (`StageEvent::Command::effect_in`), so a live battle plays
    /// them on time as a replay does.
    fn schedule(
        &mut self,
        stage: &BattleStage,
        tick: usize,
        event: &StageEvent,
        library: Option<&crate::battle_effects::EffectLibrary>,
    ) {
        let Some(library) = library else {
            return;
        };
        let StageEvent::Command { unit, .. } = event else {
            return;
        };
        // Enemy attacks keep the scene's attack cameras, and an effect the
        // plugin does not play keeps the stand-in.
        if *unit >= self.party || !self.scheduled.insert((*unit, tick)) {
            return;
        }
        let Some(effect) = crate::battle_effects::command_effect(stage, tick, event) else {
            return;
        };
        // An effect without cameras (a short one with `UseCamera=False`)
        // leaves the camera as it is.
        let cameras = library.cameras(effect.id);
        if !cameras.plays.is_empty() {
            self.own_camera.insert(effect.id);
        }
        let start = effect.start.round() as usize;
        // `SeqStateUpdateStart`: an effect with flag 0x8000 skips `SfxStateReset`, so draws no
        // side bit.
        let draws = library.flags(effect.id) & 0x8000 == 0;
        let center = self.base_center(&effect.targets);
        let at = self.effect_starts.partition_point(|other| other.0 <= start);
        self.effect_starts.insert(at, (start, draws, center));
        // Battle Camera: Fixed turns an effect's cameras off unless its
        // header forces them (`RecomputeGate323268`).
        if stage.fixed_camera && !cameras.forced {
            return;
        }
        for play in cameras.plays.iter() {
            let entry = (
                start + play.frame as usize,
                effect.caster,
                effect.targets.first().copied(),
                play.choices.clone(),
                start + effect.timing.length as usize,
            );
            let at = self
                .effect_plays
                .partition_point(|other| other.0 <= entry.0);
            self.effect_plays.insert(at, entry);
        }
    }

    /// The camera shown at stage frame `tick` (`None` until the script
    /// has placed it). Runs forward from the last call.
    /// `poses` gives each unit's offset from its battle position and its
    /// facing while a command turns it, at a tick (the camera follows a
    /// unit that steps forward, runs and turns).
    pub fn camera_at(
        &mut self,
        stage: &BattleStage,
        tick: usize,
        poses: &dyn Fn(usize) -> Vec<([f32; 3], Option<f32>)>,
        library: Option<&crate::battle_effects::EffectLibrary>,
    ) -> Option<BattleCamera> {
        let last = stage.frames.len();
        while self.ticks <= tick && self.ticks < last.max(1) {
            let now = poses(self.ticks);
            for (index, actor) in self.world.actors.iter_mut().enumerate() {
                let unit = &stage.units[index];
                let (offset, facing) = now.get(index).copied().unwrap_or(([0.0; 3], None));
                actor.position = [
                    (unit.position[0] + offset[0]) as i16,
                    -(unit.position[1] + offset[1]) as i16,
                    (unit.position[2] + offset[2]) as i16,
                ];
                actor.yaw = yaw(facing.unwrap_or(unit.turn));
            }
            self.step(stage, self.ticks, library);
            self.ticks += 1;
        }
        self.camera.map(|camera| unity_camera(&camera))
    }

    /// Ticks run so far.
    pub fn ticks(&self) -> usize {
        self.ticks
    }

    /// Battle ticks the opening camera (set 0, `SFX_StartPlungeCamera`) takes until the script
    /// raises the camera phase (`SFX.GetCameraPhase() == 1`). The battle waits in
    /// `PHASE_ENTER` until then (`SFX.UpdateCamera`, `SFX.cs:1606`): no menu, no ATB. `None`
    /// when the script never raises it within `limit` ticks.
    pub fn opening_ticks(&self, limit: usize) -> Option<usize> {
        let mut script = self.script.clone();
        if script.phase == 1 {
            return Some(0);
        }
        (1..=limit).find(|_| {
            script.update(&self.world);
            script.phase == 1
        })
    }

    fn step(
        &mut self,
        stage: &BattleStage,
        tick: usize,
        library: Option<&crate::battle_effects::EffectLibrary>,
    ) {
        // A command recorded since the director was made (a live battle):
        // its effect cameras, before anything of this tick runs.
        if let Some(frame) = stage.frames.get(tick) {
            for event in &frame.events {
                self.schedule(stage, tick, event, library);
            }
        }
        let mut due = Vec::new();
        self.pending.retain(|entry| {
            if entry.0 <= tick {
                due.push(entry.clone());
                false
            } else {
                true
            }
        });
        while self
            .effect_starts
            .first()
            .is_some_and(|start| start.0 <= tick)
        {
            let (_, draws, center) = self.effect_starts.remove(0);
            // `SfxResolveActors`: the request's target centre.
            self.script.effect_center = center;
            if draws {
                self.script.draw_side();
            }
        }
        while self.effect_plays.first().is_some_and(|play| play.0 <= tick) {
            let (_, caster, target, choices, end) = self.effect_plays.remove(0);
            // `SfxOpStartTrackBlock` mode 3: the side bit's entry, at most
            // the last.
            let pick = usize::from(self.script.side()).min(choices.len() - 1);
            let set = &choices[pick];
            self.busy = Some(caster);
            self.effect_end = Some(end);
            self.script
                .start_effect_camera(set, 0, caster, target, &self.world);
        }
        if self.effect_end.is_some_and(|end| end <= tick) {
            self.effect_end = None;
            self.busy = None;
            self.script.begin_default();
        }
        if let Some((at, _, _, _)) = &self.reaction
            && *at <= tick
            && let Some((_, camera, caster, target)) = self.reaction.take()
        {
            match camera {
                Reaction::Set(set) => {
                    self.script
                        .start_track_block(set, 80, caster, target, &self.world)
                }
                Reaction::Plugin(set) => {
                    self.script
                        .start_effect_camera(&set, 80, caster, target, &self.world)
                }
            }
        }
        for (_, caster, targets, camera_no) in due {
            let center = self.center(&targets);
            self.script.set_enemy_camera(
                caster - self.party,
                caster,
                targets.first().copied(),
                center,
                camera_no,
                &self.world,
            );
        }
        if let Some(frame) = stage.frames.get(tick) {
            for event in &frame.events {
                match event {
                    StageEvent::Command {
                        unit,
                        cmd_no,
                        sub_no,
                        targets,
                        short_summon,
                        effect_in,
                    } => self.command(
                        stage,
                        library,
                        tick,
                        *unit,
                        (*cmd_no, *sub_no, *short_summon, *effect_in),
                        targets,
                    ),
                    // `SeqExecRunCamera` 0x12 when
                    // `ShouldPlayAlternateCamera` let it play.
                    StageEvent::AlternateCamera { unit, play } => {
                        if let Some((owner, targets, camera_no)) = self.alternate.take()
                            && owner == *unit
                            && *play
                        {
                            self.pending.push((tick, owner, targets, camera_no));
                        }
                    }
                    // An effect camera runs to the end of its effect.
                    StageEvent::Finished { unit, .. }
                        if self.busy == Some(*unit) && self.effect_end.is_none() =>
                    {
                        self.busy = None;
                        self.pending.clear();
                        self.reaction = None;
                        self.script.begin_default();
                    }
                    _ => {}
                }
            }
        }
        if !self.victory
            && tick + 1 == stage.frames.len()
            && stage.outcome.as_deref() == Some("victory")
        {
            self.victory = true;
            self.script.set_camera(2, 150);
        }
        if let Some(camera) = self.script.update(&self.world) {
            self.camera = Some(camera);
        }
    }

    fn command(
        &mut self,
        stage: &BattleStage,
        library: Option<&crate::battle_effects::EffectLibrary>,
        tick: usize,
        unit: usize,
        (cmd_no, sub_no, short_summon, effect_in): (u32, i32, bool, Option<u32>),
        targets: &[usize],
    ) {
        let target = targets.first().copied();
        if unit < self.party {
            // A drawn spell plays its own effect cameras (`effect_plays`).
            // Attack plays a short effect with `UseCamera=False`
            // (`SFXData.IsShortSpecialEffect`) and sequence-only commands
            // (Steal, Jump) play none: the camera stays.
            let (serial, character) = match stage.units.get(unit).map(|unit| &unit.model) {
                Some(tantalus_tools::battle_stage::StageModel::Player { serial, character }) => {
                    (Some(*serial), Some(*character))
                }
                _ => (None, None),
            };
            let Some((id, timing)) =
                stage
                    .effects
                    .effect_by(cmd_no, sub_no, targets.len(), short_summon, serial)
            else {
                return;
            };
            if self.own_camera.contains(&id) {
                return;
            }
            let flags = library.map_or(0, |library| library.flags(id));
            let Some(camera) = reaction_camera(
                flags,
                stage.fixed_camera,
                character,
                target.is_some_and(|target| target < self.party),
                library,
            ) else {
                return;
            };
            self.busy = Some(unit);
            // `SfxTriggerReaction` runs when the command's effect plays
            // (`PlaySFX`), after an attacker's run: here its
            // `EffectApplied` (scheduled when the command starts, so a
            // live battle is on time too), less the effect's hit frames.
            let hit = timing.hit as usize;
            let at = crate::battle_effects::effect_applied(stage, tick, unit, effect_in)
                .saturating_sub(hit)
                .max(tick);
            self.reaction = Some((at, camera, unit, target));
            return;
        }
        self.busy = Some(unit);
        // The enemy's attack effect (`SFX_Play`) carries its targets' centre.
        self.script.effect_center = self.base_center(targets);
        let camera = usize::try_from(sub_no)
            .ok()
            .and_then(|index| stage.attack_cameras.get(index).copied().flatten());
        let center = self.center(targets);
        self.script.camera_caster = Some(unit);
        self.script.camera_target = target;
        self.script.target_center = center;
        match camera {
            Some(camera) if camera.run => {
                self.pending.push((
                    tick + camera.frame as usize,
                    unit,
                    targets.to_vec(),
                    camera.camera_no,
                ));
            }
            // 0x12 reads its `CameraNo` only when it plays.
            Some(camera) if camera.alternate => {
                self.alternate = Some((unit, targets.to_vec(), camera.camera_no));
                self.script
                    .start_track_block(5 + unit - self.party, 80, unit, target, &self.world);
            }
            _ => {
                self.script.context.camera_no = camera.map_or(0, |camera| camera.camera_no);
                self.script
                    .start_track_block(5 + unit - self.party, 80, unit, target, &self.world);
            }
        }
    }

    /// `BTL_VFX_REQ.UpdateTargetAveragePosition`: the targets' mean base x and z
    /// (`trgcpos`), integer division as the C#.
    fn base_center(&self, targets: &[usize]) -> [i16; 3] {
        let points: Vec<[i16; 3]> = targets
            .iter()
            .filter_map(|&index| self.world.actors.get(index))
            .map(|actor| actor.base)
            .collect();
        if points.is_empty() {
            return [0; 3];
        }
        let n = points.len() as i32;
        let x: i32 = points.iter().map(|p| i32::from(p[0])).sum();
        let z: i32 = points.iter().map(|p| i32::from(p[2])).sum();
        [(x / n) as i16, 0, (z / n) as i16]
    }

    /// `SeqSubTargetAveragePos`: the targets' mean x and z.
    fn center(&self, targets: &[usize]) -> [i16; 3] {
        let points: Vec<[i16; 3]> = targets
            .iter()
            .filter_map(|&index| self.world.actors.get(index))
            .map(|actor| actor.position)
            .collect();
        if points.is_empty() {
            return [0; 3];
        }
        let n = points.len() as i32;
        let x: i32 = points.iter().map(|p| i32::from(p[0])).sum();
        let z: i32 = points.iter().map(|p| i32::from(p[2])).sum();
        [(x / n) as i16, 0, (z / n) as i16]
    }
}

/// The camera a party member's effect starts (`SfxTriggerReaction`).
#[derive(Debug, Clone, PartialEq)]
enum Reaction {
    /// A set of the scene's camera block.
    Set(usize),
    /// A set of the plugin's built-in camera table.
    Plugin(std::sync::Arc<Vec<u8>>),
}

/// The reaction camera of a party member's effect with flag word `flags`
/// (`unk_18004F020`, read back by `SeqStateUpdateStart` as
/// `dword_18032318C`), or `None` when it plays none
/// (`u0010_SeqStateUpdate.cs` `SeqStateUpdateStart`, `u0009_EvalOrbitTrack.cs`
/// `SfxTriggerReaction`, `u0007` `RecomputeGate323268`):
/// - flag 0x8000 (Attack, Steal, Jump and 115 more) takes the other path: no
///   state reset and no reaction.
/// - the camera gate is off with Battle Camera: Fixed unless flag 0x8 forces it.
/// - a code (`flags & 0xFE`) whose high nibble is 0xF plays none.
/// - high nibble 1 or 7 (the summons) plays the plugin's own table: `+44` for
///   Garnet (`bi_slot_no == 2`), `+48` for anyone else.
/// - otherwise set 3 when it aims at a party member, set 4 when at an enemy.
///
/// Not modelled: the caster's statuses (`0x10000000`, `0x4000000` stop it;
/// Trance, `0x4000`, takes table `+52`/`+56`), the scene's `FixedCamera1/2`
/// flags and the context flags at `+64`.
fn reaction_camera(
    flags: u16,
    fixed_camera: bool,
    character: Option<u8>,
    at_party: bool,
    library: Option<&crate::battle_effects::EffectLibrary>,
) -> Option<Reaction> {
    if flags & 0x8000 != 0 || (fixed_camera && flags & 0x8 == 0) {
        return None;
    }
    let code = flags & 0xFE;
    match code >> 4 {
        0xF => None,
        1 | 7 => {
            let field = if character == Some(2) { 44 } else { 48 };
            library?.plugin_camera(field).map(Reaction::Plugin)
        }
        _ => Some(Reaction::Set(if at_party { 3 } else { 4 })),
    }
}

/// `eulerAngles.y * 11.3777781` (`SFX.cs` Get Angles): degrees in
/// 4096ths, as Unity reports them (0 to 360).
fn yaw(degrees: f32) -> i16 {
    (degrees.rem_euclid(360.0) * 11.377_778) as i16
}

/// `PsxCamera.PsxMatrix2UnityMatrix` and `PsxProj2UnityProj`: the PSX view
/// (y down) as a Unity `worldToCameraMatrix` and its projection distance.
pub fn unity_camera(camera: &PsxCamera) -> BattleCamera {
    let r = |row: usize, column: usize| camera.rotation[row][column] as f32 / 4096.0;
    let t = camera.translation.map(|value| value as f32);
    let rows = [
        [r(0, 0), -r(0, 1), r(0, 2), t[0]],
        [-r(1, 0), r(1, 1), -r(1, 2), -t[1]],
        [-r(2, 0), r(2, 1), -r(2, 2), -t[2]],
        [0.0, 0.0, 0.0, 1.0],
    ];
    BattleCamera {
        view: Mat4::from_cols_array_2d(&rows).transpose(),
        near: camera.h.max(1) as f32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reaction_cameras_follow_the_effects_flag_word() {
        // Steal (200), Jump (393) and Attack (100): 0x83FF, no reaction.
        assert_eq!(reaction_camera(0x83FF, false, Some(0), false, None), None);
        // Potion (8): 0x300, set 3 on a party member, 4 on an enemy.
        assert_eq!(
            reaction_camera(0x300, false, Some(1), true, None),
            Some(Reaction::Set(3))
        );
        assert_eq!(
            reaction_camera(0x300, false, Some(1), false, None),
            Some(Reaction::Set(4))
        );
        // A code with the high nibble 0xF plays none.
        assert_eq!(reaction_camera(0x00F0, false, Some(0), false, None), None);
        // Battle Camera: Fixed turns it off unless flag 0x8 forces it.
        assert_eq!(reaction_camera(0x3040, true, Some(0), false, None), None);
        assert_eq!(
            reaction_camera(0x3048, true, Some(0), false, None),
            Some(Reaction::Set(4))
        );
        // A summon's (nibble 1 or 7) comes from the plugin's table: none
        // without the plugin.
        assert_eq!(reaction_camera(0x3018, false, Some(2), false, None), None);
    }
}
