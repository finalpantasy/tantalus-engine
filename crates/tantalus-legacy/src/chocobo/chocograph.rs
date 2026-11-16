//! Chocograph list rules (`Global/ChocographUI.cs`) and the treasure
//! achievement bookkeeping (`Global/EMinigame.cs`).

use crate::saves::ff9_tree::EventGlobal;

/// `ChocographUI.HintMapMax` (`ChocographUI.cs:428`).
pub const HINT_MAP_MAX: usize = 24;

/// `ChocographUI.Icon` (`ChocographUI.cs:556-568`), same order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Field,
    Reef,
    Mountain,
    Sea,
    Sky,
    Unknown,
    BoxOpen,
    BoxDisableOpen,
    BoxClose,
    BoxDisableClose,
}

impl Icon {
    /// `GetIconSprite` and `GetAbilitySprite` (`ChocographUI.cs:318-333, 348-367`).
    pub fn sprite_name(self) -> &'static str {
        match self {
            Icon::Field => "chocograph_icon_field",
            Icon::Reef => "chocograph_icon_reef",
            Icon::Mountain => "chocograph_icon_mt",
            Icon::Sea => "chocograph_icon_sea",
            Icon::Sky => "chocograph_icon_sky",
            Icon::Unknown => "chocograph_icon_unknown_null",
            Icon::BoxOpen => "chocograph_box_open",
            Icon::BoxDisableOpen => "chocograph_box_open_null",
            Icon::BoxClose => "chocograph_box_close",
            Icon::BoxDisableClose => "chocograph_box_close_null",
        }
    }

    /// Terrain icons are the first five values and double as the required
    /// ability level (`GetIconType` casts the enum to int).
    fn level(self) -> i32 {
        self as i32
    }

    fn from_ability_index(i: i32) -> Option<Icon> {
        Some(match i {
            0 => Icon::Field,
            1 => Icon::Reef,
            2 => Icon::Mountain,
            3 => Icon::Sea,
            4 => Icon::Sky,
            5 => Icon::Unknown,
            _ => return None,
        })
    }
}

/// `ChocographUI.ChocographRequirement` (`ChocographUI.cs:480-506`): the
/// terrain class of each of the 24 pictures. Pictures 0-5 need Field, 6-11
/// Reef, 12-15 Mountain, 16-19 Sea, 20-23 Sky.
pub const REQUIREMENT: [Icon; HINT_MAP_MAX] = [
    Icon::Field,
    Icon::Field,
    Icon::Field,
    Icon::Field,
    Icon::Field,
    Icon::Field,
    Icon::Reef,
    Icon::Reef,
    Icon::Reef,
    Icon::Reef,
    Icon::Reef,
    Icon::Reef,
    Icon::Mountain,
    Icon::Mountain,
    Icon::Mountain,
    Icon::Mountain,
    Icon::Sea,
    Icon::Sea,
    Icon::Sea,
    Icon::Sea,
    Icon::Sky,
    Icon::Sky,
    Icon::Sky,
    Icon::Sky,
];

/// Text ids into `ff9choco.mes` (`FF9TextTool.cs:681-683`, `ChocographUI.cs:99`).
pub const NAME_TEXT_START: usize = 10;
pub const HELP_TEXT_START: usize = 34;
pub const DETAIL_TEXT_START: usize = 58;
/// Help text shown while the picture is out of reach (`ChocographUI.cs:99`).
pub const OUT_OF_REACH_TEXT: usize = 5;

/// Why a picture cannot be selected (`OnKeyConfirm`, `ChocographUI.cs:145-154`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectError {
    /// Index is not 0..24.
    OutOfRange,
    /// The map bit is not set (`!hasMap[id]`).
    NoMap,
    /// `ability <= GetIconType(id)`.
    AbilityTooLow,
}

