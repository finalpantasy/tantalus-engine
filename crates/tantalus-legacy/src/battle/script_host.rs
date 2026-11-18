//! Battle AI host and driver (milestone B6, part 2): runs decoded
//! `EVT_BATTLE_*` functions against a mutable view of the battle so enemy
//! scripts pick actions.
//!
//! Sources (Memoria pin `c716e1fa`, paths under `Assembly-CSharp/`):
//! * `Global/Event/Engine/EventEngine.cs` `RequestAction` (lines 285-325),
//!   `Request`/`Call`/`Return`, `GetSysList`/`SetSysList`, `ProcessCodeExt`,
//!   `GetBattleCharData`/`SetBattleCharData`, the object list (`addObjPtrList`,
//!   `_opLStart`) and the battle init (`InitBattle`, lines 548-595).
//! * `Global/Event/Engine/EventEngine.ProcessEvents.cs` (the `btlchk == 2`
//!   loop that starts the ATB function with `Request(obj, 3, 5)`).
//! * `Global/Event/Engine/EventEngine.DoEventCode.cs` `BTLCMD` (0x38),
//!   `BTLSET` (0x4A), `MESB` (0xD0), `BTLSEQ` (0xE5).
//! * `Global/EBin.cs` (`ProcessCode`, instance variable buffer, `EnterFunctionLoop`).
//! * `Global/btl_scrp.cs` (`GetBattleData`, `SetBattleData`, `GetCurrentCommandData`,
//!   `SetCurrentCommandData`, `GetBattleID`).
//! * `Global/Objects/ObjTable.cs` (vanilla `player_link` from entry index).
//!
//! Function selection, as the source does it (tags are `Request(obj, level, tag)`):
//!
//! | event | tag | level | caller |
//! |---|---|---|---|
//! | init | 0 | 0 | object creation (`Obj` ctor sets `ip = GetIP(sid, 0)`) |
//! | main loop | 1 | 7 | after tag 0 returns (`Return`, `stateInit`) |
//! | ATB ready | 5 | 3 | `btlchk == 2` loop after `RequestAction(EnemyAtk)` |
//! | counter | 6 | 1 | `RequestAction(EnemyCounter)` |
//! | reaction | 7 | 2 | `RequestAction(EnemyReaction)` |
//! | dying | 9 | 0 | `RequestAction(EnemyDying)` |
//!
//! A request is accepted only when `level < obj.level` and the entry has the
//! tag; it pushes a frame and the previous function resumes when it returns.
//!
//! Not ported (listed, not guessed):
//! * Timing between `HonoluluBattleMain` and `ServiceEvents` inside one frame
//!   is not reference-checked: this driver advances every object once per
//!   [`BattleScriptDriver::tick`] and resumes a lower frame on the tick after
//!   a higher one returns.
//! * Cross-object requests (`RequestScript*`): none of the 562 battle scripts
//!   contain one; the host answers them with the zero-state defaults.
//! * `RunBattleCode` codes other than 38 (gil), 43 and 44 only record an
//!   event; [`EnemyAiHooks`] hands 32-35 to the machine each frame
//!   ([`Battle::battle_code`]); 36, 37, 39 and 40 are not applied.
//! * No effect plugin runs, so the SFX callback 112 flag that some scripts
//!   wait on (`gEventGlobal[199] |= 16`, `SFX.cs:879-881`, raised mid-effect
//!   for Moonlight Slash, Kuja's Transform and others) is raised when a
//!   special attack is queued.
//! * System-list element 6 / `GetCurrentCommandData` kinds that read ability
//!   data (0-16, 1003-1009, 1011-1024) record [`AiEvent::UnmodeledRead`] and
//!   answer 0; kinds 17-19, 1000-1002 and 1010 are modeled.
//! * The battle hook gets `&Battle`, so script writes to unit members stay in
//!   the host view; [`EnemyAiHooks::take_dirty_views`] returns them for
//!   [`super::host::apply_unit_views`].
//! * Sysvars 1-23, 191+ (field context) answer 0, as in [`super::host`].
//! * The AI random draws come from the driver's own [`BattleRandom`], not the
//!   machine's stream (the hook is not handed the machine RNG).

use super::host::{
    BattleDataView, CurrentCommandView, UnitView, battle_sysvar, battle_views, character_data,
    set_character_data,
};
use super::machine::{Battle, BattleHooks, EnemyAction, EnemyReactionKind, EnemyReactionRequest};
use super::queue::cmd;
use super::rng::BattleRandom;
use crate::event::{
    EventBinary, EventProgram, EventVm, HostEvent, KeyState, LegacyEventHost, ObjectMemberRef,
    VariableMemory, VariableScope, VariableType, VmError, VmStatus,
};
use std::collections::BTreeMap;
use tantalus_contracts::{EngineError, EntityId, Tick};

/// Function tags (`GetIP(sid, tag)`).
pub mod tag {
    pub const INIT: u16 = 0;
    pub const LOOP: u16 = 1;
    pub const ATB: u16 = 5;
    pub const COUNTER: u16 = 6;
    pub const REACTION: u16 = 7;
    pub const DYING: u16 = 9;
}

/// Script levels (`Request(obj, level, tag)`).
pub mod level {
    pub const DYING: u8 = 0;
    pub const COUNTER: u8 = 1;
    pub const REACTION: u8 = 2;
    pub const ATB: u8 = 3;
    /// `cEventLevelN - 1`.
    pub const IDLE: u8 = 7;
}

/// The command a `BTLCMD` (opcode 0x38) queues, by the level it ran at
/// (`DoEventCode.cs:1177-1215`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnemyCommandKind {
    /// Level 3: `SetEnemyCommand(EnemyAtk)`.
    Attack,
    /// Level 1: `SetEnemyCommand(EnemyCounter)`.
    Counter,
    /// Level 0: `SetEnemyCommand(EnemyDying)`.
    Dying,
}

