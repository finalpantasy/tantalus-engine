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
    /// Each unit's Trance gauge (`trance`, 0 to 255) and whether it is in
    /// Trance; `None` without a gauge (`HasTrance`).
    pub trance: Vec<Option<(u8, bool)>>,
    /// Each unit's `stat.cur` (`BattleStatus` bits), for the status effects drawn on it.
    pub status: Vec<u64>,
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
        /// `info.short_summon`: a summon plays its short effect.
        short_summon: bool,
        /// Ticks from this frame to the command's effect, as the machine
        /// schedules it when the command starts (`Battle::effect_delay`):
        /// a live view starts the effect on time without waiting for the
        /// `Effect` event. `None` when the machine has no running slot.
        effect_in: Option<u32>,
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
    /// An enemy's `RunCamera` 0x12 ran; `play` says whether its camera
    /// plays (`SFX.ShouldPlayAlternateCamera`).
    AlternateCamera {
        unit: usize,
        play: bool,
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
                let id = machine.running.iter().copied().find(|&id| {
                    let slot = &machine.cmds.slots[id];
                    slot.regist == Some(unit) && slot.cmd_no == cmd_no
                });
                let slot = id.map(|id| &machine.cmds.slots[id]);
                let effect_in = id.map(|id| machine.effect_delay(id));
                let (sub_no, tar_id, short_summon) = slot.map_or((0, 0, false), |slot| {
                    (slot.sub_no, slot.tar_id, slot.short_summon)
                });
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
                    short_summon,
                    effect_in,
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
            BattleEvent::AlternateCamera { unit, play } => {
                events.push(StageEvent::AlternateCamera { unit, play });
            }
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

/// The turns of one enemy attack sequence (`SeqExecTurn`,
/// `btlseq.cs:839`): toward the targets' average position (`0x8001`)
/// with its offset in degrees and its frames, and back to the battle
/// rotation (`0x8000`) with its frames.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AttackTurns {
    pub toward: Option<(f32, u8)>,
    pub back: Option<u8>,
}

/// [`AttackTurns`] of every sequence in the scene's sequence file, by
/// attack number.
pub fn attack_turns(bytes: &[u8]) -> Vec<AttackTurns> {
    use tantalus_assets::pc::battle_sequence::TurnTarget;
    let Ok(file) = SequenceFile::parse(bytes, "raw17") else {
        return Vec::new();
    };
    (0..file.slots.len())
        .map(|index| {
            let mut turns = AttackTurns::default();
            for placed in file.sequence(index).map_or(&[][..], |s| &s.ops[..]) {
                if let SeqOp::Turn {
                    target,
                    offset,
                    frames,
                } = placed.op
                {
                    match target {
                        TurnTarget::FaceTarget if turns.toward.is_none() => {
                            turns.toward = Some((f32::from(offset) / 4096.0 * 360.0, frames));
                        }
                        TurnTarget::BattleRotation if turns.toward.is_some() => {
                            turns.back = Some(frames);
                        }
                        _ => {}
                    }
                }
            }
            turns
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
    /// Each enemy's `tar_bone` (`btl_init.cs:163`: `Bone[3]` of its type),
    /// the bone its damage numbers and the target cursor sit on
    /// (`btl2d.GetFreeEntry`); `None` for the party.
    pub tar_bones: Vec<Option<u8>>,
    /// Each unit's `mesh_banish` (`btl_init.cs:162`: `Mesh[1]` of an enemy's
    /// type, 65535 for the party), the meshes Vanish hides; 65535 too for an
    /// enemy drawn with a party model (`btl_mot.HideMesh`,
    /// `ModelFactory.IsUseAsEnemyCharacter`).
    pub banish_meshes: Vec<u16>,
    /// Where each unit's status icons and effects sit (`btl2d.GetIconPosition`): the party's
    /// from their battle parameters, an enemy's from its type (`ENEMY_TYPE.icon_*`).
    pub status_icons: Vec<Option<tantalus_assets::pc::battle_status_sps::StatusIcons>>,
    pub frames: Vec<StageFrame>,
    pub outcome: Option<String>,
    /// [`attack_animations`] of the scene's sequence file, by enemy attack
    /// number (`sub_no`).
    pub attack_anims: Vec<(Vec<u16>, bool)>,
    /// [`attack_turns`] of the scene's sequence file.
    pub attack_turns: Vec<AttackTurns>,
    /// Config Battle Camera: Fixed (`cfg.camera == 1`).
    pub fixed_camera: bool,
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
    /// The special effect each enemy attack sequence starts, by attack
    /// number (`sub_no`): [`attack_vfx`].
    pub attack_effects: Vec<Option<AttackVfx>>,
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
    /// Op 0x12: it runs only when `ShouldPlayAlternateCamera` says so
    /// ([`StageEvent::AlternateCamera`]).
    pub alternate: bool,
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
                        alternate: op == CameraOp::RunIfAlternate,
                        camera_no: id,
                    }),
                    _ => None,
                })
        })
        .collect()
}

