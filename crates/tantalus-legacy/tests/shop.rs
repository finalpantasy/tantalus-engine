//! Hand-derived cases for the shop, synthesis, inventory and equipment core.
//!
//! Item rows are copied by hand from the pinned stock CSVs under
//! `Memoria.Patcher/StreamingAssets/Data/Items` (Items, Armors, Weapons,
//! Stats, Synthesis, ShopItems, ItemEffects); the expected numbers are worked
//! out from the source arithmetic in each test comment.

use std::collections::BTreeMap;

use tantalus_legacy::shop::data::item_type;
use tantalus_legacy::shop::*;

const POTION: u16 = 236;
const DAGGER: u16 = 1;
const MAGE_MASHER: u16 = 2;
const WRIST: u16 = 88;
const LEATHER_WRIST: u16 = 89;
const GARNET_GEM: u16 = 224;

/// Zidane is character 0: mask bit 11. Steiner is 3: bit 8.
const ZIDANE: u64 = 0x800;

fn item(price: u32, kind: u8, equip: u64) -> ItemData {
    ItemData {
        price,
        selling_price: default_selling_price(price),
        equip,
        kind,
        sort: 0.0,
        eq_lv: 0.0,
        bonus: 0,
        ability: vec![],
        weapon_power: None,
        armor: None,
    }
}

fn db() -> ItemDb {
    let mut items = BTreeMap::new();
    // 236 Potion: price 50, Item|Usable.
    items.insert(POTION, item(50, item_type::ITEM | item_type::USABLE, 0));
    // 224 Garnet (gem): Item|Usable too; price 0 in this test row.
    items.insert(GARNET_GEM, item(0, item_type::ITEM | item_type::USABLE, 0));
    // 001 Dagger: price 320, quality 1, sort 22, Zidane only, power 12, AA:101.
    let mut d = item(320, item_type::WEAPON, ZIDANE);
    d.sort = 22.0;
    d.eq_lv = 1.0;
    d.ability = vec![101];
    d.weapon_power = Some(12);
    items.insert(DAGGER, d);
    // 002 Mage Masher: price 500, quality 2, sort 23, power 14, AA:102 AA:101.
    let mut d = item(500, item_type::WEAPON, ZIDANE);
    d.sort = 23.0;
    d.eq_lv = 2.0;
    d.ability = vec![102, 101];
    d.weapon_power = Some(14);
    items.insert(MAGE_MASHER, d);
    // 088 Wrist: armlet, price 130, bonus 1, armor row 0 (0,5,0,3), SA:46.
    // Characters: Zidane Vivi Garnet Quina Eiko Amarant Cinna (not Steiner).
    let wrist_mask = [0u32, 1, 2, 5, 6, 7, 8]
        .iter()
        .fold(0u64, |m, c| m | equip_mask_from_char_id(*c));
    let mut d = item(130, item_type::ARMLET, wrist_mask);
    d.bonus = 1;
    d.ability = vec![192 + 46];
    d.armor = Some(ArmorData {
        p_def: 0,
        p_ev: 5,
        m_def: 0,
        m_ev: 3,
    });
    items.insert(WRIST, d);
    // 089 Leather Wrist: price 200, bonus 2 (spirit +1), armor row 1 (0,7,0,5),
    // SA:19 and AA:29.
    let mut d = item(200, item_type::ARMLET, wrist_mask);
    d.bonus = 2;
    d.ability = vec![192 + 19, 29];
    d.armor = Some(ArmorData {
        p_def: 0,
        p_ev: 7,
        m_def: 0,
        m_ev: 5,
    });
    items.insert(LEATHER_WRIST, d);
    ItemDb {
        items,
        bonuses: vec![
            StatBonus::default(),
            StatBonus::default(),
            StatBonus {
                wpr: 1,
                ..StatBonus::default()
            },
        ],
    }
}

fn shops() -> ShopTable {
    let mut shops = BTreeMap::new();
    shops.insert(0, vec![DAGGER, MAGE_MASHER, WRIST, LEATHER_WRIST]);
    shops.insert(1, vec![POTION]);
    shops.insert(2, vec![POTION, DAGGER]);
    shops.insert(3, vec![POTION]);
    ShopTable { shops }
}

