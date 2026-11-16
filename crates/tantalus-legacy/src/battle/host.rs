//! Battle script host (G5, first slice): answers the event-engine sysvar and
//! system-list reads battle scripts depend on, so decoded `EVT_BATTLE_*`
//! functions execute against real battle state instead of zero-state.
//!
//! Sources (Memoria pin `c716e1fa`):
//! * `Global/Event/Engine/EventEngine.GetSysvar.cs` — sysvars 0-31 and 191,
//!   with codes below 192 falling through to `btl_scrp.GetBattleData`.
//! * `Global/btl_scrp.cs` (`GetBattleData`) — sysvars 32-44; the default
//!   result is `UInt32::MAX` (read back as -1).
//! * `Global/Event/Engine/EventEngine.cs` (`GetSysList`/`SetSysList`) —
//!   element numbers mask with 7; elements 2/3/4 are the live player/enemy/
//!   all unit-id masks, element 5 selects the spell stat and element 6 reads
//!   or writes it.
//!
//! Deviations of this slice, to be closed as the battle model grows:
//! * Sysvars 1-5 and 7-23 are field-context reads; this host answers them
//!   with the probe default (0) instead of field state.
//! * Sysvars 28/29 use the [`CurrentCommandView`] path only; the
//!   `_btlCmdPrmCmd`/`_btlCmdPrmSub` invocation-parameter path is expressed
//!   by the caller populating `current_command`.
//! * System-list element 6 (`GetCurrentCommandData`/`SetCurrentCommandData`,
//!   spell-stat access) stores through element 5 but does not mutate a
//!   command; it reads back 0.
//! * Object-member reads answer `GetCharacterData` over caller-keyed
//!   [`UnitView`]s; writes (`SetCharacterData`) are accepted but do not
//!   mutate units yet.

use super::machine::{Battle, Phase, UnitSched};
use super::rng::BattleRandom;
use super::unit::BattleUnit;
use crate::event::probe::ProbeHost;
use crate::event::{LegacyEventHost, ObjectMemberRef};
use std::collections::BTreeMap;
use tantalus_contracts::{EngineError, EntityId};

/// Command data behind sysvars 28/29 and 36-39/43.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CurrentCommandView {
    /// `cmd_no` (sysvar 28, `BattleCommandId`, 0 = none).
    pub cmd_no: i32,
    /// `sub_no` (sysvar 29: ability/item id, 0 when none).
    pub sub_no: i32,
    /// `curCmd.regist.btl_id` (sysvar 36).
    pub regist_btl_id: u8,
    /// `curCmd.tar_id` (sysvar 37).
    pub tar_id: u32,
    /// Weapon effect element (sysvar 38).
    pub weapon_element: u8,
    /// Command element (sysvar 39).
    pub cmd_element: u8,
    /// `curCmd.info.effect_counter` (sysvar 43).
    pub effect_counter: u32,
}

/// One battle unit's script-visible state: the `GetCharacterData` read table
/// (`btl_scrp.cs`, ids 35-149). Callers key units by the entity id the
/// script object runs as; enemy-only members read the enemy fields (players
/// leave them defaulted).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnitView {
    pub max_hp: i32,
    pub hp: i32,
    pub max_mp: i32,
    pub mp: i32,
    pub max_atb: i32,
    pub atb: i32,
    pub level: i32,
    /// `btl.stat.invalid`: members 42/43 read the high/low 24 bits.
    pub status_invalid: u64,
    /// `btl.stat.permanent` (auto-status): members 44/45.
    pub status_permanent: u64,
    /// `btl.stat.cur`: members 46/47.
    pub status_current: u64,
    pub element_invalid: u8,
    pub element_absorb: u8,
    pub element_half: u8,
    pub element_weak: u8,
    pub targetable: i32,
    pub disappear: i32,
    pub model: i32,
    pub model_visible: i32,
    pub row: i32,
    pub column: i32,
    pub model_type: i32,
    /// Player `category` (member 67).
    pub category: i32,
    /// `EnemyCategory` byte (member 68); member 113 returns this for enemies
    /// and `1` (Humanoid) for players.
    pub class: u8,
    pub is_player: bool,
    pub stand_animation: i32,
    pub enemy_id: i32,
    pub strength: i32,
    pub magic: i32,
    pub physical_defence: i32,
    pub physical_evade: i32,
    pub magical_defence: i32,
    pub magical_evade: i32,
    pub steal_items: [i32; 4],
    pub bonus_items: [i32; 4],
    pub bonus_card: i32,
    pub flags: i32,
    pub out_of_reach: bool,
    pub die_fade_rate: i32,
    pub motion: i32,
    /// Current attack ability id while acting (member 114).
    pub current_attack: i32,
    pub pos: [i32; 3],
    pub rot: [i32; 3],
    pub bonus_exp: i32,
    pub bonus_gil: i32,
    pub trance: i32,
    pub trance_gauge: i32,
}

