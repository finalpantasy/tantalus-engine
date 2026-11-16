//! Command data the formulas read: `CMD_DATA` (`Global/CMD_DATA.cs:20-60`)
//! through `BattleCommand` (`Memoria/Battle/Calculator/BattleCommand.cs`).

use super::unit::Elements;

/// `BattleCommandId.SummonEiko` (`Memoria/Data/Battle/BattleCommandId.cs:25`).
pub const COMMAND_SUMMON_EIKO: u32 = 20;
/// `BattleCommandId.MagicCounter` (`BattleCommandId.cs:55`).
pub const COMMAND_MAGIC_COUNTER: u32 = 50;

/// `ITEM_DATA` fields read by item scripts (`BattleCommand.cs:24-28`):
/// `Ref.Power`, `Ref.Rate`, `status`, `Ref.Elements`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ItemEffect {
    pub power: i32,
    pub hit_rate: i32,
    pub status: u64,
    pub elements: Elements,
}

/// What the item tables give an Item command: `ff9item.GetItemEffect(item)`
/// (`ITEM_DATA`): `Ref.ScriptId`, the [`ItemEffect`] fields and `info.ForDead`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ItemData {
    pub effect: ItemEffect,
    pub script_id: i32,
    pub for_dead: bool,
}

/// One executing command. For an `AA_DATA` row, `CMD_DATA.SetAAData`
/// (`CMD_DATA.cs:38-54`) copies: `script_id` = `Ref.ScriptId` (table
/// `btl_ref.prog_no`), `power` = `Ref.Power`, `hit_rate` = `Ref.Rate`,
/// `element` = `element_for_bonus` = `Ref.Elements` (table `btl_ref.attr`),
/// `ability_status` = `add_status[AddStatusNo]` (table `status_sets[add_no]`),
/// `ability_category` = `Category`, `ability_type` = `Type`, `mp_cost` = `MP`,
/// `target_type` = `Info.Target` (table `info.cursor`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BattleCommand {
    /// `cmd_no`.
    pub command_id: u32,
    /// `tar_id`.
    pub target_id: u16,
    pub script_id: i32,
    pub power: i32,
    pub hit_rate: i32,
    pub element: Elements,
    pub element_for_bonus: Elements,
    pub ability_status: u64,
    /// `AbilityCategory`: 8 = physical, 16 = magical, 64 = no damage motion
    /// (`SBattleCalculator.cs:94,191,197,339`).
    pub ability_category: u8,
    /// `AbilityType`: bit 4 marks a Garnet summon cost multiplier
    /// (`btl_cmd.cs:1208`).
    pub ability_type: u8,
    /// `aa.Info.Target` (`TargetType`, `TargetType.cs`).
    pub target_type: u8,
    /// `info.cursor & 1` (`BattleCommand.cs:110`).
    pub is_many_target: bool,
    /// `IsShortRange` (`btl_util.IsAttackShortRange`, `btl_util.cs:311`).
    pub is_short_range: bool,
    /// `info.ReflectNull`.
    pub is_reflect_null: bool,
    /// `info.short_summon != 0`.
    pub is_short_summon: bool,
    /// `Some(reflec.tar_id[0..4])` when `info.reflec == 1`, else `None`
    /// (`BattleCommand.GetReflectMultiplierOnTarget`, `BattleCommand.cs:204`).
    pub reflect_target_ids: Option<[u16; 4]>,
    /// `aa.MP`.
    pub mp_cost: i32,
    /// `info.CustomMPCost`.
    pub custom_mp_cost: Option<i32>,
    /// `Item` when `ItemId != NoItem`.
    pub item: Option<ItemEffect>,
}

impl BattleCommand {
    pub fn new(script_id: i32) -> Self {
        Self {
            command_id: 0,
            target_id: 0,
            script_id,
            power: 0,
            hit_rate: 0,
            element: Elements::NONE,
            element_for_bonus: Elements::NONE,
            ability_status: 0,
            ability_category: 0,
            ability_type: 0,
            target_type: 0,
            is_many_target: false,
            is_short_range: false,
            is_reflect_null: false,
            is_short_summon: false,
            reflect_target_ids: None,
            mp_cost: 0,
            custom_mp_cost: None,
            item: None,
        }
    }

    /// `IsDevided` (`BattleCommand.cs:129`): a many-target command whose
    /// target type is `ManyAny..=ManyEnemy` (3..=5).
    pub fn is_divided(&self) -> bool {
        self.is_many_target && (3..=5).contains(&self.target_type)
    }

    /// `IsGround` (`BattleCommand.cs:138`): the command has the Earth element.
    pub fn is_ground(&self) -> bool {
        self.element.contains(Elements::EARTH)
    }

    /// `GetReflectMultiplierOnTarget` (`BattleCommand.cs:204`).
    pub fn reflect_multiplier_on_target(&self, target_id: u16) -> i32 {
        let Some(ids) = self.reflect_target_ids else {
            return 1;
        };
        let count = ids.iter().filter(|id| **id & target_id != 0).count() as i32;
        count.max(1)
    }

    /// `GetCommandMPCost` (`CMD_DATA.cs:56`).
    pub fn command_mp_cost(&self) -> i32 {
        self.custom_mp_cost.unwrap_or(self.mp_cost)
    }
}
