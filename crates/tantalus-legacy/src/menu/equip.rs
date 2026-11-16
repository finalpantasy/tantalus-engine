//! Equip menu rules: slot lists, the stat preview, equip and Off, the two
//! Optimize modes, equipment resistances (`Global/EquipUI.cs`,
//! `Global/ff9feqp.cs`, `Global/btl_eqp.cs`).

use std::collections::{BTreeMap, BTreeSet};

use super::ability::update_character;
use super::{MOONSTONE, MenuCharacter, MenuConfig, SupportTable};
use crate::battle::Elements;
use crate::shop::data::item_type;
use crate::shop::{
    EQUIP_SLOTS, EquipError, EquipStats, Equipment, Inventory, ItemDb, NO_ITEM, can_equip,
    equip_candidates, equip_item, stats, unequip,
};

/// `EquipUI.partMask` (the order of `ff9feqp.FF9FEqp_Equip`,
/// `ff9feqp.cs:67-73`): slot 0 weapon, 1 helmet, 2 armlet, 3 armor,
/// 4 accessory. This is the same table as `shop::equip_part_of` uses.
pub fn slot_item_type(slot: usize) -> Option<u8> {
    [
        item_type::WEAPON,
        item_type::HELMET,
        item_type::ARMLET,
        item_type::ARMOR,
        item_type::ACCESSORY,
    ]
    .get(slot)
    .copied()
}

/// The list the item picker shows for a slot (`EquipUI.DisplayInventory`,
/// `EquipUI.cs:770-848`): held items the character can equip there, sorted
/// by `eq_lv` then id; an empty list holds one `NoItem` placeholder row.
pub fn equip_list(db: &ItemDb, inv: &Inventory, ch: &MenuCharacter, slot: usize) -> Vec<u16> {
    let v = equip_candidates(db, inv, ch.equip_mask(), slot);
    if v.is_empty() { vec![NO_ITEM] } else { v }
}

/// What a confirm changed. Slots are indexed 0..5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EquipReport {
    pub changed: [bool; EQUIP_SLOTS],
    /// Net change of the Moonstone counter
    /// (`ff9item.DecreaseMoonStoneCount`, `ff9item.2.cs:408-411`,
    /// `gEvtReservedArray[0]`): -1 for each equipped Moonstone taken off.
    pub moonstone_delta: i32,
}

impl EquipReport {
    pub fn any_changed(&self) -> bool {
        self.changed.iter().any(|c| *c)
    }
}

/// Item list confirm (`EquipUI.OnKeyConfirmInventoryGroupButton`,
/// `EquipUI.cs:175-211`): the `NoItem` placeholder does nothing; otherwise
/// the old piece goes back to the inventory, the new piece comes out, and
/// the character is updated (`FF9FEqp_UpdatePlayer`: support abilities no
/// longer granted are switched off, the stones are recounted).
pub fn equip_from_list(
    db: &ItemDb,
    inv: &mut Inventory,
    ch: &mut MenuCharacter,
    t: &SupportTable,
    cfg: MenuConfig,
    slot: usize,
    item: u16,
) -> Result<EquipReport, EquipError> {
    let mut report = EquipReport::default();
    if item == NO_ITEM {
        return Ok(report);
    }
    let old = ch.equip.get(slot).copied().unwrap_or(NO_ITEM);
    let mask = ch.equip_mask();
    equip_item(db, inv, &mut ch.equip, mask, slot, item)?;
    if old == MOONSTONE {
        report.moonstone_delta -= 1;
    }
    report.changed[slot] = true;
    update_character(ch, db, t, cfg);
    Ok(report)
}

/// Off action (`EquipUI.OnKeyConfirmEquipmentGroupButton`,
/// `EquipUI.cs:213-240`): never the weapon slot, nothing for an empty slot.
pub fn unequip_slot(
    db: &ItemDb,
    inv: &mut Inventory,
    ch: &mut MenuCharacter,
    t: &SupportTable,
    cfg: MenuConfig,
    slot: usize,
) -> Result<EquipReport, EquipError> {
    let mut report = EquipReport::default();
    let removed = unequip(inv, &mut ch.equip, slot)?;
    if let Some(id) = removed {
        if id == MOONSTONE {
            report.moonstone_delta -= 1;
        }
        report.changed[slot] = true;
        update_character(ch, db, t, cfg);
    }
    Ok(report)
}

// ---------------------------------------------------------------------------
// Stat preview (`EquipUI.DisplayParameter`, `EquipUI.cs:540-617`)
// ---------------------------------------------------------------------------