/// `btl_scrp.GetCharacterData` (btl_scrp.cs): unknown ids return the
/// `16777215` (0xFFFFFF) default.
fn character_data(unit: &UnitView, id: u8) -> i32 {
    let low24 = |value: u64| (value & 0xFF_FFFF) as i32;
    let high24 = |value: u64| ((value >> 24) & 0xFF_FFFF) as i32;
    match id {
        35 => unit.max_hp,
        36 => unit.hp,
        37 => unit.max_mp,
        38 => unit.mp,
        39 => unit.max_atb,
        40 => unit.atb,
        41 => unit.level,
        42 => high24(unit.status_invalid),
        43 => low24(unit.status_invalid),
        44 => high24(unit.status_permanent),
        45 => low24(unit.status_permanent),
        46 => high24(unit.status_current),
        47 => low24(unit.status_current),
        48 => i32::from(unit.element_invalid),
        49 => i32::from(unit.element_absorb),
        50 => i32::from(unit.element_half),
        51 => i32::from(unit.element_weak),
        52 => unit.targetable,
        53 => unit.disappear,
        57 => unit.model,
        58 => unit.model_visible,
        64 => unit.row,
        65 => unit.column,
        66 => unit.model_type,
        67 => unit.category,
        68 => i32::from(unit.class),
        69 => unit.stand_animation,
        70 => unit.enemy_id,
        72 => unit.strength,
        73 => unit.magic,
        74 => unit.physical_defence,
        75 => unit.physical_evade,
        76 => unit.magical_defence,
        77 => unit.magical_evade,
        // Enemy-only reads leave the reference default for players
        // (`if (btl.bi.player == 0)` guards in `GetCharacterData`).
        100..=103 if !unit.is_player => unit.steal_items[usize::from(id - 100)],
        104..=107 if !unit.is_player => unit.bonus_items[usize::from(id - 104)],
        108 if !unit.is_player => unit.bonus_card,
        109 if !unit.is_player => unit.flags,
        110 => i32::from(unit.out_of_reach),
        111 if !unit.is_player => unit.die_fade_rate,
        112 => unit.motion,
        113 => {
            if unit.is_player {
                1 // EnemyCategory::Humanoid
            } else {
                i32::from(unit.class)
            }
        }
        114 => unit.current_attack,
        140..=142 => unit.pos[usize::from(id - 140)],
        143..=145 => unit.rot[usize::from(id - 143)],
        146 if !unit.is_player => unit.bonus_exp,
        147 if !unit.is_player => unit.bonus_gil,
        148 => {
            if unit.trance_gauge != 0 {
                unit.trance
            } else {
                0
            }
        }
        149 => unit.trance_gauge,
        _ => 0xFF_FFFF,
    }
}

