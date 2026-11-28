//! Hand-derived cases for the Equip and Ability menu rules
//! (`tantalus_legacy::menu`).
//!
//! Source: Memoria pin `c716e1fa`. Item rows are copied by hand from the
//! stock CSVs under `Memoria.Patcher/StreamingAssets/Data` (Items, Armors,
//! Weapons, Stats; Characters/BaseStats and Characters/Abilities/Zidane and
//! AbilityGems). Ids marked "synthetic" are made up to isolate one rule.
//! Every expected number is worked out in the test comment.

use std::collections::BTreeMap;

use tantalus_legacy::battle::Elements;
use tantalus_legacy::battle::rewards::PaEntry;
use tantalus_legacy::menu::ability::{self, *};
use tantalus_legacy::menu::equip::*;
use tantalus_legacy::menu::*;
use tantalus_legacy::shop::data::item_type;
use tantalus_legacy::shop::{
    ArmorData, EquipError, Inventory, ItemData, ItemDb, NO_ITEM, Slot, StatBonus,
    equip_mask_from_char_id,
};

const DAGGER: u16 = 1;
const MAGE_MASHER: u16 = 2;
const WRIST: u16 = 88;
const LEATHER_WRIST: u16 = 89;
const GLASS_ARMLET: u16 = 90;
const BONE_WRIST: u16 = 91;
const MYTHRIL_ARMLET: u16 = 92;
const LEATHER_HAT: u16 = 112;
const MOONSTONE: u16 = 229;
const ZIDANE: u32 = 0;
const STEINER: u32 = 3;

fn sa(n: i32) -> i32 {
    192 + n
}

/// Ids 0, 1, 2, 5, 6, 7, 8, 9, 10: the "1;1;1;0;0;1;1;1;1;1;1;0" mask of the
/// armlets in `Items.csv` (Steiner, Freya and Beatrix excluded).
fn armlet_mask() -> u64 {
    [0u32, 1, 2, 5, 6, 7, 8, 9, 10]
        .iter()
        .fold(0, |m, c| m | equip_mask_from_char_id(*c))
}

fn row(kind: u8, equip: u64, eq_lv: f32, bonus: usize, ability: Vec<i32>) -> ItemData {
    ItemData {
        price: 0,
        selling_price: 0,
        equip,
        kind,
        sort: 0.0,
        eq_lv,
        bonus,
        ability,
        weapon_power: None,
        armor: None,
    }
}

fn armor(p_def: i32, p_ev: i32, m_def: i32, m_ev: i32) -> Option<ArmorData> {
    Some(ArmorData {
        p_def,
        p_ev,
        m_def,
        m_ev,
    })
}

/// Stats.csv rows 0..=5 (`Stats.csv`: Speed;Strength;Magic;Spirit), the rest
/// zero: Leather Wrist 2 spirit +1, Bone Wrist 4 strength +1, Mythril 5
/// spirit +1.
fn db() -> ItemDb {
    let mut items = BTreeMap::new();
    let zidane = equip_mask_from_char_id(0);
    // 001 Dagger: quality 1, Zidane only, power 12, AA:101.
    let mut d = row(item_type::WEAPON, zidane, 1.0, 0, vec![101]);
    d.weapon_power = Some(12);
    items.insert(DAGGER, d);
    // 002 Mage Masher: quality 2, power 14, AA:102, AA:101.
    let mut d = row(item_type::WEAPON, zidane, 2.0, 0, vec![102, 101]);
    d.weapon_power = Some(14);
    items.insert(MAGE_MASHER, d);
    // 088 Wrist: quality 20, bonus 1, armor 0 (pdef 0, pev 5, mdef 0, mev 3), SA:46.
    let mut d = row(item_type::ARMLET, armlet_mask(), 20.0, 1, vec![sa(46)]);
    d.armor = armor(0, 5, 0, 3);
    items.insert(WRIST, d);
    // 089 Leather Wrist: quality 22, bonus 2, armor 1 (0,7,0,5), SA:19 AA:29.
    let mut d = row(item_type::ARMLET, armlet_mask(), 22.0, 2, vec![sa(19), 29]);
    d.armor = armor(0, 7, 0, 5);
    items.insert(LEATHER_WRIST, d);
    // 090 Glass Armlet: quality 24, bonus 3 (half Water), armor 2 (0,10,0,7), SA:23 SA:49.
    let mut d = row(
        item_type::ARMLET,
        armlet_mask(),
        24.0,
        3,
        vec![sa(23), sa(49)],
    );
    d.armor = armor(0, 10, 0, 7);
    items.insert(GLASS_ARMLET, d);
    // 091 Bone Wrist: quality 26, bonus 4 (strength +1, Earth boost), armor 3 (0,13,0,9), SA:25.
    let mut d = row(item_type::ARMLET, armlet_mask(), 26.0, 4, vec![sa(25)]);
    d.armor = armor(0, 13, 0, 9);
    items.insert(BONE_WRIST, d);
    // 092 Mythril Armlet: quality 28, bonus 5 (spirit +1), armor 4 (0,17,0,11), SA:14.
    let mut d = row(item_type::ARMLET, armlet_mask(), 28.0, 5, vec![sa(14)]);
    d.armor = armor(0, 17, 0, 11);
    items.insert(MYTHRIL_ARMLET, d);
    // 112 Leather Hat: helmet, quality 51, bonus 25, AA:25.
    let mut d = row(item_type::HELMET, armlet_mask(), 51.0, 0, vec![25]);
    d.armor = armor(0, 0, 0, 0);
    items.insert(LEATHER_HAT, d);
    // 229 Moonstone: accessory (and item, gem), all characters, quality 0, AA:11 SA:19.
    items.insert(
        MOONSTONE,
        row(
            item_type::ACCESSORY | item_type::ITEM | item_type::GEM,
            0xFFF,
            0.0,
            0,
            vec![11, sa(19)],
        ),
    );
    let mut bonuses = vec![StatBonus::default(); 8];
    bonuses[2].wpr = 1;
    bonuses[4].str_ = 1;
    bonuses[5].wpr = 1;
    ItemDb { items, bonuses }
}

fn inv(list: &[(u16, u8)]) -> Inventory {
    Inventory {
        slots: list.iter().map(|&(id, count)| Slot { id, count }).collect(),
        ..Inventory::default()
    }
}

/// Support-ability gem costs from `AbilityGems.csv` for the rows used here;
/// every other row costs 0 in this table.
fn gems() -> SupportTable {
    let mut rows = vec![SupportRow::default(); 64];
    for (i, g) in [
        (0, 15),
        (1, 6),
        (3, 10),
        (14, 2),
        (19, 4),
        (23, 5),
        (25, 3),
        (46, 3),
        (49, 1),
    ] {
        rows[i].gems = g;
    }
    SupportTable { rows }
}