/// What the highlighted entry would do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewChange {
    /// An item in the picker list.
    Item(u16),
    /// The Off cursor on a slot.
    Off,
}

/// `GetPlayerStat(0..9)` order: dex, str, mgc, wpr, weapon power, physical
/// defense, physical evade, magical defense, magical evade
/// (`Global/PLAYER.cs:109-124`).
pub fn stat_array(s: &EquipStats) -> [i32; 9] {
    [
        s.dex, s.str_, s.mgc, s.wpr, s.attack, s.p_def, s.p_ev, s.m_def, s.m_ev,
    ]
}

/// Arrow color of one stat row: green when the new value is higher, red
/// when lower, white and a still arrow when equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trend {
    Up,
    Down,
    Same,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatPreview {
    pub current: [i32; 9],
    pub preview: [i32; 9],
    pub trend: [Trend; 9],
}

/// The compare columns for the cursor, or `None` when the source shows the
/// plain stats: an item preview needs a real item; the Off preview needs a
/// non-weapon slot that holds something. Putting back the item already in
/// the slot previews no change. Max HP and MP previews and support-ability
/// feature effects are not modeled.
pub fn preview_stats(
    db: &ItemDb,
    ch: &MenuCharacter,
    slot: usize,
    change: PreviewChange,
) -> Option<StatPreview> {
    if slot >= EQUIP_SLOTS {
        return None;
    }
    let new_id = match change {
        PreviewChange::Item(NO_ITEM) => return None,
        PreviewChange::Item(id) => id,
        PreviewChange::Off => {
            if slot == 0 || ch.equip[slot] == NO_ITEM {
                return None;
            }
            NO_ITEM
        }
    };
    let cur = stat_array(&stats(db, ch.basis, &ch.equip));
    let prev = if new_id == ch.equip[slot] {
        cur
    } else {
        let mut e = ch.equip;
        e[slot] = new_id;
        stat_array(&stats(db, ch.basis, &e))
    };
    let mut trend = [Trend::Same; 9];
    for i in 0..9 {
        trend[i] = match cur[i].cmp(&prev[i]) {
            std::cmp::Ordering::Less => Trend::Up,
            std::cmp::Ordering::Greater => Trend::Down,
            std::cmp::Ordering::Equal => Trend::Same,
        };
    }
    Some(StatPreview {
        current: cur,
        preview: prev,
        trend,
    })
}

// ---------------------------------------------------------------------------
// Optimize
// ---------------------------------------------------------------------------

fn level_of(db: &ItemDb, id: u16) -> f32 {
    db.get(id).map_or(0.0, |d| d.eq_lv)
}

/// `CharacterEquipment.Change` (`Memoria/Data/Characters/
/// CharacterEquipment.cs:102-122`): same item, no-op; else the old piece
/// returns to the inventory (Moonstone lowers the counter), the new piece is
/// removed with a missing piece ignored, and the slot is set.
fn change_slot(
    inv: &mut Inventory,
    equip: &mut Equipment,
    slot: usize,
    item: u16,
    report: &mut EquipReport,
) {
    let old = equip[slot];
    if old == item {
        return;
    }
    if old != NO_ITEM {
        if old == MOONSTONE {
            report.moonstone_delta -= 1;
        }
        inv.add(old, 1);
    }
    if item != NO_ITEM {
        inv.remove(item, 1);
    }
    equip[slot] = item;
    report.changed[slot] = true;
}

/// Optimize, strongest mode (`EquipUI.EquipStrongest`, `EquipUI.cs:1220-1262`):
/// for each slot in order, take the held equippable item with the highest
/// `eq_lv` (the first in inventory order wins a tie) when it beats the
/// equipped piece's `eq_lv` (an empty slot counts as the lowest value); the
/// old piece goes back first. Then `FF9FEqp_UpdatePlayer` runs.
pub fn equip_strongest(
    db: &ItemDb,
    inv: &mut Inventory,
    ch: &mut MenuCharacter,
    t: &SupportTable,
    cfg: MenuConfig,
) -> EquipReport {
    let mut report = EquipReport::default();
    let mask = ch.equip_mask();
    let mut best_id = NO_ITEM;
    for slot in 0..EQUIP_SLOTS {
        let current_level = if ch.equip[slot] == NO_ITEM {
            f32::MIN
        } else {
            level_of(db, ch.equip[slot])
        };
        let mut best_level = f32::MIN;
        let held: Vec<u16> = inv.held().map(|s| s.id).collect();
        for id in held {
            if can_equip(db, id, mask, slot) && level_of(db, id) > best_level {
                best_level = level_of(db, id);
                best_id = id;
            }
        }
        if best_level > current_level {
            let old = ch.equip[slot];
            if old != NO_ITEM {
                if old == MOONSTONE {
                    report.moonstone_delta -= 1;
                }
                inv.add(old, 1);
            }
            if inv.remove(best_id, 1) != 0 {
                ch.equip[slot] = best_id;
                report.changed[slot] = true;
            }
        }
    }
    update_character(ch, db, t, cfg);
    report
}

