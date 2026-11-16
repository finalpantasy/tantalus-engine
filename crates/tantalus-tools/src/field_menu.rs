//! The FF9 field main menu: the state machine and the picture model.
//!
//! Start opens it when the script allows (`MENUOFF` turns it off, see
//! `FieldWorld::menu_disabled`) and the player has control. Items, Equip and
//! Status work; Abilities, Order, Card, Config and Save are listed but show a
//! notice. It reuses the legacy cores: `shop::use_item` through
//! `use_field_item`, `shop::equip_item` / `unequip`, and
//! `menu::equip::{equip_list, preview_stats}`.
//!
//! Stand-in: a character's Strength and Magic are the `BaseStats.csv` value
//! plus `level * 3 / 10` (the growth `new_game::build_stats` uses), Speed and
//! Spirit the base value, all plus the equipment bonuses; the real per-level
//! stat table (`ff9level`) is not applied.
//!
//! The model draws nothing. [`FieldMenu::frame`] returns windows in PSX
//! pixels (320x224) holding positioned text, which a client renders with the
//! game's font and window art.

use std::collections::BTreeMap;
use std::path::Path;

use tantalus_game::FieldWorld;
use tantalus_game::field::item_use::use_field_item;
use tantalus_game::field::shop::{inventory_from_world, with_shop_inventory};
use tantalus_game::new_game::BaseStats;
use tantalus_legacy::menu::MenuCharacter;
use tantalus_legacy::menu::equip::{PreviewChange, Trend, equip_list, preview_stats};
use tantalus_legacy::shop::data::item_type;
use tantalus_legacy::shop::{
    ArmorData, FieldEffect, ItemData, ItemDb, ItemUseOutcome, NO_ITEM, StatBonus, equip_item,
    stats, unequip,
};

use crate::ability_menu::{
    AbilityState, AbilityTables, Exit as AbilityExit, frame as ability_frame,
};
use crate::play::{character_name, pad};
use crate::play_save::SLOTS;
use crate::system_menu::{CardState, ConfigState, Exit as SystemExit, OrderState};

/// Row height in PSX pixels.
pub const ROW: f32 = 14.0;
const COMMANDS: [&str; 8] = [
    "Items",
    "Abilities",
    "Equip",
    "Status",
    "Order",
    "Card",
    "Config",
    "Save",
];
/// Commands that work.
const WORKING: [bool; 8] = [true, true, true, true, true, true, true, false];
const SLOT_NAMES: [&str; 5] = ["Weapon", "Helmet", "Armlet", "Armor", "Accessory"];
const STAT_NAMES: [&str; 9] = [
    "Speed", "Strength", "Magic", "Spirit", "Attack", "Defense", "Evade", "Mag Def", "Mag Eva",
];
const LIST_ROWS: usize = 9;
const PICK_ROWS: usize = 7;

/// Text color class; the client maps it to a color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tint {
    White,
    Dim,
    Yellow,
    Green,
    Red,
}

/// One piece of text, in PSX pixels from the window body's top left.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuText {
    pub x: f32,
    pub y: f32,
    pub text: String,
    pub tint: Tint,
    /// `x` is the right edge.
    pub right: bool,
}

/// A framed window in PSX pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuWindow {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub texts: Vec<MenuText>,
}

/// What to draw: windows, and the pointer position in PSX pixels.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MenuFrame {
    pub windows: Vec<MenuWindow>,
    pub pointer: Option<(f32, f32)>,
}

/// Tables the menu needs, loaded once.
#[derive(Debug, Clone, Default)]
pub struct MenuTables {
    pub db: ItemDb,
    /// Item id -> its field use effect.
    pub effects: BTreeMap<u16, FieldEffect>,
    /// Experience at which each level (1..=99) starts.
    pub exp: Vec<u32>,
    pub base: BTreeMap<i32, BaseStats>,
    pub item_names: BTreeMap<i32, String>,
    /// Character id -> default equipment (weapon, helmet, armlet, armor,
    /// accessory).
    pub default_equipment: BTreeMap<i32, [Option<u16>; 5]>,
    pub abilities: AbilityTables,
    /// Tetra Master card names by card id and collector class names by level
    /// (`ETC/minista.mes`, `ETC/card.mes`).
    pub card_names: BTreeMap<i32, String>,
    pub card_levels: BTreeMap<i32, String>,
}

fn rows(text: &str) -> impl Iterator<Item = Vec<&str>> {
    text.lines()
        .map(|line| line.trim_start_matches('\u{feff}'))
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|line| line.split(';').collect())
}

fn int(row: &[&str], index: usize) -> Option<i32> {
    row.get(index)?.trim().parse().ok()
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read(path)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .map_err(|error| format!("cannot read {}: {error}", path.display()))
}

/// `AA:x` is ability x, `SA:x` is support ability 192 + x.
fn abilities(text: &str) -> Vec<i32> {
    text.split(',')
        .filter_map(|part| {
            let (kind, id) = part.trim().split_once(':')?;
            let id: i32 = id.trim().parse().ok()?;
            match kind.trim() {
                "AA" => Some(id),
                "SA" => Some(id + 192),
                _ => None,
            }
        })
        .collect()
}

