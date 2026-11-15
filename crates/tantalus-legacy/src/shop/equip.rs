//! Equipment legality, stat changes and ability learning from equipment.

use std::cmp::Ordering;

use super::data::{ItemDb, item_type};
use super::inventory::Inventory;
use super::{EQUIP_SLOTS, NO_ITEM};

/// Equipped item ids by slot: weapon, helmet, armlet, armor, accessory
/// (`ff9feqp.cs:57-64`).
pub type Equipment = [u16; EQUIP_SLOTS];

/// Slot masks in slot order (`Global/ff9feqp.cs:57-64`, `ff9item.2.cs:284-299`).
const PART_MASK: [u8; EQUIP_SLOTS] = [
    item_type::WEAPON,
    item_type::HELMET,
    item_type::ARMLET,
    item_type::ARMOR,
    item_type::ACCESSORY,
];

/// `ff9play.FF9PLAY_STAT_MAX` (`Global/ff9play.cs:53`).
const STAT_MAX: [i32; 4] = [50, 99, 99, 50];
/// `ff9play.FF9PLAY_DEFPARAM_VAL_MAX` (`Global/ff9play.cs:46`).
const DEF_MAX: i32 = 999;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EquipError {
    BadSlot,
    UnknownItem,
    /// Character mask or slot type does not allow the item
    /// (`EquipUI.CanEquip`, `Global/EquipUI.cs:850-867`).
    NotEquippable,
    /// The inventory does not hold the item.
    NotInInventory,
    /// The weapon slot cannot be emptied (`EquipUI.cs:215-218` allows Off
    /// only for slots other than 0).
    WeaponSlot,
}

/// `ff9feqp.GetCharacterEquipMaskFromId` (`Global/ff9feqp.cs:10-13`).
pub fn equip_mask_from_char_id(id: u32) -> u64 {
    if id <= 11 {
        1u64 << (11 - id)
    } else {
        1u64 << id
    }
}

/// `ff9item.FF9Item_GetEquipPart` (`ff9item.2.cs:284-299`): first slot whose
/// type bit is set.
pub fn equip_part_of(db: &ItemDb, id: u16) -> Option<usize> {
    let d = db.get(id)?;
    PART_MASK.iter().position(|m| d.kind & m != 0)
}

/// `EquipUI.CanEquip` without the `use_condition` expression (`EquipUI.cs:850-867`).
/// Vanilla rows have an empty condition; non-empty conditions (NCalc) are not
/// modeled.
pub fn can_equip(db: &ItemDb, id: u16, char_mask: u64, slot: usize) -> bool {
    let Some(d) = db.get(id) else {
        return false;
    };
    slot < EQUIP_SLOTS && d.equip & char_mask != 0 && d.kind & PART_MASK[slot] != 0
}

/// `EquipUI.DisplayInventory` list (`EquipUI.cs:770-850`): held items the
/// character can equip in `slot`, ordered by `eq_lv` then item id.
pub fn equip_candidates(db: &ItemDb, inv: &Inventory, char_mask: u64, slot: usize) -> Vec<u16> {
    let mut v: Vec<u16> = inv
        .held()
        .map(|s| s.id)
        .filter(|id| can_equip(db, *id, char_mask, slot))
        .collect();
    v.sort_by(|a, b| {
        let la = db.get(*a).map_or(0.0, |d| d.eq_lv);
        let lb = db.get(*b).map_or(0.0, |d| d.eq_lv);
        if la > lb || (la == lb && a > b) {
            Ordering::Greater
        } else {
            Ordering::Less
        }
    });
    v
}

/// Derived stats from `ff9play.FF9Play_Update` (`Global/ff9play.cs:263-312`)
/// and `PLAYER.GetPlayerStat` (`Global/PLAYER.cs:109-124`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EquipStats {
    pub dex: i32,
    pub str_: i32,
    pub mgc: i32,
    pub wpr: i32,
    /// Weapon `Ref.Power`, 0 with no weapon.
    pub attack: i32,
    pub p_def: i32,
    pub p_ev: i32,
    pub m_def: i32,
    pub m_ev: i32,
}