/// `LearnableItemAbilities` (`EquipUI.cs:1062-1191`): for the pieces the
/// character wears and holds, the items worth equipping to learn, mapped to
/// the AP still missing. `i32::MAX` stands for "no progress yet" like the
/// source.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LearnableItems {
    map: BTreeMap<u16, i32>,
}

impl LearnableItems {
    pub fn new(db: &ItemDb, inv: &Inventory, ch: &MenuCharacter) -> Self {
        // Ability id -> AP left (`MaxValue` when the AP is still 0). The
        // source `Add` throws on a duplicate id; the first row wins here.
        let mut left: BTreeMap<i32, i32> = BTreeMap::new();
        for p in &ch.learnable {
            let cur = super::ability::ap_of(ch, p.id);
            let max = super::ability::max_ap_of(ch, p.id);
            if cur < max {
                left.entry(p.id)
                    .or_insert(if cur > 0 { max - cur } else { i32::MAX });
            }
        }
        let mask = ch.equip_mask();
        let mut seen: BTreeSet<u16> = BTreeSet::new();
        let mut ids: Vec<u16> = Vec::new();
        for &id in ch.equip.iter().filter(|&&i| i != NO_ITEM) {
            if seen.insert(id) {
                ids.push(id);
            }
        }
        for s in inv.held() {
            if seen.insert(s.id) {
                ids.push(s.id);
            }
        }
        let mut key_to_item: BTreeMap<u64, u16> = BTreeMap::new();
        let mut item_abilities: BTreeMap<u16, Vec<i32>> = BTreeMap::new();
        for id in ids {
            let Some(d) = db.get(id) else { continue };
            if d.equip & mask == 0 {
                continue;
            }
            let mut abil: Vec<i32> = d
                .ability
                .iter()
                .copied()
                .filter(|a| left.contains_key(a))
                .collect();
            if abil.is_empty() {
                continue;
            }
            abil.sort_unstable();
            let mut key = d.equip;
            for (i, a) in abil.iter().enumerate() {
                key |= (*a as u64).wrapping_shl(((i + 2) * 8) as u32);
            }
            match key_to_item.get(&key) {
                Some(&old) if level_of(db, old) < d.eq_lv => {
                    key_to_item.insert(key, id);
                }
                Some(_) => {}
                None => {
                    key_to_item.insert(key, id);
                }
            }
            item_abilities.insert(id, abil);
        }
        let mut map = BTreeMap::new();
        for &id in key_to_item.values() {
            let mut total: i32 = 0;
            for a in &item_abilities[&id] {
                let l = left[a];
                if l != i32::MAX {
                    total += l;
                }
            }
            if total == 0 {
                total = i32::MAX;
            }
            map.insert(id, total);
        }
        Self { map }
    }

    /// `IsLearnable`: the AP left, or `None`.
    pub fn left_ap(&self, id: u16) -> Option<i32> {
        self.map.get(&id).copied()
    }
}