/// `btl_scrp.SetCharacterData` (btl_scrp.cs): applies one member write to a
/// unit. Engine-side effects (mesh/weapon renderers, shadows, texanim,
/// bonus collection, counter/summon command scheduling, ATB kill switches)
/// are not modeled and fall through silently, mirroring the reference's
/// no-default switch. Enemy-only members are ignored for players, as the
/// reference's `btl.bi.player == 0` guard does.
fn set_character_data(unit: &mut UnitView, id: u8, val: i32) {
    let low24 = 0xFF_FFFFu64;
    match id {
        35 => unit.max_hp = val,
        36 => unit.hp = val,
        37 => unit.max_mp = val,
        38 => unit.mp = val,
        40 => unit.atb = val,
        // 42 sign-extends the value before shifting (C# `(UInt64)val`),
        // 43 truncates to 32 bits (`(UInt32)val`) and clears bits 32-39
        // through the reference mask.
        42 => {
            unit.status_invalid =
                (unit.status_invalid & 0xFFFF_0000_00FF_FFFF) | (((val as i64) as u64) << 24);
        }
        43 => {
            unit.status_invalid =
                (unit.status_invalid & 0xFFFF_FFFF_FF00_0000) | (val as u32 as u64);
        }
        // 44-47 replace the addressed 24-bit half; the reference's
        // remove/add pair nets to a half-write on the view.
        44 => {
            unit.status_permanent =
                (unit.status_permanent & !(low24 << 24)) | ((val as u64 & low24) << 24);
        }
        45 => {
            unit.status_permanent = (unit.status_permanent & !low24) | (val as u64 & low24);
        }
        46 => {
            unit.status_current =
                (unit.status_current & !(low24 << 24)) | ((val as u64 & low24) << 24);
        }
        47 => {
            unit.status_current = (unit.status_current & !low24) | (val as u64 & low24);
        }
        48 => unit.element_invalid = val as u8,
        49 => unit.element_absorb = val as u8,
        50 => unit.element_half = val as u8,
        51 => unit.element_weak = val as u8,
        52 => unit.targetable = val,
        53 => unit.disappear = val,
        59 => unit.model_visible |= i32::from(val as u16),
        60 => unit.model_visible &= !i32::from(val as u16),
        69 => unit.stand_animation = val,
        72 => unit.strength = val,
        73 => unit.magic = val,
        74 => unit.physical_defence = val,
        75 => unit.physical_evade = val,
        76 => unit.magical_defence = val,
        77 => unit.magical_evade = val,
        100..=103 if !unit.is_player => unit.steal_items[usize::from(id - 100)] = val,
        104..=107 if !unit.is_player => unit.bonus_items[usize::from(id - 104)] = val,
        108 if !unit.is_player => unit.bonus_card = val,
        109 if !unit.is_player => unit.flags = val,
        110 => unit.out_of_reach = val != 0,
        111 => unit.die_fade_rate = val,
        112 => unit.motion = val,
        113 if !unit.is_player => unit.class = val as u8,
        140 => unit.pos[0] = val,
        141 => unit.pos[1] = -val,
        142 => unit.pos[2] = val,
        143 => unit.rot[0] = val,
        144 => unit.rot[1] = val,
        145 => unit.rot[2] = val,
        146 if !unit.is_player => unit.bonus_exp = val,
        147 if !unit.is_player => unit.bonus_gil = val,
        148 => unit.trance = val,
        _ => {}
    }
}

/// Battle state the sysvar and system-list reads answer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BattleDataView {
    /// Sysvar 24: `_ff9.btlMapNo`.
    pub battle_map_no: i32,
    /// Sysvar 25: `btlseq.BtlSeqBusy()`.
    pub seq_busy: bool,
    /// Sysvar 26: `gMode == 4` (battle result screen).
    pub result_mode: bool,
    /// Sysvar 27: `_ff9.btl_result`.
    pub battle_result: i32,
    /// Sysvars 30/34: `btl_phase`.
    pub btl_phase: u32,
    /// Sysvar 31: `btl_scene.PatNum` (enemy pattern id).
    pub pattern_id: i32,
    /// Sysvar 32: `btl_scene.Info.StartType`.
    pub start_type: u8,
    /// Sysvars 6/33: party gil.
    pub gil: u32,
    /// Sysvar 35: `btl_seq`.
    pub btl_seq: u32,
    /// Sysvar 40: `btl_load_status`.
    pub load_status: u32,
    /// Sysvar 41: `battle.btl_bonus.exp`.
    pub bonus_exp: u32,
    /// Sysvar 42: Eiko in the party used the Last Phoenix command.
    pub phoenix_used: bool,
    /// Sysvar 44: `battle.btl_bonus.ap`.
    pub bonus_ap: u16,
    /// Current command, when one is resolving.
    pub current_command: Option<CurrentCommandView>,
    /// System-list element 2: mask of player `btl_id`s (`GetBattleID(0)`).
    pub player_id_mask: u32,
    /// System-list element 3: mask of enemy `btl_id`s (`GetBattleID(1)`).
    pub enemy_id_mask: u32,
    /// System-list element 4: mask of every `btl_id` (`GetBattleID(2)`).
    pub all_id_mask: u32,
}

