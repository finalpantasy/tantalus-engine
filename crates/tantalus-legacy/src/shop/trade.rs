//! Buying, selling and synthesis.

use super::data::{ItemDb, Recipe, ShopTable, item_type};
use super::inventory::Inventory;
use super::{GIL_MAX, ITEM_COUNT_MAX, NO_ITEM};

/// `ShopUI.ShopType` for the buy side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShopKind {
    /// Items only.
    Item,
    /// Equipment list, plus synthesis rows when recipes exist.
    Weapon,
    /// Synthesis only (shop id not in the shop table).
    Synthesis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeError {
    UnknownShop,
    UnknownItem,
    /// The item is not in this shop's list.
    NotSold,
    UnknownRecipe,
    /// Selling price is negative (`ShopUI.cs:365`, `:440`).
    CannotSell,
    /// Count outside `1..=max` (the quantity dialog's bounds,
    /// `ShopUI.cs:1156-1197`), or the row is disabled.
    BadCount,
}

/// `ff9buy.FF9Buy_GetType` (`Memoria/Data/ff9buy.cs:48-58`) combined with the
/// grocery rule in `ShopUI.InitializeData` (`Global/ShopUI.cs:240-264`): an
/// item shop that has recipes or any equipment is shown as a weapon list.
pub fn shop_kind(db: &ItemDb, shops: &ShopTable, recipes: &[Recipe], shop_id: i32) -> ShopKind {
    let Some(list) = shops.shops.get(&shop_id) else {
        return ShopKind::Synthesis;
    };
    let any_equip = list.iter().any(|i| {
        db.get(*i)
            .is_some_and(|d| d.kind & item_type::BUY_WEAPON != 0)
    });
    let any_non_equip = list.iter().any(|i| {
        db.get(*i)
            .is_some_and(|d| d.kind & item_type::BUY_WEAPON == 0)
    });
    if any_non_equip {
        // Item shop; becomes a weapon list when it also sells equipment or
        // has recipes (`ShopUI.cs:253-258`).
        if any_equip || !synthesis_list(recipes, shop_id).is_empty() {
            ShopKind::Weapon
        } else {
            ShopKind::Item
        }
    } else {
        ShopKind::Weapon
    }
}

/// Recipes listed in a shop (`ShopUI.InitializeMixList`, `ShopUI.cs:1146-1154`):
/// shop id in the recipe's set and a result other than `NoItem`, in table order.
pub fn synthesis_list(recipes: &[Recipe], shop_id: i32) -> Vec<&Recipe> {
    recipes
        .iter()
        .filter(|r| r.shops.contains(&shop_id) && r.result != NO_ITEM)
        .collect()
}

/// `StartCountItem` (`ShopUI.cs:1156-1170`): the largest purchasable count,
/// or `None` when the row is disabled (stack full or not enough gil, the
/// enable rule at `ShopUI.cs:772-775`).
pub fn max_buy_count(db: &ItemDb, inv: &Inventory, gil: u32, item: u16) -> Option<i32> {
    let d = db.get(item)?;
    let have = inv.count(item);
    if have < ITEM_COUNT_MAX && d.price <= gil {
        let afford = gil
            .checked_div(d.price)
            .map_or(ITEM_COUNT_MAX, |n| n as i32);
        Some((ITEM_COUNT_MAX - have).min(afford))
    } else {
        None
    }
}

/// Row enabled in the buy list (`ShopUI.cs:775`).
pub fn item_enabled(db: &ItemDb, inv: &Inventory, gil: u32, item: u16) -> bool {
    max_buy_count(db, inv, gil, item).is_some()
}

