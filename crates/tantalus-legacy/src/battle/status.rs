//! Status infliction, resistance and removal.
//!
//! Ports `btl_stat.AlterStatus`, `AlterStatuses`, `RemoveStatus`,
//! `RemoveStatuses`, `SetOprStatusCount` (`Global/btl_stat.cs`) and the
//! `Apply` bodies of the default status scripts
//! (`Memoria/Scripts/DefaultStatus/*.cs`) as far as they change data the
//! formulas read. Visual, sound, ATB-coefficient, queued-command and
//! animation side effects are not modeled (listed per status below).

use super::rng::BattleRandom;
use super::unit::BattleUnit;

// `BattleStatus` bit values, `Memoria/Data/Battle/BattleStatus.cs:11-42`.
pub const PETRIFY: u64 = 1 << 0;
pub const VENOM: u64 = 1 << 1;
pub const VIRUS: u64 = 1 << 2;
pub const SILENCE: u64 = 1 << 3;
pub const BLIND: u64 = 1 << 4;
pub const TROUBLE: u64 = 1 << 5;
pub const ZOMBIE: u64 = 1 << 6;
pub const EASY_KILL: u64 = 1 << 7;
pub const DEATH: u64 = 1 << 8;
pub const LOW_HP: u64 = 1 << 9;
pub const CONFUSE: u64 = 1 << 10;
pub const BERSERK: u64 = 1 << 11;
pub const STOP: u64 = 1 << 12;
pub const AUTO_LIFE: u64 = 1 << 13;
pub const TRANCE: u64 = 1 << 14;
pub const DEFEND: u64 = 1 << 15;
pub const POISON: u64 = 1 << 16;
pub const SLEEP: u64 = 1 << 17;
pub const REGEN: u64 = 1 << 18;
pub const HASTE: u64 = 1 << 19;
pub const SLOW: u64 = 1 << 20;
pub const FLOAT: u64 = 1 << 21;
pub const SHELL: u64 = 1 << 22;
pub const PROTECT: u64 = 1 << 23;
pub const HEAT: u64 = 1 << 24;
pub const FREEZE: u64 = 1 << 25;
pub const VANISH: u64 = 1 << 26;
pub const DOOM: u64 = 1 << 27;
pub const MINI: u64 = 1 << 28;
pub const REFLECT: u64 = 1 << 29;
pub const JUMP: u64 = 1 << 30;
pub const GRADUAL_PETRIFY: u64 = 1 << 31;

// `BattleStatusConst` (`BattleStatus.cs:117-172`), values spelled out from
// the C# initializers.
pub const ANY_POSITIVE: u64 =
    AUTO_LIFE | TRANCE | DEFEND | REGEN | HASTE | FLOAT | SHELL | PROTECT | VANISH | REFLECT;
pub const ANY_NEGATIVE: u64 = PETRIFY
    | VENOM
    | VIRUS
    | SILENCE
    | BLIND
    | TROUBLE
    | ZOMBIE
    | DEATH
    | CONFUSE
    | STOP
    | POISON
    | SLEEP
    | SLOW
    | HEAT
    | FREEZE
    | DOOM
    | MINI
    | GRADUAL_PETRIFY;
pub const CONTI_COUNT: u64 = POISON
    | SLEEP
    | REGEN
    | HASTE
    | SLOW
    | FLOAT
    | SHELL
    | PROTECT
    | HEAT
    | FREEZE
    | VANISH
    | REFLECT;
pub const NO_RESET: u64 = PETRIFY
    | VENOM
    | VIRUS
    | SILENCE
    | BLIND
    | TROUBLE
    | ZOMBIE
    | DEATH
    | LOW_HP
    | CONFUSE
    | BERSERK
    | STOP
    | AUTO_LIFE
    | TRANCE
    | DEFEND
    | DOOM
    | GRADUAL_PETRIFY;