/// Synthesis row 0, Butterfly Sword: shops 32,33,34,35,38, price 300,
/// result 7, ingredients Dagger and Mage Masher.
fn butterfly() -> Recipe {
    Recipe {
        id: 0,
        shops: vec![32, 33, 34, 35, 38],
        price: 300,
        result: 7,
        ingredients: vec![DAGGER, MAGE_MASHER],
    }
}

#[test]
fn add_caps_at_99_and_reports_added() {
    let mut inv = Inventory::default();
    assert_eq!(inv.add(POTION, 98), 98);
    // 98 + 5 > 99, so count becomes 99 - 98 = 1.
    assert_eq!(inv.add(POTION, 5), 1);
    assert_eq!(inv.count(POTION), 99);
    assert_eq!(inv.add(POTION, 1), 0);
    // A request above 99 is clamped to 99 first.
    assert_eq!(inv.add(DAGGER, 500), 99);
    assert_eq!(inv.add(NO_ITEM, 1), 0);
}

#[test]
fn empty_slot_is_reused_before_the_list_grows() {
    let mut inv = Inventory::default();
    inv.add(POTION, 3);
    inv.add(DAGGER, 1);
    assert_eq!(inv.remove(POTION, 10), 3); // clamped to what is held
    assert_eq!(inv.count(POTION), 0);
    inv.add(MAGE_MASHER, 2);
    assert_eq!(inv.slots.len(), 2);
    assert_eq!(inv.slots[0].id, MAGE_MASHER);
    assert_eq!(inv.remove(WRIST, 1), 0);
}

#[test]
fn key_items_use_the_generic_ids() {
    let mut inv = Inventory::default();
    // Item id 259 is key item 3 (259 % 1000 in 256..512).
    assert_eq!(inv.add_generic(259, 5), Ok(1));
    assert!(inv.has_important(3));
    assert_eq!(inv.count_generic(259), Ok(1));
    inv.use_important(3);
    assert!(inv.is_used_important(3));
    assert_eq!(inv.remove_generic(259, 1), Ok(1));
    assert!(!inv.has_important(3));
    assert!(!inv.is_used_important(3));
    assert_eq!(inv.remove_generic(259, 1), Ok(0));
    assert_eq!(inv.add_generic(259, 0), Ok(0));
    assert_eq!(inv.add_generic(520, 1), Err(GenericError::CardNotPorted));
    assert_eq!(inv.add_generic(236, 2), Ok(2));
}

#[test]
fn arrange_auto_orders_by_sort_then_id_and_drops_empties() {
    let db = db();
    let mut inv = Inventory::default();
    inv.add(MAGE_MASHER, 1); // sort 23
    inv.add(DAGGER, 1); // sort 22
    inv.add(POTION, 1); // sort 0
    inv.add(WRIST, 1); // sort 0, id 88 after potion? 88 < 236
    inv.add(LEATHER_WRIST, 1);
    inv.remove(LEATHER_WRIST, 1); // leaves an empty slot
    inv.arrange_auto(&db);
    let ids: Vec<u16> = inv.slots.iter().map(|s| s.id).collect();
    // Sort 0 ties break by id: 88 then 236; then 22, then 23.
    assert_eq!(ids, vec![WRIST, POTION, DAGGER, MAGE_MASHER]);
}

#[test]
fn buy_three_potions_with_exact_gil() {
    let (db, shops) = (db(), shops());
    let mut inv = Inventory::default();
    let mut gil = 150;
    // 150 / 50 = 3, stack room 99, so the dialog maximum is 3.
    assert_eq!(max_buy_count(&db, &inv, gil, POTION), Some(3));
    assert_eq!(
        buy(&db, &shops, &mut inv, &mut gil, 1, POTION, 4),
        Err(TradeError::BadCount)
    );
    assert_eq!(buy(&db, &shops, &mut inv, &mut gil, 1, POTION, 3), Ok(3));
    assert_eq!(gil, 0);
    assert_eq!(inv.count(POTION), 3);
    assert!(!item_enabled(&db, &inv, gil, POTION));
    // 149 gil buys only 2.
    assert_eq!(
        max_buy_count(&db, &Inventory::default(), 149, POTION),
        Some(2)
    );
    // Not in this shop's list.
    assert_eq!(
        buy(&db, &shops, &mut inv, &mut 500, 0, POTION, 1),
        Err(TradeError::NotSold)
    );
}