/// Zidane's preset rows from `Abilities/Zidane.csv` (subset):
/// AA:101 40, AA:102 40, SA:0 95, SA:3 25, SA:14 35, SA:19 30, SA:23 40,
/// SA:25 35, SA:46 45.
fn zidane() -> MenuCharacter {
    let rows = [
        (101, 40),
        (102, 40),
        (sa(0), 95),
        (sa(3), 25),
        (sa(14), 35),
        (sa(19), 30),
        (sa(23), 40),
        (sa(25), 35),
        (sa(46), 45),
    ];
    let mut ch = MenuCharacter::new(ZIDANE);
    // Zidane's `BaseStats.csv` row: Speed 23, Strength 21, Magic 18, Spirit 23, Gems 18.
    ch.basis = [23, 21, 18, 23];
    ch.learnable = rows
        .iter()
        .map(|&(id, max_ap)| PaEntry { id, max_ap })
        .collect();
    ch.pa = vec![0; rows.len()];
    ch.max_capa = 18;
    ch.cur_capa = 18;
    ch.basis_capa = 18;
    ch.cur_hp = 100;
    ch.cur_mp = 50;
    ch
}

fn set_ap(ch: &mut MenuCharacter, id: i32, ap: i32) {
    let i = ch.ability_index(id).unwrap();
    ch.pa[i] = ap;
}

fn ctx() -> AbilityContext {
    AbilityContext::default()
}

// ---------------------------------------------------------------------------
// Equip
// ---------------------------------------------------------------------------

#[test]
fn slot_types_follow_the_part_mask_order() {
    // `ff9feqp.cs:67-73`: weapon, helmet, armlet, armor, accessory.
    assert_eq!(slot_item_type(0), Some(item_type::WEAPON));
    assert_eq!(slot_item_type(1), Some(item_type::HELMET));
    assert_eq!(slot_item_type(2), Some(item_type::ARMLET));
    assert_eq!(slot_item_type(3), Some(item_type::ARMOR));
    assert_eq!(slot_item_type(4), Some(item_type::ACCESSORY));
    assert_eq!(slot_item_type(5), None);
}

#[test]
fn picker_lists_sort_by_quality_and_empty_lists_get_a_placeholder() {
    let db = db();
    let z = zidane();
    let held = inv(&[(BONE_WRIST, 1), (WRIST, 1), (LEATHER_WRIST, 1), (DAGGER, 1)]);
    // Armlet list: quality 20, 22, 26 -> Wrist, Leather Wrist, Bone Wrist
    // (`EquipUI.cs:808-818`).
    assert_eq!(
        equip_list(&db, &held, &z, 2),
        vec![WRIST, LEATHER_WRIST, BONE_WRIST]
    );
    // No helmet held: one NoItem row (`EquipUI.cs:820-821`).
    assert_eq!(equip_list(&db, &held, &z, 1), vec![NO_ITEM]);
    // Steiner (mask bit 8) cannot wear the armlets: placeholder.
    let steiner = MenuCharacter::new(STEINER);
    assert_eq!(equip_list(&db, &held, &steiner, 2), vec![NO_ITEM]);
}

#[test]
fn preview_compares_nine_stats_and_marks_trends() {
    let db = db();
    let mut z = zidane();
    z.equip[2] = WRIST;
    // Current: base 23/21/18/23 plus Wrist (bonus 1 is zero) and its armor
    // (0,5,0,3): [23,21,18,23, 0 attack, 0 pdef, 5 pev, 0 mdef, 3 mev].
    // Leather Wrist: bonus 2 gives spirit +1, armor (0,7,0,5):
    // [23,21,18,24, 0, 0, 7, 0, 5] (`ff9play.cs:263-310`, `EquipUI.cs:552-583`).
    let p = preview_stats(&db, &z, 2, PreviewChange::Item(LEATHER_WRIST)).unwrap();
    assert_eq!(p.current, [23, 21, 18, 23, 0, 0, 5, 0, 3]);
    assert_eq!(p.preview, [23, 21, 18, 24, 0, 0, 7, 0, 5]);
    assert_eq!(p.trend[3], Trend::Up);
    assert_eq!(p.trend[6], Trend::Up);
    assert_eq!(p.trend[0], Trend::Same);
    // Off on the wrist: armor and evade drop to 0 -> Down on rows 6 and 8.
    let off = preview_stats(&db, &z, 2, PreviewChange::Off).unwrap();
    assert_eq!(off.preview, [23, 21, 18, 23, 0, 0, 0, 0, 0]);
    assert_eq!(off.trend[6], Trend::Down);
    assert_eq!(off.trend[8], Trend::Down);
    // Putting back the worn item previews no change (`EquipUI.cs:589-599`).
    let same = preview_stats(&db, &z, 2, PreviewChange::Item(WRIST)).unwrap();
    assert_eq!(same.current, same.preview);
    assert!(same.trend.iter().all(|t| *t == Trend::Same));
}

#[test]
fn preview_hides_for_placeholder_empty_slot_and_weapon_off() {
    let db = db();
    let mut z = zidane();
    z.equip[0] = DAGGER;
    // NoItem row, Off on an empty slot, and Off on the weapon give no compare
    // columns (`EquipUI.cs:542-544`).
    assert!(preview_stats(&db, &z, 2, PreviewChange::Item(NO_ITEM)).is_none());
    assert!(preview_stats(&db, &z, 2, PreviewChange::Off).is_none());
    assert!(preview_stats(&db, &z, 0, PreviewChange::Off).is_none());
    // A better weapon: attack 12 -> 14.
    let w = preview_stats(&db, &z, 0, PreviewChange::Item(MAGE_MASHER)).unwrap();
    assert_eq!((w.current[4], w.preview[4]), (12, 14));
    assert_eq!(w.trend[4], Trend::Up);
}

#[test]
fn equip_moves_items_and_recounts_stones() {
    let db = db();
    let t = gems();
    let cfg = MenuConfig::default();
    let mut z = zidane();
    let mut held = inv(&[(WRIST, 1), (BONE_WRIST, 1)]);
    // Equip Wrist (SA:46 costs 3 stones). Switch Flee-Gil on while equipped.
    let r = equip_from_list(&db, &mut held, &mut z, &t, cfg, 2, WRIST).unwrap();
    assert!(r.changed[2]);
    assert_eq!(z.equip[2], WRIST);
    assert_eq!(held.count(WRIST), 0);
    let eqp = equipped_abilities(&db, &z.equip);
    assert_eq!(
        toggle_support(&mut z, &t, &ctx(), &eqp, sa(46)),
        Ok(SupportToggle::Enabled)
    );
    assert_eq!(z.cur_capa, 15);
    // Swap to Bone Wrist: Wrist goes back, SA:46 is not learned (0 < 45) and
    // no longer granted, so it is switched off and the stones recount to 18
    // (`ff9feqp.cs:55-70`, `ff9abil.cs:259-285`).
    equip_from_list(&db, &mut held, &mut z, &t, cfg, 2, BONE_WRIST).unwrap();
    assert_eq!(held.count(WRIST), 1);
    assert_eq!(held.count(BONE_WRIST), 0);
    assert!(!z.is_enabled(46));
    assert_eq!(z.cur_capa, 18);
}