/// One row of the picture list (`DisplayChocographList`, `ChocographUI.cs:285-307`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChocographEntry {
    /// `false`: the row is hidden and its help is disabled.
    pub has_map: bool,
    pub can_reach: bool,
    /// Box sprite; `None` when the row is hidden.
    pub box_icon: Option<Icon>,
    /// `i + ChocographNameStartIndex`.
    pub name_text: usize,
    /// `i + ChocographHelpStartIndex`, or 5 when out of reach.
    pub help_text: usize,
    /// `i + ChocographDetailStartIndex`.
    pub detail_text: usize,
    /// `"chocograph_map_" + i.ToString("D2")`.
    pub map_sprite: String,
}

/// Chocograph list state: the bytes `GetEventData` reads plus `hintmap_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChocographBook {
    found: u32,
    opened: u32,
    ability: i32,
    /// `FF9StateGlobal.hintmap_id` (0 = none, otherwise picture index + 1).
    pub hintmap_id: i32,
    /// `ChocographUI.CurrentSelectedChocograph`, -1 for none.
    pub selected: i32,
}

impl ChocographBook {
    /// `GetEventData` (`ChocographUI.cs:238-262`), including
    /// `UpdateEquipedHintMap` first.
    pub fn from_event(global: &EventGlobal, hintmap_id: i32) -> Self {
        let bytes = global.as_bytes();
        let mut found = 0u32;
        let mut opened = 0u32;
        for i in 0..3 {
            found |= u32::from(bytes[187 + i]) << (i * 8);
            opened |= u32::from(bytes[184 + i]) << (i * 8);
        }
        let mut book = Self {
            found,
            opened,
            ability: i32::from(bytes[191]),
            hintmap_id,
            selected: -1,
        };
        book.update_equipped_hint_map();
        book
    }

    /// `UpdateEquipedHintMap` (`ChocographUI.cs:225-236`).
    pub fn update_equipped_hint_map(&mut self) {
        if self.hintmap_id > HINT_MAP_MAX as i32 {
            self.hintmap_id = 0;
            self.selected = -1;
        } else {
            self.selected = self.hintmap_id.max(0) - 1;
        }
    }

    /// `ability` (byte 191): the picture needs `ability > terrain level`.
    pub fn ability(&self) -> i32 {
        self.ability
    }

    pub fn has_map(&self, i: usize) -> bool {
        i < HINT_MAP_MAX && self.found & (1 << i) != 0
    }

    pub fn has_gem(&self, i: usize) -> bool {
        i < HINT_MAP_MAX && self.opened & (1 << i) != 0
    }

    /// `mapCount` (`InventoryNumber` label).
    pub fn map_count(&self) -> u32 {
        (self.found & 0xFF_FFFF).count_ones()
    }

    /// `gemCount` (`LocationFoundNumber` label).
    pub fn gem_count(&self) -> u32 {
        (self.opened & 0xFF_FFFF).count_ones()
    }

    /// `ability > GetIconType(i)` (`ChocographUI.cs:296`).
    pub fn can_reach(&self, i: usize) -> bool {
        i < HINT_MAP_MAX && self.ability > REQUIREMENT[i].level()
    }

    /// One row of `DisplayChocographList`.
    pub fn entry(&self, i: usize) -> Option<ChocographEntry> {
        if i >= HINT_MAP_MAX {
            return None;
        }
        let has_map = self.has_map(i);
        let can_reach = self.can_reach(i);
        let box_icon = has_map.then(|| match (self.has_gem(i), can_reach) {
            (true, true) => Icon::BoxOpen,
            (true, false) => Icon::BoxDisableOpen,
            (false, true) => Icon::BoxClose,
            (false, false) => Icon::BoxDisableClose,
        });
        Some(ChocographEntry {
            has_map,
            can_reach,
            box_icon,
            name_text: i + NAME_TEXT_START,
            help_text: if can_reach {
                i + HELP_TEXT_START
            } else {
                OUT_OF_REACH_TEXT
            },
            detail_text: i + DETAIL_TEXT_START,
            map_sprite: format!("chocograph_map_{i:02}"),
        })
    }