impl MenuTables {
    /// Reads the item and character tables under the Memoria data root.
    pub fn load(
        memoria: &Path,
        base: BTreeMap<i32, BaseStats>,
        exp: Vec<u32>,
        item_names: BTreeMap<i32, String>,
        active_names: BTreeMap<i32, String>,
        support_names: BTreeMap<i32, String>,
    ) -> Result<Self, String> {
        let data = memoria.join("Memoria.Patcher/StreamingAssets/Data");
        let items = data.join("Items");
        let characters = data.join("Characters");
        let bonus_text = read(&items.join("Stats.csv"))?;
        let mut bonuses: Vec<StatBonus> = Vec::new();
        for row in rows(&bonus_text) {
            let Some(id) = int(&row, 1).and_then(|id| usize::try_from(id).ok()) else {
                continue;
            };
            if bonuses.len() <= id {
                bonuses.resize(id + 1, StatBonus::default());
            }
            bonuses[id] = StatBonus {
                dex: int(&row, 2).unwrap_or(0),
                str_: int(&row, 3).unwrap_or(0),
                mgc: int(&row, 4).unwrap_or(0),
                wpr: int(&row, 5).unwrap_or(0),
            };
        }
        let weapon_text = read(&items.join("Weapons.csv"))?;
        let weapons: BTreeMap<i32, i32> = rows(&weapon_text)
            .filter_map(|row| Some((int(&row, 1)?, int(&row, 6)?)))
            .collect();
        let armor_text = read(&items.join("Armors.csv"))?;
        let armors: BTreeMap<i32, ArmorData> = rows(&armor_text)
            .filter_map(|row| {
                Some((
                    int(&row, 1)?,
                    ArmorData {
                        p_def: int(&row, 2)?,
                        p_ev: int(&row, 3)?,
                        m_def: int(&row, 4)?,
                        m_ev: int(&row, 5)?,
                    },
                ))
            })
            .collect();
        let effect_text = read(&items.join("ItemEffects.csv"))?;
        let effect_rows: BTreeMap<i32, FieldEffect> = rows(&effect_text)
            .filter_map(|row| {
                Some((
                    int(&row, 0)?,
                    FieldEffect {
                        script_id: u8::try_from(int(&row, 7)?).ok()?,
                        power: int(&row, 8)?,
                        status: 0,
                    },
                ))
            })
            .collect();
        let item_text = read(&items.join("Items.csv"))?;
        let mut db = ItemDb {
            bonuses,
            ..ItemDb::default()
        };
        let mut effects = BTreeMap::new();
        for row in rows(&item_text) {
            let (Some(id), Some(price)) = (int(&row, 0), int(&row, 4)) else {
                continue;
            };
            let Ok(id16) = u16::try_from(id) else {
                continue;
            };
            let mut kind = 0u8;
            for (bit, mask) in [128u8, 64, 32, 16, 8, 4, 2, 1].into_iter().enumerate() {
                if int(&row, 11 + bit) == Some(1) {
                    kind |= mask;
                }
            }
            let mut equip = 0u64;
            for character in 0..12usize {
                if int(&row, 20 + character) == Some(1) {
                    equip |= 1u64 << (11 - character);
                }
            }
            db.items.insert(
                id16,
                ItemData {
                    price: price.max(0) as u32,
                    selling_price: int(&row, 5).unwrap_or(0),
                    equip,
                    kind,
                    sort: int(&row, 19).unwrap_or(0) as f32,
                    eq_lv: int(&row, 8).unwrap_or(0) as f32,
                    bonus: int(&row, 9).unwrap_or(0).max(0) as usize,
                    ability: abilities(row.get(10).copied().unwrap_or("")),
                    weapon_power: int(&row, 1).and_then(|w| weapons.get(&w).copied()),
                    armor: int(&row, 2).and_then(|a| armors.get(&a).copied()),
                },
            );
            if let Some(effect) = int(&row, 3).and_then(|e| effect_rows.get(&e)) {
                effects.insert(id16, *effect);
            }
        }
        let parameters = read(&characters.join("CharacterParameters.csv"))?;
        let sets: BTreeMap<i32, i32> = rows(&parameters)
            .filter_map(|row| Some((int(&row, 0)?, int(&row, 5)?)))
            .collect();
        let equipment_text = read(&characters.join("DefaultEquipment.csv"))?;
        let set_items: BTreeMap<i32, [Option<u16>; 5]> = rows(&equipment_text)
            .filter_map(|row| {
                let mut slots = [None; 5];
                for (slot, value) in slots.iter_mut().enumerate() {
                    *value = int(&row, 2 + slot).and_then(|id| u16::try_from(id).ok());
                }
                Some((int(&row, 1)?, slots))
            })
            .collect();
        let default_equipment = sets
            .iter()
            .filter_map(|(character, set)| Some((*character, *set_items.get(set)?)))
            .collect();
        Ok(Self {
            db,
            effects,
            exp,
            base,
            item_names,
            default_equipment,
            abilities: AbilityTables::load(&data, active_names, support_names)?,
            card_names: BTreeMap::new(),
            card_levels: BTreeMap::new(),
        })
    }

    /// Level 1..=99 for a total experience.
    pub fn level(&self, exp: u64) -> usize {
        self.exp
            .iter()
            .take_while(|start| u64::from(**start) <= exp)
            .count()
            .clamp(1, 99)
    }