#[test]
fn a_learned_ability_stays_on_after_the_item_comes_off() {
    let db = db();
    let t = gems();
    let cfg = MenuConfig::default();
    let mut z = zidane();
    set_ap(&mut z, sa(46), 45); // mastered
    let mut held = inv(&[(WRIST, 1)]);
    equip_from_list(&db, &mut held, &mut z, &t, cfg, 2, WRIST).unwrap();
    let eqp = equipped_abilities(&db, &z.equip);
    toggle_support(&mut z, &t, &ctx(), &eqp, sa(46)).unwrap();
    unequip_slot(&db, &mut held, &mut z, &t, cfg, 2).unwrap();
    // `pa[k] < Ap` is false for a mastered ability: it stays on, 15 stones left.
    assert!(z.is_enabled(46));
    assert_eq!(z.cur_capa, 15);
    assert_eq!(held.count(WRIST), 1);
}

#[test]
fn off_rules_and_moonstone_counter() {
    let db = db();
    let t = gems();
    let cfg = MenuConfig::default();
    let mut z = zidane();
    z.equip[0] = DAGGER;
    z.equip[4] = MOONSTONE;
    let mut held = inv(&[]);
    // The weapon slot cannot be emptied (`EquipUI.cs:215-218`).
    assert_eq!(
        unequip_slot(&db, &mut held, &mut z, &t, cfg, 0),
        Err(EquipError::WeaponSlot)
    );
    // An empty slot is a no-op.
    let r = unequip_slot(&db, &mut held, &mut z, &t, cfg, 2).unwrap();
    assert!(!r.any_changed());
    // Off on the Moonstone lowers the counter by 1 (`EquipUI.cs:225-228`).
    let r = unequip_slot(&db, &mut held, &mut z, &t, cfg, 4).unwrap();
    assert!(r.changed[4]);
    assert_eq!(r.moonstone_delta, -1);
    assert_eq!(held.count(MOONSTONE), 1);
    assert_eq!(z.equip[4], NO_ITEM);
    // Equipping over a Moonstone also lowers it (`EquipUI.cs:184-188`).
    let mut held = inv(&[(MOONSTONE, 1)]);
    let r = equip_from_list(&db, &mut held, &mut z, &t, cfg, 4, MOONSTONE).unwrap();
    assert_eq!(r.moonstone_delta, 0);
    let mut held = inv(&[(MOONSTONE, 1)]);
    let mut z2 = zidane();
    z2.equip[4] = MOONSTONE;
    let r = equip_from_list(&db, &mut held, &mut z2, &t, cfg, 4, MOONSTONE).unwrap();
    assert_eq!(r.moonstone_delta, -1);
}

#[test]
fn equip_errors_and_placeholder_leave_everything_alone() {
    let db = db();
    let t = gems();
    let cfg = MenuConfig::default();
    let mut z = zidane();
    let mut held = inv(&[(MAGE_MASHER, 1), (LEATHER_HAT, 1)]);
    let before = (z.clone(), held.clone());
    // Hat into the armlet slot: wrong kind.
    assert_eq!(
        equip_from_list(&db, &mut held, &mut z, &t, cfg, 2, LEATHER_HAT),
        Err(EquipError::NotEquippable)
    );
    // Not held.
    assert_eq!(
        equip_from_list(&db, &mut held, &mut z, &t, cfg, 2, WRIST),
        Err(EquipError::NotInInventory)
    );
    // The placeholder row confirms to nothing.
    let r = equip_from_list(&db, &mut held, &mut z, &t, cfg, 1, NO_ITEM).unwrap();
    assert!(!r.any_changed());
    assert_eq!((z, held), before);
}

#[test]
fn strongest_takes_the_highest_quality_per_slot() {
    let db = db();
    let t = gems();
    let cfg = MenuConfig::default();
    let mut z = zidane();
    z.equip[0] = DAGGER; // quality 1
    let mut held = inv(&[
        (MAGE_MASHER, 1),
        (WRIST, 1),
        (LEATHER_WRIST, 1),
        (BONE_WRIST, 1),
        (LEATHER_HAT, 1),
    ]);
    let r = equip_strongest(&db, &mut held, &mut z, &t, cfg);
    // Weapon 2 > 1; armlet Bone Wrist (26) over 20 and 22; hat (51).
    assert_eq!(
        z.equip,
        [MAGE_MASHER, LEATHER_HAT, BONE_WRIST, NO_ITEM, NO_ITEM]
    );
    assert_eq!(r.changed, [true, true, true, false, false]);
    // The old dagger returned; the others stay held (`EquipUI.cs:1243-1259`).
    assert_eq!(held.count(DAGGER), 1);
    assert_eq!(held.count(WRIST), 1);
    assert_eq!(held.count(LEATHER_WRIST), 1);
    assert_eq!(held.count(BONE_WRIST), 0);
    // A second press changes nothing: nothing is strictly better.
    let r = equip_strongest(&db, &mut held, &mut z, &t, cfg);
    assert!(!r.any_changed());
}

#[test]
fn strongest_keeps_an_equal_piece_and_takes_the_first_of_a_tie() {
    let mut d = db();
    // Synthetic armlets 200 and 201 with the same quality 30.
    for id in [200u16, 201] {
        d.items.insert(
            id,
            row(item_type::ARMLET, armlet_mask(), 30.0, 0, Vec::new()),
        );
    }
    let t = gems();
    let cfg = MenuConfig::default();
    let mut z = zidane();
    let mut held = inv(&[(201, 1), (200, 1)]);
    equip_strongest(&d, &mut held, &mut z, &t, cfg);
    // Strict `>`: the first item in inventory order, 201, wins.
    assert_eq!(z.equip[2], 201);
    // Now 200 has the same quality as the worn piece: not strictly better.
    let r = equip_strongest(&d, &mut held, &mut z, &t, cfg);
    assert!(!r.any_changed());
    assert_eq!(z.equip[2], 201);
}

#[test]
fn ability_optimize_prefers_the_lowest_quality_learnable_piece() {
    let db = db();
    let t = gems();
    let cfg = MenuConfig::default();
    let mut z = zidane();
    // AA:101 on the worn Dagger is learnable (0 of 40); a Leather Hat in the
    // helmet slot grants AA:25, which is not in the preset.
    z.equip[0] = DAGGER;
    z.equip[1] = LEATHER_HAT;
    // Bone Wrist's SA:25 has 10 of 35 AP.
    set_ap(&mut z, sa(25), 10);
    let mut held = inv(&[(BONE_WRIST, 1), (MYTHRIL_ARMLET, 1), (WRIST, 1)]);
    let r = equip_for_ability_learning(&db, &mut held, &mut z, &t, cfg);
    // Slot 0: the Dagger is learnable and the held list has no weapon: stays.
    assert_eq!(z.equip[0], DAGGER);
    // Slot 1: the hat teaches nothing and something is learnable elsewhere:
    // it comes off (`EquipUI.cs:1038-1045`).
    assert_eq!(z.equip[1], NO_ITEM);
    assert_eq!(held.count(LEATHER_HAT), 1);
    // Slot 2: of the three learnable armlets the lowest quality wins (Wrist,
    // 20). The source never updates `learnableLeftAp` after the first pick,
    // so the 25 AP left on Bone Wrist is not preferred.
    assert_eq!(z.equip[2], WRIST);
    assert_eq!(r.changed, [false, true, true, false, false]);
}