    /// `OnKeyConfirm` on a list row (`ChocographUI.cs:142-160`): select the
    /// picture and store `hintmap_id = 1 + i`.
    pub fn select(&mut self, i: usize) -> Result<(), SelectError> {
        if i >= HINT_MAP_MAX {
            return Err(SelectError::OutOfRange);
        }
        if !self.has_map(i) {
            return Err(SelectError::NoMap);
        }
        if !self.can_reach(i) {
            return Err(SelectError::AbilityTooLow);
        }
        self.selected = i as i32;
        self.hintmap_id = 1 + i as i32;
        Ok(())
    }

    /// The Cancel sub menu (`ChocographUI.cs:123-133`): clears the selection
    /// and `hintmap_id`. `false` when nothing was selected (error sound only).
    pub fn cancel_selection(&mut self) -> bool {
        if self.selected == -1 {
            return false;
        }
        self.hintmap_id = 0;
        self.selected = -1;
        true
    }

    /// `DisplayChocoboAbilityInfo` (`ChocographUI.cs:264-268`): one terrain
    /// icon per ability point. The source writes into a 5-slot array, so an
    /// ability above 5 throws there; this returns `None` for that case.
    pub fn ability_icons(&self) -> Option<Vec<Icon>> {
        if self.ability > 5 {
            return None;
        }
        (0..self.ability).map(Icon::from_ability_index).collect()
    }

    /// `DisplayHint` requirement row (`ChocographUI.cs:340-345`): for
    /// `k in 0..=requirement`, the icon `k` if the ability reaches it, else
    /// the unknown icon (index 5).
    pub fn required_icons(&self, i: usize) -> Option<Vec<Icon>> {
        if i >= HINT_MAP_MAX {
            return None;
        }
        let need = REQUIREMENT[i].level();
        (0..=need)
            .map(|k| Icon::from_ability_index(if self.ability <= k { 5 } else { k }))
            .collect()
    }
}

/// `EMinigame.AllTreasureCheating` (`EMinigame.cs:449-455`): all 24 found,
/// 23 opened (bit 0 left closed), ability 5.
pub fn all_treasure_cheat(global: &mut EventGlobal) {
    global.set_chocograph_opened(0xFF_FFFE);
    global.set_chocograph_found(0xFF_FFFF);
    global.set_chocograph_ability(5);
}

/// `EMinigame.AllTreasureAchievement` and its initializer
/// (`EMinigame.cs:430-447`), `numOfTreasures = 24` (`EMinigame.cs:762`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreasureTracker {
    last: u32,
}

impl TreasureTracker {
    /// `InitializeAllTreasureAchievement`.
    pub fn new(global: &EventGlobal) -> Self {
        Self {
            last: global.chocograph_opened() & 0xFF_FFFF,
        }
    }

    /// Run each frame on the world map. `Some(24)` when the opened bits just
    /// became all set (reported to `AllTreasure`).
    pub fn update(&mut self, global: &EventGlobal) -> Option<u32> {
        let flags = global.chocograph_opened() & 0xFF_FFFF;
        let hit = self.last != flags && flags == 0xFF_FFFF;
        self.last = flags;
        hit.then_some(24)
    }
}

/// Treasure Hunter rank letter for a point total. The thresholds are from the
/// comment above `EventState.GetTreasureHunterPoints`
/// (`Global/Event/EventState.cs:55-63`); no C# code applies them, the field
/// scripts do. The point total itself is `EventGlobal::treasure_hunter_points`.
pub fn treasure_hunter_rank(points: u32) -> char {
    match points {
        0..=199 => 'H',
        200..=219 => 'G',
        220..=239 => 'F',
        240..=259 => 'E',
        260..=279 => 'D',
        280..=299 => 'C',
        300..=319 => 'B',
        320..=339 => 'A',
        _ => 'S',
    }
}