    fn item_name(&self, id: u16) -> String {
        self.item_names
            .get(&i32::from(id))
            .cloned()
            .unwrap_or_else(|| format!("Item {id}"))
    }

    /// Gives every character without recorded equipment the default set.
    pub fn seed_equipment(&self, world: &mut FieldWorld) {
        for (character, slots) in &self.default_equipment {
            world.equipment.entry(*character).or_insert(*slots);
        }
    }

    pub(crate) fn character(&self, world: &FieldWorld, id: i32) -> MenuCharacter {
        let mut character = MenuCharacter::new(id as u32);
        if let Some(base) = self.base.get(&id) {
            // `ff9level.FF9Level_GetDex/Str/Mgc/Wpr` with no level-up bonus.
            let level = self.level(world.character_exp.get(&id).copied().unwrap_or(0)) as i32;
            character.basis = [
                (i32::from(base.speed) + level / 10).min(50),
                (i32::from(base.strength) + level * 3 / 10).min(99),
                (i32::from(base.magic) + level * 3 / 10).min(99),
                (i32::from(base.spirit) + level * 3 / 20).min(50),
            ];
        }
        for (slot, item) in world.equipped(id).iter().enumerate() {
            character.equip[slot] = item.unwrap_or(NO_ITEM);
        }
        character
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Screen {
    Main,
    /// Choose the member for Equip, Status or Abilities.
    Member {
        pick: Pick,
        cursor: usize,
    },
    Items {
        cursor: usize,
    },
    ItemTarget {
        item: u16,
        list: usize,
        cursor: usize,
    },
    Equip {
        member: i32,
        slot: usize,
        /// Open item picker: entries (`None` is Off) and the cursor.
        picker: Option<(Vec<Option<u16>>, usize)>,
    },
    Status {
        member: i32,
    },
    Abilities(AbilityState),
    Order(OrderState),
    Cards(CardState),
    Config(ConfigState),
    /// The save (`save`) or load screen. `direct` ones were opened by a
    /// script or at start and close the menu on cancel.
    Slots {
        save: bool,
        cursor: usize,
        direct: bool,
    },
}

/// Which screen the member pick leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pick {
    Equip,
    Status,
    Abilities,
}

/// Something the menu asks the game to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    /// Write the game to this slot (1-based).
    Save(usize),
    /// Replace the game with this slot.
    Load(usize),
}

/// The open menu.
#[derive(Debug, Clone)]
pub struct FieldMenu {
    screen: Screen,
    main: usize,
    message: Option<String>,
    /// One line per save slot, filled by the game while a slot screen is up.
    pub slot_labels: Vec<String>,
    /// Set when `slot_labels` should be read again.
    pub slots_stale: bool,
    /// A save or load the game should carry out.
    pub action: Option<MenuAction>,
    /// Counts presses; a client redraws when it changes.
    pub revision: u64,
    /// Set when the last press was refused (the game plays the refusal sound).
    pub refused: bool,
}

pub(crate) fn step(cursor: usize, len: usize, pressed: i32) -> usize {
    if len == 0 {
        return 0;
    }
    if pressed & pad::UP != 0 {
        (cursor + len - 1) % len
    } else if pressed & pad::DOWN != 0 {
        (cursor + 1) % len
    } else {
        cursor.min(len - 1)
    }
}

/// Held item ids shown in the Items list, in inventory order.
fn item_rows(world: &FieldWorld, tables: &MenuTables) -> Vec<(u16, i32)> {
    let mut list: Vec<(u16, i32)> = world
        .items
        .iter()
        .filter_map(|(id, count)| {
            let id = u16::try_from(*id).ok()?;
            let data = tables.db.get(id)?;
            (*count > 0 && data.kind & (item_type::ITEM | item_type::USABLE) != 0)
                .then_some((id, *count))
        })
        .collect();
    list.sort_by(|a, b| {
        let key = |id: u16| tables.db.get(id).map_or(0.0, |data| data.sort);
        key(a.0).total_cmp(&key(b.0)).then(a.0.cmp(&b.0))
    });
    list
}

fn usable(tables: &MenuTables, id: u16) -> bool {
    tables
        .db
        .get(id)
        .is_some_and(|data| data.kind & item_type::USABLE != 0)
        && tables.effects.contains_key(&id)
}

fn text(x: f32, row: f32, value: impl Into<String>, tint: Tint) -> MenuText {
    MenuText {
        x,
        y: row * ROW,
        text: value.into(),
        tint,
        right: false,
    }
}

fn right(x: f32, row: f32, value: impl Into<String>, tint: Tint) -> MenuText {
    MenuText {
        right: true,
        ..text(x, row, value, tint)
    }
}

pub(crate) fn window(x: f32, y: f32, w: f32, rows: f32, texts: Vec<MenuText>) -> MenuWindow {
    MenuWindow {
        x,
        y,
        w,
        h: rows * ROW + 6.0,
        texts,
    }
}

impl FieldMenu {
    pub fn new() -> Self {
        Self {
            screen: Screen::Main,
            main: 0,
            message: None,
            refused: false,
            slot_labels: Vec::new(),
            slots_stale: false,
            action: None,
            revision: 0,
        }
    }

    /// The save screen a moogle or save point opens.
    pub fn save_screen() -> Self {
        Self {
            screen: Screen::Slots {
                save: true,
                cursor: 0,
                direct: true,
            },
            slots_stale: true,
            ..Self::new()
        }
    }

