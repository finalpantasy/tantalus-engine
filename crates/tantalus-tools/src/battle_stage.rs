//! What a fought battle looked like, for a client to draw: the scene, the
//! units at their battle positions and a per-tick record of HP and ATB.
//! Runtime only (never serialized), filled by [`crate::field_battle`].
//!
//! Positions and turns follow Memoria `btl_init.cs`:
//! - `PutMonster`: an enemy that is not a multipart slave stands at
//!   `(Xpos, -Ypos, Zpos)` turned `Euler(0, Rot & 4095, 180)`.
//! - `SetupBattlePlayer` / `SetupBattlePlayerSingle`: players stand 632
//!   apart, centred on x 0 (the first at `(count - 1) * 632 / 2`), at height
//!   0 and z `PLAYER_ORIGINAL_Z` (-1560), 400 further back in the back row
//!   (`row == 0`), turned `Euler(0, 180, 180)` for a normal start.

use tantalus_assets::pc::battle_scene::{BattleScene, Placement};
use tantalus_assets::pc::battle_sequence::{SeqOp, SequenceFile};
use tantalus_legacy::battle::BattleUnit;
use tantalus_legacy::battle::machine::{Battle, BattleEvent};

/// `btl_init.PLAYER_ORIGINAL_Z`.
pub const PLAYER_ORIGINAL_Z: f32 = -1560.0;
/// `SetupBattlePlayer` gap between players.
pub const PLAYER_GAP: f32 = 632.0;
/// Battle logic ticks per second (`atb::BATTLE_TPS`).
pub const BATTLE_TICKS_PER_SECOND: f64 = 15.0;

/// One unit on the battlefield.
#[derive(Debug, Clone, PartialEq)]
pub enum StageModel {
    /// A party member: `CharacterId` and `CharacterSerialNumber` (the
    /// `BattleParameters.csv` row with its model and motions).
    Player { character: u8, serial: u8 },
    /// An enemy: `MonAddr[TypeNo].Geo` and the type's motions
    /// (`FF9BattleDB.Animation` ids, `BattleUnit.cs:876-881`): `mot[0]` the
    /// idle, `mot[2]` the hit, and `mot[3]` (flag 2, `useDieDmg`) or
    /// `mot[4]` the death.
    Enemy {
        geo: i16,
        idle: u16,
        hit: u16,
        death: u16,
    },
    /// A multipart slave: no model of its own.
    None,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StageUnit {
    pub model: StageModel,
    /// Battle space (Unity, y up).
    pub position: [f32; 3],
    /// Turn about the up axis in degrees (the `Euler` y).
    pub turn: f32,
    pub max_hp: u32,
    pub max_mp: u32,
    /// An enemy's `TypeNo`, which numbers its name in the battle text.
    pub type_no: Option<u8>,
    /// Display name (`FF9TextTool.BattleText(typeNo)` for enemies), when a
    /// caller looked it up.
    pub name: Option<String>,
}

/// The machine after one battle tick.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StageFrame {
    pub tick: u64,
    pub hp: Vec<u32>,
    pub mp: Vec<u32>,
    /// `cur.at` and `max.at` per unit.
    pub atb: Vec<(i16, i16)>,
    /// Units still in `btl_list`.
    pub present: Vec<bool>,
    /// `phase/menu-off sequence`.
    pub phase: String,
    /// What happened during the tick.
    pub events: Vec<StageEvent>,
    /// Each unit's `bi.row` (players: 0 back, 1 front), which Change flips.
    pub rows: Vec<u8>,
}

/// A machine event a client animates.
#[derive(Debug, Clone, PartialEq)]
pub enum StageEvent {
    /// `unit` starts a command (`CommandStarted`) aimed at `targets`.
    Command {
        unit: usize,
        cmd_no: u32,
        sub_no: i32,
        targets: Vec<usize>,
    },
    /// A command's effect landed (`EffectApplied`): `damage` is the HP
    /// lost (negative for a heal), 0 a miss or no change.
    Effect {
        caster: usize,
        target: usize,
        damage: i64,
    },
    Died {
        unit: usize,
    },
    /// `unit`'s command ended (`CommandFinished`).
    Finished {
        unit: usize,
        cmd_no: u32,
    },
    /// Jump: `unit` left the field.
    Jumped {
        unit: usize,
    },
    /// Spear landed: `unit` is back.
    Landed {
        unit: usize,
    },
    /// Eat or Cook on `target`: 0 cannot eat, 1 too strong, 2 tastes bad,
    /// 3 learned `ability`.
    Ate {
        caster: usize,
        target: usize,
        result: u8,
        ability: i32,
    },
    /// Steal on `target`: the item taken, or `None`.
    Stole {
        caster: usize,
        target: usize,
        item: Option<u32>,
    },
}