/// A typed `SetEnemyCommand` call (`btl_cmd.cs:346`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnemyCommand {
    /// `SysList(1)`: the running object's `btl_id` bit.
    pub caster: u16,
    pub kind: EnemyCommandKind,
    /// `sub_no`: the enemy attack index.
    pub attack: i32,
    /// `SysList(0)` at the instruction (`(UInt16)` cast).
    pub target_mask: u16,
}

/// What the scripts asked the battle to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiEvent {
    Command(EnemyCommand),
    /// Level 3 `BTLCMD` skipped because a status script auto-attacks
    /// (`IAutoAttackStatusScript.OnATB`).
    AttackSuppressed {
        caster: u16,
        attack: i32,
    },
    /// `BTLCMD` at level 2 (reaction) or above 3 has no case in the source.
    AttackIgnored {
        caster: u16,
        level: u8,
        attack: i32,
    },
    /// `BTLSEQ`: `btlseq.StartBtlSeq(SysList(1), SysList(0), attack)`.
    SpecialAttack {
        caster: u16,
        target_mask: u16,
        attack: i32,
    },
    /// `BTLSET`: `SetBattleData(code, value)`.
    BattleCode {
        code: u32,
        value: i32,
    },
    /// `MESB`: battle text id.
    Message {
        text: i32,
    },
    /// `BEND`.
    TerminateBattle,
    /// Member 71 write (`btl_scrp.SetCharacterData` case 71):
    /// `btl_sys.SetBonus(btl)` pays the unit's gil, EXP and drops, as its
    /// death would. Scripted bosses that never reach 0 HP pay this way.
    SetBonus {
        unit: EntityId,
    },
    /// A read or write the port does not model; the value read was 0.
    UnmodeledRead {
        what: &'static str,
        id: i32,
    },
    /// An `InitializeObject` source the engine maps through the party table.
    UnmodeledSpawn {
        source: u8,
    },
}

/// Mutable battle view the host answers from.
#[derive(Debug, Clone, Default)]
pub struct BattleScriptState {
    pub data: BattleDataView,
    /// Units keyed by `EntityId::new(btl_id)`.
    pub units: BTreeMap<EntityId, UnitView>,
    /// `_objPtrList`: entry index per object slot (bit `1 << slot` is the
    /// object's `btl_id`).
    pub objects: [Option<usize>; 8],
    /// `_sysList` raw stores (elements 2, 3, 4 and 6 are computed on read).
    pub sys_list: [u16; 8],
    /// `gEventGlobal`.
    pub global: Option<VariableMemory>,
    /// `EventContext.mapvar`.
    pub map: Option<VariableMemory>,
    /// `btl_id` mask of units whose status script auto-attacks on ATB.
    pub auto_attack_mask: u16,
    /// `gMemberTarget`: the object slot last selected (`None` = null object).
    pub member_target: Option<usize>,
    /// `_btlCmdPrmCmd`/`_btlCmdPrmSub`, set by the last non-ATB `RequestAction`.
    pub cmd_params: (i32, i32),
    /// Everything the scripts asked for, in order.
    pub events: Vec<AiEvent>,
    spawns: Vec<(u8, u8)>,
}

impl BattleScriptState {
    pub fn new(data: BattleDataView, units: BTreeMap<EntityId, UnitView>) -> Self {
        Self {
            data,
            units,
            global: Some(VariableMemory::global()),
            map: Some(VariableMemory::map()),
            ..Self::default()
        }
    }

    /// Index of the unit a member read or write resolves to
    /// (`GetBattleCharData`: the first list index whose object is the target;
    /// a null target matches the first empty slot).
    fn member_unit_bit(&self) -> Option<u16> {
        let target = self.member_target.and_then(|slot| self.objects[slot]);
        (0..8)
            .find(|&index| self.objects[index] == target)
            .map(|index| 1u16 << index)
    }

    fn member_entity(&self) -> EntityId {
        self.member_unit_bit()
            .map_or(EntityId::new(u64::MAX), |bit| EntityId::new(u64::from(bit)))
    }
}

/// Fields the scene supplies that the machine does not carry.
#[derive(Debug, Clone, Default)]
pub struct SceneOverlay {
    pub battle_map_no: i32,
    pub pattern_id: i32,
    pub start_type: u8,
    pub gil: u32,
    pub bonus_ap: u16,
    pub load_status: u32,
    /// Script frames that read `load_status` as 0 before it takes its
    /// value. `battle.BattleLoadLoop` sets `LOAD_INITNPC`/`LOAD_INITCHR`
    /// some frames after the scripts start, so objects spawned by entry 0
    /// reach their load waits before the bits appear and pass them in the
    /// same frame as entry 0 (EF_E006's watcher relies on that order).
    pub load_frames: u32,
    /// The field clock (`TimerUI`) when it is shown at the start of the fight.
    pub timer: Option<BattleTimer>,
}

/// `TimerUI` as the fight starts: `TimerUI.Update` keeps counting down in real time while
/// the battle runs, so the battle scripts see it fall at the battle tick rate.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BattleTimer {
    /// `TimerUI.Time` in seconds.
    pub time: f32,
    /// `TimerUI.Play`.
    pub running: bool,
}

impl BattleTimer {
    /// `TimerUI.Time` after `ticks` battle ticks, floored at zero.
    pub fn time_after(&self, ticks: u32) -> f32 {
        if !self.running {
            return self.time;
        }
        let elapsed = ticks as f32 / super::atb::BATTLE_TPS as f32;
        (self.time - elapsed).max(0.0)
    }
}