/// Optimize, ability mode (`EquipUI.EquipForAbilityLearning`,
/// `EquipUI.cs:950-1060`), the button label "Ability" (`EquipUI.cs:45-52`).
/// Per slot it picks the lowest-quality learnable item, remembers the
/// highest-quality equippable one, and then equips: learnable items first;
/// else, when nothing is learnable anywhere but a better piece exists,
/// the better pieces; else it empties non-weapon slots that hold something
/// with nothing to learn. The source compares `leftAp <= learnableLeftAp`
/// against the value taken from the first candidate and never updates it
/// afterwards; that is kept.
pub fn equip_for_ability_learning(
    db: &ItemDb,
    inv: &mut Inventory,
    ch: &mut MenuCharacter,
    t: &SupportTable,
    cfg: MenuConfig,
) -> EquipReport {
    let mut report = EquipReport::default();
    let mask = ch.equip_mask();
    let learnable = LearnableItems::new(db, inv, ch);
    let mut to_remove = [NO_ITEM; EQUIP_SLOTS];
    let mut to_learn = [NO_ITEM; EQUIP_SLOTS];
    let mut to_equip = [NO_ITEM; EQUIP_SLOTS];
    let mut can_learn = false;
    let mut can_equip_better = false;
    for slot in 0..EQUIP_SLOTS {
        let mut better_id = NO_ITEM;
        let mut better_level = f32::MIN;
        let mut learnable_id = NO_ITEM;
        let mut learnable_level = f32::MAX;
        let mut learnable_left = i32::MAX;
        let worn = ch.equip[slot];
        if worn != NO_ITEM {
            better_level = level_of(db, worn);
            better_id = worn;
            if let Some(l) = learnable.left_ap(worn) {
                to_learn[slot] = worn;
                learnable_id = worn;
                learnable_level = level_of(db, worn);
                learnable_left = l;
            } else {
                to_remove[slot] = worn;
            }
        }
        let mut equippable = false;
        let held: Vec<u16> = inv.held().map(|s| s.id).collect();
        for id in held {
            if !can_equip(db, id, mask, slot) {
                continue;
            }
            let lv = level_of(db, id);
            if lv > better_level {
                better_id = id;
                better_level = lv;
                equippable = true;
            }
            if let Some(l) = learnable.left_ap(id)
                && lv < learnable_level
                && l <= learnable_left
            {
                learnable_id = id;
                learnable_level = lv;
                can_learn = true;
            }
        }
        to_learn[slot] = learnable_id;
        to_equip[slot] = better_id;
        if equippable && learnable_id == NO_ITEM {
            can_equip_better = true;
        }
    }
    for slot in 0..EQUIP_SLOTS {
        let id = to_learn[slot];
        if id != NO_ITEM {
            change_slot(inv, &mut ch.equip, slot, id, &mut report);
            continue;
        }
        if can_learn || !can_equip_better {
            if slot != 0 && to_remove[slot] != NO_ITEM {
                change_slot(inv, &mut ch.equip, slot, NO_ITEM, &mut report);
            }
            continue;
        }
        let id = to_equip[slot];
        if id != NO_ITEM {
            change_slot(inv, &mut ch.equip, slot, id, &mut report);
        }
    }
    update_character(ch, db, t, cfg);
    report
}

// ---------------------------------------------------------------------------
// Resistances and added statuses (`btl_eqp.cs`, `BattleUnit.cs`)
// ---------------------------------------------------------------------------

/// A row of `Stats.csv` (`Memoria/Data/Items/ItemStats.cs:9-22`) reduced to
/// the element columns, one byte per element set (`EffectElement` bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EquipElementRow {
    /// `def_attr.invalid` ("GuardElement"): nullified.
    pub invalid: u8,
    pub absorb: u8,
    pub half: u8,
    pub weak: u8,
    /// `p_up_attr` ("BonusElement"): the attack element boost.
    pub bonus: u8,
}

/// The five element sets of the character before the battle starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EquipResistances {
    pub guard: Elements,
    pub absorb: Elements,
    pub half: Elements,
    pub weak: Elements,
    pub bonus: Elements,
}

/// `btl_eqp.InitEquipPrivilegeAttrib` (`btl_eqp.cs:13-28`): each set is the
/// union of the five pieces' rows (`IncrementDefAttr` and
/// `ElementSet.AddMultiple`, `btl_init.cs:547-553`,
/// `EffectElement.cs:141-145`). A piece whose bonus index has no row is
/// skipped (the source throws).
pub fn equip_resistances(
    db: &ItemDb,
    rows: &[EquipElementRow],
    equip: &Equipment,
) -> EquipResistances {
    let mut r = EquipResistances::default();
    for &id in equip.iter().filter(|&&i| i != NO_ITEM) {
        let Some(row) = db.get(id).and_then(|d| rows.get(d.bonus)) else {
            continue;
        };
        r.guard.0 |= row.invalid;
        r.absorb.0 |= row.absorb;
        r.half.0 |= row.half;
        r.weak.0 |= row.weak;
        r.bonus.0 |= row.bonus;
    }
    r
}

/// The status set a weapon adds to a hit (`BattleUnit.WeaponStatus`,
/// `Memoria/Battle/Calculator/BattleUnit.cs:229`): the entry of
/// `FF9Battle.add_status` named by the weapon's status index; 0 for no
/// weapon or an index past the table.
pub fn weapon_added_status(status_sets: &[u32], weapon_status_index: Option<usize>) -> u32 {
    weapon_status_index
        .and_then(|i| status_sets.get(i).copied())
        .unwrap_or(0)
}