    /// The load screen.
    pub fn load_screen() -> Self {
        Self {
            screen: Screen::Slots {
                save: false,
                cursor: 0,
                direct: true,
            },
            slots_stale: true,
            ..Self::new()
        }
    }

    /// Shows a result line (the game reports a save here).
    pub fn say(&mut self, message: String) {
        self.message = Some(message);
        self.revision += 1;
    }

    /// One pad press (edge bits). Returns `false` when the menu closed.
    pub fn press(&mut self, pressed: i32, world: &mut FieldWorld, tables: &MenuTables) -> bool {
        self.revision += 1;
        let confirm = pressed & (pad::CONFIRM | pad::CIRCLE) != 0;
        let cancel = pressed & (pad::CANCEL | pad::CROSS) != 0;
        let party = world.party.len().min(4);
        match self.screen.clone() {
            Screen::Main => {
                self.main = step(self.main, COMMANDS.len(), pressed);
                if cancel || pressed & pad::START != 0 {
                    return false;
                }
                if confirm {
                    self.message = None;
                    match (self.main, WORKING[self.main]) {
                        (0, _) => self.screen = Screen::Items { cursor: 0 },
                        (1, _) if party > 0 => {
                            self.screen = Screen::Member {
                                pick: Pick::Abilities,
                                cursor: 0,
                            };
                        }
                        (7, _) => {
                            if world.save_point {
                                self.screen = Screen::Slots {
                                    save: true,
                                    cursor: 0,
                                    direct: false,
                                };
                                self.slots_stale = true;
                            } else {
                                self.refused = true;
                                self.message =
                                    Some("You can only save at a save point (a moogle).".into());
                            }
                        }
                        (2, _) if party > 0 => {
                            self.screen = Screen::Member {
                                pick: Pick::Equip,
                                cursor: 0,
                            };
                        }
                        (3, _) if party > 0 => {
                            self.screen = Screen::Member {
                                pick: Pick::Status,
                                cursor: 0,
                            };
                        }
                        (4, _) if party > 0 => self.screen = Screen::Order(OrderState::default()),
                        (5, _) => self.screen = Screen::Cards(CardState::default()),
                        (6, _) => self.screen = Screen::Config(ConfigState::default()),
                        (_, false) => {
                            self.refused = true;
                            self.message = Some(format!(
                                "{} is not available in this build yet.",
                                COMMANDS[self.main]
                            ));
                        }
                        _ => {}
                    }
                }
            }
            Screen::Member { pick, cursor } => {
                let cursor = step(cursor, party, pressed);
                self.screen = Screen::Member { pick, cursor };
                if cancel {
                    self.screen = Screen::Main;
                } else if confirm && let Some(member) = world.party.get(cursor).copied() {
                    self.screen = match pick {
                        Pick::Equip => Screen::Equip {
                            member,
                            slot: 0,
                            picker: None,
                        },
                        Pick::Status => Screen::Status { member },
                        Pick::Abilities => Screen::Abilities(AbilityState::new(member)),
                    };
                }
            }
            Screen::Items { cursor } => {
                let list = item_rows(world, tables);
                let cursor = step(cursor, list.len(), pressed);
                self.screen = Screen::Items { cursor };
                if cancel {
                    self.message = None;
                    self.screen = Screen::Main;
                } else if confirm && let Some((item, _)) = list.get(cursor) {
                    if usable(tables, *item) && party > 0 {
                        self.message = None;
                        self.screen = Screen::ItemTarget {
                            item: *item,
                            list: cursor,
                            cursor: 0,
                        };
                    } else {
                        self.refused = true;
                        self.message = Some("That cannot be used here.".into());
                    }
                }
            }
            Screen::ItemTarget { item, list, cursor } => {
                let cursor = step(cursor, party, pressed);
                self.screen = Screen::ItemTarget { item, list, cursor };
                if cancel {
                    self.message = None;
                    self.screen = Screen::Items { cursor: list };
                } else if confirm && let Some(member) = world.party.get(cursor).copied() {
                    self.use_item(world, tables, item, member);
                    if world.items.get(&i32::from(item)).copied().unwrap_or(0) <= 0 {
                        self.screen = Screen::Items { cursor: list };
                    }
                }
            }
            Screen::Equip {
                member,
                slot,
                picker,
            } => self.press_equip(pressed, world, tables, member, slot, picker),
            Screen::Status { .. } => {
                if cancel || confirm {
                    self.screen = Screen::Member {
                        pick: Pick::Status,
                        cursor: 0,
                    };
                }
            }
            Screen::Abilities(mut state) => {
                let seed = (self.revision as u32).wrapping_mul(2_654_435_761) >> 24;
                let exit = state.press(pressed, world, tables, &mut self.message, seed);
                self.screen = match exit {
                    AbilityExit::Back => Screen::Member {
                        pick: Pick::Abilities,
                        cursor: world
                            .party
                            .iter()
                            .position(|id| *id == state.member)
                            .unwrap_or(0),
                    },
                    AbilityExit::Stay => Screen::Abilities(state),
                };
            }
            Screen::Order(mut state) => {
                self.screen = match state.press(pressed, world) {
                    SystemExit::Back => Screen::Main,
                    SystemExit::Stay => Screen::Order(state),
                };
            }
            Screen::Config(mut state) => {
                self.screen = match state.press(pressed, world) {
                    SystemExit::Back => Screen::Main,
                    SystemExit::Stay => Screen::Config(state),
                };
            }
            Screen::Cards(mut state) => {
                self.screen = match state.press(pressed, world) {
                    SystemExit::Back => Screen::Main,
                    SystemExit::Stay => Screen::Cards(state),
                };
            }
            Screen::Slots {
                save,
                cursor,
                direct,
            } => {
                let cursor = step(cursor, SLOTS, pressed);
                self.screen = Screen::Slots {
                    save,
                    cursor,
                    direct,
                };
                if cancel {
                    self.message = None;
                    if direct {
                        return false;
                    }
                    self.screen = Screen::Main;
                } else if confirm {
                    let filled = self
                        .slot_labels
                        .get(cursor)
                        .is_some_and(|label| !label.ends_with("Empty"));
                    if save {
                        self.action = Some(MenuAction::Save(cursor + 1));
                    } else if filled {
                        self.action = Some(MenuAction::Load(cursor + 1));
                    } else {
                        self.refused = true;
                        self.message = Some("That slot is empty.".into());
                    }
                }
            }
        }
        true
    }