#[test]
fn buy_is_limited_by_the_stack_cap_and_free_items() {
    let db = db();
    let mut inv = Inventory::default();
    inv.add(POTION, 98);
    assert_eq!(max_buy_count(&db, &inv, 1_000_000, POTION), Some(1));
    inv.add(POTION, 1);
    assert_eq!(max_buy_count(&db, &inv, 1_000_000, POTION), None);
    // Price 0 uses the stack cap, not a division.
    assert_eq!(
        max_buy_count(&db, &Inventory::default(), 0, GARNET_GEM),
        Some(99)
    );
}

#[test]
fn sell_pays_half_price_and_caps_gil() {
    let db = db();
    let mut inv = Inventory::default();
    inv.add(POTION, 4);
    let mut gil = 100;
    // Selling price is 50 / 2 = 25 each, 3 sold = 75.
    assert_eq!(sell(&db, &mut inv, &mut gil, POTION, 3), Ok(75));
    assert_eq!(gil, 175);
    assert_eq!(inv.count(POTION), 1);
    assert_eq!(
        sell(&db, &mut inv, &mut gil, POTION, 2),
        Err(TradeError::BadCount)
    );
    // 9,999,990 + 25 caps at 9,999,999.
    let mut gil = 9_999_990;
    assert_eq!(sell(&db, &mut inv, &mut gil, POTION, 1), Ok(25));
    assert_eq!(gil, GIL_MAX);
}

#[test]
fn unsellable_item_is_refused() {
    let mut db = db();
    db.items.get_mut(&POTION).unwrap().selling_price = -1;
    let mut inv = Inventory::default();
    inv.add(POTION, 1);
    let mut gil = 0;
    assert_eq!(
        sell(&db, &mut inv, &mut gil, POTION, 1),
        Err(TradeError::CannotSell)
    );
    assert_eq!(inv.count(POTION), 1);
}

#[test]
fn shop_kind_follows_the_item_types() {
    let (db, shops) = (db(), shops());
    let recipes = [butterfly()];
    assert_eq!(shop_kind(&db, &shops, &recipes, 0), ShopKind::Weapon);
    assert_eq!(shop_kind(&db, &shops, &recipes, 1), ShopKind::Item);
    // Items plus equipment: grocery, shown as a weapon list.
    assert_eq!(shop_kind(&db, &shops, &recipes, 2), ShopKind::Weapon);
    // Unknown shop id: synthesis only.
    assert_eq!(shop_kind(&db, &shops, &recipes, 32), ShopKind::Synthesis);
    // An item shop that also lists a recipe becomes a weapon list.
    let mut r = butterfly();
    r.shops = vec![3];
    assert_eq!(shop_kind(&db, &shops, &[r], 3), ShopKind::Weapon);
}

#[test]
fn synthesis_pays_and_consumes_ingredients() {
    let recipes = [butterfly()];
    let mut inv = Inventory::default();
    inv.add(DAGGER, 2);
    inv.add(MAGE_MASHER, 1);
    let mut gil = 1000;
    // One craft: needs Dagger and Mage Masher, costs 300.
    assert_eq!(max_synth_count(&inv, gil, &recipes[0]), Some(1));
    assert_eq!(
        synthesize(&mut inv, &mut gil, &recipes, 32, 0, 2),
        Err(TradeError::BadCount)
    );
    assert_eq!(synthesize(&mut inv, &mut gil, &recipes, 32, 0, 1), Ok(1));
    assert_eq!(gil, 700);
    assert_eq!(inv.count(7), 1);
    assert_eq!(inv.count(DAGGER), 1);
    assert_eq!(inv.count(MAGE_MASHER), 0);
    // Not offered at shop 36, and the ingredients are gone now.
    assert_eq!(
        synthesize(&mut inv, &mut gil, &recipes, 36, 0, 1),
        Err(TradeError::UnknownRecipe)
    );
    assert!(!can_synthesize(&inv, gil, &recipes[0]));
}