#[test]
fn ability_optimize_equips_better_pieces_when_nothing_is_learnable() {
    let mut d = db();
    let t = gems();
    let cfg = MenuConfig::default();
    // Synthetic pieces with no abilities.
    d.items.insert(
        200,
        row(item_type::ARMLET, armlet_mask(), 10.0, 0, Vec::new()),
    );
    d.items.insert(
        201,
        row(item_type::ARMLET, armlet_mask(), 40.0, 0, Vec::new()),
    );
    let mut z = zidane();
    z.equip[2] = 200;
    let mut held = inv(&[(201, 1)]);
    let r = equip_for_ability_learning(&d, &mut held, &mut z, &t, cfg);
    // Nothing is learnable and a better piece exists: take it
    // (`EquipUI.cs:1047-1058`).
    assert_eq!(z.equip[2], 201);
    assert!(r.changed[2]);
    assert_eq!(held.count(200), 1);
}

#[test]
fn ability_optimize_with_nothing_to_do_keeps_the_gear() {
    let db = db();
    let t = gems();
    let cfg = MenuConfig::default();
    let mut z = zidane();
    z.equip[1] = LEATHER_HAT; // AA:25 is not in the preset
    let mut held = inv(&[]);
    let r = equip_for_ability_learning(&db, &mut held, &mut z, &t, cfg);
    // No learnable piece anywhere (`canLearn` false) and nothing better
    // (`canEquip` false): the useless hat is taken off.
    assert_eq!(z.equip[1], NO_ITEM);
    assert!(r.changed[1]);
    assert_eq!(held.count(LEATHER_HAT), 1);
}

#[test]
fn learnable_items_sum_the_ap_left_and_dedupe_equal_keys() {
    let db = db();
    let mut z = zidane();
    set_ap(&mut z, sa(25), 10); // 25 left on Bone Wrist
    set_ap(&mut z, sa(23), 15); // 25 left
    z.equip[2] = GLASS_ARMLET; // SA:23, SA:49 (49 not in the preset)
    let held = inv(&[(BONE_WRIST, 1)]);
    let l = LearnableItems::new(&db, &held, &z);
    assert_eq!(l.left_ap(BONE_WRIST), Some(25));
    // Glass Armlet teaches only SA:23 (SA:49 is not in the preset): 40 - 15.
    assert_eq!(l.left_ap(GLASS_ARMLET), Some(25));
    // Mythril Armlet is not held or worn.
    assert_eq!(l.left_ap(MYTHRIL_ARMLET), None);
}

#[test]
fn resistances_are_the_union_over_the_five_pieces() {
    let db = db();
    // Stats.csv rows: 3 Glass Armlet half Water(16); 4 Bone Wrist boost
    // Earth(8); 5 nothing. Synthetic fourth piece: row 6 weak Fire(1),
    // guard Cold(2), absorb Thunder(4).
    let mut rows = vec![EquipElementRow::default(); 8];
    rows[3].half = Elements::AQUA;
    rows[4].bonus = Elements::EARTH;
    rows[6] = EquipElementRow {
        invalid: Elements::COLD,
        absorb: Elements::THUNDER,
        half: Elements::AQUA | Elements::WIND,
        weak: Elements::FIRE,
        bonus: 0,
    };
    let mut d = db;
    d.items
        .insert(210, row(item_type::ARMOR, 0xFFF, 0.0, 6, Vec::new()));
    let mut equip = [NO_ITEM; 5];
    equip[2] = GLASS_ARMLET;
    equip[3] = 210;
    let r = equip_resistances(&d, &rows, &equip);
    assert_eq!(r.half, Elements(Elements::AQUA | Elements::WIND));
    assert_eq!(r.weak, Elements(Elements::FIRE));
    assert_eq!(r.guard, Elements(Elements::COLD));
    assert_eq!(r.absorb, Elements(Elements::THUNDER));
    assert_eq!(r.bonus, Elements::NONE);
    // Swap the armlet for the Bone Wrist: the Water half goes, Earth boost on.
    equip[2] = BONE_WRIST;
    let r = equip_resistances(&d, &rows, &equip);
    assert_eq!(r.half, Elements(Elements::AQUA | Elements::WIND));
    assert_eq!(r.bonus, Elements(Elements::EARTH));
    // Empty: nothing.
    assert_eq!(
        equip_resistances(&d, &rows, &[NO_ITEM; 5]),
        EquipResistances::default()
    );
}

#[test]
fn weapon_status_comes_from_the_add_status_table() {
    let sets = [0u32, 0x10, 0x20];
    assert_eq!(weapon_added_status(&sets, Some(2)), 0x20);
    assert_eq!(weapon_added_status(&sets, Some(9)), 0);
    assert_eq!(weapon_added_status(&sets, None), 0);
}

// ---------------------------------------------------------------------------
// Ability
// ---------------------------------------------------------------------------

#[test]
fn ability_ids_map_to_indexes() {
    // `ff9abil.cs:316-360`: id % 256 < 192 is active.
    assert!(is_active(101));
    assert!(!is_active(sa(0)));
    assert!(is_support(sa(46)));
    assert_eq!(support_index(sa(46)), Some(46));
    assert_eq!(support_index(101), None);
    // Pool 1: id 256 + 192 + 5 is support 64 + 5.
    assert_eq!(support_index(256 + 192 + 5), Some(69));
    assert_eq!(support_ability_id(69), 256 + 192 + 5);
    assert_eq!(active_index(101), Some(101));
    assert_eq!(active_index(256 + 10), Some(192 + 10));
    assert_eq!(active_index(sa(3)), None);
}

#[test]
fn stone_stock_grows_with_level() {
    // `FF9Level_GetCap`: Gems + level * 4 / 10 + (bonus.cap >> 5). Zidane's
    // base is 18: level 1 gives 18 + 0, level 10 gives 22, level 99 gives
    // 18 + 39 = 57; a bonus.cap of 64 adds 2.
    assert_eq!(max_capacity(18, 1, 0), 18);
    assert_eq!(max_capacity(18, 10, 0), 22);
    assert_eq!(max_capacity(18, 99, 0), 57);
    assert_eq!(max_capacity(18, 99, 64), 59);
    assert_eq!(max_capacity(18, 99, 31), 57);
}

#[test]
fn learn_state_and_ap_bar() {
    let mut z = zidane();
    assert!(has_ap(&z));
    assert!(has_sa(&z));
    assert_eq!(ap_of(&z, 999), -1);
    assert_eq!(max_ap_of(&z, 101), 40);
    assert!(!is_master(&z, 101));
    set_ap(&mut z, 101, 25);
    let bar = ap_bar(&z, 101, true).unwrap();
    assert_eq!((bar.cur, bar.max, bar.complete), (25, 40, false));
    assert!(bar.show_text);
    assert!((bar.ratio - 0.625).abs() < 1e-6);
    set_ap(&mut z, 101, 40);
    assert!(is_master(&z, 101));
    let bar = ap_bar(&z, 101, true).unwrap();
    assert!(bar.complete);
    assert!(!bar.show_text);
    assert_eq!(bar.ratio, 1.0);
    assert!(ap_bar(&z, 999, true).is_none());
    // A preset without AP (every max 0) counts as having no AP.
    let mut free = zidane();
    for p in &mut free.learnable {
        p.max_ap = 0;
    }
    assert!(!has_ap(&free));
}