    fn use_item(&mut self, world: &mut FieldWorld, tables: &MenuTables, item: u16, member: i32) {
        let Some(effect) = tables.effects.get(&item) else {
            return;
        };
        let name = character_name(member);
        let seed = (self.revision as u32).wrapping_mul(2_654_435_761) >> 24;
        self.message = Some(
            match use_field_item(world, &tables.db, item, effect, member, || seed) {
                Ok(ItemUseOutcome::Used {
                    hp_gained,
                    mp_gained,
                }) => match (hp_gained, mp_gained) {
                    (0, 0) => format!("{name}: no change."),
                    (hp, 0) => format!("{name} recovered {hp} HP."),
                    (0, mp) => format!("{name} recovered {mp} MP."),
                    (hp, mp) => format!("{name} recovered {hp} HP and {mp} MP."),
                },
                Ok(ItemUseOutcome::NoEffect) => "It had no effect.".into(),
                Err(_) => "That cannot be used here.".into(),
            },
        );
    }

    fn press_equip(
        &mut self,
        pressed: i32,
        world: &mut FieldWorld,
        tables: &MenuTables,
        member: i32,
        slot: usize,
        picker: Option<(Vec<Option<u16>>, usize)>,
    ) {
        let confirm = pressed & (pad::CONFIRM | pad::CIRCLE) != 0;
        let cancel = pressed & (pad::CANCEL | pad::CROSS) != 0;
        match picker {
            None => {
                let slot = step(slot, 5, pressed);
                self.screen = Screen::Equip {
                    member,
                    slot,
                    picker: None,
                };
                if cancel {
                    self.screen = Screen::Member {
                        pick: Pick::Equip,
                        cursor: world.party.iter().position(|id| *id == member).unwrap_or(0),
                    };
                } else if confirm {
                    let character = tables.character(world, member);
                    let inventory = inventory_from_world(world);
                    let mut entries: Vec<Option<u16>> = Vec::new();
                    if slot != 0 && character.equip[slot] != NO_ITEM {
                        entries.push(None);
                    }
                    entries.extend(
                        equip_list(&tables.db, &inventory, &character, slot)
                            .into_iter()
                            .filter(|id| *id != NO_ITEM)
                            .map(Some),
                    );
                    if entries.is_empty() {
                        self.refused = true;
                        self.message = Some("Nothing to equip there.".into());
                    } else {
                        self.message = None;
                        self.screen = Screen::Equip {
                            member,
                            slot,
                            picker: Some((entries, 0)),
                        };
                    }
                }
            }
            Some((entries, cursor)) => {
                let cursor = step(cursor, entries.len(), pressed);
                self.screen = Screen::Equip {
                    member,
                    slot,
                    picker: Some((entries.clone(), cursor)),
                };
                if cancel {
                    self.screen = Screen::Equip {
                        member,
                        slot,
                        picker: None,
                    };
                } else if confirm {
                    let mut equipment = tables.character(world, member).equip;
                    let mask = tantalus_legacy::shop::equip_mask_from_char_id(member as u32);
                    let done = match entries[cursor] {
                        Some(id) => with_shop_inventory(world, |inventory, _| {
                            equip_item(&tables.db, inventory, &mut equipment, mask, slot, id)
                        })
                        .is_ok(),
                        None => with_shop_inventory(world, |inventory, _| {
                            unequip(inventory, &mut equipment, slot).map(|_| ())
                        })
                        .is_ok(),
                    };
                    if done {
                        let item = equipment[slot];
                        world.equip(member, slot, (item != NO_ITEM).then_some(item));
                    }
                    self.screen = Screen::Equip {
                        member,
                        slot,
                        picker: None,
                    };
                }
            }
        }
    }