#[test]
fn synthesis_count_uses_gil_stack_and_ingredients() {
    let r = butterfly();
    let mut inv = Inventory::default();
    inv.add(DAGGER, 5);
    inv.add(MAGE_MASHER, 4);
    // gil 1000 / 300 = 3; ingredients allow 4; stack room 99.
    assert_eq!(max_synth_count(&inv, 1000, &r), Some(3));
    // Gil below the price disables the row.
    assert_eq!(max_synth_count(&inv, 299, &r), None);
    inv.add(7, 98);
    assert_eq!(max_synth_count(&inv, 1000, &r), Some(1));
    inv.add(7, 1);
    assert_eq!(max_synth_count(&inv, 1000, &r), None);
}

#[test]
fn repeated_ingredient_is_removed_once_per_entry() {
    let r = Recipe {
        id: 9,
        shops: vec![40],
        price: 0,
        result: 7,
        ingredients: vec![POTION, POTION, 255],
    };
    let mut inv = Inventory::default();
    inv.add(POTION, 5);
    let mut gil = 0;
    // Two potions per craft: 5 / 2 = 2 crafts.
    assert_eq!(max_synth_count(&inv, gil, &r), Some(2));
    assert_eq!(synthesize(&mut inv, &mut gil, &[r], 40, 9, 2), Ok(2));
    assert_eq!(inv.count(POTION), 1);
    assert_eq!(inv.count(7), 2);
}

#[test]
fn equip_masks_and_legality() {
    let db = db();
    assert_eq!(equip_mask_from_char_id(0), 0x800);
    assert_eq!(equip_mask_from_char_id(3), 0x100);
    assert_eq!(equip_mask_from_char_id(12), 1 << 12);
    let steiner = equip_mask_from_char_id(3);
    assert!(can_equip(&db, DAGGER, ZIDANE, 0));
    assert!(!can_equip(&db, DAGGER, steiner, 0));
    // Armlets are slot 2 (weapon, helmet, armlet, armor, accessory).
    assert_eq!(equip_part_of(&db, WRIST), Some(2));
    assert!(can_equip(&db, WRIST, ZIDANE, 2));
    assert!(!can_equip(&db, WRIST, ZIDANE, 1));
    assert!(!can_equip(&db, WRIST, steiner, 2));
    assert_eq!(equip_part_of(&db, POTION), None);
}

#[test]
fn equip_change_moves_items_and_changes_stats() {
    let db = db();
    let mut inv = Inventory::default();
    inv.add(WRIST, 1);
    inv.add(LEATHER_WRIST, 1);
    let mut eq: Equipment = [NO_ITEM; 5];
    let basis = [20, 30, 25, 10];
    let bare = stats(&db, basis, &eq);
    assert_eq!((bare.p_ev, bare.m_ev, bare.wpr), (0, 0, 10));

    equip_item(&db, &mut inv, &mut eq, ZIDANE, 2, WRIST).unwrap();
    assert_eq!(inv.count(WRIST), 0);
    let wrist = stats(&db, basis, &eq);
    assert_eq!((wrist.p_ev, wrist.m_ev, wrist.wpr), (5, 3, 10));

    equip_item(&db, &mut inv, &mut eq, ZIDANE, 2, LEATHER_WRIST).unwrap();
    // Wrist went back to the bag, Leather Wrist left it.
    assert_eq!(inv.count(WRIST), 1);
    assert_eq!(inv.count(LEATHER_WRIST), 0);
    let leather = stats(&db, basis, &eq);
    // Evade 5 to 7, magic evade 3 to 5, spirit +1 from bonus row 2.
    assert_eq!(
        (leather.p_ev - wrist.p_ev, leather.m_ev - wrist.m_ev),
        (2, 2)
    );
    assert_eq!(leather.wpr - wrist.wpr, 1);

    assert_eq!(unequip(&mut inv, &mut eq, 2), Ok(Some(LEATHER_WRIST)));
    assert_eq!(inv.count(LEATHER_WRIST), 1);
    assert_eq!(unequip(&mut inv, &mut eq, 0), Err(EquipError::WeaponSlot));
}

