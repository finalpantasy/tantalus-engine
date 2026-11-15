//! Plain data the rules run on.

use std::collections::BTreeMap;

/// `ItemType` bits (`Memoria/Data/Items/ItemInfo.cs:171-185`).
pub mod item_type {
    pub const WEAPON: u8 = 128;
    pub const ARMLET: u8 = 64;
    pub const HELMET: u8 = 32;
    pub const ARMOR: u8 = 16;
    pub const ACCESSORY: u8 = 8;
    pub const ITEM: u8 = 4;
    pub const GEM: u8 = 2;
    pub const USABLE: u8 = 1;
    pub const ANY_EQUIPMENT: u8 = WEAPON | ARMLET | HELMET | ARMOR | ACCESSORY;
    /// `ff9buy.FF9BUY_TYPE_WEAPON` (`Memoria/Data/ff9buy.cs:14`).
    pub const BUY_WEAPON: u8 = WEAPON | ARMLET | HELMET | ARMOR;
}

/// `ItemDefence` values for an armor piece (`ff9armor.ArmorData`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ArmorData {
    pub p_def: i32,
    pub p_ev: i32,
    pub m_def: i32,
    pub m_ev: i32,
}

/// `ItemStats` row used by `ff9play.FF9Play_Update` (`Global/ff9play.cs:290-295`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StatBonus {
    pub dex: i32,
    pub str_: i32,
    pub mgc: i32,
    pub wpr: i32,
}

/// `FF9ITEM_DATA` (`Global/ff9/FF9ITEM_DATA.cs`) reduced to what the rules use.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemData {
    pub price: u32,
    /// Negative means the item cannot be sold (`Global/ShopUI.cs:365`, `:440`).
    pub selling_price: i32,
    /// Character equip mask, bit `11 - id` for ids 0..=11 (`Global/ff9feqp.cs:10`).
    pub equip: u64,
    /// `ItemType` bits, see [`item_type`].
    pub kind: u8,
    /// `FF9ITEM_DATA.sort` (inventory arrange order).
    pub sort: f32,
    /// `FF9ITEM_DATA.eq_lv` (equip list order).
    pub eq_lv: f32,
    /// Index into [`ItemDb::bonuses`].
    pub bonus: usize,
    /// Ability ids (0 = none; >= 192 support; see `Memoria/Data/ff9abil.cs:316-345`).
    pub ability: Vec<i32>,
    /// `weapon_id >= 0` and its `Ref.Power`.
    pub weapon_power: Option<i32>,
    /// `armor_id >= 0` and its defence row.
    pub armor: Option<ArmorData>,
}

/// Memoria's default when no explicit selling price column exists:
/// `price / 2` (`Memoria/Data/Items/ItemInfo.cs:54-55`). The pinned stock
/// `Items.csv` carries an explicit column that equals `price / 2` on all 256
/// rows (checked by hand); the installed tables carry no selling price.
pub fn default_selling_price(price: u32) -> i32 {
    (price / 2) as i32
}

/// Item and equipment tables.
#[derive(Debug, Clone, Default)]
pub struct ItemDb {
    pub items: BTreeMap<u16, ItemData>,
    pub bonuses: Vec<StatBonus>,
}

impl ItemDb {
    pub fn get(&self, id: u16) -> Option<&ItemData> {
        self.items.get(&id)
    }
}

/// `ff9buy.ShopItems`: shop id to item ids in display order.
#[derive(Debug, Clone, Default)]
pub struct ShopTable {
    pub shops: BTreeMap<i32, Vec<u16>>,
}

/// `FF9MIX_DATA` (`FF9/FF9MIX_DATA.cs`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipe {
    pub id: i32,
    pub shops: Vec<i32>,
    pub price: u32,
    /// `NoItem` (255) means the row is not listed (`Global/ShopUI.cs:1148`).
    pub result: u16,
    /// Entries equal to `NoItem` are skipped (`FF9MIX_DATA.cs:76-85`).
    pub ingredients: Vec<u16>,
}

/// Byte shop flags to shop ids: bit `i` is shop `32 + i`
/// (`FF9/FF9MIX_DATA.cs:41-49`).
pub fn shops_from_flags(flags: u8) -> Vec<i32> {
    (0..8)
        .filter(|i| flags & (1 << i) != 0)
        .map(|i| 32 + i)
        .collect()
}

impl Recipe {
    /// `IngredientsAsDictionary` (`FF9MIX_DATA.cs:76-90`), ordered by item id.
    pub fn ingredient_counts(&self) -> BTreeMap<u16, i32> {
        let mut m = BTreeMap::new();
        for &i in &self.ingredients {
            if i != crate::shop::NO_ITEM {
                *m.entry(i).or_insert(0) += 1;
            }
        }
        m
    }
}