    /// What to draw now.
    pub fn frame(&self, world: &FieldWorld, tables: &MenuTables) -> MenuFrame {
        let mut frame = MenuFrame::default();
        match &self.screen {
            Screen::Main => {
                frame.windows.push(self.commands(world));
                frame.windows.push(window(
                    8.0,
                    132.0,
                    84.0,
                    1.0,
                    vec![
                        text(8.0, 0.0, "Gil", Tint::White),
                        right(76.0, 0.0, world.gil.to_string(), Tint::White),
                    ],
                ));
                frame
                    .windows
                    .push(party_window(world, tables, 98.0, 214.0, 4));
                frame.pointer = Some((10.0, 8.0 + 3.0 + (self.main as f32 + 0.5) * ROW));
            }
            Screen::Member { cursor, .. } => {
                frame.windows.push(self.commands(world));
                frame
                    .windows
                    .push(party_window(world, tables, 98.0, 214.0, 4));
                frame.pointer = Some((102.0, 8.0 + 3.0 + (*cursor as f32 * 3.0 + 0.5) * ROW));
            }
            Screen::Items { cursor } => {
                self.items_frame(&mut frame, world, tables, *cursor, None);
            }
            Screen::ItemTarget { list, cursor, .. } => {
                self.items_frame(&mut frame, world, tables, *list, Some(*cursor));
            }
            Screen::Equip {
                member,
                slot,
                picker,
            } => self.equip_frame(&mut frame, world, tables, *member, *slot, picker.as_ref()),
            Screen::Status { member } => status_frame(&mut frame, world, tables, *member),
            Screen::Abilities(state) => ability_frame(state, &mut frame, world, tables),
            Screen::Order(state) => state.frame(&mut frame, world),
            Screen::Config(state) => state.frame(&mut frame, world),
            Screen::Cards(state) => state.frame(&mut frame, world, tables),
            Screen::Slots { save, cursor, .. } => {
                let mut texts = Vec::new();
                for slot in 0..SLOTS {
                    let label = self
                        .slot_labels
                        .get(slot)
                        .cloned()
                        .unwrap_or_else(|| format!("{}  Empty", slot + 1));
                    let tint = if label.ends_with("Empty") && !*save {
                        Tint::Dim
                    } else {
                        Tint::White
                    };
                    texts.push(text(12.0, slot as f32, label, tint));
                }
                frame
                    .windows
                    .push(window(8.0, 8.0, 304.0, SLOTS as f32, texts));
                frame.windows.push(window(
                    8.0,
                    8.0 + SLOTS as f32 * ROW + 12.0,
                    120.0,
                    1.0,
                    vec![text(
                        8.0,
                        0.0,
                        if *save { "Save" } else { "Load" },
                        Tint::Yellow,
                    )],
                ));
                frame.pointer = Some((10.0, 8.0 + 3.0 + (*cursor as f32 + 0.5) * ROW));
            }
        }
        if let Some(message) = &self.message {
            frame.windows.push(window(
                8.0,
                200.0,
                304.0,
                1.0,
                vec![text(6.0, 0.0, message.clone(), Tint::White)],
            ));
        }
        frame
    }

    fn commands(&self, world: &FieldWorld) -> MenuWindow {
        window(
            8.0,
            8.0,
            84.0,
            COMMANDS.len() as f32,
            COMMANDS
                .iter()
                .enumerate()
                .map(|(row, name)| {
                    let on = WORKING[row] || (row == 7 && world.save_point);
                    let tint = if on { Tint::White } else { Tint::Dim };
                    text(12.0, row as f32, *name, tint)
                })
                .collect(),
        )
    }

    fn items_frame(
        &self,
        frame: &mut MenuFrame,
        world: &FieldWorld,
        tables: &MenuTables,
        cursor: usize,
        target: Option<usize>,
    ) {
        let list = item_rows(world, tables);
        let top = cursor.saturating_sub(LIST_ROWS - 1);
        let mut texts = Vec::new();
        for (row, (id, count)) in list.iter().skip(top).take(LIST_ROWS).enumerate() {
            let tint = if usable(tables, *id) {
                Tint::White
            } else {
                Tint::Dim
            };
            texts.push(text(12.0, row as f32, tables.item_name(*id), tint));
            texts.push(right(142.0, row as f32, count.to_string(), tint));
        }
        if list.is_empty() {
            texts.push(text(12.0, 0.0, "No items", Tint::Dim));
        }
        frame
            .windows
            .push(window(8.0, 8.0, 150.0, LIST_ROWS as f32, texts));
        frame
            .windows
            .push(party_window(world, tables, 164.0, 148.0, 4));
        match target {
            None => {
                frame.pointer = Some((10.0, 8.0 + 3.0 + ((cursor - top) as f32 + 0.5) * ROW));
            }
            Some(member) => {
                frame.pointer = Some((168.0, 8.0 + 3.0 + (member as f32 * 3.0 + 0.5) * ROW));
            }
        }
    }