#[test]
fn active_rows_need_learning_or_equipment_and_mp() {
    let db = db();
    let mut z = zidane();
    // Flee: kind bit 1 (usable in the menu), no MP; Detect costs 10.
    let actions = {
        let mut v = vec![ActionInfo::default(); 120];
        v[101] = ActionInfo { mp: 0, kind: 1 };
        v[102] = ActionInfo { mp: 10, kind: 1 };
        v
    };
    let eqp = equipped_abilities(&db, &z.equip);
    let c = ctx();
    // Not learned (0 of 40) and not worn: hidden (`AbilityUI.cs:1257-1265`).
    assert_eq!(
        check_aa_type(&z, &c, &eqp, &actions, 101),
        AbilityType::NoDraw
    );
    set_ap(&mut z, 101, 40);
    assert_eq!(
        check_aa_type(&z, &c, &eqp, &actions, 101),
        AbilityType::Enable
    );
    // An ability the preset lacks: hidden.
    assert_eq!(
        check_aa_type(&z, &c, &eqp, &actions, 103),
        AbilityType::NoDraw
    );
    // Silence (bit 3) grays everything.
    z.status = 0x8;
    assert_eq!(
        check_aa_type(&z, &c, &eqp, &actions, 101),
        AbilityType::CantSpell
    );
    z.status = 0;
    // The worn Mage Masher grants AA:102 before it is learned; 10 MP of 50.
    z.equip[0] = MAGE_MASHER;
    let eqp = equipped_abilities(&db, &z.equip);
    assert_eq!(
        check_aa_type(&z, &c, &eqp, &actions, 102),
        AbilityType::Enable
    );
    z.cur_mp = 9;
    assert_eq!(
        check_aa_type(&z, &c, &eqp, &actions, 102),
        AbilityType::CantSpell
    );
    // Garnet's summon flag quadruples the cost of a summon-type action.
    let summon = ActionInfo {
        mp: 20,
        kind: 1 | 4,
    };
    assert_eq!(mp_cost(&summon, false), 20);
    assert_eq!(mp_cost(&summon, true), 80);
}

#[test]
fn the_active_lock_option_hides_learned_abilities_except_for_quina() {
    let db = db();
    let actions = vec![ActionInfo { mp: 0, kind: 1 }; 120];
    let mut z = zidane();
    set_ap(&mut z, 101, 40);
    let c = AbilityContext {
        config: MenuConfig::from_lock_option(2),
        garnet_summon_flag: false,
    };
    let eqp = equipped_abilities(&db, &z.equip);
    assert_eq!(
        check_aa_type(&z, &c, &eqp, &actions, 101),
        AbilityType::NoDraw
    );
    z.char_id = QUINA;
    assert_eq!(
        check_aa_type(&z, &c, &eqp, &actions, 101),
        AbilityType::Enable
    );
    assert_eq!(
        MenuConfig::from_lock_option(1),
        MenuConfig {
            lock_support: true,
            lock_active: false,
        }
    );
    assert_eq!(
        MenuConfig::from_lock_option(3),
        MenuConfig {
            lock_support: true,
            lock_active: true,
        }
    );
    assert_eq!(MenuConfig::from_lock_option(0), MenuConfig::default());
}

#[test]
fn lists_split_active_and_support_in_preset_order() {
    let z = zidane();
    let l = ability_lists(&z, &gems());
    assert_eq!(l.active, vec![101, 102]);
    assert_eq!(
        l.support,
        vec![sa(0), sa(3), sa(14), sa(19), sa(23), sa(25), sa(46)]
    );
    let types: Vec<AbilityType> = l.active.iter().map(|_| AbilityType::NoDraw).collect();
    assert_eq!(first_active_row(&types, false), 0);
    let types = [
        AbilityType::NoDraw,
        AbilityType::Selected,
        AbilityType::Enable,
    ];
    // Support lists stop on Selected; active lists skip it.
    assert_eq!(first_active_row(&types, true), 1);
    assert_eq!(first_active_row(&types, false), 2);
}

#[test]
fn sub_menus_follow_hp_status_and_the_preset() {
    let z = zidane();
    let l = ability_lists(&z, &gems());
    let open = SubMenuAccess::default();
    assert_eq!(sub_menu_enabled(&z, &l, &open), (true, true));
    let mut dead = zidane();
    dead.cur_hp = 0;
    assert_eq!(sub_menu_enabled(&dead, &l, &open), (false, true));
    let mut silent = zidane();
    silent.status = 0x8;
    assert_eq!(sub_menu_enabled(&silent, &l, &open), (false, true));
    let no_magic = SubMenuAccess {
        no_magic_event: true,
        ..open
    };
    assert_eq!(sub_menu_enabled(&z, &l, &no_magic), (false, true));
    let locked = SubMenuAccess {
        support_disabled_by_main_menu: true,
        ..open
    };
    assert_eq!(sub_menu_enabled(&z, &l, &locked), (true, false));
    // No passive in the preset: Equip sub-menu off.
    let mut active_only = zidane();
    active_only.learnable.retain(|p| p.id < 192);
    active_only.pa.truncate(active_only.learnable.len());
    assert_eq!(sub_menu_enabled(&active_only, &l, &open), (true, false));
}

#[test]
fn enabling_support_abilities_spends_and_returns_stones() {
    let db = db();
    let t = gems();
    let mut z = zidane();
    set_ap(&mut z, sa(0), 95); // Auto-Reflect, 15 stones
    set_ap(&mut z, sa(3), 25); // Auto-Regen, 10 stones
    let eqp = equipped_abilities(&db, &z.equip);
    let c = ctx();
    assert_eq!(check_sa_type(&z, &t, &c, &eqp, sa(3)), AbilityType::Enable);
    assert_eq!(
        toggle_support(&mut z, &t, &c, &eqp, sa(3)),
        Ok(SupportToggle::Enabled)
    );
    // 18 - 10 = 8; Auto-Reflect (15) no longer fits.
    assert_eq!(z.cur_capa, 8);
    assert_eq!(
        check_sa_type(&z, &t, &c, &eqp, sa(0)),
        AbilityType::CantSpell
    );
    assert_eq!(
        toggle_support(&mut z, &t, &c, &eqp, sa(0)),
        Err(SupportRefusal::NotAvailable(AbilityType::CantSpell))
    );
    assert_eq!(
        check_sa_type(&z, &t, &c, &eqp, sa(3)),
        AbilityType::Selected
    );
    // Turn Auto-Regen off: 18 again, then Auto-Reflect fits (18 - 15 = 3).
    assert_eq!(
        toggle_support(&mut z, &t, &c, &eqp, sa(3)),
        Ok(SupportToggle::Disabled)
    );
    assert_eq!(z.cur_capa, 18);
    toggle_support(&mut z, &t, &c, &eqp, sa(0)).unwrap();
    assert_eq!((z.cur_capa, z.enabled.clone()), (3, vec![0]));
    // An unlearned, unworn ability is hidden and refuses.
    assert_eq!(
        toggle_support(&mut z, &t, &c, &eqp, sa(46)),
        Err(SupportRefusal::NotAvailable(AbilityType::NoDraw))
    );
    // A physical ability id is not a support row.
    assert!(toggle_support(&mut z, &t, &c, &eqp, 101).is_err());
}

