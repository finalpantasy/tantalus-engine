//! Target selection rules (milestone B5).
//!
//! A target set is a bit mask of `btl_id`s: players use bits 0..=3 (`0x0F`),
//! enemies bits 4..=7 (`0xF0`), everyone `0xFF` (`btl_cmd.cs:185-191`).

use super::rng::BattleRandom;

/// `TargetType` (`Memoria/Data/Battle/TargetType.cs`).
pub mod target_type {
    pub const SINGLE_ANY: u8 = 0;
    pub const SINGLE_ALLY: u8 = 1;
    pub const SINGLE_ENEMY: u8 = 2;
    pub const MANY_ANY: u8 = 3;
    pub const MANY_ALLY: u8 = 4;
    pub const MANY_ENEMY: u8 = 5;
    pub const ALL: u8 = 6;
    pub const ALL_ALLY: u8 = 7;
    pub const ALL_ENEMY: u8 = 8;
    pub const RANDOM: u8 = 9;
    pub const RANDOM_ALLY: u8 = 10;
    pub const RANDOM_ENEMY: u8 = 11;
    pub const EVERYONE: u8 = 12;
    pub const SELF: u8 = 13;
    pub const AUTOMATIC: u8 = 14;
    pub const SPECIAL: u8 = 15;
}

/// `Comn.countBits` (`FF9/Comn.cs:41`).
pub fn count_bits(mut mask: u64) -> u32 {
    let mut n = 0;
    while mask != 0 {
        n += 1;
        mask &= mask - 1;
    }
    n
}

/// `Comn.randomID` (`FF9/Comn.cs:19-38`): pick one set bit of `mask` with
/// `index = count * random8() >> 8`. No draw when the mask is empty.
pub fn random_id(mask: u32, rng: &mut dyn BattleRandom) -> u32 {
    let bits: Vec<u32> = (0..32).filter(|b| mask & (1 << b) != 0).collect();
    if bits.is_empty() {
        return 0;
    }
    let index = (bits.len() as i32 * rng.random8()) >> 8;
    1u32 << bits[index as usize]
}

/// A unit as the target rules see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetView {
    pub btl_id: u16,
    pub is_player: bool,
    pub dead: bool,
    /// `bi.target != 0`.
    pub targetable: bool,
    /// `out_of_reach`.
    pub out_of_reach: bool,
}

/// `btl_util.GetRandomBtlID` (`FF9/btl_util.cs:232`): one random unit of the
/// player side (`player` true) or the enemy side, alive unless `allow_dead`,
/// targetable. The draw is `UnityEngine.Random.Range(0, count)`, taken even
/// for a single candidate; no draw when there is none.
pub fn get_random_btl_id(
    units: &[TargetView],
    player: bool,
    allow_dead: bool,
    rng: &mut dyn BattleRandom,
) -> u16 {
    let candidates: Vec<u16> = units
        .iter()
        .filter(|u| u.is_player == player && (allow_dead || !u.dead) && u.targetable)
        .map(|u| u.btl_id)
        .collect();
    if candidates.is_empty() {
        return 0;
    }
    let i = rng.random_range(0, candidates.len() as i32);
    candidates[i as usize]
}

/// `btl_cmd.GetRandomTargetForCommand` (`btl_cmd.cs:313-334`), for counter
/// attacks. `target` is the command's `TargetType`.
pub fn random_target_for_command(
    units: &[TargetView],
    caster_id: u16,
    target: u8,
    default_ally: bool,
    for_dead: bool,
    rng: &mut dyn BattleRandom,
) -> u16 {
    if target == target_type::ALL {
        return 0xFF;
    }
    if target == target_type::SELF {
        return caster_id;
    }
    let mut mask: u16 = if default_ally { 0x0F } else { 0xF0 };
    if !for_dead {
        for u in units {
            if u.btl_id & mask != 0 && u.dead {
                mask &= !u.btl_id;
            }
        }
    }
    if target != target_type::ALL
        && target != target_type::ALL_ALLY
        && target != target_type::ALL_ENEMY
    {
        mask = random_id(u32::from(mask), rng) as u16;
    }
    mask
}

/// Result of `CheckTargetCondition`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetCheck {
    /// Valid; the (possibly narrowed or re-picked) target mask.
    Valid(u16),
    /// No valid target.
    Invalid,
}

/// `btl_cmd.CheckTargetCondition` (`btl_cmd.cs:1221-1272`) after `forDead` is
/// known. Keeps targetable units in `tar_id` that are alive (or any player
/// when `for_dead`) and, for a short-range command, in reach. When none
/// remain and the command is single-target (`cursor & 1 == 0`) a new target is
/// drawn: the short-range branch picks with `Comn.randomID` among units in
/// reach on the sides present in `tar_id`; otherwise
/// `GetRandomBtlID(tar_id & 15, for_dead)` (side of the low nibble, so an
/// enemy-only mask re-picks an enemy).
pub fn check_target_condition(
    units: &[TargetView],
    tar_id: u16,
    cursor: u8,
    short_range: bool,
    for_dead: bool,
    rng: &mut dyn BattleRandom,
) -> TargetCheck {
    if tar_id == 0 {
        return TargetCheck::Invalid;
    }
    let mut valid: u16 = 0;
    for u in units {
        if u.targetable
            && u.btl_id & tar_id != 0
            && ((for_dead && u.is_player) || !u.dead)
            && (!u.out_of_reach || !short_range)
        {
            valid |= u.btl_id;
        }
    }
    if valid != 0 {
        return TargetCheck::Valid(valid);
    }
    if cursor & 1 != 0 {
        return TargetCheck::Invalid;
    }
    let picked: u16;
    if short_range {
        let allow_player = tar_id & 0x0F != 0;
        let allow_enemy = tar_id & 0xF0 != 0;
        let mut in_range: u16 = 0;
        for u in units {
            if ((allow_enemy && !u.is_player) || (allow_player && u.is_player))
                && u.targetable
                && !u.out_of_reach
                && (!u.dead || for_dead)
            {
                in_range |= u.btl_id;
            }
        }
        picked = random_id(u32::from(in_range), rng) as u16;
        if picked == 0 {
            return TargetCheck::Invalid;
        }
    } else {
        picked = get_random_btl_id(units, tar_id & 15 != 0, for_dead, rng);
    }
    if picked != 0 {
        TargetCheck::Valid(picked)
    } else {
        TargetCheck::Invalid
    }
}