pub const BATTLE_END: u64 = PETRIFY | VENOM | STOP;
pub const PENALTY_EVADE: u64 = PETRIFY | VENOM | VIRUS | BLIND | CONFUSE | STOP | SLEEP | FREEZE;
pub const REMOVE_ON_PHYSICALLY_ATTACKED: u64 = CONFUSE | SLEEP;
pub const REMOVE_ON_MAGICALLY_ATTACKED: u64 = VANISH;
pub const CANNOT_TRANCE: u64 = PETRIFY | VENOM | ZOMBIE | DEATH | STOP | TRANCE | FREEZE;
pub const ZOMBIE_EFFECT: u64 = ZOMBIE;
pub const APPLY_REFLECT: u64 = REFLECT;
pub const APPLY_TROUBLE: u64 = TROUBLE;

/// Result of `AlterStatus`, `btl_stat.cs:556-559`. The numeric order matters:
/// `AlterStatuses` keeps the maximum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AlterResult {
    /// `ALTER_RESIST` 0: blocked by resistance ("Guard").
    Resist = 0,
    /// `ALTER_INVALID` 1: blocked for another reason ("Miss").
    Invalid = 1,
    /// `ALTER_SUCCESS_NO_SET` 2: had an effect, status not registered.
    SuccessNoSet = 2,
    /// `ALTER_SUCCESS` 3.
    Success = 3,
}

/// `RemoveStatus` return values (`btl_stat.cs:178,196`): 1 = nothing removed,
/// 2 = removed.
pub const REMOVE_NOTHING: u32 = 1;
pub const REMOVE_DONE: u32 = 2;

/// One row of `BattleStatusDataEntry` (`Memoria/Data/Battle/BattleStatusDataEntry.cs:13-19`).
/// The vanilla table (`battle_tables.rs` `StatusDataRow`) maps
/// `opr_cnt` -> `opr_cnt`, `conti_cnt` -> `conti_cnt`, `clear` ->
/// `clear_on_apply`, `invalid` -> `immunity_provided`; the mapping was checked
/// against the shipped `StatusData.csv` header (`ClearOnApply`,
/// `ImmunityProvided`) and the Petrify row (clear = Doom, immunity = every
/// other status).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StatusDataRow {
    pub opr_cnt: u8,
    pub conti_cnt: u16,
    pub clear_on_apply: u64,
    pub immunity_provided: u64,
}

/// Status data indexed by `BattleStatusId` (0..=31 supplied by the vanilla
/// table; ids above are treated as unknown and always `Invalid`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatusTable {
    pub rows: Vec<StatusDataRow>,
}

impl StatusTable {
    pub fn new(rows: Vec<StatusDataRow>) -> Self {
        Self { rows }
    }

    pub fn row(&self, id: u8) -> Option<&StatusDataRow> {
        self.rows.get(usize::from(id))
    }
}

/// Battle-wide switches the status code reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StatusEnv {
    /// `FF9Battle.btl_phase > PHASE_ENTER` (`PetrifyStatusScript.cs`).
    pub phase_after_enter: bool,
    /// `Configuration.Battle.Speed` (`PetrifyStatusScript.cs`).
    pub battle_speed: i32,
    /// `btl_phase == PHASE_MENU_OFF` (`btl_stat.RemoveStatus`, `btl_stat.cs:178`).
    pub menu_off_phase: bool,
}

pub struct StatusCtx<'a> {
    pub table: &'a StatusTable,
    pub env: &'a StatusEnv,
}

fn ids_of(mask: u64) -> impl Iterator<Item = u8> {
    (0u8..64).filter(move |i| mask & (1u64 << i) != 0)
}

/// `btl_stat.RemoveStatus` (`btl_stat.cs:173`). Script `Remove()` bodies only
/// matter here for Death (`death_f` cleared, `DeathStatusScript.cs:46`); the
/// others are animation or ATB-coefficient only.
pub fn remove_status(unit: &mut BattleUnit, id: u8, env: &StatusEnv) -> u32 {
    let status = 1u64 << id;
    if unit.status_permanent & status != 0
        || unit.status_current & status == 0
        || (!unit.is_player && env.menu_off_phase && status & BATTLE_END != 0)
    {
        return REMOVE_NOTHING;
    }
    unit.status_current &= !status;
    if status == DEATH {
        unit.death_flag = false;
    }
    REMOVE_DONE
}