    fn equip_frame(
        &self,
        frame: &mut MenuFrame,
        world: &FieldWorld,
        tables: &MenuTables,
        member: i32,
        slot: usize,
        picker: Option<&(Vec<Option<u16>>, usize)>,
    ) {
        let character = tables.character(world, member);
        let level = tables.level(world.character_exp.get(&member).copied().unwrap_or(0));
        let mut texts = vec![
            text(8.0, 0.0, character_name(member), Tint::Yellow),
            right(172.0, 0.0, format!("Lv {level}"), Tint::White),
        ];
        for (row, name) in SLOT_NAMES.iter().enumerate() {
            let item = character.equip[row];
            let label = if item == NO_ITEM {
                "-".to_string()
            } else {
                tables.item_name(item)
            };
            texts.push(text(8.0, row as f32 + 1.0, *name, Tint::Yellow));
            texts.push(text(72.0, row as f32 + 1.0, label, Tint::White));
        }
        frame.windows.push(window(8.0, 8.0, 180.0, 6.0, texts));
        let change = picker.map(|(entries, cursor)| match entries[*cursor] {
            Some(id) => PreviewChange::Item(id),
            None => PreviewChange::Off,
        });
        let preview = change.and_then(|change| preview_stats(&tables.db, &character, slot, change));
        let now = stats(&tables.db, character.basis, &character.equip);
        let current = tantalus_legacy::menu::equip::stat_array(&now);
        let mut texts = Vec::new();
        for (row, name) in STAT_NAMES.iter().enumerate() {
            texts.push(text(6.0, row as f32, *name, Tint::Yellow));
            match &preview {
                Some(preview) => {
                    let tint = match preview.trend[row] {
                        Trend::Up => Tint::Green,
                        Trend::Down => Tint::Red,
                        Trend::Same => Tint::White,
                    };
                    texts.push(right(
                        78.0,
                        row as f32,
                        current[row].to_string(),
                        Tint::White,
                    ));
                    texts.push(text(82.0, row as f32, ">", Tint::White));
                    texts.push(right(
                        114.0,
                        row as f32,
                        preview.preview[row].to_string(),
                        tint,
                    ));
                }
                None => texts.push(right(
                    114.0,
                    row as f32,
                    current[row].to_string(),
                    Tint::White,
                )),
            }
        }
        frame.windows.push(window(192.0, 8.0, 120.0, 9.0, texts));
        match picker {
            None => {
                frame.pointer = Some((10.0, 8.0 + 3.0 + (slot as f32 + 1.5) * ROW));
            }
            Some((entries, cursor)) => {
                let top = cursor.saturating_sub(PICK_ROWS - 1);
                let texts = entries
                    .iter()
                    .skip(top)
                    .take(PICK_ROWS)
                    .enumerate()
                    .map(|(row, entry)| {
                        let label =
                            entry.map_or_else(|| "Off".to_string(), |id| tables.item_name(id));
                        text(12.0, row as f32, label, Tint::White)
                    })
                    .collect();
                frame.windows.push(window(
                    8.0,
                    102.0,
                    180.0,
                    entries.len().min(PICK_ROWS) as f32,
                    texts,
                ));
                frame.pointer = Some((10.0, 102.0 + 3.0 + ((cursor - top) as f32 + 0.5) * ROW));
            }
        }
    }
}

impl Default for FieldMenu {
    fn default() -> Self {
        Self::new()
    }
}

/// Name, level, HP and MP of up to `limit` party members, three rows each.
pub(crate) fn party_window(
    world: &FieldWorld,
    tables: &MenuTables,
    x: f32,
    w: f32,
    limit: usize,
) -> MenuWindow {
    let mut texts = Vec::new();
    for (index, member) in world.party.iter().take(limit).enumerate() {
        let base = index as f32 * 3.0;
        let stats = world.character_stats.get(member);
        let level = tables.level(world.character_exp.get(member).copied().unwrap_or(0));
        texts.push(text(14.0, base, character_name(*member), Tint::White));
        texts.push(right(w - 10.0, base, format!("Lv {level}"), Tint::White));
        let (hp, max_hp, mp, max_mp) = stats.map_or((0, 0, 0, 0), |s| {
            (s.current_hp, s.max_hp, s.current_mp, s.max_mp)
        });
        let low = max_hp > 0 && hp * 4 <= max_hp;
        texts.push(text(14.0, base + 1.0, "HP", Tint::Yellow));
        texts.push(right(
            w - 10.0,
            base + 1.0,
            format!("{hp}/{max_hp}"),
            if hp <= 0 || low {
                Tint::Red
            } else {
                Tint::White
            },
        ));
        texts.push(text(14.0, base + 2.0, "MP", Tint::Yellow));
        texts.push(right(
            w - 10.0,
            base + 2.0,
            format!("{mp}/{max_mp}"),
            Tint::White,
        ));
    }
    window(x, 8.0, w, 12.0, texts)
}

