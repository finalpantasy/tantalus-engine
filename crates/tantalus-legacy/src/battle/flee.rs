//! Fleeing (milestone B5).
//!
//! Two entry points exist in the source:
//!
//! * The party flee key (hold both shoulder buttons). After the key has been
//!   held for more than one second of real time `BattleHUD.Update` calls
//!   `btl_sys.CheckEscape(true)` (`Global/battle/BattleHUD/BattleHUD.Unity.cs:44-54`),
//!   which runs script 56 (`FF9/btl_sys.cs:17,306-321`,
//!   `Memoria.Scripts/Sources/Battle/0056_FleeIterationScript.cs`):
//!   `if (CanEscape()) TryEscape();`. `TryEscape` rolls the success chance.
//!   The one-second hold is UI real time, not a tick count, so the caller
//!   decides when to call [`try_escape_roll`].
//! * The Flee ability (script 57, `0057_FleeScript.cs`): when `CanEscape()`
//!   it always queues `SysEscape` with `sub_no 0`; that variant costs gil at
//!   the end of the battle (`battle.cs:456-473`).

use super::rng::BattleRandom;

/// Side totals read by `BattleCalculator.TryEscape`
/// (`Memoria/Battle/Calculator/BattleCalculator.cs:355-373`): the loop counts
/// every unit in `btl_list` (dead ones included) and sums `BTL_DATA.level`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EscapeSides {
    pub player_count: i32,
    pub enemy_count: i32,
    pub player_levels: i32,
    pub enemy_levels: i32,
}

/// `CanEscape` (`BattleCalculator.cs:305`): the scene allows running and some
/// player is not under a `CannotEscape` status.
pub fn can_escape(runaway: bool, any_player_can_escape: bool) -> bool {
    runaway && any_player_can_escape
}

/// The success threshold in `TryEscape` (`BattleCalculator.cs:375-384`):
/// counts and level sums of 0 read as 1, then
/// `(Int16)(200 / (enemyLevels / enemyCount) * (playerLevels / playerCount) / 16)`
/// with C# integer division left to right.
pub fn escape_rate(sides: EscapeSides) -> Result<i16, &'static str> {
    let enemy_count = if sides.enemy_count == 0 {
        1
    } else {
        sides.enemy_count
    };
    let player_count = if sides.player_count == 0 {
        1
    } else {
        sides.player_count
    };
    let enemy_levels = if sides.enemy_levels == 0 {
        1
    } else {
        sides.enemy_levels
    };
    let player_levels = if sides.player_levels == 0 {
        1
    } else {
        sides.player_levels
    };
    let avg_enemy = enemy_levels / enemy_count;
    if avg_enemy == 0 {
        // `200 / 0` throws in the source (average enemy level under 1 with a
        // nonzero sum); not invented here.
        return Err("average enemy level is 0 (division by zero in TryEscape)");
    }
    Ok((200 / avg_enemy * (player_levels / player_count) / 16) as i16)
}

/// The roll in `TryEscape` (`BattleCalculator.cs:385`):
/// `rate > random16() % 100`. One `random16` draw. (The earlier return when an
/// enemy command is running is the caller's check.)
pub fn try_escape_roll(rate: i16, rng: &mut dyn BattleRandom) -> bool {
    i32::from(rate) > rng.random16() % 100
}

/// `BTL_FLAG_ABILITY_FLEE` gil loss (`Global/battle/battle.cs:456-471`):
/// a tenth of the gil the battle would have paid, taken from the party, capped
/// at what the party has. Returns `(lost, remaining)`.
pub fn flee_gil_loss(battle_gil: u32, enemy_bonus_gil: u32, party_gil: u32) -> (u32, u32) {
    let gil = battle_gil.wrapping_add(enemy_bonus_gil);
    let lost = gil / 10;
    if party_gil > lost {
        (lost, party_gil - lost)
    } else {
        (party_gil, 0)
    }
}