/// Snapshots the machine into script state: [`battle_views`] plus the id
/// masks (`GetBattleID`, live list members only), the running command
/// (`cur_cmd`) and the scene overlay.
pub fn sync_state_from_battle(
    state: &mut BattleScriptState,
    battle: &Battle,
    overlay: &SceneOverlay,
) {
    let (mut data, units) = battle_views(battle);
    for (unit, sched) in battle.units.iter().zip(&battle.sched) {
        if !sched.in_list {
            continue;
        }
        data.all_id_mask |= u32::from(unit.id);
        if unit.is_player {
            data.player_id_mask |= u32::from(unit.id);
        } else {
            data.enemy_id_mask |= u32::from(unit.id);
        }
    }
    data.battle_map_no = overlay.battle_map_no;
    data.pattern_id = overlay.pattern_id;
    data.start_type = overlay.start_type;
    data.gil = overlay.gil;
    data.bonus_ap = overlay.bonus_ap;
    data.load_status = overlay.load_status;
    if let Some(timer) = overlay.timer {
        data.timer_shown = true;
        data.timer_seconds = timer.time_after(battle.tick_no).round_ties_even() as i32;
    }
    data.current_command = battle.running.first().map(|&id| {
        let slot = &battle.cmds.slots[id];
        CurrentCommandView {
            cmd_no: slot.cmd_no as i32,
            sub_no: slot.sub_no,
            regist_btl_id: slot.regist.map_or(0, |unit| battle.units[unit].id as u8),
            tar_id: u32::from(slot.tar_id),
            weapon_element: 0,
            cmd_element: 0,
            effect_counter: 0,
        }
    });
    state.data = data;
    state.units = units;
}

/// `LegacyEventHost` answering the battle sysvars 24-44, system-list elements
/// 0-6, battle-character members and the battle instructions.
pub struct BattleScriptHost<'a> {
    state: &'a mut BattleScriptState,
    rng: &'a mut dyn BattleRandom,
    instance: &'a mut VariableMemory,
    /// `gExec.level` of the running function.
    level: u8,
}

impl<'a> BattleScriptHost<'a> {
    pub fn new(
        state: &'a mut BattleScriptState,
        rng: &'a mut dyn BattleRandom,
        instance: &'a mut VariableMemory,
        level: u8,
    ) -> Self {
        Self {
            state,
            rng,
            instance,
            level,
        }
    }

    fn running(&self) -> u16 {
        self.state.sys_list[1]
    }

    /// `btl_scrp.GetCurrentCommandData` for element 6 reads.
    fn command_data(&mut self, kind: u16) -> u16 {
        let Some(command) = self.state.data.current_command.as_ref() else {
            return 0;
        };
        match kind {
            17 | 1000 => command.tar_id as u16,
            18 | 1001 => command.cmd_no as u16,
            19 | 1002 => command.sub_no as u16,
            1010 => u16::from(command.cmd_element),
            0..=16 | 1003..=1009 | 1011..=1024 => {
                self.state.events.push(AiEvent::UnmodeledRead {
                    what: "current command data",
                    id: i32::from(kind),
                });
                0
            }
            _ => 0,
        }
    }

    /// `btl_scrp.SetCurrentCommandData` for element 6 writes.
    fn set_command_data(&mut self, kind: u16, value: i32) {
        let Some(command) = self.state.data.current_command.as_mut() else {
            return;
        };
        match kind {
            17 | 1000 => command.tar_id = u32::from(value as u16),
            18 | 1001 => command.cmd_no = value,
            19 | 1002 => command.sub_no = value,
            1010 => command.cmd_element = value as u8,
            0..=16 | 1003..=1009 | 1011..=1024 => {
                self.state.events.push(AiEvent::UnmodeledRead {
                    what: "current command data write",
                    id: i32::from(kind),
                });
            }
            _ => {}
        }
    }

    /// `SetBattleData` for the codes whose effect is state the scripts read
    /// back; the rest only record.
    fn battle_code(&mut self, code: u32, value: i32) {
        match code {
            38 => {
                // `party.gil += (UInt32)val`, capped at 9,999,999.
                self.state.data.gil = self
                    .state
                    .data
                    .gil
                    .wrapping_add(value as u32)
                    .min(9_999_999);
            }
            43 => {
                if let Some(command) = self.state.data.current_command.as_mut() {
                    command.effect_counter = value as u32;
                }
            }
            44 => self.state.data.bonus_ap = value as u16,
            _ => {}
        }
        self.state.events.push(AiEvent::BattleCode { code, value });
    }
}

