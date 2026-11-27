//! The shop screen a field script opens with `MENU(2, shop)` (`ShopUI`,
//! `Global/ShopUI.cs`): Buy and Sell, the equipment comparison for the
//! party, and the synthesis list. The rules are the legacy trade functions
//! run through the field shop bridge; this file is the state machine and the
//! frame the window draws.

use std::path::Path;

use tantalus_game::FieldWorld;
use tantalus_game::field::shop::{inventory_from_world, with_shop_inventory};
use tantalus_legacy::shop::{
    Recipe, ShopKind, ShopTable, buy, can_synthesize, equip_mask_from_char_id, item_enabled,
    max_buy_count, max_sell_count, max_synth_count, sell, shop_comparison, shop_kind,
    synthesis_list, synthesize,
};

use crate::field_menu::{MenuFrame, MenuTables, MenuText, ROW, Tint, step, window};
use crate::play::pad;

/// Rows of the list window.
const LIST_ROWS: usize = 8;
const COMMANDS: [&str; 3] = ["Buy", "Sell", "Exit"];

/// Shop assortments and synthesis recipes (`ShopItems.csv`, `Synthesis.csv`).
#[derive(Debug, Clone, Default)]
pub struct ShopTables {
    pub shops: ShopTable,
    pub recipes: Vec<Recipe>,
}

fn rows(text: &str) -> impl Iterator<Item = Vec<&str>> {
    text.lines()
        .map(|line| line.trim_start_matches('\u{feff}'))
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|line| line.split(';').collect())
}

fn ints(text: &str) -> Vec<i32> {
    text.split(',')
        .filter_map(|part| part.trim().parse().ok())
        .collect()
}

impl ShopTables {
    /// Reads the tables under the Memoria data root.
    pub fn load(memoria: &Path) -> Result<Self, String> {
        let items = memoria.join("Memoria.Patcher/StreamingAssets/Data/Items");
        let read = |name: &str| {
            std::fs::read(items.join(name))
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .map_err(|error| format!("cannot read {name}: {error}"))
        };
        let mut tables = Self::default();
        for row in rows(&read("ShopItems.csv")?) {
            let Some(id) = row.get(1).and_then(|id| id.trim().parse::<i32>().ok()) else {
                continue;
            };
            let list = ints(row.get(2).copied().unwrap_or(""))
                .into_iter()
                .filter_map(|item| u16::try_from(item).ok())
                .collect();
            tables.shops.shops.insert(id, list);
        }
        for row in rows(&read("Synthesis.csv")?) {
            let number = |index: usize| row.get(index).and_then(|v| v.trim().parse::<i64>().ok());
            let (Some(id), Some(price), Some(result)) = (number(1), number(3), number(4)) else {
                continue;
            };
            tables.recipes.push(Recipe {
                id: id as i32,
                shops: ints(row.get(2).copied().unwrap_or("")),
                price: price.max(0) as u32,
                result: u16::try_from(result).unwrap_or(255),
                ingredients: ints(row.get(5).copied().unwrap_or(""))
                    .into_iter()
                    .filter_map(|item| u16::try_from(item).ok())
                    .collect(),
            });
        }
        Ok(tables)
    }
}

/// One line of the buy list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry {
    item: u16,
    price: u32,
    /// `Some` for a synthesis row.
    recipe: Option<i32>,
    enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Command(usize),
    Buy(usize),
    Sell(usize),
    /// The quantity dialog over the list the player came from.
    Count {
        sell: bool,
        row: usize,
        count: i32,
        max: i32,
    },
}

/// Party members the comparison shows at once (`charInfoHud`).
const COMPARE_WIDTH: usize = 4;

/// The open shop.
#[derive(Debug, Clone)]
pub struct ShopMenu {
    id: i32,
    kind: ShopKind,
    screen: Screen,
    /// First party member shown in the comparison (`availableCharaStart`).
    chara_start: usize,
    /// Counts presses; a client redraws when it changes.
    pub revision: u64,
    /// Set when the last press was refused (the game plays the refusal sound).
    pub refused: bool,
}