/// The [`StageEvent`]s in `log` (new machine log entries). `hp` is each
/// unit's HP after the previous events and is kept up to date.
pub fn stage_events(
    machine: &Battle,
    log: &[(u32, BattleEvent)],
    hp: &mut [u32],
) -> Vec<StageEvent> {
    let mut events = Vec::new();
    for (_, event) in log {
        match *event {
            BattleEvent::CommandStarted { unit, cmd_no } => {
                let slot = machine
                    .running
                    .iter()
                    .map(|&id| &machine.cmds.slots[id])
                    .find(|slot| slot.regist == Some(unit) && slot.cmd_no == cmd_no);
                let (sub_no, tar_id) = slot.map_or((0, 0), |slot| (slot.sub_no, slot.tar_id));
                let targets = machine
                    .units
                    .iter()
                    .enumerate()
                    .filter(|(_, other)| other.id & tar_id != 0)
                    .map(|(index, _)| index)
                    .collect();
                events.push(StageEvent::Command {
                    unit,
                    cmd_no,
                    sub_no,
                    targets,
                });
            }
            BattleEvent::EffectApplied {
                caster,
                target,
                target_hp,
            } => {
                let before = hp.get(target).copied().unwrap_or(target_hp);
                if let Some(slot) = hp.get_mut(target) {
                    *slot = target_hp;
                }
                events.push(StageEvent::Effect {
                    caster,
                    target,
                    damage: i64::from(before) - i64::from(target_hp),
                });
            }
            BattleEvent::UnitDied { unit } => events.push(StageEvent::Died { unit }),
            BattleEvent::CommandFinished { unit, cmd_no } => {
                events.push(StageEvent::Finished { unit, cmd_no });
            }
            BattleEvent::Jumped { unit } => events.push(StageEvent::Jumped { unit }),
            BattleEvent::Landed { unit } => events.push(StageEvent::Landed { unit }),
            BattleEvent::Ate {
                caster,
                target,
                result,
            } => {
                use tantalus_legacy::battle::calc::EatResult;
                let (result, ability) = match result {
                    EatResult::CannotEat => (0, 0),
                    EatResult::Failed => (1, 0),
                    EatResult::TasteBad => (2, 0),
                    EatResult::Yummy(ability) => (3, ability),
                };
                events.push(StageEvent::Ate {
                    caster,
                    target,
                    result,
                    ability,
                });
            }
            BattleEvent::Stole {
                caster,
                target,
                item,
            } => events.push(StageEvent::Stole {
                caster,
                target,
                item,
            }),
            _ => {}
        }
    }
    // Status ticks, items and script writes change HP outside effects.
    for (slot, unit) in hp.iter_mut().zip(&machine.units) {
        *slot = unit.cur_hp;
    }
    events
}