impl LegacyEventHost for BattleScriptHost<'_> {
    fn apply(&mut self, events: &[HostEvent]) -> Result<(), EngineError> {
        // Battle instructions were handled in `note_event`.
        for event in events {
            if let HostEvent::InitializeObject { source, unique_id } = event {
                self.state.spawns.push((*source, *unique_id));
            }
        }
        Ok(())
    }

    fn note_event(&mut self, event: &HostEvent) {
        let caster = self.running();
        let target_mask = self.state.sys_list[0];
        match event {
            HostEvent::QueueEnemyAttack { attack } => {
                let attack = *attack;
                let record = match self.level {
                    level::ATB => {
                        // `gExec.btlchk = 0` happens before the auto-attack check.
                        if self.state.auto_attack_mask & caster != 0 {
                            AiEvent::AttackSuppressed { caster, attack }
                        } else {
                            AiEvent::Command(EnemyCommand {
                                caster,
                                kind: EnemyCommandKind::Attack,
                                attack,
                                target_mask,
                            })
                        }
                    }
                    level::COUNTER => AiEvent::Command(EnemyCommand {
                        caster,
                        kind: EnemyCommandKind::Counter,
                        attack,
                        target_mask,
                    }),
                    level::DYING => AiEvent::Command(EnemyCommand {
                        caster,
                        kind: EnemyCommandKind::Dying,
                        attack,
                        target_mask,
                    }),
                    other => AiEvent::AttackIgnored {
                        caster,
                        level: other,
                        attack,
                    },
                };
                self.state.events.push(record);
            }
            HostEvent::QueueSpecialAttack { attack } => {
                if let Some(global) = self.state.global.as_mut() {
                    let flags = global.get(199, VariableType::Byte).unwrap_or(0);
                    global.set(199, VariableType::Byte, flags | 16);
                }
                self.state.events.push(AiEvent::SpecialAttack {
                    caster,
                    target_mask,
                    attack: *attack,
                });
            }
            HostEvent::RunBattleCode { code, value } => self.battle_code(*code, *value),
            HostEvent::ShowBattleMessage { text } => {
                self.state.events.push(AiEvent::Message { text: *text });
            }
            HostEvent::TerminateBattle => self.state.events.push(AiEvent::TerminateBattle),
            _ => {}
        }
    }

    fn window_is_open(&self, _window: u8) -> bool {
        false
    }

    /// `GetBattleCharData` (`EventEngine.cs:953`) over the member target.
    fn read_object_member(&mut self, reference: ObjectMemberRef) -> Result<i32, EngineError> {
        Ok(self
            .state
            .units
            .get(&reference.object)
            .map_or(0, |unit| character_data(unit, reference.member)))
    }

    /// `gMemberTarget = _objPtrList[slot]`; `None` re-reads the last target.
    fn select_object_member_target(&mut self, slot: Option<u8>) -> Result<EntityId, EngineError> {
        if let Some(slot) = slot {
            self.state.member_target = Some(usize::from(slot & 7));
        }
        Ok(self.state.member_entity())
    }

    /// `SetBattleCharData` (`EventEngine.cs:970`).
    fn write_object_member(
        &mut self,
        reference: ObjectMemberRef,
        value: i32,
    ) -> Result<(), EngineError> {
        if let Some(unit) = self.state.units.get_mut(&reference.object) {
            set_character_data(unit, reference.member, value);
            if reference.member == 71 {
                self.state.events.push(AiEvent::SetBonus {
                    unit: reference.object,
                });
            }
        }
        Ok(())
    }

    fn read_system_variable(&mut self, variable: u8) -> Result<i32, EngineError> {
        let params = (self.level <= level::REACTION).then_some(self.state.cmd_params);
        let rng = &mut *self.rng;
        Ok(battle_sysvar(
            &self.state.data,
            variable,
            params,
            &mut || rng.random8() as u8,
        ))
    }

    /// `EventEngine.GetSysList`.
    fn read_system_list(&mut self, index: u8) -> Result<i32, EngineError> {
        let value = match index & 7 {
            2 => self.state.data.player_id_mask,
            3 => self.state.data.enemy_id_mask,
            4 => self.state.data.all_id_mask,
            6 => u32::from(self.command_data(self.state.sys_list[5])),
            slot => u32::from(self.state.sys_list[usize::from(slot)]),
        };
        Ok(value as i32)
    }

    /// `EventEngine.SetSysList`.
    fn write_system_list(&mut self, index: u8, value: i32) -> Result<(), EngineError> {
        let slot = usize::from(index & 7);
        self.state.sys_list[slot] = value as u16;
        if slot == 6 {
            self.set_command_data(self.state.sys_list[5], value);
        }
        Ok(())
    }

    fn read_atan_entry(&mut self, _offset: i32) -> Result<u16, EngineError> {
        Ok(0)
    }

    fn resolve_object_uid(&mut self, uid: i32) -> Result<EntityId, EngineError> {
        Ok(EntityId::new(u64::from(uid as u32)))
    }

    fn find_object_uid(&mut self, uid: u8) -> Result<Option<i32>, EngineError> {
        Ok(Some(i32::from(uid)))
    }

    fn read_object_position(&mut self, _object: EntityId) -> Result<[f32; 3], EngineError> {
        Ok([0.0; 3])
    }

    fn read_item_count(&mut self, _item: i32) -> Result<i32, EngineError> {
        Ok(0)
    }

    fn read_animation_frame(
        &mut self,
        _actor: EntityId,
        _animation: i32,
    ) -> Result<i32, EngineError> {
        Ok(0)
    }

    fn read_bgi_animation_frame(&mut self, _animation: u32) -> Result<i32, EngineError> {
        Ok(0)
    }

    fn read_bgi_character_triangle(&mut self, _character: i32) -> Result<i32, EngineError> {
        Ok(0)
    }

    fn read_bgi_character_floor(&mut self, _character: i32) -> Result<i32, EngineError> {
        Ok(0)
    }

    fn read_character_max_mp(&mut self, _character: i32) -> Result<i32, EngineError> {
        Ok(0)
    }

    fn party_contains(&mut self, _character: i32) -> Result<bool, EngineError> {
        Ok(false)
    }

    fn add_party_member(&mut self, _character: i32) -> Result<bool, EngineError> {
        Ok(false)
    }

    fn party_menu_is_open(&self) -> bool {
        false
    }

    fn actor_turn_is_active(&mut self, _actor: EntityId) -> Result<bool, EngineError> {
        Ok(false)
    }

    fn shared_script_is_active(&mut self, _actor: EntityId) -> Result<bool, EngineError> {
        Ok(false)
    }

    fn actor_animation_is_active(&mut self, _actor: EntityId) -> Result<bool, EngineError> {
        Ok(false)
    }

    fn field_actor_movement_is_active(&mut self, _actor: EntityId) -> Result<bool, EngineError> {
        Ok(false)
    }

    fn field_exit_movement_is_active(&mut self, _actor: EntityId) -> Result<bool, EngineError> {
        Ok(false)
    }

    fn actor_jump_is_active(&mut self, _actor: EntityId) -> Result<bool, EngineError> {
        Ok(false)
    }

    fn read_random_u8(&mut self) -> Result<u8, EngineError> {
        Ok(self.rng.random8() as u8)
    }

    fn read_key_on(&mut self) -> Result<i32, EngineError> {
        Ok(0)
    }

    fn read_key_state(&mut self, _state: KeyState) -> Result<i32, EngineError> {
        Ok(0)
    }

    fn read_character_stat(
        &mut self,
        _character: i32,
        _stat: crate::event::CharacterStat,
    ) -> Result<i32, EngineError> {
        Ok(0)
    }

    fn caller_request_acceptable(&mut self, _level: i32) -> Result<bool, EngineError> {
        Ok(true)
    }

    fn script_request_acceptable(
        &mut self,
        _target: EntityId,
        _level: i32,
    ) -> Result<bool, EngineError> {
        Ok(true)
    }

    fn script_request_is_complete(&mut self, _entity: EntityId) -> Result<bool, EngineError> {
        Ok(true)
    }

    fn read_custom_variable(&mut self, _variable: u16) -> Result<i32, EngineError> {
        Ok(0)
    }

    /// Global and Map scopes are the shared `gEventGlobal` / `mapvar`; the
    /// Instance scope is the running object's own buffer.
    fn variable_memory(&mut self, scope: VariableScope) -> Option<&mut VariableMemory> {
        match scope {
            VariableScope::Global => self.state.global.as_mut(),
            VariableScope::Map => self.state.map.as_mut(),
            VariableScope::Instance => Some(&mut *self.instance),
        }
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// A failure while running a battle script function.
#[derive(Debug, Clone, PartialEq)]
pub struct AiError {
    pub entry: usize,
    pub tag: u16,
    pub error: VmError,
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "battle script entry {} tag {}: {}",
            self.entry, self.tag, self.error
        )
    }
}