impl ShopMenu {
    /// Opens shop `id` (`ShopUI.Show`): synthesis shops go straight to the
    /// list, the others start on the Buy / Sell / Exit command.
    pub fn new(id: i32, tables: &MenuTables, shops: &ShopTables) -> Self {
        let kind = shop_kind(&tables.db, &shops.shops, &shops.recipes, id);
        Self {
            id,
            kind,
            screen: if kind == ShopKind::Synthesis {
                Screen::Buy(0)
            } else {
                Screen::Command(0)
            },
            chara_start: 0,
            revision: 0,
            refused: false,
        }
    }

    /// The buy list: the shop's goods, then its recipes.
    fn entries(&self, world: &FieldWorld, tables: &MenuTables, shops: &ShopTables) -> Vec<Entry> {
        let inventory = inventory_from_world(world);
        let mut list = Vec::new();
        if self.kind != ShopKind::Synthesis {
            for &item in shops.shops.shops.get(&self.id).into_iter().flatten() {
                let price = tables.db.get(item).map_or(0, |data| data.price);
                list.push(Entry {
                    item,
                    price,
                    recipe: None,
                    enabled: item_enabled(&tables.db, &inventory, world.gil, item),
                });
            }
        }
        if self.kind != ShopKind::Item {
            for recipe in synthesis_list(&shops.recipes, self.id) {
                list.push(Entry {
                    item: recipe.result,
                    price: recipe.price,
                    recipe: Some(recipe.id),
                    enabled: can_synthesize(&inventory, world.gil, recipe),
                });
            }
        }
        list
    }

    /// What the player holds, in inventory order, with whether it sells.
    fn sell_rows(&self, world: &FieldWorld, tables: &MenuTables) -> Vec<(u16, i32, bool)> {
        inventory_from_world(world)
            .held()
            .map(|slot| {
                let sells = tables
                    .db
                    .get(slot.id)
                    .is_some_and(|data| data.selling_price >= 0);
                (slot.id, i32::from(slot.count), sells)
            })
            .collect()
    }

    /// One pad press (edge bits). False when the shop closed.
    pub fn press(
        &mut self,
        pressed: i32,
        world: &mut FieldWorld,
        tables: &MenuTables,
        shops: &ShopTables,
    ) -> bool {
        self.revision += 1;
        let confirm = pressed & (pad::CONFIRM | pad::CIRCLE) != 0;
        let cancel = pressed & (pad::CANCEL | pad::CROSS) != 0;
        // `ShopUI.OnKeyLeftTrigger` / `OnKeyRightTrigger`: L1 and R1 slide the
        // four-wide window over the party in the comparison.
        if matches!(self.screen, Screen::Buy(_)) {
            if pressed & pad::L1 != 0 && self.chara_start > 0 {
                self.chara_start -= 1;
            } else if pressed & pad::R1 != 0 && self.chara_start + COMPARE_WIDTH < world.party.len()
            {
                self.chara_start += 1;
            }
        }
        match self.screen {
            Screen::Command(cursor) => {
                let cursor = step(cursor, COMMANDS.len(), pressed);
                self.screen = Screen::Command(cursor);
                if cancel {
                    return false;
                }
                if confirm {
                    match cursor {
                        0 => self.screen = Screen::Buy(0),
                        1 if !self.sell_rows(world, tables).is_empty() => {
                            self.screen = Screen::Sell(0);
                        }
                        1 => self.refused = true,
                        _ => return false,
                    }
                }
            }
            Screen::Buy(cursor) => {
                let entries = self.entries(world, tables, shops);
                let cursor = step(cursor, entries.len(), pressed);
                self.screen = Screen::Buy(cursor);
                if cancel {
                    if self.kind == ShopKind::Synthesis {
                        return false;
                    }
                    self.screen = Screen::Command(0);
                } else if confirm {
                    let max = entries.get(cursor).and_then(|entry| {
                        let inventory = inventory_from_world(world);
                        match entry.recipe {
                            Some(id) => shops
                                .recipes
                                .iter()
                                .find(|recipe| recipe.id == id)
                                .and_then(|recipe| max_synth_count(&inventory, world.gil, recipe)),
                            None => max_buy_count(&tables.db, &inventory, world.gil, entry.item),
                        }
                    });
                    match max {
                        Some(max) if max > 0 => {
                            self.screen = Screen::Count {
                                sell: false,
                                row: cursor,
                                count: 1,
                                max,
                            };
                        }
                        _ => self.refused = true,
                    }
                }
            }
            Screen::Sell(cursor) => {
                let rows = self.sell_rows(world, tables);
                let cursor = step(cursor, rows.len(), pressed);
                self.screen = Screen::Sell(cursor);
                if cancel || rows.is_empty() {
                    self.screen = Screen::Command(1);
                } else if confirm {
                    let (item, _, sells) = rows[cursor];
                    let max = max_sell_count(&inventory_from_world(world), item);
                    if sells && max > 0 {
                        self.screen = Screen::Count {
                            sell: true,
                            row: cursor,
                            count: 1,
                            max,
                        };
                    } else {
                        self.refused = true;
                    }
                }
            }
            Screen::Count {
                sell: selling,
                row,
                count,
                max,
            } => {
                // Left and right change the count by 1, down and up by 10.
                let mut count = count;
                if pressed & pad::RIGHT != 0 {
                    count += 1;
                } else if pressed & pad::LEFT != 0 {
                    count -= 1;
                } else if pressed & pad::UP != 0 {
                    count += 10;
                } else if pressed & pad::DOWN != 0 {
                    count -= 10;
                }
                let count = count.clamp(1, max);
                let list = if selling {
                    Screen::Sell(row)
                } else {
                    Screen::Buy(row)
                };
                if cancel {
                    self.screen = list;
                } else if confirm {
                    if self.finish(selling, row, count, world, tables, shops) {
                        self.screen = list;
                    } else {
                        self.refused = true;
                    }
                } else {
                    self.screen = Screen::Count {
                        sell: selling,
                        row,
                        count,
                        max,
                    };
                }
            }
        }
        true
    }