#[test]
fn equip_errors_leave_the_inventory_alone() {
    let db = db();
    let mut inv = Inventory::default();
    inv.add(DAGGER, 1);
    let mut eq: Equipment = [NO_ITEM; 5];
    let steiner = equip_mask_from_char_id(3);
    assert_eq!(
        equip_item(&db, &mut inv, &mut eq, steiner, 0, DAGGER),
        Err(EquipError::NotEquippable)
    );
    assert_eq!(
        equip_item(&db, &mut inv, &mut eq, ZIDANE, 0, MAGE_MASHER),
        Err(EquipError::NotInInventory)
    );
    assert_eq!(inv.count(DAGGER), 1);
}

#[test]
fn stats_clamp_to_the_maxima() {
    let db = db();
    let mut eq: Equipment = [NO_ITEM; 5];
    eq[2] = LEATHER_WRIST;
    // Spirit 50 + 1 caps at 50; dexterity 60 caps at 50.
    let s = stats(&db, [60, 99, 99, 50], &eq);
    assert_eq!((s.dex, s.str_, s.mgc, s.wpr), (50, 99, 99, 50));
}

#[test]
fn weapon_candidates_sort_by_quality_and_attack_follows_the_weapon() {
    let db = db();
    let mut inv = Inventory::default();
    inv.add(MAGE_MASHER, 1);
    inv.add(DAGGER, 1);
    inv.add(WRIST, 1);
    assert_eq!(
        equip_candidates(&db, &inv, ZIDANE, 0),
        vec![DAGGER, MAGE_MASHER]
    );
    let mut eq: Equipment = [NO_ITEM; 5];
    equip_item(&db, &mut inv, &mut eq, ZIDANE, 0, DAGGER).unwrap();
    assert_eq!(stats(&db, [0; 4], &eq).attack, 12);
    equip_item(&db, &mut inv, &mut eq, ZIDANE, 0, MAGE_MASHER).unwrap();
    assert_eq!(stats(&db, [0; 4], &eq).attack, 14);
    assert_eq!(inv.count(DAGGER), 1);
}

#[test]
fn equipment_ability_flags() {
    let db = db();
    let mut eq: Equipment = [NO_ITEM; 5];
    eq[2] = LEATHER_WRIST;
    eq[0] = DAGGER;
    // Support abilities only: Leather Wrist's 192 + 19.
    assert_eq!(equipped_support_abilities(&db, &eq), vec![19]);
    // Mage Masher teaches 102 and 101; the list holds 101 (20/20) and 102.
    let learnable = [(101, 20), (102, 25)];
    let ap = [20, 5];
    let l = learn_state(&db, MAGE_MASHER, &learnable, &ap);
    assert_eq!(l.len(), 2);
    assert_eq!(
        (l[0].ability, l[0].progress, l[0].mastered),
        (102, Some((5, 25)), false)
    );
    assert_eq!(
        (l[1].ability, l[1].progress, l[1].mastered),
        (101, Some((20, 20)), true)
    );
    // An ability the character does not have.
    let l = learn_state(&db, MAGE_MASHER, &[(101, 20)], &[0]);
    assert_eq!(l[0].progress, None);
}