/// The first effect an enemy attack sequence starts: the `Vfx` op
/// (`SeqExecVfx`: `SetBattleVfx(effect, [bone0, bone1, arg0, flag])`) or
/// the `SVfx` timer (`SeqExecSVfx`: `[param, 0, 0, 0]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttackVfx {
    pub effect: u16,
    /// `monbone[0]`, `monbone[1]` and `arg0` of the request.
    pub args: [i16; 3],
}

/// Per attack slot: `None` when the sequence file has no sequence for it
/// (`seq_work_set.SeqData[n] == 0`, `btlseq.cs:89`), otherwise whether the
/// sequence reaches an effect point: a `Calc` op (`SeqExecCalc` ->
/// `ExecVfxCommand`) or an effect (`Vfx`, `SVfx`, whose `EffectPoint` runs
/// `ExecVfxCommand`, `UnifiedBattleSequencer.cs:971-992`). Unparsable bytes
/// give an empty list.
pub fn attack_sequence_effects(bytes: &[u8]) -> Vec<Option<bool>> {
    let Ok(file) = SequenceFile::parse(bytes, "raw17") else {
        return Vec::new();
    };
    (0..file.slots.len())
        .map(|index| {
            file.sequence(index).map(|sequence| {
                sequence.ops.iter().any(|placed| {
                    matches!(
                        placed.op,
                        SeqOp::Calc | SeqOp::Vfx { .. } | SeqOp::SVfx { .. }
                    )
                })
            })
        })
        .collect()
}

/// Per sequence in the scene's sequence file, the stand animation its last
/// "Change Stand Animation" op (code 17) sets: `true` for the alternate one
/// (`BattleActionThread.cs:369`), `None` without the op or the sequence.
pub fn attack_stand_animations(bytes: &[u8]) -> Vec<Option<bool>> {
    let Ok(file) = SequenceFile::parse(bytes, "raw17") else {
        return Vec::new();
    };
    (0..file.slots.len())
        .map(|index| {
            file.sequence(index).and_then(|sequence| {
                sequence
                    .ops
                    .iter()
                    .rev()
                    .find_map(|placed| match placed.op {
                        SeqOp::DefaultIdle { enabled } => Some(enabled != 0),
                        _ => None,
                    })
            })
        })
        .collect()
}

/// [`AttackVfx`] of every sequence in the scene's sequence file.
pub fn attack_vfx(bytes: &[u8]) -> Vec<Option<AttackVfx>> {
    use tantalus_assets::pc::battle_sequence::{InterpretOptions, MarkerKind, interpret};
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
                    MarkerKind::VfxStart { effect, args, .. } => Some(AttackVfx { effect, args }),
                    MarkerKind::SpecialVfxTrigger { effect, param } => Some(AttackVfx {
                        effect,
                        args: [i16::from(param), 0, 0],
                    }),
                    _ => None,
                })
        })
        .collect()
}

