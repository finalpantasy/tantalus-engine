//! Shop bridge: legacy trade rules run against field world state as one
//! transaction, committing to the field inventory only on success.

use tantalus_game::FieldWorld;
use tantalus_game::field::shop::{apply_inventory_to_world, with_shop_inventory};
use tantalus_legacy::shop::{ItemData, ItemDb, ShopTable, TradeError, trade};

fn potion() -> ItemData {
    ItemData {
        price: 50,
        selling_price: 25,
        equip: 0,
        kind: 0,
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
    db.items.insert(12, potion());
    db
}

fn shop_table() -> ShopTable {
    ShopTable {
        shops: std::collections::BTreeMap::from([(1, vec![12])]),
    }
}

#[test]
fn buy_commits_items_and_gil_into_the_world() {
    let db = db();
    let shops = shop_table();
    let mut world = FieldWorld::new();
    world.gil = 200;
    // A pre-existing stack the shop row does not touch.
    world.add_item_amount(30, 1);

    with_shop_inventory(&mut world, |inventory, gil| {
        trade::buy(&db, &shops, inventory, gil, 1, 12, 2)
    })
    .unwrap();

    assert_eq!(world.items[&12], 2, "bought potions land in field state");
    assert_eq!(world.items[&30], 1, "unrelated stacks survive");
    assert_eq!(world.gil, 100);
}

#[test]
fn failed_trade_leaves_the_world_unchanged() {
    let db = db();
    let shops = shop_table();
    let mut world = FieldWorld::new();
    world.gil = 200;
    world.add_item_amount(12, 1);

    // Item 30 is not sold by shop 1.
    let error = with_shop_inventory(&mut world, |inventory, gil| {
        trade::buy(&db, &shops, inventory, gil, 1, 30, 1)
    })
    .unwrap_err();
    assert_eq!(error, TradeError::NotSold);
    assert_eq!(world.items[&12], 1, "no partial commit");
    assert_eq!(world.gil, 200);
}

#[test]
fn sell_removes_the_stack_and_caps_gil() {
    let db = db();
    let mut world = FieldWorld::new();
    world.gil = 9_999_990;
    world.add_item_amount(12, 3);

    let gained = with_shop_inventory(&mut world, |inventory, gil| {
        trade::sell(&db, inventory, gil, 12, 3)
    })
    .unwrap();
    assert_eq!(gained, 75, "reported gain is pre-cap");
    assert_eq!(world.gil, 9_999_999, "reference gil cap");
    assert!(world.items.is_empty(), "emptied stack leaves the map");

    // The inventory view skips the NoItem sentinel and clamps to the cap.
    world.items.insert(255, 4);
    world.items.insert(12, 120);
    let inventory = tantalus_game::field::shop::inventory_from_world(&world);
    assert_eq!(inventory.count(12), 99);
    assert_eq!(inventory.count(255), 0);
    // Write-back restores a legal map.
    apply_inventory_to_world(&inventory, &mut world);
    assert_eq!(world.items[&12], 99);
    assert_eq!(world.items[&255], 4, "sentinel keys are not shop-managed");
}