/// `LegacyEventHost` for battle script execution.
///
/// Everything outside the battle sysvar/system-list surface delegates to the
/// zero-state probe defaults; battle-specific reads come from
/// [`BattleDataView`]. `rng` supplies sysvar 0 (`Comn.random8`); when absent
/// the read returns 0.
pub struct BattleScriptHost<'a> {
    data: &'a BattleDataView,
    units: std::collections::BTreeMap<EntityId, UnitView>,
    sys_list: [u16; 8],
    rng: Option<&'a mut dyn BattleRandom>,
    probe: ProbeHost,
}

impl<'a> BattleScriptHost<'a> {
    pub fn new(data: &'a BattleDataView) -> Self {
        Self::with_units(data, std::collections::BTreeMap::new())
    }

    pub fn with_units(
        data: &'a BattleDataView,
        units: std::collections::BTreeMap<EntityId, UnitView>,
    ) -> Self {
        Self {
            data,
            units,
            sys_list: [0; 8],
            rng: None,
            probe: ProbeHost,
        }
    }

    pub fn with_random(data: &'a BattleDataView, rng: &'a mut dyn BattleRandom) -> Self {
        Self {
            data,
            units: std::collections::BTreeMap::new(),
            sys_list: [0; 8],
            rng: Some(rng),
            probe: ProbeHost,
        }
    }

    /// The unit map after script writes.
    pub fn units(&self) -> &std::collections::BTreeMap<EntityId, UnitView> {
        &self.units
    }

    /// Consumes the host, returning the (possibly written) unit map.
    pub fn into_units(self) -> std::collections::BTreeMap<EntityId, UnitView> {
        self.units
    }

    /// Current system-list contents (element numbers masked with 7).
    pub fn sys_list(&self) -> [u16; 8] {
        self.sys_list
    }
}