/// `btl_stat.RemoveStatuses` (`btl_stat.cs:195`): maximum over the list,
/// starting at 1.
pub fn remove_statuses(unit: &mut BattleUnit, mask: u64, env: &StatusEnv) -> u32 {
    let mut result = REMOVE_NOTHING;
    for id in ids_of(mask) {
        result = result.max(remove_status(unit, id, env));
    }
    result
}

/// `Apply` of the default status script for `id` (`DefaultStatus/*.cs`).
///
/// Modeled: Berserk and Confuse (attack-command check), Haste and Slow
/// (opposite status swap), Mini (toggle), Petrify (deferred through the
/// `SysStone` command or immediate), Death, EasyKill (Death resist), Zombie
/// (trance gauge reset), Doom and Gradual Petrify (counter 10).
/// Not modeled because they only touch ATB coefficient, HUD, models or the
/// command queue: Haste/Slow ATB coefficient, Trance `SysTrans` command,
/// Jump countdown and target, Vanish/Float/Confuse delayed modifiers,
/// Death's `SysTrans` removal, `ChangeStat` (status id 32).
fn apply_script(unit: &mut BattleUnit, id: u8, ctx: &StatusCtx<'_>) -> AlterResult {
    let status = 1u64 << id;
    match status {
        BERSERK | CONFUSE => {
            if !unit.can_use_attack_command {
                return AlterResult::Resist;
            }
        }
        HASTE => {
            if unit.status_current & SLOW != 0 {
                return if remove_status(unit, 20, ctx.env) == REMOVE_DONE {
                    AlterResult::SuccessNoSet
                } else {
                    AlterResult::Resist
                };
            }
        }
        SLOW => {
            if unit.status_current & HASTE != 0 {
                return if remove_status(unit, 19, ctx.env) == REMOVE_DONE {
                    AlterResult::SuccessNoSet
                } else {
                    AlterResult::Resist
                };
            }
        }
        MINI => {
            if unit.status_permanent & MINI != 0 {
                return AlterResult::Invalid;
            }
            if unit.status_current & MINI != 0 {
                remove_status(unit, 28, ctx.env);
                return AlterResult::SuccessNoSet;
            }
        }
        PETRIFY => {
            remove_status(unit, 31, ctx.env);
            if !unit.petrify_command_pending
                && ctx.env.phase_after_enter
                && ctx.env.battle_speed < 3
            {
                unit.petrify_command_pending = true;
                return AlterResult::SuccessNoSet;
            }
            unit.cur_atb = 0;
            unit.petrify_command_pending = false;
        }
        DEATH => {
            if unit.cur_hp > 0 {
                unit.kill();
            } else {
                unit.set_current_hp(0);
            }
            unit.cur_atb = 0;
        }
        EASY_KILL => unit.status_resist |= DEATH,
        ZOMBIE => {
            if unit.is_player && unit.status_current & TRANCE == 0 {
                unit.trance = 0;
            }
        }
        DOOM => unit.doom_counter = 10,
        GRADUAL_PETRIFY => unit.gradual_petrify_counter = 10,
        _ => {}
    }
    AlterResult::Success
}

/// `btl_stat.SetOprStatusCount` (`btl_stat.cs:127`) for the statuses with an
/// `IOprStatusScript`: Venom, Poison, Regen, Doom, Gradual Petrify.
fn set_opr_status_count(unit: &mut BattleUnit, id: u8, ctx: &StatusCtx<'_>) {
    let status = 1u64 << id;
    if status & (VENOM | POISON | REGEN | DOOM | GRADUAL_PETRIFY) == 0 {
        return;
    }
    let Some(row) = ctx.table.row(id) else {
        return;
    };
    let will = i32::from(unit.will);
    let default_factor: u16 = if status == VENOM || status == POISON {
        ((will as u32) << 2) as u16
    } else {
        ((60 - will) << 2) as u16
    };
    let mut opr = i32::from((i32::from(row.opr_cnt) * i32::from(default_factor)) as i16);
    if status == DOOM || status == GRADUAL_PETRIFY {
        // `SetupDoomOpr` / `SetupGradualPetrifyOpr`: single-precision math in
        // the source (`DoomStatusScript.cs:34`).
        let factor = unit.duration_factor.get(&id).copied().unwrap_or(1.0);
        let value = factor * f32::from(row.conti_cnt) * ((60 - will) << 3) as f32 / 10.0;
        opr = value as i32;
    }
    unit.opr.insert(id, opr);
}