/// Base stats (dex, str, mgc, wpr) plus equipment, clamped to the stat
/// maxima. Support-ability feature hooks (`FF9Play_SAFeature_Update`) and
/// max HP/MP are not modeled.
pub fn stats(db: &ItemDb, basis: [i32; 4], equip: &Equipment) -> EquipStats {
    let mut s = EquipStats {
        dex: basis[0],
        str_: basis[1],
        mgc: basis[2],
        wpr: basis[3],
        ..EquipStats::default()
    };
    for &id in equip.iter().filter(|&&i| i != NO_ITEM) {
        let Some(d) = db.get(id) else { continue };
        if let Some(a) = d.armor {
            s.p_def += a.p_def;
            s.p_ev += a.p_ev;
            s.m_def += a.m_def;
            s.m_ev += a.m_ev;
        }
        if let Some(b) = db.bonuses.get(d.bonus) {
            s.dex += b.dex;
            s.str_ += b.str_;
            s.mgc += b.mgc;
            s.wpr += b.wpr;
        }
    }
    s.dex = s.dex.min(STAT_MAX[0]);
    s.str_ = s.str_.min(STAT_MAX[1]);
    s.mgc = s.mgc.min(STAT_MAX[2]);
    s.wpr = s.wpr.min(STAT_MAX[3]);
    s.p_def = s.p_def.min(DEF_MAX);
    s.p_ev = s.p_ev.min(DEF_MAX);
    s.m_def = s.m_def.min(DEF_MAX);
    s.m_ev = s.m_ev.min(DEF_MAX);
    if equip[0] != NO_ITEM {
        s.attack = db.get(equip[0]).and_then(|d| d.weapon_power).unwrap_or(0);
    }
    s
}

/// Equip `new_id` into `slot` (`EquipUI.OnKeyConfirmInventoryGroupButton`,
/// `EquipUI.cs:175-200`): the old piece returns to the inventory, the new one
/// is removed. The source adds the old piece before removing the new one and
/// would duplicate it if the removal failed; here legality and stock are
/// checked first so the inventory is untouched on error.
pub fn equip_item(
    db: &ItemDb,
    inv: &mut Inventory,
    equip: &mut Equipment,
    char_mask: u64,
    slot: usize,
    new_id: u16,
) -> Result<(), EquipError> {
    if slot >= EQUIP_SLOTS {
        return Err(EquipError::BadSlot);
    }
    if db.get(new_id).is_none() {
        return Err(EquipError::UnknownItem);
    }
    if !can_equip(db, new_id, char_mask, slot) {
        return Err(EquipError::NotEquippable);
    }
    if inv.count(new_id) < 1 {
        return Err(EquipError::NotInInventory);
    }
    let old = equip[slot];
    if old != NO_ITEM {
        inv.add(old, 1);
    }
    inv.remove(new_id, 1);
    equip[slot] = new_id;
    Ok(())
}

/// The Off action (`EquipUI.cs:213-240`): not allowed for slot 0. Returns
/// the removed item id, if any.
pub fn unequip(
    inv: &mut Inventory,
    equip: &mut Equipment,
    slot: usize,
) -> Result<Option<u16>, EquipError> {
    if slot >= EQUIP_SLOTS {
        return Err(EquipError::BadSlot);
    }
    if slot == 0 {
        return Err(EquipError::WeaponSlot);
    }
    let old = equip[slot];
    if old == NO_ITEM {
        return Ok(None);
    }
    inv.add(old, 1);
    equip[slot] = NO_ITEM;
    Ok(Some(old))
}

/// Non-zero ability ids on an item (`FF9ITEM_DATA.ability`).
pub fn abilities_of(db: &ItemDb, id: u16) -> Vec<i32> {
    db.get(id)
        .map(|d| d.ability.iter().copied().filter(|a| *a != 0).collect())
        .unwrap_or_default()
}