    /// Carries out the trade the quantity dialog confirmed.
    fn finish(
        &self,
        selling: bool,
        row: usize,
        count: i32,
        world: &mut FieldWorld,
        tables: &MenuTables,
        shops: &ShopTables,
    ) -> bool {
        if selling {
            let Some(&(item, _, _)) = self.sell_rows(world, tables).get(row) else {
                return false;
            };
            return with_shop_inventory(world, |inventory, gil| {
                sell(&tables.db, inventory, gil, item, count)
            })
            .is_ok();
        }
        let Some(entry) = self.entries(world, tables, shops).get(row).copied() else {
            return false;
        };
        match entry.recipe {
            Some(recipe) => with_shop_inventory(world, |inventory, gil| {
                synthesize(inventory, gil, &shops.recipes, self.id, recipe, count)
            })
            .is_ok(),
            None => with_shop_inventory(world, |inventory, gil| {
                buy(
                    &tables.db,
                    &shops.shops,
                    inventory,
                    gil,
                    self.id,
                    entry.item,
                    count,
                )
            })
            .is_ok(),
        }
    }

    /// How many of `item` the party wears (`ff9item.FF9Item_GetEquipCount`).
    fn worn(world: &FieldWorld, item: u16) -> usize {
        world
            .party
            .iter()
            .map(|member| {
                world
                    .equipped(*member)
                    .iter()
                    .filter(|slot| **slot == Some(item))
                    .count()
            })
            .sum()
    }