#[test]
fn shop_comparison_uses_the_source_part_mapping() {
    let db = db();
    let mut eq: Equipment = [NO_ITEM; 5];
    eq[0] = DAGGER;
    // Weapon: old power 12, new 14.
    let c = shop_comparison(&db, MAGE_MASHER, ZIDANE, &eq).unwrap();
    assert_eq!(
        (c.part, c.old_rating, c.new_rating, c.can_equip),
        (0, 12, 14, true)
    );
    // Armlet part 2 compares physical evade: 5 now, 7 with Leather Wrist.
    eq[2] = WRIST;
    let c = shop_comparison(&db, LEATHER_WRIST, ZIDANE, &eq).unwrap();
    assert_eq!((c.part, c.old_rating, c.new_rating), (2, 5, 7));
    // Steiner cannot equip it.
    let c = shop_comparison(&db, LEATHER_WRIST, equip_mask_from_char_id(3), &eq).unwrap();
    assert!(!c.can_equip);
    assert_eq!(shop_comparison(&db, POTION, ZIDANE, &eq), None);
    assert_eq!(shop_defence(&db, 2, &eq), 5);
    assert_eq!(shop_defence(&db, 4, &eq), 0);
}

fn target(hp: u32, max_hp: u32, mp: u32, max_mp: u32) -> FieldTarget {
    FieldTarget {
        hp,
        max_hp,
        mp,
        max_mp,
        ..FieldTarget::default()
    }
}

fn potion_fx() -> FieldEffect {
    // ItemEffects 236 Potion: script 69, power 10.
    FieldEffect {
        script_id: 69,
        power: 10,
        status: 0,
    }
}

#[test]
fn potion_heals_power_times_ten_and_is_consumed() {
    let db = db();
    let mut inv = Inventory::default();
    inv.add(POTION, 2);
    let mut t = target(100, 300, 0, 0);
    // 10 power * attack 10 = 100.
    let r = use_item(&db, &mut inv, POTION, &potion_fx(), &mut t, || 0).unwrap();
    assert_eq!(
        r,
        ItemUseOutcome::Used {
            hp_gained: 100,
            mp_gained: 0
        }
    );
    assert_eq!(t.hp, 200);
    assert_eq!(inv.count(POTION), 1);
    // Clamped to max HP: only 100 more fits.
    let r = use_item(&db, &mut inv, POTION, &potion_fx(), &mut t, || 0).unwrap();
    assert_eq!(
        r,
        ItemUseOutcome::Used {
            hp_gained: 100,
            mp_gained: 0
        }
    );
    assert_eq!(inv.count(POTION), 0);
    assert_eq!(
        use_item(&db, &mut inv, POTION, &potion_fx(), &mut t, || 0),
        Err(FieldItemError::NotUsable)
    );
}

#[test]
fn potion_misses_on_full_dead_or_petrified_targets() {
    let db = db();
    let mut inv = Inventory::default();
    inv.add(POTION, 1);
    for mut t in [
        target(300, 300, 0, 0),
        target(0, 300, 0, 0),
        FieldTarget {
            status: 1, // Petrify
            ..target(10, 300, 0, 0)
        },
        FieldTarget {
            status: 1 << 6, // Zombie
            ..target(10, 300, 0, 0)
        },
    ] {
        let before = t;
        let r = use_item(&db, &mut inv, POTION, &potion_fx(), &mut t, || 0).unwrap();
        assert_eq!(r, ItemUseOutcome::NoEffect);
        assert_eq!(t, before);
        assert_eq!(inv.count(POTION), 1);
    }
}