#[test]
fn the_stone_recount_drops_what_does_not_fit_and_over_returns_stones() {
    // `ff9abil.cs:259-285`. Two abilities on with a stock of 18: Auto-Reflect
    // (15) fits, Auto-Regen (10) does not. The source switches it off and then
    // adds its cost to the stones left although it was never taken:
    // min(3 + 10, 18) = 13, not 3. Kept as the source has it.
    let t = gems();
    let mut z = zidane();
    z.enabled = vec![0, 3];
    z.basis_capa = 18;
    play_update(&mut z, &t, MenuConfig::default());
    assert_eq!(z.enabled, vec![0]);
    assert_eq!(z.cur_capa, 13);
}

#[test]
fn forced_abilities_cost_nothing_and_support_lock_skips_the_recount() {
    let t = gems();
    let mut z = zidane();
    z.forced.insert(3);
    z.enabled = vec![3];
    assert_eq!(gem_cost(&z, &t, 3), 0);
    assert_eq!(gem_cost(&z, &t, 0), 15);
    play_update(&mut z, &t, MenuConfig::default());
    assert_eq!(z.cur_capa, 18);
    // Under the support lock the recount is skipped and `cur.capa` comes
    // from `basis.capa` (`ff9play.cs:267`, `ff9abil.cs:261-262`).
    let mut z = zidane();
    z.enabled = vec![0, 3];
    z.basis_capa = 7;
    play_update(
        &mut z,
        &t,
        MenuConfig {
            lock_support: true,
            lock_active: false,
        },
    );
    assert_eq!((z.cur_capa, z.enabled.clone()), (7, vec![0, 3]));
}

#[test]
fn support_rows_follow_the_lock_option() {
    let db = db();
    let t = gems();
    let mut z = zidane();
    z.equip[2] = WRIST; // grants SA:46
    let eqp = equipped_abilities(&db, &z.equip);
    let c = AbilityContext {
        config: MenuConfig::from_lock_option(1),
        garnet_summon_flag: false,
    };
    // Locked to the equipment: worn and in the preset -> CantDisable; in the
    // preset but not worn -> CantSpell (`AbilityUI.cs:1281-1290`).
    assert_eq!(
        check_sa_type(&z, &t, &c, &eqp, sa(46)),
        AbilityType::CantDisable
    );
    assert_eq!(
        check_sa_type(&z, &t, &c, &eqp, sa(0)),
        AbilityType::CantSpell
    );
    // Not in the preset at all: hidden.
    assert_eq!(check_sa_type(&z, &t, &c, &eqp, sa(60)), AbilityType::NoDraw);
    // Update after equipment change switches everything to "worn or not".
    let cfg = c.config;
    z.enabled = vec![0];
    ability::update_sa(&mut z, &db, &t, cfg);
    assert_eq!(z.enabled, vec![46]);
}

#[test]
fn banished_hidden_and_forced_rows() {
    let db = db();
    let t = gems();
    let mut z = zidane();
    set_ap(&mut z, sa(0), 95);
    set_ap(&mut z, sa(3), 25);
    let eqp = equipped_abilities(&db, &z.equip);
    let c = ctx();
    z.hidden.insert(0);
    assert_eq!(check_sa_type(&z, &t, &c, &eqp, sa(0)), AbilityType::NoDraw);
    z.hidden.clear();
    z.banished.insert(0);
    assert_eq!(
        check_sa_type(&z, &t, &c, &eqp, sa(0)),
        AbilityType::CantSpell
    );
    z.forced.insert(0);
    assert_eq!(
        check_sa_type(&z, &t, &c, &eqp, sa(0)),
        AbilityType::CantDisable
    );
    // A selected ability whose whole hierarchy is forced refuses to switch off.
    z.banished.clear();
    z.enabled = vec![0];
    assert_eq!(
        toggle_support(&mut z, &t, &c, &eqp, sa(0)),
        Err(SupportRefusal::AllForced)
    );
}

/// Synthetic boosted chain: SA:5 (cost 4) with boosted versions 6 (cost 8)
/// and 7 (cost 3); the preset holds 5, 6 and 7. Real `AbilityGems.csv`
/// leaves the Boosted column empty (HP+10% and HP+20% are separate rows).
fn boosted_setup() -> (MenuCharacter, SupportTable) {
    let mut t = gems();
    t.rows[5] = SupportRow {
        gems: 4,
        boosted: vec![6, 7],
    };
    t.rows[6].gems = 8;
    t.rows[7].gems = 3;
    let mut z = MenuCharacter::new(ZIDANE);
    z.learnable = [5, 6, 7]
        .iter()
        .map(|&n| PaEntry {
            id: sa(n),
            max_ap: 10,
        })
        .collect();
    z.pa = vec![10, 10, 10];
    z.max_capa = 20;
    z.cur_capa = 20;
    z.basis_capa = 20;
    (z, t)
}

#[test]
fn boosted_versions_fold_into_one_row_and_step_up() {
    let (mut z, t) = boosted_setup();
    let db = db();
    let eqp = equipped_abilities(&db, &z.equip);
    let c = ctx();
    // Only the base row is listed (`AbilityUI.cs:1438-1452`).
    assert_eq!(ability_lists(&z, &t).support, vec![sa(5)]);
    assert_eq!(boosted_max_level(&z, &t, 5), 2);
    assert_eq!(base_of_boosted(&t, 7), 5);
    assert_eq!(hierarchy(&t, 6), vec![5, 6, 7]);
    // Confirm: base on (4), 16 left.
    toggle_support(&mut z, &t, &c, &eqp, sa(5)).unwrap();
    assert_eq!((z.cur_capa, boosted_level(&z, &t, 5)), (16, 0));
    // Confirm again: next boosted version (cost 8) on, 8 left.
    assert_eq!(
        toggle_support(&mut z, &t, &c, &eqp, sa(5)),
        Ok(SupportToggle::BoostedUp)
    );
    assert_eq!((z.cur_capa, boosted_level(&z, &t, 5)), (8, 1));
    // The row shows the level-1 name and the summed cost 4 + 8 = 12.
    assert_eq!(
        support_row_display(&z, &t, 5, AbilityType::Selected),
        (6, 12)
    );
    // Third confirm: version 7 costs 3 <= 8, on, 5 left.
    toggle_support(&mut z, &t, &c, &eqp, sa(5)).unwrap();
    assert_eq!((z.cur_capa, boosted_level(&z, &t, 5)), (5, 2));
    // Fourth: no next version, everything off, 20 back.
    assert_eq!(
        toggle_support(&mut z, &t, &c, &eqp, sa(5)),
        Ok(SupportToggle::Disabled)
    );
    assert_eq!((z.cur_capa, z.enabled.len()), (20, 0));
}