fn status_frame(frame: &mut MenuFrame, world: &FieldWorld, tables: &MenuTables, member: i32) {
    let character = tables.character(world, member);
    let exp = world.character_exp.get(&member).copied().unwrap_or(0);
    let level = tables.level(exp);
    let next = tables
        .exp
        .get(level)
        .map_or(0, |start| u64::from(*start).saturating_sub(exp));
    let (hp, max_hp, mp, max_mp) = world
        .character_stats
        .get(&member)
        .map_or((0, 0, 0, 0), |s| {
            (s.current_hp, s.max_hp, s.current_mp, s.max_mp)
        });
    let all = stats(&tables.db, character.basis, &character.equip);
    let all = tantalus_legacy::menu::equip::stat_array(&all);
    let mut texts = vec![
        text(8.0, 0.0, character_name(member), Tint::Yellow),
        text(8.0, 1.0, "Level", Tint::Yellow),
        right(140.0, 1.0, level.to_string(), Tint::White),
        text(8.0, 2.0, "EXP", Tint::Yellow),
        right(140.0, 2.0, exp.to_string(), Tint::White),
        text(8.0, 3.0, "Next", Tint::Yellow),
        right(140.0, 3.0, next.to_string(), Tint::White),
        text(8.0, 4.0, "HP", Tint::Yellow),
        right(140.0, 4.0, format!("{hp}/{max_hp}"), Tint::White),
        text(8.0, 5.0, "MP", Tint::Yellow),
        right(140.0, 5.0, format!("{mp}/{max_mp}"), Tint::White),
    ];
    for row in 0..4 {
        texts.push(text(8.0, 6.0 + row as f32, STAT_NAMES[row], Tint::Yellow));
        texts.push(right(
            140.0,
            6.0 + row as f32,
            all[row].to_string(),
            Tint::White,
        ));
    }
    for row in 4..9 {
        let at = (row - 4) as f32;
        texts.push(text(160.0, at, STAT_NAMES[row], Tint::Yellow));
        texts.push(right(296.0, at, all[row].to_string(), Tint::White));
    }
    for (row, name) in SLOT_NAMES.iter().enumerate() {
        let item = character.equip[row];
        let label = if item == NO_ITEM {
            "-".to_string()
        } else {
            tables.item_name(item)
        };
        texts.push(text(
            160.0,
            6.0 + row as f32,
            name.to_string(),
            Tint::Yellow,
        ));
        texts.push(text(222.0, 6.0 + row as f32, label, Tint::White));
    }
    frame.windows.push(window(8.0, 8.0, 304.0, 11.0, texts));
}

#[cfg(test)]
mod tests {
    use super::*;
    use tantalus_game::field::world::CharacterStats;

    fn tables() -> MenuTables {
        let mut tables = MenuTables {
            exp: vec![0, 16, 47],
            ..MenuTables::default()
        };
        // Potion (script 69, power 10: restores 100 HP) and two daggers for Zidane.
        tables.db.items.insert(
            236,
            ItemData {
                price: 50,
                selling_price: 25,
                equip: 0,
                kind: item_type::ITEM | item_type::USABLE,
                sort: 0.0,
                eq_lv: 0.0,
                bonus: 0,
                ability: vec![],
                weapon_power: None,
                armor: None,
            },
        );
        tables.effects.insert(
            236,
            FieldEffect {
                script_id: 69,
                power: 10,
                status: 0,
            },
        );
        for (id, power) in [(1u16, 12), (2, 20)] {
            tables.db.items.insert(
                id,
                ItemData {
                    price: 0,
                    selling_price: 0,
                    equip: 1 << 11,
                    kind: item_type::WEAPON,
                    sort: 0.0,
                    eq_lv: 0.0,
                    bonus: 0,
                    ability: vec![],
                    weapon_power: Some(power),
                    armor: None,
                },
            );
        }
        tables.db.bonuses.push(StatBonus::default());
        tables
    }

    fn world() -> FieldWorld {
        let mut world = FieldWorld::new();
        world.party = vec![0];
        world.character_stats.insert(
            0,
            CharacterStats {
                current_hp: 10,
                max_hp: 150,
                current_mp: 5,
                max_mp: 5,
            },
        );
        world.items.insert(236, 2);
        world.items.insert(2, 1);
        world.equip(0, 0, Some(1));
        world
    }

    #[test]
    fn level_comes_from_the_experience_table() {
        let tables = tables();
        assert_eq!(tables.level(0), 1);
        assert_eq!(tables.level(16), 2);
        assert_eq!(tables.level(1000), 3);
    }

    #[test]
    fn a_potion_heals_the_chosen_member_and_is_used_up() {
        let (tables, mut world) = (tables(), world());
        let mut menu = FieldMenu::new();
        // Items, the Potion, the first member.
        for _ in 0..3 {
            assert!(menu.press(pad::CONFIRM, &mut world, &tables));
        }
        assert_eq!(world.character_stats[&0].current_hp, 110);
        assert_eq!(world.items[&236], 1);
        assert!(menu.message.as_deref().unwrap().contains("recovered"));
        // Cancel back out, then cancel closes.
        for _ in 0..2 {
            assert!(menu.press(pad::CANCEL, &mut world, &tables));
        }
        assert!(!menu.press(pad::CANCEL, &mut world, &tables));
    }

    #[test]
    fn equip_previews_then_swaps_with_the_inventory() {
        let (tables, mut world) = (tables(), world());
        let mut menu = FieldMenu::new();
        menu.press(pad::DOWN, &mut world, &tables);
        menu.press(pad::DOWN, &mut world, &tables);
        // Equip, member, weapon slot: the picker holds the held dagger.
        for _ in 0..3 {
            menu.press(pad::CONFIRM, &mut world, &tables);
        }
        let frame = menu.frame(&world, &tables);
        let stats = frame.windows.iter().find(|w| w.x == 192.0).unwrap();
        assert!(
            stats
                .texts
                .iter()
                .any(|t| t.text == "20" && t.tint == Tint::Green)
        );
        menu.press(pad::CONFIRM, &mut world, &tables);
        assert_eq!(world.equipped(0)[0], Some(2));
        assert_eq!(world.items.get(&1), Some(&1));
        assert_eq!(world.items.get(&2), None);
    }

    #[test]
    fn start_closes_the_menu() {
        let (tables, mut world) = (tables(), world());
        let mut menu = FieldMenu::new();
        assert!(!menu.press(pad::START, &mut world, &tables));
    }
}