impl std::error::Error for AiError {}

struct Frame {
    vm: EventVm,
    tag: u16,
    /// Level of the function (`obj.level` while it runs).
    level: u8,
    /// `obj.level` before the request (restored on return).
    saved_level: u8,
    /// The object's own function (tag 0 or 1), not a request.
    base: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObjState {
    New,
    Init,
    Running,
}

struct AiObject {
    entry: usize,
    slot: Option<usize>,
    state: ObjState,
    level: u8,
    frames: Vec<Frame>,
    instance: VariableMemory,
    btlchk: u8,
}

/// `ObjTable.ReadData` vanilla `player_link`: the entry index that links to a
/// party character (`CharacterId` 0-7, 11 = Beatrix) in a script of
/// `entry_count` entries (`GetEventCharacterSId` returns the first match).
pub fn character_entry(entry_count: usize, character: u8) -> Option<usize> {
    (0..entry_count)
        .find(|&index| crate::event::party::player_link(entry_count, index) == Some(character))
}

/// One script entry: its local-variable byte count and decoded functions.
#[derive(Debug, Clone, Default)]
pub struct ScriptEntry {
    /// `ObjTable.varn`.
    pub variable_count: u8,
    pub functions: BTreeMap<u16, EventProgram>,
}

/// Decoded functions of one battle script, by entry index.
#[derive(Debug, Clone, Default)]
pub struct ScriptTable {
    pub entries: Vec<ScriptEntry>,
}

impl ScriptTable {
    /// Decodes every entry of `binary`. An entry that fails to decode keeps
    /// the functions that did decode (the decode report lists the rest).
    pub fn from_binary(binary: &EventBinary) -> Self {
        let entries = binary
            .entries
            .iter()
            .map(|entry| {
                let mut functions = BTreeMap::new();
                if let Ok(report) = binary.decode_entry_with_diagnostics(entry.index) {
                    for function in report.functions {
                        functions.insert(function.tag, function.program);
                    }
                }
                ScriptEntry {
                    variable_count: entry.variable_count,
                    functions,
                }
            })
            .collect();
        Self { entries }
    }
}

/// Runs a battle script's objects against a [`BattleScriptState`].
pub struct BattleScriptDriver {
    table: ScriptTable,
    pub state: BattleScriptState,
    rng: Box<dyn BattleRandom + Send>,
    objects: Vec<AiObject>,
    next_slot: usize,
    ticks: u64,
    attacks: Vec<EnemyCommand>,
    /// Level 1 and level 0 commands (`EnemyCounter`, `EnemyDying`) not yet
    /// taken by the machine.
    reactions: Vec<EnemyCommand>,
    /// Number of `events` entries already scanned into `attacks`.
    scanned: usize,
}

impl BattleScriptDriver {
    /// Creates the battle objects the way `InitBattle` does: the main object
    /// (entry 0, `NewThread(0, 0)`), then one actor per party member whose
    /// character entry exists (`party` is character ids in party order),
    /// which fill object slots 0..; script-created actors start at slot 4.
    pub fn new(
        table: ScriptTable,
        mut state: BattleScriptState,
        rng: Box<dyn BattleRandom + Send>,
        party: &[u8],
    ) -> Self {
        if state.global.is_none() {
            state.global = Some(VariableMemory::global());
        }
        if state.map.is_none() {
            state.map = Some(VariableMemory::map());
        }
        let mut driver = Self {
            table,
            state,
            rng,
            objects: Vec::new(),
            next_slot: 0,
            ticks: 0,
            attacks: Vec::new(),
            reactions: Vec::new(),
            scanned: 0,
        };
        if !driver.table.entries.is_empty() {
            driver.spawn(0, false);
        }
        let count = driver.table.entries.len();
        for &character in party.iter().take(4) {
            if let Some(entry) = character_entry(count, character) {
                driver.spawn(entry, true);
            }
        }
        driver.next_slot = 4;
        driver
    }