#[test]
fn boosted_step_stops_when_the_next_version_does_not_fit() {
    let (mut z, mut t) = boosted_setup();
    t.rows[6].gems = 30;
    let db = db();
    let eqp = equipped_abilities(&db, &z.equip);
    let c = ctx();
    toggle_support(&mut z, &t, &c, &eqp, sa(5)).unwrap();
    // Version 6 needs 30 > 16 left: the source switches the whole chain off.
    toggle_support(&mut z, &t, &c, &eqp, sa(5)).unwrap();
    assert!(z.enabled.is_empty());
    assert_eq!(z.cur_capa, 20);
}

#[test]
fn the_special_key_turns_on_the_whole_chain_and_steps_down_one_level() {
    let (mut z, t) = boosted_setup();
    let db = db();
    let eqp = equipped_abilities(&db, &z.equip);
    let c = ctx();
    // Enable: base 4 + 8 + 3 = 15, 5 left (`AbilityUI.cs:626-660`).
    toggle_support_special(&mut z, &t, &c, &eqp, sa(5)).unwrap();
    assert_eq!(z.enabled, vec![5, 6, 7]);
    assert_eq!(z.cur_capa, 5);
    // Selected: switch off only the highest version (7): 8 left.
    toggle_support_special(&mut z, &t, &c, &eqp, sa(5)).unwrap();
    assert_eq!(z.enabled, vec![5, 6]);
    assert_eq!(z.cur_capa, 8);
    toggle_support_special(&mut z, &t, &c, &eqp, sa(5)).unwrap();
    assert_eq!(z.enabled, vec![5]);
    toggle_support_special(&mut z, &t, &c, &eqp, sa(5)).unwrap();
    assert!(z.enabled.is_empty());
    assert_eq!(z.cur_capa, 20);
}

#[test]
fn the_special_key_stops_at_the_first_version_that_does_not_fit() {
    let (mut z, t) = boosted_setup();
    z.max_capa = 14;
    z.cur_capa = 14;
    z.basis_capa = 14;
    let db = db();
    let eqp = equipped_abilities(&db, &z.equip);
    toggle_support_special(&mut z, &t, &ctx(), &eqp, sa(5)).unwrap();
    // 4 + 8 = 12 fits, 3 more = 15 > 14: stops after version 6.
    assert_eq!(z.enabled, vec![5, 6]);
    assert_eq!(z.cur_capa, 2);
}

#[test]
fn spending_mp_after_a_field_cast() {
    let mut z = zidane();
    z.cur_mp = 25;
    assert_eq!(spend_mp(&mut z, 10, false), (10, true));
    assert_eq!(z.cur_mp, 15);
    // The "HP and MP full" cheat keeps the MP.
    assert_eq!(spend_mp(&mut z, 10, true), (10, true));
    assert_eq!(z.cur_mp, 15);
    // 15 -> 5: not enough for another 10.
    assert_eq!(spend_mp(&mut z, 10, false), (10, false));
    assert_eq!(z.cur_mp, 5);
}

#[test]
fn item_abilities_show_learn_state_for_the_character() {
    let db = db();
    let mut z = zidane();
    set_ap(&mut z, sa(25), 20);
    z.enabled = vec![25];
    let actions = vec![ActionInfo::default(); 120];
    // Bone Wrist teaches SA:25 (20 of 35, switched on). `EquipUI.cs:720-768`.
    let s = item_ability_states(&z, &db, &actions, BONE_WRIST);
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].stone, Stone::SupportOn);
    assert!(s[0].white && s[0].in_preset);
    let bar = s[0].ap.unwrap();
    assert_eq!((bar.cur, bar.max, bar.complete), (20, 35, false));
    // Glass Armlet: SA:23 is in the preset (off), SA:49 is not.
    let s = item_ability_states(&z, &db, &actions, GLASS_ARMLET);
    assert_eq!(s[0].stone, Stone::SupportOff);
    assert_eq!(s[1].stone, Stone::SupportNull);
    assert!(!s[1].white && s[1].ap.is_none());
    // The Moonstone's AA:11 is not in Zidane's preset.
    let s = item_ability_states(&z, &db, &actions, MOONSTONE);
    assert_eq!(s[0].stone, Stone::ActiveNull);
    assert_eq!(s[1].stone, Stone::SupportOff);
    // A character with no AP table sees gray null stones everywhere.
    let mut free = zidane();
    for p in &mut free.learnable {
        p.max_ap = 0;
    }
    let s = item_ability_states(&free, &db, &actions, BONE_WRIST);
    assert_eq!(s[0].stone, Stone::SupportNull);
    assert!(!s[0].white && s[0].ap.is_none());
}

#[test]
fn item_ability_panel_pages_by_three() {
    let mut d = db();
    // Counts of non-zero abilities: 0, 1, 3, 4, 7 -> 0, 1, 1, 2, 3 pages.
    for (id, n) in [(210u16, 0usize), (211, 1), (212, 3), (213, 4), (214, 7)] {
        let mut a = vec![0, 0];
        a.extend((1..=n as i32).map(|i| 100 + i));
        d.items.insert(id, row(item_type::ARMOR, 0xFFF, 0.0, 0, a));
    }
    let pages: Vec<usize> = (210..=214).map(|id| ability_page_count(&d, id)).collect();
    assert_eq!(pages, vec![0, 1, 1, 2, 3]);
}

#[test]
fn equipped_ability_maps_collect_slots_and_items() {
    let db = db();
    let mut equip = [NO_ITEM; 5];
    equip[0] = MAGE_MASHER; // AA:102, AA:101
    equip[2] = LEATHER_WRIST; // SA:19, AA:29
    equip[4] = MOONSTONE; // AA:11, SA:19
    let e = equipped_abilities(&db, &equip);
    assert_eq!(e.parts[&101], 0b00001);
    assert_eq!(e.parts[&sa(19)], 0b10100); // slots 2 and 4
    assert_eq!(e.items[&sa(19)][2], LEATHER_WRIST);
    assert_eq!(e.items[&sa(19)][4], MOONSTONE);
    assert_eq!(e.items[&sa(19)][0], NO_ITEM);
    assert!(!e.contains(sa(46)));
}

#[test]
fn update_after_equip_matches_the_documented_refund_rule() {
    // FF9FEqp_UpdateSA gives the table cost back to the stones left (capped at
    // the stock) before FF9Play_Update recounts from scratch.
    let db = db();
    let t = gems();
    let mut z = zidane();
    z.equip[2] = NO_ITEM;
    z.enabled = vec![46]; // enabled but no longer worn and unlearned
    z.cur_capa = 15;
    ability::update_sa(&mut z, &db, &t, MenuConfig::default());
    assert!(z.enabled.is_empty());
    assert_eq!(z.cur_capa, 18); // 15 + 3, within the stock of 18
    // Not enough room: capped at the stock.
    z.enabled = vec![46];
    z.cur_capa = 17;
    ability::update_sa(&mut z, &db, &t, MenuConfig::default());
    assert_eq!(z.cur_capa, 18);
}