    /// What to draw now.
    pub fn frame(&self, world: &FieldWorld, tables: &MenuTables, shops: &ShopTables) -> MenuFrame {
        let mut frame = MenuFrame::default();
        let name = |item: u16| {
            tables
                .item_names
                .get(&i32::from(item))
                .cloned()
                .unwrap_or_else(|| format!("Item {item}"))
        };
        let text = |x: f32, row: f32, value: String, tint: Tint, right: bool| MenuText {
            x,
            y: row * ROW,
            text: value,
            tint,
            right,
        };
        // The list the cursor is in, the item under it and the count open.
        let (selling, cursor, counting) = match self.screen {
            Screen::Command(_) => (false, None, None),
            Screen::Buy(cursor) => (false, Some(cursor), None),
            Screen::Sell(cursor) => (true, Some(cursor), None),
            Screen::Count {
                sell,
                row,
                count,
                max,
            } => (sell, Some(row), Some((count, max))),
        };
        let entries = self.entries(world, tables, shops);
        let held = self.sell_rows(world, tables);
        if self.kind != ShopKind::Synthesis {
            let command = match self.screen {
                Screen::Command(command) => Some(command),
                _ => None,
            };
            let texts = COMMANDS
                .iter()
                .enumerate()
                .map(|(index, label)| {
                    let live = command.is_some() || index == usize::from(selling);
                    let tint = if live { Tint::White } else { Tint::Dim };
                    text(12.0, index as f32, (*label).to_string(), tint, false)
                })
                .collect();
            frame.windows.push(window(8.0, 8.0, 76.0, 3.0, texts));
            if let Some(command) = command {
                frame.pointer = Some((10.0, 8.0 + 3.0 + (command as f32 + 0.5) * ROW));
            }
        }
        // The list.
        let list_y = 62.0;
        let total = if selling { held.len() } else { entries.len() };
        let top = cursor
            .map(|cursor| cursor.saturating_sub(LIST_ROWS - 1))
            .unwrap_or(0)
            .min(total.saturating_sub(LIST_ROWS));
        let mut texts = Vec::new();
        for (line, index) in (top..total.min(top + LIST_ROWS)).enumerate() {
            if selling {
                let (item, count, sells) = held[index];
                let tint = if sells { Tint::White } else { Tint::Dim };
                texts.push(text(12.0, line as f32, name(item), tint, false));
                texts.push(text(188.0, line as f32, count.to_string(), tint, true));
            } else {
                let entry = entries[index];
                let tint = if entry.enabled {
                    Tint::White
                } else {
                    Tint::Dim
                };
                texts.push(text(12.0, line as f32, name(entry.item), tint, false));
                texts.push(text(
                    188.0,
                    line as f32,
                    entry.price.to_string(),
                    tint,
                    true,
                ));
            }
        }
        frame
            .windows
            .push(window(8.0, list_y, 196.0, LIST_ROWS as f32, texts));
        if let Some(cursor) = cursor {
            let line = cursor - top;
            frame.pointer = Some((10.0, list_y + 3.0 + (line as f32 + 0.5) * ROW));
        }
        // Gil.
        frame.windows.push(window(
            92.0,
            8.0,
            112.0,
            1.0,
            vec![
                text(8.0, 0.0, "Gil".into(), Tint::Yellow, false),
                text(104.0, 0.0, world.gil.to_string(), Tint::White, true),
            ],
        ));
        // The item under the cursor: how many are held and worn, then the
        // party's comparison or the recipe.
        let under = cursor.and_then(|cursor| {
            if selling {
                held.get(cursor).map(|row| row.0)
            } else {
                entries.get(cursor).map(|entry| entry.item)
            }
        });
        let mut side = Vec::new();
        let mut rows_used = 0.0;
        if let Some(item) = under {
            let owned = inventory_from_world(world).count(item);
            side.push(text(8.0, 0.0, "Owned".into(), Tint::White, false));
            side.push(text(100.0, 0.0, owned.to_string(), Tint::White, true));
            side.push(text(8.0, 1.0, "Worn".into(), Tint::White, false));
            side.push(text(
                100.0,
                1.0,
                Self::worn(world, item).to_string(),
                Tint::White,
                true,
            ));
            rows_used = 2.0;
            if !selling {
                for member in world
                    .party
                    .iter()
                    .skip(self.chara_start)
                    .take(COMPARE_WIDTH)
                {
                    let character = tables.character(world, *member);
                    let mask = equip_mask_from_char_id(*member as u32);
                    let Some(compare) = shop_comparison(&tables.db, item, mask, &character.equip)
                    else {
                        break;
                    };
                    let who = crate::name_menu::display_name(world, *member);
                    let (shown, tint) = if !compare.can_equip {
                        ("--".to_string(), Tint::Dim)
                    } else if compare.hidden {
                        ("???".to_string(), Tint::White)
                    } else if compare.new_rating > compare.old_rating {
                        (
                            format!("+{}", compare.new_rating - compare.old_rating),
                            Tint::Green,
                        )
                    } else if compare.new_rating < compare.old_rating {
                        (
                            format!("-{}", compare.old_rating - compare.new_rating),
                            Tint::Red,
                        )
                    } else {
                        (compare.new_rating.to_string(), Tint::White)
                    };
                    let who_tint = if compare.can_equip {
                        Tint::White
                    } else {
                        Tint::Dim
                    };
                    side.push(text(8.0, rows_used, who, who_tint, false));
                    side.push(text(100.0, rows_used, shown, tint, true));
                    rows_used += 1.0;
                }
                if let Some(recipe) = cursor
                    .and_then(|cursor| entries.get(cursor))
                    .and_then(|entry| entry.recipe)
                    .and_then(|id| shops.recipes.iter().find(|recipe| recipe.id == id))
                {
                    let inventory = inventory_from_world(world);
                    for (ingredient, need) in recipe.ingredient_counts() {
                        let have = inventory.count(ingredient);
                        let tint = if have >= need { Tint::White } else { Tint::Red };
                        side.push(text(8.0, rows_used, name(ingredient), tint, false));
                        side.push(text(100.0, rows_used, format!("x{need}"), tint, true));
                        rows_used += 1.0;
                    }
                }
            }
        }
        if rows_used > 0.0 {
            frame
                .windows
                .push(window(208.0, 8.0, 108.0, rows_used, side));
        }
        // The quantity dialog.
        if let Some((count, max)) = counting {
            let (verb, unit) = match (selling, cursor) {
                (true, Some(cursor)) => {
                    let price = held
                        .get(cursor)
                        .and_then(|row| tables.db.get(row.0))
                        .map_or(0, |data| data.selling_price.max(0) as u32);
                    ("Sell", price)
                }
                (false, Some(cursor)) => (
                    if entries.get(cursor).is_some_and(|e| e.recipe.is_some()) {
                        "Mix"
                    } else {
                        "Buy"
                    },
                    entries.get(cursor).map_or(0, |entry| entry.price),
                ),
                _ => ("Buy", 0),
            };
            let label = under.map(name).unwrap_or_default();
            frame.windows.push(window(
                56.0,
                list_y + LIST_ROWS as f32 * ROW + 10.0,
                208.0,
                2.0,
                vec![
                    text(8.0, 0.0, format!("{verb} {label}"), Tint::White, false),
                    text(
                        8.0,
                        1.0,
                        format!("x{count:>2} (max {max})"),
                        Tint::Yellow,
                        false,
                    ),
                    text(
                        196.0,
                        1.0,
                        (unit * count as u32).to_string(),
                        Tint::White,
                        true,
                    ),
                ],
            ));
            frame.pointer = None;
        }
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tantalus_legacy::shop::data::item_type;
    use tantalus_legacy::shop::{ItemData, StatBonus};

    fn item(price: u32, kind: u8, equip: u64, power: Option<i32>) -> ItemData {
        ItemData {
            price,
            selling_price: (price / 2) as i32,
            equip,
            kind,
            sort: 0.0,
            eq_lv: 0.0,
            bonus: 0,
            ability: vec![],
            weapon_power: power,
            armor: None,
        }
    }

    /// Shop 1 sells a dagger (id 1) and a mage rod (id 2); shop 32 mixes a
    /// sword (id 7) from a dagger and a rod; item 236 is a potion.
    fn setup() -> (MenuTables, ShopTables, FieldWorld) {
        let mut tables = MenuTables::default();
        tables.db.bonuses.push(StatBonus::default());
        tables
            .db
            .items
            .insert(1, item(100, item_type::WEAPON, 1 << 11, Some(12)));
        tables
            .db
            .items
            .insert(2, item(300, item_type::WEAPON, 1 << 10, Some(30)));
        tables
            .db
            .items
            .insert(7, item(0, item_type::WEAPON, 1 << 11, Some(20)));
        tables
            .db
            .items
            .insert(236, item(50, item_type::ITEM | item_type::USABLE, 0, None));
        tables.item_names.insert(1, "Dagger".into());
        let mut shops = ShopTables::default();
        shops.shops.shops.insert(1, vec![1, 2, 236]);
        shops.recipes.push(Recipe {
            id: 0,
            shops: vec![1, 32],
            price: 40,
            result: 7,
            ingredients: vec![1, 236],
        });
        let mut world = FieldWorld::new();
        world.party = vec![0];
        world.gil = 1000;
        (tables, shops, world)
    }

    #[test]
    fn buying_takes_gil_and_adds_the_item() {
        let (tables, shops, mut world) = setup();
        let mut shop = ShopMenu::new(1, &tables, &shops);
        // Buy, the dagger, then three of them.
        shop.press(pad::CONFIRM, &mut world, &tables, &shops);
        shop.press(pad::CONFIRM, &mut world, &tables, &shops);
        shop.press(pad::RIGHT, &mut world, &tables, &shops);
        shop.press(pad::RIGHT, &mut world, &tables, &shops);
        let frame = shop.frame(&world, &tables, &shops);
        assert!(
            frame
                .windows
                .iter()
                .flat_map(|window| &window.texts)
                .any(|text| text.text == "x 3 (max 10)")
        );
        shop.press(pad::CONFIRM, &mut world, &tables, &shops);
        assert_eq!(world.items.get(&1), Some(&3));
        assert_eq!(world.gil, 700);
        // Selling one back pays half.
        shop.press(pad::CANCEL, &mut world, &tables, &shops);
        shop.press(pad::DOWN, &mut world, &tables, &shops);
        shop.press(pad::CONFIRM, &mut world, &tables, &shops);
        shop.press(pad::CONFIRM, &mut world, &tables, &shops);
        shop.press(pad::CONFIRM, &mut world, &tables, &shops);
        assert_eq!(world.items.get(&1), Some(&2));
        assert_eq!(world.gil, 750);
    }

    #[test]
    fn l1_and_r1_slide_the_comparison_over_a_big_party() {
        let (tables, shops, mut world) = setup();
        world.party = vec![0, 1, 2, 3, 4];
        let mut shop = ShopMenu::new(1, &tables, &shops);
        shop.press(pad::CONFIRM, &mut world, &tables, &shops);
        let names = |shop: &ShopMenu, world: &FieldWorld| {
            let frame = shop.frame(world, &tables, &shops);
            let side = frame.windows.iter().find(|w| w.x == 208.0).unwrap();
            side.texts.iter().filter(|t| t.x == 8.0).count() - 2
        };
        assert_eq!(names(&shop, &world), 4);
        shop.press(pad::L1, &mut world, &tables, &shops);
        assert_eq!(shop.chara_start, 0, "L1 stops at the first member");
        shop.press(pad::R1, &mut world, &tables, &shops);
        assert_eq!(shop.chara_start, 1);
        shop.press(pad::R1, &mut world, &tables, &shops);
        assert_eq!(shop.chara_start, 1, "R1 stops when the last is shown");
        assert_eq!(names(&shop, &world), 4);
    }

    #[test]
    fn the_comparison_marks_better_and_worse_gear() {
        let (tables, shops, mut world) = setup();
        world.equip(0, 0, Some(1));
        let mut shop = ShopMenu::new(1, &tables, &shops);
        shop.press(pad::CONFIRM, &mut world, &tables, &shops);
        shop.press(pad::DOWN, &mut world, &tables, &shops);
        let frame = shop.frame(&world, &tables, &shops);
        let side = frame.windows.iter().find(|w| w.x == 208.0).unwrap();
        // The rod is for another character: dimmed.
        assert!(side.texts.iter().any(|t| t.text == "--"));
        shop.press(pad::UP, &mut world, &tables, &shops);
        let frame = shop.frame(&world, &tables, &shops);
        let side = frame.windows.iter().find(|w| w.x == 208.0).unwrap();
        assert!(side.texts.iter().any(|t| t.text == "12"));
    }

    #[test]
    fn a_recipe_uses_its_ingredients() {
        let (tables, shops, mut world) = setup();
        world.items.insert(1, 1);
        world.items.insert(236, 1);
        let mut shop = ShopMenu::new(1, &tables, &shops);
        shop.press(pad::CONFIRM, &mut world, &tables, &shops);
        // The recipe follows the three goods.
        for _ in 0..3 {
            shop.press(pad::DOWN, &mut world, &tables, &shops);
        }
        shop.press(pad::CONFIRM, &mut world, &tables, &shops);
        shop.press(pad::CONFIRM, &mut world, &tables, &shops);
        assert_eq!(world.items.get(&7), Some(&1));
        assert_eq!(world.items.get(&1), None);
        assert_eq!(world.gil, 960);
    }

    #[test]
    fn exit_and_cancel_close_the_shop() {
        let (tables, shops, mut world) = setup();
        let mut shop = ShopMenu::new(1, &tables, &shops);
        assert!(!shop.press(pad::CANCEL, &mut world, &tables, &shops));
        let mut shop = ShopMenu::new(1, &tables, &shops);
        shop.press(pad::UP, &mut world, &tables, &shops);
        assert!(!shop.press(pad::CONFIRM, &mut world, &tables, &shops));
    }
}