/// The animations of each attack sequence of a scene's `.raw17`
/// (`btlseq.SeqExecAnim`: `FF9BattleDB.Animation[AnmAddrList[AnmIDOfs +
/// code]]`), in order, by attack number (`RunSequence` plays
/// `SeqData[cmd.sub_no]`). Code 255 (the idle) is left out. The flag says
/// whether the sequence walks up to its target first (`MoveToTarget`).
pub fn attack_animations(bytes: &[u8]) -> Vec<(Vec<u16>, bool)> {
    let Ok(file) = SequenceFile::parse(bytes, "raw17") else {
        return Vec::new();
    };
    (0..file.slots.len())
        .map(|index| {
            let base = usize::from(file.slots[index].base_anim);
            let Some(sequence) = file.sequence(index) else {
                return (Vec::new(), false);
            };
            let mut anims = Vec::new();
            let mut moves = false;
            for placed in &sequence.ops {
                match placed.op {
                    SeqOp::Anim { code } if code != 255 => {
                        let id = file
                            .header
                            .anim_list
                            .get(base + usize::from(code))
                            .and_then(|&id| u16::try_from(id).ok());
                        if let Some(id) = id {
                            anims.push(id);
                        }
                    }
                    SeqOp::MoveToTarget { .. } => moves = true,
                    _ => {}
                }
            }
            (anims, moves)
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BattleStage {
    pub battle: i32,
    /// Bare scene name (`EF_E006`), the `BSC_` key without its prefix.
    pub scene: String,
    pub pattern: usize,
    /// `PatAddr[pattern].Camera`: default camera 0..=2, 3 and up random.
    pub camera: u8,
    /// Party first, then enemies, in machine order.
    pub units: Vec<StageUnit>,
    /// Each unit's `radius_collision` (empty: unknown, 0).
    pub radii: Vec<f32>,
    pub frames: Vec<StageFrame>,
    pub outcome: Option<String>,
    /// [`attack_animations`] of the scene's sequence file, by enemy attack
    /// number (`sub_no`).
    pub attack_anims: Vec<(Vec<u16>, bool)>,
    /// The sounds of each enemy attack ([`crate::battle_sound::sequence_sounds`]).
    pub sfx: Vec<Vec<crate::battle_sound::SequenceSfx>>,
    /// The scene's camera block (`camOffset` of the `.raw17`).
    pub camera_block: Option<tantalus_assets::pc::battle_camera::CameraBlock>,
    /// [`attack_cameras`] of the scene's sequence file.
    pub attack_cameras: Vec<Option<AttackCamera>>,
    /// `PatAddr[pattern].Monster[0]` x and z (camera anchor 11).
    pub put: [i16; 2],
    /// `SFX_INIT_PARAM.geo_radius` of an enemy: the first enemy's
    /// `radius_effect` (`FF9DBModelParameters` Radius), added to camera
    /// orbit distances. 0 when unknown.
    pub enemy_radius: i32,
    /// Spell and item effect ids and timing ([`crate::battle_effects`]).
    pub effects: std::sync::Arc<crate::battle_effects::EffectTable>,
}

/// A camera op in an enemy attack sequence: the frame it runs at, whether
/// it starts the enemy's camera (`RunCamera`, code 0x20) or only sets the
/// number, and `CameraNo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttackCamera {
    pub frame: u32,
    pub run: bool,
    pub camera_no: u8,
}

/// The first camera op of each attack sequence of a scene's `.raw17`.
pub fn attack_cameras(bytes: &[u8]) -> Vec<Option<AttackCamera>> {
    use tantalus_assets::pc::battle_sequence::{CameraOp, InterpretOptions, MarkerKind, interpret};
    let Ok(file) = SequenceFile::parse(bytes, "raw17") else {
        return Vec::new();
    };
    (0..file.slots.len())
        .map(|index| {
            let sequence = file.sequence(index)?;
            let options = InterpretOptions {
                base_anim: file.slots[index].base_anim,
                ..InterpretOptions::default()
            };
            interpret(sequence, &options)
                .markers
                .iter()
                .find_map(|marker| match marker.kind {
                    MarkerKind::Camera { op, id } => Some(AttackCamera {
                        frame: marker.frame,
                        run: op == CameraOp::Run,
                        camera_no: id,
                    }),
                    _ => None,
                })
        })
        .collect()
}

impl BattleStage {
    /// The scene's units: `party` (machine units) then the pattern's
    /// placements.
    pub fn new(
        battle: i32,
        scene_name: &str,
        scene: &BattleScene,
        pattern: usize,
        party: &[BattleUnit],
    ) -> Self {
        let mut units: Vec<StageUnit> = party
            .iter()
            .enumerate()
            .map(|(slot, unit)| StageUnit {
                model: StageModel::Player {
                    character: unit.character_id,
                    serial: unit.serial_no,
                },
                position: player_position(slot, party.len(), unit.row),
                turn: 180.0,
                max_hp: unit.max_hp,
                max_mp: unit.max_mp,
                type_no: None,
                name: None,
            })
            .collect();
        // `radius_collision`: 256 for players (`btl_init.cs:466`), the
        // monster type's `Radius` for enemies.
        let mut radii = vec![256.0; units.len()];
        if let Some(row) = scene.patterns.get(pattern) {
            let placements = row
                .monsters
                .iter()
                .take(usize::from(row.monster_count).min(row.monsters.len()));
            for placement in placements {
                units.push(enemy_unit(scene, placement));
                radii.push(
                    scene
                        .monster_types
                        .get(usize::from(placement.type_no))
                        .map_or(0.0, |kind| f32::from(kind.radius)),
                );
            }
        }
        Self {
            battle,
            scene: scene_name.to_owned(),
            pattern,
            camera: scene.patterns.get(pattern).map_or(0, |row| row.camera),
            units,
            radii,
            frames: Vec::new(),
            outcome: None,
            attack_anims: Vec::new(),
            sfx: Vec::new(),
            camera_block: None,
            attack_cameras: Vec::new(),
            put: scene
                .patterns
                .get(pattern)
                .map_or([0; 2], |row| [row.monsters[0].x, row.monsters[0].z]),
            enemy_radius: 0,
            effects: Default::default(),
        }
    }

    /// Records the machine units after a tick.
    pub fn record(
        &mut self,
        tick: u64,
        units: &[BattleUnit],
        max_atb: &[i16],
        present: &[bool],
        phase: &str,
    ) {
        self.frames.push(StageFrame {
            tick,
            hp: units.iter().map(|unit| unit.cur_hp).collect(),
            mp: units.iter().map(|unit| unit.cur_mp).collect(),
            atb: units
                .iter()
                .zip(max_atb)
                .map(|(unit, max)| (unit.cur_atb, *max))
                .collect(),
            present: present.to_vec(),
            phase: phase.to_owned(),
            events: Vec::new(),
            rows: units.iter().map(|unit| unit.row).collect(),
        });
    }

    /// The frame shown `seconds` into the battle.
    pub fn frame_at(&self, seconds: f64) -> Option<&StageFrame> {
        let index = (seconds.max(0.0) * BATTLE_TICKS_PER_SECOND) as usize;
        self.frames
            .get(index.min(self.frames.len().checked_sub(1)?))
    }

    /// Names the enemies from the scene's battle text: entry `TypeNo` is
    /// the type's name (`btl_init.cs:115`, `BattleHUD.cs:1134`).
    pub fn name_enemies(&mut self, text: &[String]) {
        for unit in &mut self.units {
            if let Some(name) = unit
                .type_no
                .and_then(|type_no| text.get(usize::from(type_no)))
                .filter(|name| !name.is_empty())
            {
                unit.name = Some(name.clone());
            }
        }
    }

    /// Seconds the record lasts.
    pub fn duration(&self) -> f64 {
        self.frames.len() as f64 / BATTLE_TICKS_PER_SECOND
    }
}

/// Where party member `slot` of `count` stands; `row` 0 is the back row.
pub fn player_position(slot: usize, count: usize, row: u8) -> [f32; 3] {
    let first = (count.saturating_sub(1) as i32 * PLAYER_GAP as i32 / 2) as f32;
    let back = if row == 0 { -400.0 } else { 0.0 };
    [
        first - slot as f32 * PLAYER_GAP,
        0.0,
        PLAYER_ORIGINAL_Z + back,
    ]
}

fn enemy_unit(scene: &BattleScene, placement: &Placement) -> StageUnit {
    let kind = scene.monster_types.get(usize::from(placement.type_no));
    let slave = placement.type_no > 0 && placement.flags & Placement::FLG_MULTIPART != 0;
    StageUnit {
        model: match kind {
            Some(kind) if !slave && kind.geo >= 0 => StageModel::Enemy {
                geo: kind.geo,
                idle: kind.mot[0],
                hit: kind.mot[2],
                death: kind.mot[if kind.flags & 2 != 0 { 3 } else { 4 }],
            },
            _ => StageModel::None,
        },
        position: [
            f32::from(placement.x),
            -f32::from(placement.y),
            f32::from(placement.z),
        ],
        turn: f32::from(placement.rot & 4095),
        max_hp: kind.map_or(0, |kind| kind.max_hp),
        max_mp: kind.map_or(0, |kind| kind.max_mp),
        type_no: Some(placement.type_no),
        name: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn players_stand_632_apart_around_the_centre() {
        let xs: Vec<f32> = (0..3).map(|slot| player_position(slot, 3, 1)[0]).collect();
        assert_eq!(xs, vec![632.0, 0.0, -632.0]);
        assert_eq!(player_position(0, 4, 1), [948.0, 0.0, -1560.0]);
        // Back row stands 400 further back.
        assert_eq!(player_position(1, 2, 0), [-316.0, 0.0, -1960.0]);
    }

    #[test]
    fn frames_play_at_the_battle_tick_rate() {
        let mut stage = BattleStage::default();
        for tick in 0..30 {
            stage.record(tick, &[], &[], &[], "p");
        }
        assert_eq!(stage.duration(), 2.0);
        assert_eq!(stage.frame_at(1.0).map(|frame| frame.tick), Some(15));
        assert_eq!(stage.frame_at(9.0).map(|frame| frame.tick), Some(29));
    }
}