/// Support abilities granted by the equipment, as support indexes
/// (`ff9feqp.FF9FEqp_UpdateSA` list build, `ff9feqp.cs:25-33`;
/// `ff9abil.IsAbilitySupport` and `GetSupportAbilityFromAbilityId`,
/// `Memoria/Data/ff9abil.cs:322-344`, pool 0: `id - 192`). Gem capacity
/// handling after the list is config dependent and not modeled.
pub fn equipped_support_abilities(db: &ItemDb, equip: &Equipment) -> Vec<i32> {
    equip
        .iter()
        .filter(|&&i| i != NO_ITEM)
        .flat_map(|&i| abilities_of(db, i))
        .filter(|a| *a >= 192)
        .map(|a| a - 192)
        .collect()
}

/// One ability an item teaches a character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbilityLearn {
    pub ability: i32,
    /// `Some((ap, max))` when the character's list contains the ability
    /// (`ff9abil.FF9Abil_GetIndex`, `ff9abil.cs:183-194`).
    pub progress: Option<(i32, i32)>,
    /// `FF9Abil_IsMaster` (`ff9abil.cs:196-201`).
    pub mastered: bool,
}

/// Learnable abilities of `item` for a character. `learnable` is the
/// character's `(ability id, max AP)` list in preset order and `ap` the
/// matching current AP array (`player.pa`).
pub fn learn_state(
    db: &ItemDb,
    item: u16,
    learnable: &[(i32, i32)],
    ap: &[i32],
) -> Vec<AbilityLearn> {
    abilities_of(db, item)
        .into_iter()
        .map(|a| {
            let progress = learnable
                .iter()
                .position(|(id, _)| *id == a)
                .map(|i| (ap.get(i).copied().unwrap_or(0), learnable[i].1));
            AbilityLearn {
                ability: a,
                mastered: progress.is_some_and(|(p, m)| p >= m),
                progress,
            }
        })
        .collect()
}

/// `ff9shop.FF9Shop_GetDefence` (`Memoria/Data/ff9shop.cs:9-32`): part 1
/// sums magical defence, 2 physical evade, 3 physical defence, 4 nothing,
/// over all five equipped pieces that carry armor data.
pub fn shop_defence(db: &ItemDb, part: usize, equip: &Equipment) -> i32 {
    let mut r = 0;
    for &id in equip.iter().filter(|&&i| i != NO_ITEM) {
        if let Some(a) = db.get(id).and_then(|d| d.armor) {
            r += match part {
                1 => a.m_def,
                2 => a.p_ev,
                3 => a.p_def,
                _ => 0,
            };
        }
    }
    r
}

/// Shop comparison for one character (`ShopUI.DisplayCharacterInfo`,
/// `ShopUI.cs:962-1044`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShopComparison {
    /// Slot the item goes in.
    pub part: usize,
    pub can_equip: bool,
    pub old_rating: i32,
    pub new_rating: i32,
    /// Accessories show "???" and no arrow.
    pub hidden: bool,
}

/// Weapon rating is the weapon power, armor rating is [`shop_defence`]
/// before and after swapping the item in. `None` when the item is not
/// equipment.
pub fn shop_comparison(
    db: &ItemDb,
    item: u16,
    char_mask: u64,
    equip: &Equipment,
) -> Option<ShopComparison> {
    let part = equip_part_of(db, item)?;
    let d = db.get(item)?;
    let ok = d.equip & char_mask != 0;
    let (old_rating, new_rating) = if part == 0 {
        let old = db.get(equip[0]).and_then(|w| w.weapon_power).unwrap_or(0);
        (old, d.weapon_power.unwrap_or(0))
    } else {
        let mut copy = *equip;
        copy[part] = item;
        (shop_defence(db, part, equip), shop_defence(db, part, &copy))
    };
    Some(ShopComparison {
        part,
        can_equip: ok,
        old_rating,
        new_rating,
        hidden: part == 4,
    })
}