impl LegacyEventHost for BattleScriptHost<'_> {
    fn apply(&mut self, events: &[crate::event::HostEvent]) -> Result<(), EngineError> {
        self.probe.apply(events)
    }

    fn window_is_open(&self, window: u8) -> bool {
        self.probe.window_is_open(window)
    }

    /// `GetBattleCharData` (`btl_scrp.GetCharacterData`): the unit keyed by
    /// the running object's entity; unknown objects and members read 0 and
    /// 0xFFFFFF respectively, mirroring the reference defaults.
    fn read_object_member(&mut self, reference: ObjectMemberRef) -> Result<i32, EngineError> {
        Ok(self
            .units
            .get(&reference.object)
            .map_or(0, |unit| character_data(unit, reference.member)))
    }

    fn select_object_member_target(&mut self, slot: Option<u8>) -> Result<EntityId, EngineError> {
        self.probe.select_object_member_target(slot)
    }

    /// `SetBattleCharData` (`btl_scrp.SetCharacterData`): writes apply to
    /// the keyed unit; unknown objects and engine-effect members are
    /// ignored, mirroring the reference.
    fn write_object_member(
        &mut self,
        reference: ObjectMemberRef,
        value: i32,
    ) -> Result<(), EngineError> {
        if let Some(unit) = self.units.get_mut(&reference.object) {
            set_character_data(unit, reference.member, value);
        }
        Ok(())
    }

    /// `EventEngine.GetSysvar` battle cases + `btl_scrp.GetBattleData`.
    ///
    /// Unknown codes below 192 return the `GetBattleData` default
    /// (`UInt32::MAX`, read back as -1); field-context codes 1-23 answer 0
    /// (documented deviation).
    fn read_system_variable(&mut self, variable: u8) -> Result<i32, EngineError> {
        let data = self.data;
        let value = match variable {
            0 => i32::from(self.rng_draw()),
            6 | 33 => data.gil as i32,
            24 => data.battle_map_no,
            25 => i32::from(data.seq_busy),
            26 => i32::from(data.result_mode),
            27 => data.battle_result,
            28 => data.current_command.as_ref().map_or(0, |c| c.cmd_no),
            29 => data.current_command.as_ref().map_or(0, |c| c.sub_no),
            30 | 34 => data.btl_phase as i32,
            31 => data.pattern_id,
            32 => i32::from(data.start_type),
            35 => data.btl_seq as i32,
            36 => data
                .current_command
                .as_ref()
                .map_or(-1, |c| i32::from(c.regist_btl_id)),
            37 => data
                .current_command
                .as_ref()
                .map_or(-1, |c| c.tar_id as i32),
            38 => data
                .current_command
                .as_ref()
                .map_or(-1, |c| i32::from(c.weapon_element)),
            39 => data
                .current_command
                .as_ref()
                .map_or(-1, |c| i32::from(c.cmd_element)),
            40 => data.load_status as i32,
            41 => data.bonus_exp as i32,
            42 => i32::from(data.phoenix_used),
            43 => data
                .current_command
                .as_ref()
                .map_or(-1, |c| c.effect_counter as i32),
            44 => i32::from(data.bonus_ap),
            // Field-context sysvars (1-23, 191, 192+): not modeled in the
            // battle host, answered with the probe default (documented
            // deviation).
            1..=23 | 191..=255 => 0,
            // `GetBattleData` codes below 192 with no case fall through to
            // the `UInt32::MAX` default, read back as -1.
            45..=190 => -1,
        };
        Ok(value)
    }

    /// `EventEngine.GetSysList`: element numbers mask with 7; elements 2/3/4
    /// are the live unit-id masks; element 6 is spell-stat access (stored
    /// through element 5, reads back 0 — command mutation not modeled).
    fn read_system_list(&mut self, index: u8) -> Result<i32, EngineError> {
        let value = match index & 7 {
            2 => self.data.player_id_mask,
            3 => self.data.enemy_id_mask,
            4 => self.data.all_id_mask,
            6 => 0,
            slot => u32::from(self.sys_list[usize::from(slot)]),
        };
        Ok(value as i32)
    }

    /// `EventEngine.SetSysList`: stores truncate to 16 bits. Writing element
    /// 6 (`SetCurrentCommandData`) is accepted but does not mutate a command
    /// (documented deviation).
    fn write_system_list(&mut self, index: u8, value: i32) -> Result<(), EngineError> {
        self.sys_list[usize::from(index & 7)] = value as u16;
        Ok(())
    }

    fn read_atan_entry(&mut self, offset: i32) -> Result<u16, EngineError> {
        self.probe.read_atan_entry(offset)
    }

    fn resolve_object_uid(&mut self, uid: i32) -> Result<EntityId, EngineError> {
        self.probe.resolve_object_uid(uid)
    }

    fn find_object_uid(&mut self, uid: u8) -> Result<Option<i32>, EngineError> {
        self.probe.find_object_uid(uid)
    }

    fn read_object_position(&mut self, object: EntityId) -> Result<[f32; 3], EngineError> {
        self.probe.read_object_position(object)
    }

    fn read_item_count(&mut self, item: i32) -> Result<i32, EngineError> {
        self.probe.read_item_count(item)
    }

    fn read_animation_frame(
        &mut self,
        actor: EntityId,
        animation: i32,
    ) -> Result<i32, EngineError> {
        self.probe.read_animation_frame(actor, animation)
    }

    fn read_bgi_animation_frame(&mut self, animation: u32) -> Result<i32, EngineError> {
        self.probe.read_bgi_animation_frame(animation)
    }

    fn read_bgi_character_triangle(&mut self, character: i32) -> Result<i32, EngineError> {
        self.probe.read_bgi_character_triangle(character)
    }

    fn read_bgi_character_floor(&mut self, character: i32) -> Result<i32, EngineError> {
        self.probe.read_bgi_character_floor(character)
    }

    fn read_character_max_mp(&mut self, character: i32) -> Result<i32, EngineError> {
        self.probe.read_character_max_mp(character)
    }

    fn party_contains(&mut self, character: i32) -> Result<bool, EngineError> {
        self.probe.party_contains(character)
    }

    fn add_party_member(&mut self, character: i32) -> Result<bool, EngineError> {
        self.probe.add_party_member(character)
    }

    fn party_menu_is_open(&self) -> bool {
        self.probe.party_menu_is_open()
    }

    fn actor_turn_is_active(&mut self, actor: EntityId) -> Result<bool, EngineError> {
        self.probe.actor_turn_is_active(actor)
    }

    fn shared_script_is_active(&mut self, actor: EntityId) -> Result<bool, EngineError> {
        self.probe.shared_script_is_active(actor)
    }

    fn actor_animation_is_active(&mut self, actor: EntityId) -> Result<bool, EngineError> {
        self.probe.actor_animation_is_active(actor)
    }

    fn field_actor_movement_is_active(&mut self, actor: EntityId) -> Result<bool, EngineError> {
        self.probe.field_actor_movement_is_active(actor)
    }

    fn field_exit_movement_is_active(&mut self, actor: EntityId) -> Result<bool, EngineError> {
        self.probe.field_exit_movement_is_active(actor)
    }

    fn actor_jump_is_active(&mut self, actor: EntityId) -> Result<bool, EngineError> {
        self.probe.actor_jump_is_active(actor)
    }

    fn read_random_u8(&mut self) -> Result<u8, EngineError> {
        Ok(self.rng_draw())
    }

    fn read_key_on(&mut self) -> Result<i32, EngineError> {
        self.probe.read_key_on()
    }

    fn read_key_state(&mut self, state: crate::event::KeyState) -> Result<i32, EngineError> {
        self.probe.read_key_state(state)
    }

    fn read_character_stat(
        &mut self,
        character: i32,
        stat: crate::event::CharacterStat,
    ) -> Result<i32, EngineError> {
        self.probe.read_character_stat(character, stat)
    }

    fn caller_request_acceptable(&mut self, level: i32) -> Result<bool, EngineError> {
        self.probe.caller_request_acceptable(level)
    }

    fn script_request_acceptable(
        &mut self,
        entity: EntityId,
        level: i32,
    ) -> Result<bool, EngineError> {
        self.probe.script_request_acceptable(entity, level)
    }

    fn script_request_is_complete(&mut self, entity: EntityId) -> Result<bool, EngineError> {
        self.probe.script_request_is_complete(entity)
    }

    fn read_custom_variable(&mut self, variable: u16) -> Result<i32, EngineError> {
        self.probe.read_custom_variable(variable)
    }
}

