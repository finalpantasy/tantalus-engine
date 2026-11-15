//! MP cost. FF9 has no HP-cost command in the pinned source; only MP is
//! charged (`btl_cmd.CheckMpCondition` / `ConsumeMp`).

use super::command::{BattleCommand, COMMAND_MAGIC_COUNTER};
use super::unit::BattleUnit;

/// `btl_cmd.CheckMpCondition` cost part (`Global/btl_cmd.cs:1198-1214`):
/// `GetCommandMPCost`, times 4 for a Garnet summon while
/// `battle.GARNET_SUMMON_FLAG` is set and `AbilityType & 4`, then for a
/// player `mp * mpCostFactor / 100` (default factor 100, `ff9play.cs:313`).
/// `BattleAbilityHelper.GetPatchedMPCost` (ability features) is not modeled.
pub fn command_mp_cost(
    command: &BattleCommand,
    caster: &BattleUnit,
    garnet_summon_flag: bool,
    mp_cost_factor: i32,
) -> i32 {
    let mut mp = command.command_mp_cost();
    if garnet_summon_flag && command.ability_type & 4 != 0 {
        mp = mp.wrapping_mul(4);
    }
    if caster.is_player {
        mp = mp.wrapping_mul(mp_cost_factor) / 100;
    }
    mp
}

/// `CheckMpCondition` result and effect (`btl_cmd.cs:1198-1218`,
/// `ConsumeMp` at `:1655`). `MagicCounter` is free and never deducts. Returns
/// false when the caster has less MP than the cost ("not enough MP").
/// `IsHpMpFull` and debug mode are not modeled.
pub fn try_consume_mp(
    command: &BattleCommand,
    caster: &mut BattleUnit,
    garnet_summon_flag: bool,
    mp_cost_factor: i32,
) -> bool {
    let mp = command_mp_cost(command, caster, garnet_summon_flag, mp_cost_factor);
    if command.command_id == COMMAND_MAGIC_COUNTER {
        return true;
    }
    if i64::from(caster.cur_mp) < i64::from(mp) {
        return false;
    }
    caster.cur_mp = caster.cur_mp.wrapping_sub(mp as u32);
    true
}