// ---------------------------------------------------------------------------
// Installed tables (skipped when no install is present)
// ---------------------------------------------------------------------------

fn installed_tables() -> Option<tantalus_assets::pc::battle_tables::BattleTables> {
    use tantalus_assets::pc::battle_tables::{extract_battle_tables, is_memoria_patched};
    use tantalus_assets::pc::managed_il::Assembly;
    let path = std::env::var_os("TANTALUS_ASSEMBLY_CSHARP")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(
                "E:/SteamLibrary/steamapps/common/FINAL FANTASY IX/x64/FF9_Data/Managed/Assembly-CSharp.dll",
            )
        });
    // A Memoria-patched DLL loads these tables from CSV at runtime, so they are not in its
    // IL. Memoria's installer keeps the vanilla assembly next to it as `.bak`; read that.
    let mut candidates = vec![path.clone()];
    candidates.push(path.with_extension("bak"));
    for candidate in candidates {
        let Ok(bytes) = std::fs::read(&candidate) else {
            continue;
        };
        let Ok(assembly) = Assembly::parse(&bytes) else {
            continue;
        };
        if is_memoria_patched(&assembly) {
            eprintln!(
                "skip {}: Memoria-patched, tables are not in its IL",
                candidate.display()
            );
            continue;
        }
        return Some(extract_battle_tables(&assembly).unwrap());
    }
    eprintln!("skipping: no vanilla Assembly-CSharp found (set TANTALUS_ASSEMBLY_CSHARP)");
    None
}

/// The item table of the install as an [`ItemDb`] (equip mask, type bits,
/// quality, bonus index and abilities; price, power and armor are left out).
fn installed_db(t: &tantalus_assets::pc::battle_tables::BattleTables) -> ItemDb {
    let items = t
        .items
        .iter()
        .map(|r| {
            let mut d = row(
                r.kind,
                u64::from(r.equip),
                f32::from(r.eq_lv),
                usize::from(r.bonus),
                r.ability.iter().map(|a| i32::from(*a)).collect(),
            );
            d.price = u32::from(r.price);
            (r.id as u16, d)
        })
        .collect();
    ItemDb {
        items,
        bonuses: Vec::new(),
    }
}

#[test]
fn installed_tables_agree_with_the_hand_derived_rows() {
    let Some(t) = installed_tables() else { return };
    let db = installed_db(&t);
    // Support-ability stone costs equal `AbilityGems.csv` rows 0, 14, 25, 46.
    let table = SupportTable {
        rows: t
            .support_abilities
            .iter()
            .map(|r| SupportRow {
                gems: i32::from(r.capa_val),
                boosted: Vec::new(),
            })
            .collect(),
    };
    assert_eq!(table.rows.len(), 64);
    assert_eq!([0, 14, 25, 46].map(|i| table.gems(i)), [15, 2, 3, 3]);
    // Zidane's preset (set 0) matches `Abilities/Zidane.csv`: 48 rows, AA:101
    // first with 40 AP, SA:46 with 45.
    let mut z = MenuCharacter::new(ZIDANE);
    z.learnable = t.learnable_abilities[0]
        .iter()
        .map(|p| PaEntry {
            id: i32::from(p.id),
            max_ap: i32::from(p.max_ap),
        })
        .collect();
    z.pa = vec![0; z.learnable.len()];
    assert_eq!(z.learnable.len(), 48);
    assert_eq!((z.learnable[0].id, z.learnable[0].max_ap), (101, 40));
    assert_eq!(max_ap_of(&z, sa(46)), 45);
    let lists = ability_lists(&z, &table);
    assert_eq!(&lists.active[..3], &[101, 102, 103]);
    assert!(lists.support.contains(&sa(46)));
    assert!(lists.support.iter().all(|id| is_support(*id)));
    // Max stones: the base row of Zidane is 18 (BaseStats.csv).
    assert_eq!(max_capacity(i32::from(t.level_base[0].cap), 1, 0), 18);
    // Equip masks: the Dagger is Zidane's alone.
    let dagger = db.get(DAGGER).unwrap();
    assert_eq!(dagger.equip, equip_mask_from_char_id(0));
    assert!(tantalus_legacy::shop::can_equip(
        &db,
        DAGGER,
        z.equip_mask(),
        0
    ));
    assert!(!tantalus_legacy::shop::can_equip(
        &db,
        DAGGER,
        MenuCharacter::new(STEINER).equip_mask(),
        0
    ));
    // Element rows: Glass Armlet half Water (16), Bone Wrist boost Earth (8).
    let rows: Vec<EquipElementRow> = t
        .equip_bonus
        .iter()
        .map(|r| EquipElementRow {
            invalid: r.invalid,
            absorb: r.absorb,
            half: r.half,
            weak: r.weak,
            bonus: r.p_up_attr,
        })
        .collect();
    let mut equip = [NO_ITEM; 5];
    equip[2] = GLASS_ARMLET;
    let r = equip_resistances(&db, &rows, &equip);
    assert_eq!(r.half, Elements(Elements::AQUA));
    equip[2] = BONE_WRIST;
    let r = equip_resistances(&db, &rows, &equip);
    assert_eq!(r.bonus, Elements(Elements::EARTH));
    // The Moonstone's abilities: AA:11 and SA:19 (id 211).
    equip = [NO_ITEM; 5];
    equip[4] = MOONSTONE;
    let e = equipped_abilities(&db, &equip);
    assert_eq!(
        e.parts.keys().copied().collect::<Vec<_>>(),
        vec![11, sa(19)]
    );
}

#[test]
fn installed_items_strongest_equals_the_best_quality_per_slot() {
    let Some(t) = installed_tables() else { return };
    let db = installed_db(&t);
    let table = SupportTable::default();
    for char_id in 0..8u32 {
        let mut ch = MenuCharacter::new(char_id);
        // One of every real item held.
        let mut held = inv(&db
            .items
            .keys()
            .filter(|id| **id != NO_ITEM)
            .map(|id| (*id, 1u8))
            .collect::<Vec<_>>());
        let before = held.clone();
        equip_strongest(&db, &mut held, &mut ch, &table, MenuConfig::default());
        for slot in 0..5 {
            let best = before
                .held()
                .filter(|s| tantalus_legacy::shop::can_equip(&db, s.id, ch.equip_mask(), slot))
                .map(|s| db.get(s.id).unwrap().eq_lv)
                .fold(f32::MIN, f32::max);
            let worn = ch.equip[slot];
            if best == f32::MIN {
                assert_eq!(worn, NO_ITEM, "char {char_id} slot {slot}");
            } else {
                assert_eq!(
                    db.get(worn).unwrap().eq_lv,
                    best,
                    "char {char_id} slot {slot}"
                );
                assert_eq!(held.count(worn), 0);
            }
        }
        // The worn pieces and the inventory together are the original set.
        let total: i32 = held.held().map(|s| i32::from(s.count)).sum::<i32>()
            + ch.equip.iter().filter(|e| **e != NO_ITEM).count() as i32;
        assert_eq!(total, before.held().count() as i32);
    }
}
