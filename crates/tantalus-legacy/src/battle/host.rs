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
//! * Object-member reads (`GetBattleCharData`) are not modeled; they use the
//!   probe default.

use super::rng::BattleRandom;
use crate::event::probe::ProbeHost;
use crate::event::{LegacyEventHost, ObjectMemberRef};
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
    sys_list: [u16; 8],
    rng: Option<&'a mut dyn BattleRandom>,
    probe: ProbeHost,
}

impl<'a> BattleScriptHost<'a> {
    pub fn new(data: &'a BattleDataView) -> Self {
        Self {
            data,
            sys_list: [0; 8],
            rng: None,
            probe: ProbeHost,
        }
    }

    pub fn with_random(data: &'a BattleDataView, rng: &'a mut dyn BattleRandom) -> Self {
        Self {
            data,
            sys_list: [0; 8],
            rng: Some(rng),
            probe: ProbeHost,
        }
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

    fn read_object_member(&mut self, reference: ObjectMemberRef) -> Result<i32, EngineError> {
        self.probe.read_object_member(reference)
    }

    fn select_object_member_target(&mut self, slot: Option<u8>) -> Result<EntityId, EngineError> {
        self.probe.select_object_member_target(slot)
    }

    fn write_object_member(
        &mut self,
        reference: ObjectMemberRef,
        value: i32,
    ) -> Result<(), EngineError> {
        self.probe.write_object_member(reference, value)
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