/// Buy `count` of `item` from shop `shop_id` (`ShopUI.OnKeyConfirm`,
/// `ShopUI.cs:386-413`): add to inventory, then subtract `price * added`.
/// Returns the count added. The quantity dialog bounds `count` to
/// `1..=max_buy_count`, enforced here. Stock is unlimited: the source has no
/// per-shop stock counter, only the 99 stack cap.
pub fn buy(
    db: &ItemDb,
    shops: &ShopTable,
    inv: &mut Inventory,
    gil: &mut u32,
    shop_id: i32,
    item: u16,
    count: i32,
) -> Result<i32, TradeError> {
    let list = shops.shops.get(&shop_id).ok_or(TradeError::UnknownShop)?;
    if !list.contains(&item) {
        return Err(TradeError::NotSold);
    }
    let d = db.get(item).ok_or(TradeError::UnknownItem)?;
    let max = max_buy_count(db, inv, *gil, item).ok_or(TradeError::BadCount)?;
    if !(1..=max).contains(&count) {
        return Err(TradeError::BadCount);
    }
    let added = inv.add(item, count);
    if added != 0 {
        *gil -= d.price * added as u32;
    }
    Ok(added)
}

/// `StartCountSell` (`ShopUI.cs:1189-1197`): maximum is the held count.
pub fn max_sell_count(inv: &Inventory, item: u16) -> i32 {
    inv.count(item)
}

/// Sell `count` of `item` (`ShopUI.cs:436-457`): remove from inventory, then
/// add `selling_price * count` to gil, capped at 9,999,999. Returns gil gained
/// before the cap is applied to the total. The sold-this-visit list
/// (`soldItemIdDict`) is UI state and not modeled.
pub fn sell(
    db: &ItemDb,
    inv: &mut Inventory,
    gil: &mut u32,
    item: u16,
    count: i32,
) -> Result<u32, TradeError> {
    let d = db.get(item).ok_or(TradeError::UnknownItem)?;
    if d.selling_price < 0 {
        return Err(TradeError::CannotSell);
    }
    if !(1..=max_sell_count(inv, item)).contains(&count) {
        return Err(TradeError::BadCount);
    }
    let removed = inv.remove(item, count);
    let mut gained = 0;
    if removed != 0 {
        gained = d.selling_price as u32 * count as u32;
        *gil = (*gil + gained).min(GIL_MAX);
    }
    Ok(gained)
}

/// `FF9MIX_DATA.CanBeSynthesized` (`FF9/FF9MIX_DATA.cs:92-97`).
pub fn can_synthesize(inv: &Inventory, gil: u32, r: &Recipe) -> bool {
    if inv.count(r.result) >= ITEM_COUNT_MAX || gil < r.price {
        return false;
    }
    r.ingredient_counts()
        .iter()
        .all(|(id, n)| inv.count(*id) >= *n)
}

/// `StartCountMix` (`ShopUI.cs:1172-1187`): largest craftable count, `None`
/// when the row is disabled.
pub fn max_synth_count(inv: &Inventory, gil: u32, r: &Recipe) -> Option<i32> {
    if !can_synthesize(inv, gil, r) {
        return None;
    }
    let afford = gil
        .checked_div(r.price)
        .map_or(ITEM_COUNT_MAX, |n| n as i32);
    let mut max = (ITEM_COUNT_MAX - inv.count(r.result)).min(afford);
    for (id, n) in r.ingredient_counts() {
        max = max.min(inv.count(id) / n);
    }
    Some(max)
}

/// Synthesize `count` of recipe `recipe_id` at shop `shop_id`
/// (`ShopUI.cs:414-433` Synthesis branch): add the result, then remove each
/// non-empty ingredient entry `added` times (duplicate entries remove twice),
/// then subtract `price * added` gil. Returns the count added.
pub fn synthesize(
    inv: &mut Inventory,
    gil: &mut u32,
    recipes: &[Recipe],
    shop_id: i32,
    recipe_id: i32,
    count: i32,
) -> Result<i32, TradeError> {
    let r = synthesis_list(recipes, shop_id)
        .into_iter()
        .find(|r| r.id == recipe_id)
        .ok_or(TradeError::UnknownRecipe)?;
    let max = max_synth_count(inv, *gil, r).ok_or(TradeError::BadCount)?;
    if !(1..=max).contains(&count) {
        return Err(TradeError::BadCount);
    }
    let added = inv.add(r.result, count);
    if added != 0 {
        for &i in r.ingredients.iter().filter(|&&i| i != NO_ITEM) {
            inv.remove(i, added);
        }
        *gil -= r.price * added as u32;
    }
    Ok(added)
}