/// `btl_stat.AlterStatus` (`btl_stat.cs:57`) for one status id.
///
/// Memoria extension (not in the vanilla game): `partial_resist` and the
/// `bypassResistances` rule for Death on a 0 HP target are Memoria's.
/// `StatusDurationFormula` and `StatusTickFormula` are empty by default.
pub fn alter_status(
    unit: &mut BattleUnit,
    id: u8,
    use_partial_resist: bool,
    ctx: &StatusCtx<'_>,
    rng: &mut dyn BattleRandom,
) -> AlterResult {
    let Some(row) = ctx.table.row(id).copied() else {
        return AlterResult::Invalid;
    };
    let status = 1u64 << id;
    let bypass = status == DEATH && unit.cur_hp == 0;
    if !bypass && status & unit.status_resist != 0 {
        return AlterResult::Resist;
    }
    if !bypass && use_partial_resist {
        let rate = unit.partial_resist.get(&id).copied().unwrap_or(0.0);
        if rate > 0.0 && rng.random_unit() < rate {
            return AlterResult::Invalid;
        }
    }
    if status & unit.status_permanent != 0
        || (status & unit.status_current != 0 && status & NO_RESET != 0)
    {
        return AlterResult::Invalid;
    }
    let mut invalid = if unit.has_trance_gauge { 0 } else { TRANCE };
    for cur in ids_of(unit.status_current) {
        if let Some(cur_row) = ctx.table.row(cur) {
            invalid |= cur_row.immunity_provided;
        }
    }
    if unit.petrify_command_pending
        && let Some(petrify_row) = ctx.table.row(0)
    {
        invalid |= petrify_row.immunity_provided;
    }
    if !bypass && status & invalid != 0 {
        return AlterResult::Invalid;
    }
    let result = apply_script(unit, id, ctx);
    if result != AlterResult::Success {
        return result;
    }
    unit.status_current |= status;
    remove_statuses(unit, row.clear_on_apply, ctx.env);
    if status & CONTI_COUNT != 0 {
        let will = i32::from(unit.will);
        let default_factor: i16 = if status & ANY_NEGATIVE != 0 {
            ((60 - will) << 3) as i16
        } else if status & ANY_POSITIVE != 0 {
            (will << 3) as i16
        } else {
            ((60 - will) << 2) as i16
        };
        let base = i32::from((i32::from(row.conti_cnt) * i32::from(default_factor)) as i16);
        let factor = unit.duration_factor.get(&id).copied().unwrap_or(1.0);
        let conti = i32::from((factor * base as f32) as i32 as i16);
        unit.conti.insert(id, conti);
    }
    set_opr_status_count(unit, id, ctx);
    AlterResult::Success
}

/// `btl_stat.AlterStatuses` (`btl_stat.cs:116`): ascending id order, keeps the
/// best result, starts at `Resist` (so an empty list reads as "resisted").
pub fn alter_statuses(
    unit: &mut BattleUnit,
    mask: u64,
    use_partial_resist: bool,
    ctx: &StatusCtx<'_>,
    rng: &mut dyn BattleRandom,
) -> AlterResult {
    let mut best = AlterResult::Resist;
    for id in ids_of(mask) {
        best = best.max(alter_status(unit, id, use_partial_resist, ctx, rng));
    }
    best
}

/// `btl_para.CheckPointDataStatus` (`btl_para.cs:94`), default (no overload)
/// method: Death at 0 HP; otherwise Low HP for a player at or under 1/6 max
/// HP, which also adds or removes the Low HP status as a side effect.
pub fn check_point_status(
    unit: &mut BattleUnit,
    ctx: &StatusCtx<'_>,
    rng: &mut dyn BattleRandom,
) -> u64 {
    if unit.cur_hp == 0 {
        return DEATH;
    }
    let low = unit.is_player && unit.cur_hp.wrapping_mul(6) <= unit.max_hp;
    if low {
        if unit.status_current & LOW_HP == 0 {
            alter_status(unit, 9, false, ctx, rng);
        }
        LOW_HP
    } else {
        remove_status(unit, 9, ctx.env);
        0
    }
}
