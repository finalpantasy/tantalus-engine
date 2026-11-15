//! Item use from the field menu (pure math only).
//!
//! Source: `Global/ItemUI.cs:317-450`, `Memoria/Field/SFieldCalculator.cs`,
//! `Memoria/Field/FieldCalculator.cs`. Ported scripts: 62 and 73 (cure),
//! 69 (heal part), 70 (ether), 71 (elixir), 72 (revive), 74 (gem), 76 (tent).

use super::data::{ItemDb, item_type};
use super::inventory::Inventory;

const PETRIFY: u64 = 1;
const ZOMBIE: u64 = 1 << 6;
/// `BattleStatusConst.CannotHealInMenu` (`Memoria/Data/Battle/BattleStatus.cs:153`).
const CANNOT_HEAL_IN_MENU: u64 = PETRIFY | ZOMBIE;
/// `BattleStatusConst.OutOfBattle` (`BattleStatus.cs:118`): Petrify, Venom,
/// Virus, Silence, Blind, Trouble, Zombie (bits 0 to 6).
const OUT_OF_BATTLE: u64 = 0x7F;

/// `GysahlGreens` item id, handled by a separate path (`ItemUI.cs:322-336`).
const GYSAHL_GREENS: u16 = 251;

/// Party member state the field calculator reads and writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FieldTarget {
    pub hp: u32,
    pub max_hp: u32,
    pub mp: u32,
    pub max_mp: u32,
    /// `PLAYER.status` bits (`BattleStatus`).
    pub status: u64,
    /// `PLAYER.permanent_status` bits.
    pub permanent_status: u64,
}

/// Item effect row (`ITEM_DATA`: script id, power, status mask).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldEffect {
    pub script_id: u8,
    pub power: i32,
    /// Raw `ITEM_DATA.status`; masked with `OutOfBattle` before use
    /// (`FieldCalculator.ActionData`, `FieldCalculator.cs:256`).
    pub status: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldItemError {
    UnknownItem,
    /// Not a `Usable` item or none held (`ItemUI.cs:317-319`, `:658`).
    NotUsable,
    /// Branch that is source-proven but not ported (script id, or 251 for
    /// Gysahl Greens).
    ScriptNotPorted(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemUseOutcome {
    /// The calculator reported success; one item was consumed
    /// (`ItemUI.cs:431`).
    Used { hp_gained: u32, mp_gained: u32 },
    /// The calculator reported a miss; nothing changed, nothing consumed.
    NoEffect,
}

/// `FieldCalculator.ApplyRecover` / `ApplyMpRecover`
/// (`FieldCalculator.cs:11-24`): clamp to `0..=max`.
fn clamp_add(cur: u32, add: i32, max: u32) -> u32 {
    (i64::from(cur) + i64::from(add)).clamp(0, i64::from(max)) as u32
}

/// `CanBeHealed(player, healHp, healMp)` (`FieldCalculator.cs:89-97`).
fn can_be_healed(t: &FieldTarget, heal_hp: bool, heal_mp: bool) -> bool {
    let hp = heal_hp && t.hp != t.max_hp;
    let mp = heal_mp && t.mp != t.max_mp;
    (hp || mp) && t.hp != 0 && t.status & CANNOT_HEAL_IN_MENU == 0
}

/// Use `item` once on `target` (`ItemUI.cs:420-431` target confirm path).
/// `random8` is drawn only for the revive script (`Comn.random8`, 0..=255).
pub fn use_item(
    db: &ItemDb,
    inv: &mut Inventory,
    item: u16,
    effect: &FieldEffect,
    target: &mut FieldTarget,
    random8: impl FnOnce() -> u32,
) -> Result<ItemUseOutcome, FieldItemError> {
    let d = db.get(item).ok_or(FieldItemError::UnknownItem)?;
    if d.kind & item_type::USABLE == 0 || inv.count(item) < 1 {
        return Err(FieldItemError::NotUsable);
    }
    if item == GYSAHL_GREENS {
        return Err(FieldItemError::ScriptNotPorted(GYSAHL_GREENS));
    }
    let status = effect.status & OUT_OF_BATTLE;
    let power = effect.power;
    let (mut rec_hp, mut rec_mp) = (0i32, 0i32);
    let mut miss = false;
    match effect.script_id {
        // Item Soft / Item Cure Status (`SFieldCalculator.cs:132-135`).
        62 | 73 => {
            // `RemoveStatuses`: per status bit, permanent -> 0, absent -> 1,
            // else cleared -> 2; success is the maximum (`FieldCalculator.cs:49-66`).
            let mut success = 0;
            for bit in 0..64 {
                let b = 1u64 << bit;
                if status & b == 0 {
                    continue;
                }
                let r = if target.permanent_status & b != 0 {
                    0
                } else if target.status & b == 0 {
                    1
                } else {
                    target.status &= !b;
                    2
                };
                success = success.max(r);
            }
            miss = success != 2;
        }
        // Item Potion (`SFieldCalculator.cs:136-150`); the status half calls
        // `AlterStatuses`, which needs the status immunity table.
        69 => {
            if status != 0 {
                return Err(FieldItemError::ScriptNotPorted(69));
            }
            if power > 0 {
                if can_be_healed(target, true, false) {
                    // SetupItemHeal: Attack 10; HealHp: power * attack.
                    rec_hp = power * 10;
                } else {
                    miss = true;
                }
            }
        }
        // Item Ether (`:151-157`).
        70 => {
            if can_be_healed(target, false, true) {
                rec_mp = power * 10;
            } else {
                miss = true;
            }
        }
        // Item Elixir (`:158-161`), HealFull.
        71 => {
            if can_be_healed(target, true, true) {
                rec_hp = target.max_hp as i32;
                rec_mp = target.max_mp as i32;
            } else {
                miss = true;
            }
        }
        // Item Phoenix (`:162-165`), ReviveLow.
        72 => {
            if target.hp == 0 && target.status & CANNOT_HEAL_IN_MENU == 0 {
                rec_hp = 1 + (random8() % 10) as i32;
            } else {
                miss = true;
            }
        }
        // Item Gem (`:166-172`): attack is the held count of the gem.
        74 => {
            if can_be_healed(target, true, false) {
                rec_hp = power * inv.count(item);
            } else {
                miss = true;
            }
        }
        // Item Tent (`:173-177`), RecoverHalfHpMp.
        76 => {
            if can_be_healed(target, true, true) {
                rec_hp = (target.max_hp >> 1) as i32;
                rec_mp = (target.max_mp >> 1) as i32;
            } else {
                miss = true;
            }
        }
        // Spell scripts the source also handles here, not item effects.
        s @ (10 | 12 | 13 | 15 | 16 | 30 | 37 | 50 | 89 | 103) => {
            return Err(FieldItemError::ScriptNotPorted(u16::from(s)));
        }
        // `DefaultFieldScript` default branch: Miss.
        _ => miss = true,
    }
    if miss {
        return Ok(ItemUseOutcome::NoEffect);
    }
    // `FieldCalcResult` (`SFieldCalculator.cs:201-216`).
    let (old_hp, old_mp) = (target.hp, target.mp);
    if rec_hp > 0 {
        target.hp = clamp_add(target.hp, rec_hp, target.max_hp);
    }
    if rec_mp > 0 {
        target.mp = clamp_add(target.mp, rec_mp, target.max_mp);
    }
    inv.remove(item, 1);
    Ok(ItemUseOutcome::Used {
        hp_gained: target.hp - old_hp,
        mp_gained: target.mp - old_mp,
    })
}
