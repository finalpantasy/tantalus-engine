//! Field item use bridge: the legacy field calculator heals (or misses) a
//! party member through field world state, consuming the item only on use.

use tantalus_game::FieldWorld;
use tantalus_game::field::CharacterStats;
use tantalus_game::field::item_use::{FieldItemUseError, use_field_item};
use tantalus_legacy::shop::{FieldEffect, FieldItemError, ItemData, ItemDb, ItemUseOutcome};

fn usable_item() -> ItemData {
    ItemData {
        price: 50,
        selling_price: 25,
        equip: 0,
        kind: tantalus_legacy::shop::data::item_type::USABLE,
        sort: 0.0,
        eq_lv: 0.0,
        bonus: 0,
        ability: vec![],
        weapon_power: None,
        armor: None,
    }
}

fn db() -> ItemDb {
    let mut db = ItemDb::default();
    db.items.insert(12, usable_item());
    db
}

fn world_with_character(hp: i32, max_hp: i32, items: &[(i32, i32)]) -> FieldWorld {
    let mut world = FieldWorld::new();
    world.character_stats.insert(
        0,
        CharacterStats {
            current_hp: hp,
            max_hp,
            current_mp: 20,
            max_mp: 36,
        },
    );
    for &(item, count) in items {
        world.add_item_amount(item, count);
    }
    world
}

#[test]
fn potion_heals_and_consumes_through_the_world() {
    let db = db();
    let effect = FieldEffect {
        script_id: 69,
        power: 5,
        status: 0,
    };
    let mut world = world_with_character(100, 250, &[(12, 2)]);
    let outcome = use_field_item(&mut world, &db, 12, &effect, 0, || 0).unwrap();
    assert_eq!(
        outcome,
        ItemUseOutcome::Used {
            hp_gained: 50,
            mp_gained: 0
        }
    );
    assert_eq!(world.items[&12], 1, "one unit consumed");
    assert_eq!(world.character_stats[&0].current_hp, 150);
}

#[test]
fn full_hp_is_a_miss_and_consumes_nothing() {
    let db = db();
    let effect = FieldEffect {
        script_id: 69,
        power: 5,
        status: 0,
    };
    let mut world = world_with_character(250, 250, &[(12, 2)]);
    let outcome = use_field_item(&mut world, &db, 12, &effect, 0, || 0).unwrap();
    assert_eq!(outcome, ItemUseOutcome::NoEffect);
    assert_eq!(world.items[&12], 2, "miss consumes nothing");
    assert_eq!(world.character_stats[&0].current_hp, 250);
}

#[test]
fn not_usable_and_unknown_character_fail_without_changes() {
    let db = db();
    let effect = FieldEffect {
        script_id: 69,
        power: 5,
        status: 0,
    };
    let mut world = world_with_character(100, 250, &[(12, 2)]);
    let error = use_field_item(&mut world, &db, 30, &effect, 0, || 0).unwrap_err();
    assert_eq!(error, FieldItemUseError::Field(FieldItemError::UnknownItem));
    let error = use_field_item(&mut world, &db, 12, &effect, 9, || 0).unwrap_err();
    assert_eq!(error, FieldItemUseError::UnknownCharacter);
    assert_eq!(world.items[&12], 2);
    assert_eq!(world.character_stats[&0].current_hp, 100);
}