    fn program(&self, entry: usize, tag: u16) -> Option<EventProgram> {
        self.table.entries.get(entry)?.functions.get(&tag).cloned()
    }

    fn instance_size(&self, entry: usize) -> usize {
        // `varn + 3 >> 2` words (`Obj` ctor).
        let varn = self
            .table
            .entries
            .get(entry)
            .map_or(0, |e| usize::from(e.variable_count));
        (varn + 3) & !3
    }

    fn new_frame(
        &mut self,
        entry: usize,
        tag: u16,
        level: u8,
        saved: u8,
        base: bool,
    ) -> Option<Frame> {
        let program = self.program(entry, tag)?;
        Some(Frame {
            vm: EventVm::new(program).with_entity(EntityId::new(entry as u64)),
            tag,
            level,
            saved_level: saved,
            base,
        })
    }

    /// `new Actor(sid, uid)` / `new Obj`: queues an object in state `New`.
    fn spawn(&mut self, entry: usize, actor: bool) {
        let slot = actor.then(|| {
            let slot = self.next_slot;
            self.next_slot += 1;
            slot
        });
        if let Some(slot) = slot
            && slot < 8
        {
            self.state.objects[slot] = Some(entry);
        }
        let frames = self
            .new_frame(entry, tag::INIT, 0, 0, true)
            .into_iter()
            .collect::<Vec<_>>();
        let size = self.instance_size(entry);
        self.objects.push(AiObject {
            entry,
            slot: slot.filter(|&s| s < 8),
            state: ObjState::New,
            level: 0,
            frames,
            instance: VariableMemory::zeroed(size),
            btlchk: 0,
        });
    }

    fn object_by_bit(&mut self, btl_id: u16) -> Option<&mut AiObject> {
        let slot = (0..8).find(|index| btl_id & (1 << index) != 0)?;
        self.objects.iter_mut().find(|o| o.slot == Some(slot))
    }

    /// `EventEngine.Request(obj, level, tag)`.
    fn request(&mut self, object: usize, level: u8, tag_number: u16) -> bool {
        let entry = self.objects[object].entry;
        if level >= self.objects[object].level {
            return false;
        }
        let saved = self.objects[object].level;
        let Some(frame) = self.new_frame(entry, tag_number, level, saved, false) else {
            return false;
        };
        self.objects[object].frames.push(frame);
        self.objects[object].level = level;
        true
    }

    /// `RequestAction(EnemyAtk, btl_id, ...)`: accepted when the object is
    /// above script level 3, then the end-of-frame loop starts tag 5.
    pub fn request_atb(&mut self, btl_id: u16) -> bool {
        let Some(object) = self.object_by_bit(btl_id) else {
            return false;
        };
        if object.level > level::ATB {
            object.btlchk = 2;
            true
        } else {
            false
        }
    }

    /// `RequestAction` for counter, reaction and dying requests: records
    /// `_btlCmdPrmCmd/Sub`, `SysList(0) = react_caster` (counter and dying
    /// only) and requests the function.
    pub fn request_reaction(
        &mut self,
        kind: ReactionKind,
        target_btl_id: u16,
        react_caster: u16,
        react_cmd: i32,
        react_sub: i32,
    ) -> bool {
        let (level, tag_number) = match kind {
            ReactionKind::Counter => (level::COUNTER, tag::COUNTER),
            ReactionKind::Reaction => (level::REACTION, tag::REACTION),
            ReactionKind::Dying => (level::DYING, tag::DYING),
        };
        let Some(slot) = (0..8).find(|index| target_btl_id & (1 << index) != 0) else {
            return false;
        };
        let Some(index) = self.objects.iter().position(|o| o.slot == Some(slot)) else {
            return false;
        };
        if kind != ReactionKind::Reaction {
            self.state.sys_list[0] = react_caster;
        }
        self.state.cmd_params = (react_cmd, react_sub);
        self.request(index, level, tag_number)
    }

    /// One `ProcessEvents` pass: every object runs its current function once
    /// (a new object only advances `New` -> `Init`), then the `btlchk == 2`
    /// loop starts ATB functions.
    pub fn tick(&mut self) -> Result<(), AiError> {
        let mut index = 0;
        while index < self.objects.len() {
            self.step_object(index)?;
            index += 1;
        }
        for index in 0..self.objects.len() {
            if self.objects[index].btlchk == 2 && self.request(index, level::ATB, tag::ATB) {
                self.objects[index].btlchk = 1;
            }
        }
        self.ticks += 1;
        self.collect_attacks();
        Ok(())
    }

    fn step_object(&mut self, index: usize) -> Result<(), AiError> {
        match self.objects[index].state {
            ObjState::New => {
                self.objects[index].state = ObjState::Init;
                return Ok(());
            }
            ObjState::Init | ObjState::Running => {}
        }
        if self.objects[index].frames.is_empty() {
            return Ok(());
        }
        // `ProcessCodeExt`: `SysList(1)` is the running object's bit.
        self.state.sys_list[1] = self.objects[index].slot.map_or(0, |slot| 1u16 << slot);
        let (entry, frame_tag, frame_level) = {
            let object = &self.objects[index];
            let frame = object.frames.last().expect("frame checked above");
            (object.entry, frame.tag, frame.level)
        };
        let events_before = self.state.events.len();
        let outcome = {
            let object = &mut self.objects[index];
            let frame = object.frames.last_mut().expect("frame checked above");
            let mut host = BattleScriptHost::new(
                &mut self.state,
                &mut *self.rng,
                &mut object.instance,
                frame_level,
            );
            frame.vm.step(Tick::new(self.ticks), &mut host)
        };
        let report = outcome.map_err(|error| AiError {
            entry,
            tag: frame_tag,
            error,
        });
        report?;
        // `BTLCMD` at level 3 clears `btlchk` (`DoEventCode.cs:1196`).
        if frame_level == level::ATB && self.attack_event_since(events_before) {
            self.objects[index].btlchk = 0;
        }
        self.drain_spawns();
        let done = self.objects[index]
            .frames
            .last()
            .is_some_and(|frame| frame.vm.status() == VmStatus::Completed);
        if done {
            self.return_frame(index);
        }
        Ok(())
    }