impl BattleScriptHost<'_> {
    fn rng_draw(&mut self) -> u8 {
        self.rng.as_deref_mut().map_or(0, |rng| rng.random8() as u8)
    }
}

// ---------------------------------------------------------------------------
// Machine wiring
// ---------------------------------------------------------------------------

/// Battle phase numbers (`FF9StateBattleSystem`: `PHASE_NORMAL = 4`,
/// `PHASE_MENU_OFF = 5`, `PHASE_VICTORY = 6`, `PHASE_DEFEAT = 7`,
/// `PHASE_CLOSE = 8`).
pub mod phase {
    pub const NORMAL: u32 = 4;
    pub const MENU_OFF: u32 = 5;
    pub const VICTORY: u32 = 6;
    pub const DEFEAT: u32 = 7;
    pub const CLOSE: u32 = 8;
}

/// Snapshots a [`Battle`] into the script views: one [`BattleDataView`] plus
/// per-unit [`UnitView`]s keyed by `EntityId::new(u64::from(unit.id))`, so a
/// caller can run decoded battle scripts against the live machine state with
/// [`BattleScriptHost::with_units`].
///
/// Documented mappings and gaps (v1, read-only wiring):
/// * `seq_busy` approximates `BtlSeqBusy()` with "a command is running";
///   `result_mode` is the victory/defeat phases.
/// * `btl_phase` uses the reference phase numbers; `btl_seq` stays 0 (the
///   machine tracks menu-off/closing sequences as enums, not numbers).
/// * The machine carries no scene (pattern id, start type), party gil, or AP;
///   those fields stay defaulted and callers overlay them.
/// * `column` maps the unit's list index (`bi.line_no` is list position);
///   `disappear` is the boolean scheduler flag; `category` is the player
///   character id (players only); `class` derives from the undead/flight
///   flags (Undead 16, Flight 128, else 0); `enemy_id` (slot number) and
///   `status_invalid` map `status_resist`.
pub fn battle_views(battle: &Battle) -> (BattleDataView, BTreeMap<EntityId, UnitView>) {
    let btl_phase = match battle.phase {
        Phase::Normal => phase::NORMAL,
        Phase::MenuOff => phase::MENU_OFF,
        Phase::Victory => phase::VICTORY,
        Phase::Defeat => phase::DEFEAT,
        Phase::Close => phase::CLOSE,
    };
    let data = BattleDataView {
        seq_busy: !battle.running.is_empty(),
        result_mode: matches!(battle.phase, Phase::Victory | Phase::Defeat),
        battle_result: battle.result.map_or(0, i32::from),
        btl_phase,
        bonus_exp: battle.bonus_exp,
        ..BattleDataView::default()
    };
    let units = battle
        .units
        .iter()
        .zip(&battle.sched)
        .enumerate()
        .map(|(index, (unit, sched))| {
            (
                EntityId::new(u64::from(unit.id)),
                unit_view(unit, sched, index),
            )
        })
        .collect();
    (data, units)
}