#[test]
fn ether_elixir_tent_and_phoenix() {
    let db = db();
    let mut inv = Inventory::default();
    inv.add(POTION, 8);
    let fx = |script_id, power, status| FieldEffect {
        script_id,
        power,
        status,
    };
    // Ether (script 70, power 10): +100 MP, capped at max 150 -> +50 from 100.
    let mut t = target(50, 100, 100, 150);
    assert_eq!(
        use_item(&db, &mut inv, POTION, &fx(70, 10, 0), &mut t, || 0),
        Ok(ItemUseOutcome::Used {
            hp_gained: 0,
            mp_gained: 50
        })
    );
    // Elixir (71): full HP and MP.
    let mut t = target(1, 400, 2, 120);
    assert_eq!(
        use_item(&db, &mut inv, POTION, &fx(71, 0, 0), &mut t, || 0),
        Ok(ItemUseOutcome::Used {
            hp_gained: 399,
            mp_gained: 118
        })
    );
    // Tent (76): half of max, 301 >> 1 = 150 and 99 >> 1 = 49.
    let mut t = target(10, 301, 0, 99);
    assert_eq!(
        use_item(&db, &mut inv, POTION, &fx(76, 0, 0), &mut t, || 0),
        Ok(ItemUseOutcome::Used {
            hp_gained: 150,
            mp_gained: 49
        })
    );
    // Phoenix (72): dead target, random8 = 37 -> 1 + 37 % 10 = 8.
    let mut t = target(0, 300, 0, 0);
    assert_eq!(
        use_item(&db, &mut inv, POTION, &fx(72, 0, 0), &mut t, || 37),
        Ok(ItemUseOutcome::Used {
            hp_gained: 8,
            mp_gained: 0
        })
    );
    // Phoenix on a living target misses and draws nothing.
    let mut t = target(5, 300, 0, 0);
    assert_eq!(
        use_item(&db, &mut inv, POTION, &fx(72, 0, 0), &mut t, || panic!(
            "no draw"
        )),
        Ok(ItemUseOutcome::NoEffect)
    );
    assert_eq!(inv.count(POTION), 4);
}

#[test]
fn gem_heals_power_times_held_count() {
    let db = db();
    let mut inv = Inventory::default();
    inv.add(GARNET_GEM, 3);
    // ItemEffects 224 Garnet: script 74, power 12. Count is read before the
    // item is consumed: 12 * 3 = 36.
    let fx = FieldEffect {
        script_id: 74,
        power: 12,
        status: 0,
    };
    let mut t = target(10, 300, 0, 0);
    assert_eq!(
        use_item(&db, &mut inv, GARNET_GEM, &fx, &mut t, || 0),
        Ok(ItemUseOutcome::Used {
            hp_gained: 36,
            mp_gained: 0
        })
    );
    assert_eq!(inv.count(GARNET_GEM), 2);
}

#[test]
fn cure_items_only_clear_out_of_battle_statuses() {
    let db = db();
    let mut inv = Inventory::default();
    inv.add(POTION, 5);
    // Antidote: script 73, Venom (bit 1) and Poison (bit 16); Poison is not
    // an out-of-battle status, so only Venom counts.
    let fx = FieldEffect {
        script_id: 73,
        power: 0,
        status: (1 << 1) | (1 << 16),
    };
    let mut t = FieldTarget {
        status: (1 << 1) | (1 << 16),
        ..target(10, 100, 0, 0)
    };
    assert_eq!(
        use_item(&db, &mut inv, POTION, &fx, &mut t, || 0),
        Ok(ItemUseOutcome::Used {
            hp_gained: 0,
            mp_gained: 0
        })
    );
    assert_eq!(t.status, 1 << 16);
    // Nothing to cure: miss, item kept.
    assert_eq!(
        use_item(&db, &mut inv, POTION, &fx, &mut t, || 0),
        Ok(ItemUseOutcome::NoEffect)
    );
    // Permanent status cannot be removed.
    let mut t = FieldTarget {
        status: 1 << 1,
        permanent_status: 1 << 1,
        ..target(10, 100, 0, 0)
    };
    assert_eq!(
        use_item(&db, &mut inv, POTION, &fx, &mut t, || 0),
        Ok(ItemUseOutcome::NoEffect)
    );
    assert_eq!(inv.count(POTION), 4);
}

#[test]
fn shop_flags_expand_to_shop_ids() {
    assert_eq!(shops_from_flags(0b1000_0011), vec![32, 33, 39]);
    assert_eq!(default_selling_price(321), 160);
}

#[test]
fn unported_scripts_are_reported_not_guessed() {
    let db = db();
    let mut inv = Inventory::default();
    inv.add(POTION, 1);
    let mut t = target(1, 10, 0, 0);
    // Potion script with a status part needs the immunity table.
    let fx = FieldEffect {
        script_id: 69,
        power: 10,
        status: 1 << 5,
    };
    assert_eq!(
        use_item(&db, &mut inv, POTION, &fx, &mut t, || 0),
        Err(FieldItemError::ScriptNotPorted(69))
    );
}