    fn attack_event_since(&self, from: usize) -> bool {
        self.state.events[from..].iter().any(|event| {
            matches!(
                event,
                AiEvent::Command(EnemyCommand {
                    kind: EnemyCommandKind::Attack,
                    ..
                }) | AiEvent::AttackSuppressed { .. }
            )
        })
    }

    /// `EventEngine.Return`: a request frame restores the saved level; the
    /// base function of a new object moves it to its main loop (level 7); a
    /// finished main loop leaves the object idle (`ip = nil`, level kept).
    fn return_frame(&mut self, index: usize) {
        let entry = self.objects[index].entry;
        let frame = self.objects[index].frames.pop().expect("a frame completed");
        if !frame.base {
            self.objects[index].level = frame.saved_level;
        } else if self.objects[index].state == ObjState::Init {
            self.objects[index].state = ObjState::Running;
            self.objects[index].level = level::IDLE;
            if let Some(main_loop) =
                self.new_frame(entry, tag::LOOP, level::IDLE, level::IDLE, true)
            {
                self.objects[index].frames.push(main_loop);
            }
        }
    }

    /// `InitializeObject` events become actors in object-list order.
    fn drain_spawns(&mut self) {
        let spawns = std::mem::take(&mut self.state.spawns);
        for (source, _unique_id) in spawns {
            if (251..=254).contains(&source) {
                self.state.events.push(AiEvent::UnmodeledSpawn { source });
                continue;
            }
            if usize::from(source) < self.table.entries.len() {
                self.spawn(usize::from(source), true);
            }
        }
    }

    fn collect_attacks(&mut self) {
        for event in &self.state.events[self.scanned..] {
            if let AiEvent::Command(command) = event {
                if command.kind == EnemyCommandKind::Attack {
                    self.attacks.push(*command);
                } else {
                    self.reactions.push(*command);
                }
            }
        }
        self.scanned = self.state.events.len();
    }

    /// Removes and returns the oldest queued level-3 attack of `caster`.
    pub fn take_attack(&mut self, caster: u16) -> Option<EnemyCommand> {
        let position = self.attacks.iter().position(|c| c.caster == caster)?;
        Some(self.attacks.remove(position))
    }

    /// Removes and returns every queued counter and dying command, oldest
    /// first (`SetEnemyCommand` at script level 1 and 0).
    pub fn take_reactions(&mut self) -> Vec<EnemyCommand> {
        std::mem::take(&mut self.reactions)
    }

    /// True while an object runs a counter, reaction or dying function (script
    /// level 0 to 2): the driver has to keep ticking until it returns.
    pub fn reaction_active(&self) -> bool {
        self.objects.iter().any(|o| o.level <= level::REACTION)
    }

    /// Number of live objects (main object, party actors, script actors).
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    /// Current script level of the object in `slot`.
    pub fn object_level(&self, slot: usize) -> Option<u8> {
        self.objects
            .iter()
            .find(|o| o.slot == Some(slot))
            .map(|o| o.level)
    }

    /// Entry index of the object in `slot`.
    pub fn object_entry(&self, slot: usize) -> Option<usize> {
        self.state.objects.get(slot).copied().flatten()
    }

    /// Runs `tick` until `caster` has a queued attack or `max_ticks` pass.
    pub fn run_until_attack(
        &mut self,
        caster: u16,
        max_ticks: u32,
    ) -> Result<Option<EnemyCommand>, AiError> {
        for _ in 0..max_ticks {
            if let Some(command) = self.take_attack(caster) {
                return Ok(Some(command));
            }
            self.tick()?;
        }
        Ok(self.take_attack(caster))
    }
}

/// Which `RequestAction` flavor [`BattleScriptDriver::request_reaction`] runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReactionKind {
    Counter,
    Reaction,
    Dying,
}

// ---------------------------------------------------------------------------
// Machine wiring
// ---------------------------------------------------------------------------

/// [`BattleHooks`] that answers `enemy_atb` with the battle script.
///
/// Once per battle tick the first `enemy_atb` call syncs the machine into the
/// script state and advances the driver one tick; each call then either hands
/// back the attack the script queued for that unit or files the ATB request
/// (once, until a command comes back).
pub struct EnemyAiHooks<H: BattleHooks> {
    pub inner: H,
    pub driver: BattleScriptDriver,
    pub overlay: SceneOverlay,
    last_tick: Option<u32>,
    /// Script events already scanned for `SetBattleData` 32-35.
    codes_seen: usize,
    /// `BTLSEQ` calls found by the last [`BattleHooks::frame`].
    specials: Vec<(u16, i32, u16)>,
    outstanding: Vec<u16>,
    synced: BTreeMap<EntityId, UnitView>,
    pub error: Option<AiError>,
}

impl<H: BattleHooks> EnemyAiHooks<H> {
    pub fn new(inner: H, driver: BattleScriptDriver, overlay: SceneOverlay) -> Self {
        Self {
            inner,
            driver,
            overlay,
            last_tick: None,
            codes_seen: 0,
            specials: Vec::new(),
            outstanding: Vec::new(),
            synced: BTreeMap::new(),
            error: None,
        }
    }