impl BattleStage {
    /// Each party `SysTrans` (`cmd_no` 59): its unit, the tick it starts,
    /// and whether it enters Trance (the unit's status then). The model
    /// changes [`SYS_TRANS_MODEL_FRAME`] ticks in (`btl_vfx.SetTranceModel`).
    ///
    /// [`SYS_TRANS_MODEL_FRAME`]: tantalus_legacy::battle::machine::SYS_TRANS_MODEL_FRAME
    pub fn trance_changes(&self) -> Vec<(usize, usize, bool)> {
        let mut out = Vec::new();
        for (tick, frame) in self.frames.iter().enumerate() {
            for event in &frame.events {
                if let StageEvent::Command { unit, cmd_no, .. } = event
                    && *cmd_no == tantalus_legacy::battle::queue::cmd::SYS_TRANS
                    && let Some(Some((_, entering))) = frame.trance.get(*unit)
                {
                    out.push((*unit, tick, *entering));
                }
            }
        }
        out
    }

    /// Whether `unit` shows its Trance model at stage tick `tick`.
    pub fn shows_trance_model(&self, unit: usize, tick: f64) -> bool {
        let swap = f64::from(tantalus_legacy::battle::machine::SYS_TRANS_MODEL_FRAME);
        self.trance_changes()
            .into_iter()
            .rfind(|&(changed, start, _)| changed == unit && start as f64 + swap <= tick)
            .is_some_and(|(_, _, entering)| entering)
    }

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
        let mut tar_bones = vec![None; units.len()];
        let mut banish_meshes = vec![u16::MAX; units.len()];
        let mut status_icons: Vec<_> = party
            .iter()
            .map(|unit| {
                tantalus_assets::pc::battle_status_sps::PLAYER_ICONS
                    .get(usize::from(unit.serial_no))
                    .copied()
            })
            .collect();
        if let Some(row) = scene.patterns.get(pattern) {
            let placements = row
                .monsters
                .iter()
                .take(usize::from(row.monster_count).min(row.monsters.len()));
            for placement in placements {
                units.push(enemy_unit(scene, placement));
                tar_bones.push(
                    scene
                        .monster_types
                        .get(usize::from(placement.type_no))
                        .map(|kind| kind.bone[3]),
                );
                banish_meshes.push(
                    scene
                        .monster_types
                        .get(usize::from(placement.type_no))
                        .map_or(u16::MAX, |kind| {
                            // `HideMesh(mesh_banish, isVanish: true)` hides every mesh of a
                            // party model used as an enemy.
                            if tantalus_assets::pc::battle_models::is_use_as_enemy_character(
                                i64::from(kind.geo),
                            ) {
                                u16::MAX
                            } else {
                                kind.mesh[1]
                            }
                        }),
                );
                status_icons.push(scene.monster_types.get(usize::from(placement.type_no)).map(
                    |kind| tantalus_assets::pc::battle_status_sps::StatusIcons {
                        bone: kind.icon_bone,
                        y: kind.icon_y,
                        z: kind.icon_z,
                    },
                ));
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
            tar_bones,
            banish_meshes,
            status_icons,
            frames: Vec::new(),
            outcome: None,
            attack_anims: Vec::new(),
            attack_turns: Vec::new(),
            fixed_camera: false,
            sfx: Vec::new(),
            camera_block: None,
            attack_cameras: Vec::new(),
            attack_effects: Vec::new(),
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
            trance: units
                .iter()
                .map(|unit| {
                    unit.has_trance_gauge.then(|| {
                        (
                            unit.trance,
                            unit.is_under_any_status(tantalus_legacy::battle::status::TRANCE),
                        )
                    })
                })
                .collect(),
            status: units.iter().map(|unit| unit.status_current).collect(),
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