fn unit_view(unit: &BattleUnit, sched: &UnitSched, index: usize) -> UnitView {
    UnitView {
        max_hp: unit.max_hp as i32,
        hp: unit.cur_hp as i32,
        max_mp: unit.max_mp as i32,
        mp: unit.cur_mp as i32,
        max_atb: i32::from(sched.max_atb),
        atb: i32::from(unit.cur_atb),
        level: i32::from(unit.level),
        status_invalid: unit.status_resist,
        status_permanent: unit.status_permanent,
        status_current: unit.status_current,
        element_invalid: unit.guard_elements.0,
        element_absorb: unit.absorb_elements.0,
        element_half: unit.half_elements.0,
        element_weak: unit.weak_elements.0,
        targetable: i32::from(sched.targetable),
        disappear: i32::from(sched.disappeared),
        row: i32::from(unit.row),
        column: index as i32,
        category: if unit.is_player {
            i32::from(unit.character_id)
        } else {
            0
        },
        class: if unit.is_undead {
            16 // EnemyCategory::Undead
        } else if unit.is_flight {
            128 // EnemyCategory::Flight
        } else {
            0
        },
        is_player: unit.is_player,
        strength: i32::from(unit.strength),
        magic: i32::from(unit.magic),
        physical_defence: unit.physical_defence,
        physical_evade: unit.physical_evade,
        magical_defence: unit.magic_defence,
        magical_evade: unit.magic_evade,
        bonus_exp: sched.bonus_exp as i32,
        out_of_reach: sched.out_of_reach,
        die_fade_rate: i32::from(sched.die_fade_rate),
        ..UnitView::default()
    }
}

/// Applies script-mutated unit views back onto the machine (the write-back
/// half of [`battle_views`]). Fields the machine does not own (position,
/// model, motion, steal/bonus tables, level) are dropped, as are views for
/// unknown entities.
///
/// Faithfulness notes: hp/mp/ATB and stat writes use the reference's raw
/// casts (`(UInt32)val`, `(Int16)val`, `(Byte)val`); a negative hp write
/// wraps like the source. Trance is not carried by the machine and is not
/// written back.
pub fn apply_unit_views(battle: &mut Battle, units: &BTreeMap<EntityId, UnitView>) {
    for (unit, sched) in battle.units.iter_mut().zip(battle.sched.iter_mut()) {
        let Some(view) = units.get(&EntityId::new(u64::from(unit.id))) else {
            continue;
        };
        unit.cur_hp = view.hp as u32;
        unit.cur_mp = view.mp as u32;
        unit.cur_atb = view.atb as i16;
        unit.status_current = view.status_current;
        unit.status_permanent = view.status_permanent;
        unit.status_resist = view.status_invalid;
        unit.guard_elements.0 = view.element_invalid;
        unit.absorb_elements.0 = view.element_absorb;
        unit.half_elements.0 = view.element_half;
        unit.weak_elements.0 = view.element_weak;
        unit.strength = view.strength as u8;
        unit.magic = view.magic as u8;
        unit.physical_defence = view.physical_defence;
        unit.physical_evade = view.physical_evade;
        unit.magic_defence = view.magical_defence;
        unit.magic_evade = view.magical_evade;
        unit.row = view.row as u8;
        sched.targetable = view.targetable != 0;
        sched.disappeared = view.disappear != 0;
        sched.out_of_reach = view.out_of_reach;
        sched.die_fade_rate = view.die_fade_rate as u8;
        sched.bonus_exp = view.bonus_exp as u32;
    }
}
