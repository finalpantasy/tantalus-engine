//! Plays the scene's battle cameras over a stage record: the opening,
//! the default camera, the reaction camera of each command and an enemy's
//! own camera (`tantalus_assets::pc::battle_camera`, the special-effect
//! plugin's camera script), one script frame per battle tick.
//!
//! When the host starts which camera:
//! - `SFX.StartPlungeCamera` at the start: set 0, its variant
//!   `CameraNo` (`PatAddr[PatNum].Camera`; 3 and up are random, 0 here).
//! - At an effect's start (`SeqStateUpdate` -> `SfxStateReset`,
//!   `SfxTriggerReaction`): a party member's command takes set 3 when it
//!   aims at the party and 4 otherwise; an enemy's takes its own set
//!   (5 + slot). Here the effect starts with the command.
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
    /// The unit whose command holds the camera.
    busy: Option<usize>,
    victory: bool,
    party: usize,
}

impl CameraDirector {
    /// `heights` are the units' model heights (battle units). `None` when
    /// the scene has no camera block.
    pub fn new(stage: &BattleStage, heights: &[f32]) -> Option<Self> {
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
                yaw: (unit.turn.rem_euclid(360.0) / 360.0 * 4096.0) as i16,
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
        Some(Self {
            script,
            world,
            ticks: 0,
            camera: None,
            pending: Vec::new(),
            busy: None,
            victory: false,
            party,
        })
    }

    /// The camera shown at stage frame `tick` (`None` until the script
    /// has placed it). Runs forward from the last call.
    /// `offsets` gives each unit's offset from its battle position at a
    /// tick (the camera follows a unit that steps forward or runs).
    pub fn camera_at(
        &mut self,
        stage: &BattleStage,
        tick: usize,
        offsets: &dyn Fn(usize) -> Vec<[f32; 3]>,
    ) -> Option<BattleCamera> {
        let last = stage.frames.len();
        while self.ticks <= tick && self.ticks < last.max(1) {
            let now = offsets(self.ticks);
            for (index, actor) in self.world.actors.iter_mut().enumerate() {
                let unit = &stage.units[index];
                let offset = now.get(index).copied().unwrap_or([0.0; 3]);
                actor.position = [
                    (unit.position[0] + offset[0]) as i16,
                    -(unit.position[1] + offset[1]) as i16,
                    (unit.position[2] + offset[2]) as i16,
                ];
            }
            self.step(stage, self.ticks);
            self.ticks += 1;
        }
        self.camera.map(|camera| unity_camera(&camera))
    }

    /// Ticks run so far.
    pub fn ticks(&self) -> usize {
        self.ticks
    }

    fn step(&mut self, stage: &BattleStage, tick: usize) {
        let mut due = Vec::new();
        self.pending.retain(|entry| {
            if entry.0 <= tick {
                due.push(entry.clone());
                false
            } else {
                true
            }
        });
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
                    } => self.command(stage, tick, *unit, *cmd_no, *sub_no, targets),
                    StageEvent::Finished { unit, .. } if self.busy == Some(*unit) => {
                        self.busy = None;
                        self.pending.clear();
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
        tick: usize,
        unit: usize,
        cmd_no: u32,
        sub_no: i32,
        targets: &[usize],
    ) {
        let target = targets.first().copied();
        if unit < self.party {
            // A drawn spell keeps the default camera on the scene: its own
            // effect cameras are not read.
            let drawn = stage
                .effects
                .effect_of(cmd_no, sub_no, targets.len())
                .is_some_and(|(id, _)| crate::battle_effects::EffectKind::of(id).is_some());
            if drawn {
                return;
            }
            self.busy = Some(unit);
            let set = if target.is_some_and(|target| target < self.party) {
                3
            } else {
                4
            };
            self.script
                .start_track_block(set, 80, unit, target, &self.world);
            return;
        }
        self.busy = Some(unit);
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
            _ => {
                self.script.context.camera_no = camera.map_or(0, |camera| camera.camera_no);
                self.script
                    .start_track_block(5 + unit - self.party, 80, unit, target, &self.world);
            }
        }
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