    /// Unit views the scripts changed since the last sync, for
    /// [`super::host::apply_unit_views`].
    pub fn take_dirty_views(&mut self) -> BTreeMap<EntityId, UnitView> {
        let dirty = self
            .driver
            .state
            .units
            .iter()
            .filter(|(id, view)| self.synced.get(id) != Some(view))
            .map(|(id, view)| (*id, view.clone()))
            .collect();
        self.synced = self.driver.state.units.clone();
        dirty
    }
}

impl<H: BattleHooks> EnemyAiHooks<H> {
    /// Once per battle tick: sync the machine into the script state and
    /// advance the driver one tick.
    fn advance(&mut self, battle: &Battle) {
        if self.last_tick == Some(battle.tick_no) {
            return;
        }
        self.last_tick = Some(battle.tick_no);
        // Gil and AP are script-writable (`RunBattleCode` 38 and 44).
        self.overlay.gil = self.driver.state.data.gil;
        self.overlay.bonus_ap = self.driver.state.data.bonus_ap;
        sync_state_from_battle(&mut self.driver.state, battle, &self.overlay);
        if self.driver.ticks < u64::from(self.overlay.load_frames) {
            self.driver.state.data.load_status = 0;
        }
        self.synced = self.driver.state.units.clone();
        if let Err(error) = self.driver.tick() {
            self.error = Some(error);
        }
    }
}

impl<H: BattleHooks> BattleHooks for EnemyAiHooks<H> {
    fn spec(
        &mut self,
        cmd_no: u32,
        sub_no: i32,
        caster: &super::unit::BattleUnit,
    ) -> Option<super::queue::CommandSpec> {
        self.inner.spec(cmd_no, sub_no, caster)
    }

    fn item(&mut self, item_id: i32) -> Option<super::command::ItemData> {
        self.inner.item(item_id)
    }

    fn attack_sequence(&mut self, attack: i32) -> Option<bool> {
        self.inner.attack_sequence(attack)
    }

    fn attack_stand_animation(&mut self, attack: i32) -> Option<bool> {
        self.inner.attack_stand_animation(attack)
    }

    fn special_attacks(&mut self) -> Vec<(u16, i32, u16)> {
        std::mem::take(&mut self.specials)
    }

    fn item_weapon_power(&mut self, item_id: i32) -> Option<i32> {
        self.inner.item_weapon_power(item_id)
    }

    /// `RequestAction(EnemyCounter | EnemyDying | EnemyReaction, target,
    /// caster, cmd, sub)` (`EventEngine.cs:285`): the object accepts it only
    /// when it is above the function's script level.
    fn enemy_reaction(&mut self, _battle: &Battle, request: EnemyReactionRequest) {
        let kind = match request.kind {
            EnemyReactionKind::Counter => ReactionKind::Counter,
            EnemyReactionKind::Dying => ReactionKind::Dying,
            EnemyReactionKind::Reaction => ReactionKind::Reaction,
        };
        self.driver.request_reaction(
            kind,
            request.target_id,
            request.caster_id,
            request.cmd_no as i32,
            request.sub_no,
        );
    }

    /// The counter and dying commands the AI queued. While a reaction
    /// function runs the driver is advanced here, once per battle tick (the
    /// same guard `enemy_atb` uses); with none running nothing is ticked, so
    /// a battle without reactions keeps the cadence it had.
    fn enemy_reaction_commands(&mut self, battle: &Battle) -> Vec<(u16, EnemyAction)> {
        if self.driver.reaction_active() {
            self.advance(battle);
        }
        self.driver
            .take_reactions()
            .into_iter()
            .map(|command| {
                let cmd_no = match command.kind {
                    EnemyCommandKind::Dying => cmd::ENEMY_DYING,
                    _ => cmd::ENEMY_COUNTER,
                };
                (
                    command.caster,
                    EnemyAction {
                        cmd_no,
                        sub_no: command.attack,
                        tar_id: command.target_mask,
                    },
                )
            })
            .collect()
    }

    fn unit_writes(&mut self) -> BTreeMap<EntityId, UnitView> {
        self.take_dirty_views()
    }

    /// One script pass per machine tick, so each object's loop (tag 1)
    /// runs every frame as in `ServiceEvents`, not only on a full gauge.
    fn frame(&mut self, battle: &Battle) -> Vec<(u32, i32)> {
        self.advance(battle);
        let events = &self.driver.state.events[self.codes_seen..];
        let codes = events
            .iter()
            .filter_map(|event| match event {
                AiEvent::BattleCode { code, value } if (32..=35).contains(code) => {
                    Some((*code, *value))
                }
                _ => None,
            })
            .collect();
        self.specials
            .extend(events.iter().filter_map(|event| match event {
                AiEvent::SpecialAttack {
                    caster,
                    target_mask,
                    attack,
                } => Some((*caster, *attack, *target_mask)),
                _ => None,
            }));
        self.codes_seen = self.driver.state.events.len();
        codes
    }

    fn enemy_atb(&mut self, battle: &Battle, unit: usize) -> Option<EnemyAction> {
        let id = battle.units[unit].id;
        self.advance(battle);
        if let Some(command) = self.driver.take_attack(id) {
            self.outstanding.retain(|&c| c != id);
            return Some(EnemyAction {
                cmd_no: cmd::ENEMY_ATK,
                sub_no: command.attack,
                tar_id: command.target_mask,
            });
        }
        if !self.outstanding.contains(&id) && self.driver.request_atb(id) {
            self.outstanding.push(id);
        }
        None
    }
}
