//! ATB gauge timing (milestone B5).
//!
//! Source paths are relative to `Assembly-CSharp/` of the Memoria pin
//! `c716e1fa`.
//!
//! Tick rate. One battle tick is one call of
//! `HonoluluBattleMain.UpdateBattleFrame` (`Global/Honolulu/HonoluluBattleMain.cs:669`),
//! which runs `battle.BattleMain()` and then `YMenu_ManagerActiveTime()`
//! (the ATB add, `:405`). The shipped tick rate is `BattleTPS = 15`
//! (`Memoria/Configuration/Structure/GraphicsSection.cs:37`); the add per tick
//! is independent of the real frame rate.

use super::rng::BattleRandom;

/// `Memoria.Configuration.Graphics.BattleTPS` default
/// (`Memoria/Configuration/Structure/GraphicsSection.cs:37`).
pub const BATTLE_TPS: u32 = 15;

/// `ATBSpeedFactor` default, `BindSingle(nameof(ATBSpeedFactor), 4.0f)`
/// (`Memoria/Configuration/Structure/BattleSection.cs:64`).
pub const DEFAULT_ATB_SPEED_FACTOR: f32 = 4.0;

/// `FF9CFG` default `btl_speed = 1` (`Global/ff9/FF9CFG.cs:16`).
pub const DEFAULT_BTL_SPEED_CONFIG: u8 = 1;

/// `Configuration.Battle.ATBSpeedFactor` getter
/// (`Memoria/Configuration/Access/Battle.cs:17`): values below 1.0 read as 1.0.
pub fn clamp_speed_factor(factor: f32) -> f32 {
    if factor < 1.0 { 1.0 } else { factor }
}

/// `btl_para.GetATBCoef` (`Global/btl_para.cs:66-75`): config 0 -> 8,
/// 1 -> 10, 2 -> 14, anything else -> 10.
pub fn base_atb_coef(btl_speed_config: u8) -> i32 {
    match btl_speed_config {
        0 => 8,
        1 => 10,
        2 => 14,
        _ => 10,
    }
}

/// `btl_para.SetupATBCoef` stores an `SByte` (`btl_para.cs:61-64`).
pub fn setup_atb_coef(coef: i32) -> i8 {
    coef as i8
}

/// Coefficient under Haste, `GetATBCoef() * 3 / 2` with C# integer division
/// (`Memoria/Scripts/DefaultStatus/HasteStatusScript.cs:19`).
pub fn hasted_coef(base: i32) -> i8 {
    setup_atb_coef(base * 3 / 2)
}

/// Coefficient under Slow, `GetATBCoef() * 2 / 3`
/// (`Memoria/Scripts/DefaultStatus/SlowStatusScript.cs:19`).
pub fn slowed_coef(base: i32) -> i8 {
    setup_atb_coef(base * 2 / 3)
}

/// `btl_para.GetMaxATB` (`Global/btl_para.cs:56-59`):
/// `(Int16)((60 - Dexterity) * 40 << 2)`. `*` binds tighter than `<<` in C#.
/// A Dexterity above 59 gives a value at or below 0; the source then divides
/// by it in `random16() % max.at` (`btl_init.cs:430`), which this port reports
/// as an error rather than inventing a result.
pub fn max_atb(dexterity: u8) -> i16 {
    (((60 - i32::from(dexterity)) * 40) << 2) as i16
}

/// The add per tick: `(Int16)(at_coef * ATBSpeedFactor)`
/// (`HonoluluBattleMain.cs:459`). `SByte * Single` is a float product; the
/// cast to `Int16` truncates toward zero.
pub fn atb_step(coef: i8, speed_factor: f32) -> i16 {
    (f32::from(coef) * speed_factor) as i16
}

/// Where a battle starts: `battle_start_type_tags`
/// (`FF9/btl_sys.cs:181`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartType {
    Normal,
    /// `BTL_START_BACK_ATTACK`: players start at 0.
    BackAttack,
    /// `BTL_START_FIRST_ATTACK` (preemptive): players at `max - 1`, enemies 0.
    FirstAttack,
}

/// Initial gauge of a player (`btl_init.OrganizePlayerData`,
/// `Global/btl_init.cs:422-431`): back attack 0, preemptive `max - 1`,
/// otherwise `random16() % max`.
pub fn initial_player_atb(max: i16, start: StartType, rng: &mut dyn BattleRandom) -> Option<i16> {
    match start {
        StartType::BackAttack => Some(0),
        StartType::FirstAttack => Some(max.wrapping_sub(1)),
        StartType::Normal => {
            if max <= 0 {
                return None;
            }
            Some((rng.random16() % i32::from(max)) as i16)
        }
    }
}

/// Initial gauge of an enemy (`btl_init.OrganizeEnemyData`, `btl_init.cs:494`,
/// then the start-type override at `btl_init.cs:295-300`): `random16() % max`
/// is always drawn, and a preemptive start overwrites it with 0. The draw
/// still happens, so the draw order is preserved.
pub fn initial_enemy_atb(max: i16, start: StartType, rng: &mut dyn BattleRandom) -> Option<i16> {
    if max <= 0 {
        return None;
    }
    let drawn = (rng.random16() % i32::from(max)) as i16;
    Some(if start == StartType::FirstAttack {
        0
    } else {
        drawn
    })
}

/// What one unit's gauge step did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GaugeStep {
    /// The unit is skipped (dead, selecting, or ATB switched off).
    Skipped,
    /// Still below max after this tick.
    Filling,
    /// At max this tick (the unit is ready to act, subject to
    /// `PreventATBConfirm`).
    Full,
}

/// Inputs of one gauge step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GaugeInput {
    pub max_atb: i16,
    pub coef: i8,
    pub speed_factor: f32,
    /// `FF9BMenu_IsEnableAtb()` (`HonoluluBattleMain.cs:440`).
    pub advance: bool,
    /// `Configuration.Battle.IsKeepRestTimeInBattle` (default false).
    pub keep_rest_time: bool,
    pub hp: u32,
    /// `BTL_DATA.sel_mode`.
    pub sel_mode: u8,
    /// `bi.atb != 0`.
    pub atb_active: bool,
}

/// The gauge part of `ProcessActiveTime` for one unit
/// (`HonoluluBattleMain.cs:445-465,492-493`), battle speed mode 0.
///
/// When the gauge is below max and `advance` is false it does not move.
pub fn step_gauge(cur_atb: &mut i16, g: GaugeInput) -> GaugeStep {
    if g.hp == 0 || g.sel_mode != 0 || !g.atb_active {
        return GaugeStep::Skipped;
    }
    if *cur_atb < g.max_atb && g.advance {
        *cur_atb = cur_atb.wrapping_add(atb_step(g.coef, g.speed_factor));
    }
    if *cur_atb < g.max_atb {
        return GaugeStep::Filling;
    }
    if !g.keep_rest_time {
        *cur_atb = g.max_atb;
    }
    GaugeStep::Full
}
