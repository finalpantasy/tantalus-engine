//! The field menu's Order, Card and Config screens.
//!
//! * Order (`MainMenuUI.ToggleOrder`): pick a slot, then another to swap the two
//!   members, or the same slot again to flip that member between the front and
//!   back row. The party order is `FieldWorld::party`, the rows are
//!   `FieldWorld::back_row`; a battle reads both. Stand-in: only slots that hold
//!   a member can be picked (the world keeps no empty slots between members).
//! * Config (`ConfigUI`): the rules for each row come from
//!   `tantalus_legacy::party_menu::config`; the values live in
//!   `FieldWorld::settings` and are saved with the game.
//! * Card (`CardUI`): the Tetra Master collection, with the collector's points and the
//!   win, lose and draw counts the card games keep (`tantalus_game::cards`).

use tantalus_game::FieldWorld;
use tantalus_game::field::GameSettings;
use tantalus_legacy::party_menu::config::{
    Configurator, is_slider, nudge_slider, slider_from_atb_mode,
};
use tantalus_legacy::party_menu::main_menu::{OrderMember, toggle_order};

use crate::field_menu::{MenuFrame, MenuTables, MenuText, ROW, Tint, step, window};
use crate::play::pad;

/// Outcome of a press on one of these screens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Stay,
    Back,
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

// --- Order ------------------------------------------------------------------------------

/// The Order screen: the cursor slot and the first slot picked.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OrderState {
    cursor: usize,
    first: Option<usize>,
}

impl OrderState {
    pub fn press(&mut self, pressed: i32, world: &mut FieldWorld) -> Exit {
        let len = world.party.len().min(4);
        self.cursor = step(self.cursor, len, pressed);
        if pressed & (pad::CANCEL | pad::CROSS) != 0 {
            if self.first.take().is_some() {
                return Exit::Stay;
            }
            return Exit::Back;
        }
        if pressed & (pad::CONFIRM | pad::CIRCLE) != 0 && len > 0 {
            match self.first {
                None => self.first = Some(self.cursor),
                Some(first) => {
                    self.first = None;
                    apply_order(world, first, self.cursor);
                }
            }
        }
        Exit::Stay
    }

    pub fn frame(&self, frame: &mut MenuFrame, world: &FieldWorld) {
        let mut texts = Vec::new();
        for (slot, member) in world.party.iter().take(4).enumerate() {
            let tint = if self.first == Some(slot) {
                Tint::Yellow
            } else {
                Tint::White
            };
            let row = if world.back_row.contains(member) {
                "Back"
            } else {
                "Front"
            };
            texts.push(text(
                12.0,
                slot as f32,
                crate::name_menu::display_name(world, *member),
                tint,
            ));
            texts.push(right(172.0, slot as f32, row, tint));
        }
        frame.windows.push(window(
            8.0,
            8.0,
            184.0,
            world.party.len().min(4) as f32,
            texts,
        ));
        let help = match self.first {
            None => "Pick a member to move or to change rows.",
            Some(_) => "Pick the slot to swap with, or the same member to change rows.",
        };
        frame.windows.push(window(
            8.0,
            8.0 + 4.0 * ROW + 12.0,
            304.0,
            1.0,
            vec![text(8.0, 0.0, help, Tint::White)],
        ));
        frame.pointer = Some((10.0, 8.0 + 3.0 + (self.cursor as f32 + 0.5) * ROW));
    }
}

/// Runs `toggle_order` on the world's party and rows.
fn apply_order(world: &mut FieldWorld, current: usize, target: usize) {
    let mut slots: [Option<OrderMember>; 4] = [None; 4];
    for (slot, member) in world.party.iter().take(4).enumerate() {
        slots[slot] = Some(OrderMember {
            id: u8::try_from(*member).unwrap_or(0),
            row: u8::from(!world.back_row.contains(member)),
        });
    }
    if toggle_order(&mut slots, current, target).is_err() {
        return;
    }
    let rest: Vec<i32> = world.party.iter().skip(4).copied().collect();
    world.party = slots
        .iter()
        .flatten()
        .map(|member| i32::from(member.id))
        .chain(rest)
        .collect();
    for member in slots.iter().flatten() {
        let id = i32::from(member.id);
        // The legacy record's row is 0 for the back row, as in the battle.
        if member.row == 0 {
            world.back_row.insert(id);
        } else {
            world.back_row.remove(&id);
        }
    }
}

// --- Config -----------------------------------------------------------------------------

/// The rows shown, top to bottom.
const CONFIG_ROWS: [(&str, Configurator); 13] = [
    ("Music Volume", Configurator::MusicVolume),
    ("Sound Volume", Configurator::SoundVolume),
    ("Movie Volume", Configurator::MovieVolume),
    ("Cursor", Configurator::Cursor),
    ("ATB", Configurator::Atb),
    ("ATB Mode", Configurator::AtbMode),
    ("Battle Speed", Configurator::BattleSpeed),
    ("Battle Camera", Configurator::BattleCamera),
    ("Field Message", Configurator::FieldMessage),
    ("Movement", Configurator::Movement),
    ("Here Icon", Configurator::HereIcon),
    ("Window", Configurator::WindowColor),
    ("Vibration", Configurator::Vibration),
];
const CONFIG_VISIBLE: usize = 9;

/// Slider step of one arrow press.
fn slider_step(row: Configurator) -> f32 {
    match row {
        Configurator::FieldMessage => 1.0 / 6.0,
        Configurator::BattleSpeed => 1.0 / 2.0,
        Configurator::AtbMode => 1.0 / 3.0,
        _ => 1.0 / 20.0,
    }
}

/// The two choice names of a toggle row (value 0, value 1), from what the
/// reference code does with each value.
fn choice_names(row: Configurator) -> (&'static str, &'static str) {
    match row {
        Configurator::Cursor => ("Initial", "Memory"),
        Configurator::Atb => ("Active", "Wait"),
        Configurator::BattleCamera => ("Auto", "Fixed"),
        Configurator::Movement => ("Walk", "Run"),
        Configurator::HereIcon => ("On", "Off"),
        Configurator::WindowColor => ("Gray", "Blue"),
        Configurator::Vibration => ("On", "Off"),
        _ => ("0", "1"),
    }
}

fn slider_text(settings: &GameSettings, row: Configurator) -> String {
    match row {
        Configurator::MusicVolume => format!("{}%", settings.music_volume),
        Configurator::SoundVolume => format!("{}%", settings.sound_volume),
        Configurator::MovieVolume => format!("{}%", settings.movie_volume),
        Configurator::BattleSpeed => ["Slow", "Normal", "Fast"]
            .get(usize::try_from(settings.btl_speed).unwrap_or(1))
            .copied()
            .unwrap_or("Normal")
            .to_owned(),
        Configurator::FieldMessage => match settings.fld_msg {
            0 => "Fast".to_owned(),
            6 => "Slow".to_owned(),
            n => (n + 1).to_string(),
        },
        Configurator::AtbMode => {
            let index = (slider_from_atb_mode(settings.atb_mode) * 3.0).round() as usize;
            ["Normal", "Fast", "Turn-based", "Simultaneous"][index.min(3)].to_owned()
        }
        _ => String::new(),
    }
}

/// The Config screen cursor.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConfigState {
    cursor: usize,
}

impl ConfigState {
    pub fn press(&mut self, pressed: i32, world: &mut FieldWorld) -> Exit {
        self.cursor = step(self.cursor, CONFIG_ROWS.len(), pressed);
        if pressed & (pad::CANCEL | pad::CROSS) != 0 {
            return Exit::Back;
        }
        let left = pressed & pad::LEFT != 0;
        let right = pressed & pad::RIGHT != 0;
        let confirm = pressed & (pad::CONFIRM | pad::CIRCLE) != 0;
        if !(left || right || confirm) {
            return Exit::Stay;
        }
        let row = CONFIG_ROWS[self.cursor].1;
        let mut settings = world.settings.to_legacy();
        let Some(value) = settings.ui_value(row) else {
            return Exit::Stay;
        };
        let new = if is_slider(row) {
            if confirm {
                return Exit::Stay;
            }
            nudge_slider(value, slider_step(row), right)
        } else {
            1.0 - value.clamp(0.0, 1.0).round()
        };
        // The ATB mode slider truncates, so nudge it onto the stop.
        let new = if row == Configurator::AtbMode {
            ((new * 3.0).round() / 3.0 + 0.01).min(1.0)
        } else {
            new
        };
        settings.set_ui_value(row, new);
        world.settings = settings.into();
        Exit::Stay
    }

    pub fn frame(&self, frame: &mut MenuFrame, world: &FieldWorld) {
        let settings = world.settings.to_legacy();
        let top = self
            .cursor
            .saturating_sub(CONFIG_VISIBLE - 1)
            .min(CONFIG_ROWS.len() - CONFIG_VISIBLE);
        let mut texts = Vec::new();
        for (index, (label, row)) in CONFIG_ROWS
            .iter()
            .enumerate()
            .skip(top)
            .take(CONFIG_VISIBLE)
        {
            let line = (index - top) as f32;
            let value = if is_slider(*row) {
                slider_text(&world.settings, *row)
            } else {
                let (off, on) = choice_names(*row);
                if settings.ui_value(*row).unwrap_or(0.0) >= 0.5 {
                    on.to_owned()
                } else {
                    off.to_owned()
                }
            };
            texts.push(text(12.0, line, *label, Tint::White));
            texts.push(right(292.0, line, value, Tint::Yellow));
        }
        frame
            .windows
            .push(window(8.0, 8.0, 304.0, CONFIG_VISIBLE as f32, texts));
        frame.windows.push(window(
            8.0,
            8.0 + CONFIG_VISIBLE as f32 * ROW + 12.0,
            304.0,
            1.0,
            vec![text(
                8.0,
                0.0,
                "Left and right change a setting.",
                Tint::White,
            )],
        ));
        frame.pointer = Some((10.0, 8.0 + 3.0 + ((self.cursor - top) as f32 + 0.5) * ROW));
    }
}

// --- Card -------------------------------------------------------------------------------

/// `TetraMasterCardId` count.
pub const CARD_KINDS: usize = 100;
const CARD_VISIBLE: usize = 9;

/// Cards owned of one kind.
fn owned(world: &FieldWorld, id: usize) -> usize {
    world
        .cards
        .iter()
        .filter(|card| usize::try_from(**card) == Ok(id))
        .count()
}

/// Collector points (`tantalus_game::cards::collector_points`).
pub fn collector_points(world: &FieldWorld) -> i32 {
    tantalus_game::cards::collector_points(world)
}

/// `MiniGame_GetCollectorLevel`.
pub fn collector_level(points: i32) -> usize {
    tantalus_game::cards::collector_level(points)
}

/// The Card screen: the card kind under the cursor and which copy is shown.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CardState {
    cursor: usize,
    copy: usize,
}

impl CardState {
    pub fn press(&mut self, pressed: i32, world: &FieldWorld) -> Exit {
        if pressed & (pad::CANCEL | pad::CROSS) != 0 {
            return Exit::Back;
        }
        let before = self.cursor;
        self.cursor = step(self.cursor, CARD_KINDS, pressed);
        if self.cursor != before {
            self.copy = 0;
        }
        let count = owned(world, self.cursor);
        if count > 1 {
            if pressed & pad::LEFT != 0 {
                self.copy = (self.copy + count - 1) % count;
            } else if pressed & pad::RIGHT != 0 {
                self.copy = (self.copy + 1) % count;
            }
        }
        Exit::Stay
    }

    pub fn frame(&self, frame: &mut MenuFrame, world: &FieldWorld, tables: &MenuTables) {
        let points = collector_points(world);
        let level = collector_level(points);
        let class = tables
            .card_levels
            .get(&(level as i32))
            .cloned()
            .unwrap_or_default();
        frame.windows.push(window(
            8.0,
            8.0,
            304.0,
            2.0,
            vec![
                text(
                    8.0,
                    0.0,
                    format!("Level {level}   {points} points"),
                    Tint::White,
                ),
                right(292.0, 0.0, class, Tint::Yellow),
                text(
                    8.0,
                    1.0,
                    format!(
                        "Win {}   Lose {}   Draw {}",
                        world.card_stats.win, world.card_stats.lose, world.card_stats.draw
                    ),
                    Tint::White,
                ),
            ],
        ));
        let top = self
            .cursor
            .saturating_sub(CARD_VISIBLE - 1)
            .min(CARD_KINDS - CARD_VISIBLE);
        let mut texts = Vec::new();
        for id in top..top + CARD_VISIBLE {
            let line = (id - top) as f32;
            let count = owned(world, id);
            let label = if count > 0 {
                tables
                    .card_names
                    .get(&(id as i32))
                    .cloned()
                    .unwrap_or_default()
            } else {
                "-----".to_owned()
            };
            let tint = if count > 0 { Tint::White } else { Tint::Dim };
            texts.push(text(12.0, line, format!("{:>3} {label}", id + 1), tint));
            if count > 1 {
                texts.push(right(172.0, line, format!("x{count}"), Tint::Yellow));
            }
        }
        let list_top = 8.0 + 2.0 * ROW + 12.0;
        frame
            .windows
            .push(window(8.0, list_top, 184.0, CARD_VISIBLE as f32, texts));
        let count = owned(world, self.cursor);
        let mut detail = vec![
            text(8.0, 0.0, "Cards", Tint::White),
            right(100.0, 0.0, world.cards.len().to_string(), Tint::Yellow),
            text(8.0, 1.0, "Types", Tint::White),
            right(
                100.0,
                1.0,
                (0..CARD_KINDS)
                    .filter(|id| owned(world, *id) > 0)
                    .count()
                    .to_string(),
                Tint::Yellow,
            ),
        ];
        if count > 0 {
            detail.push(text(
                8.0,
                3.0,
                tables
                    .card_names
                    .get(&(self.cursor as i32))
                    .cloned()
                    .unwrap_or_default(),
                Tint::White,
            ));
            if count > 1 {
                detail.push(text(
                    8.0,
                    4.0,
                    format!("{} / {count}", self.copy + 1),
                    Tint::Yellow,
                ));
            }
        }
        frame
            .windows
            .push(window(196.0, list_top, 116.0, CARD_VISIBLE as f32, detail));
        frame.pointer = Some((
            10.0,
            list_top + 3.0 + ((self.cursor - top) as f32 + 0.5) * ROW,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> FieldWorld {
        let mut world = FieldWorld::new();
        world.party = vec![0, 1, 2];
        world
    }

    #[test]
    fn swapping_moves_the_member_and_the_same_slot_twice_flips_the_row() {
        let mut world = world();
        let mut order = OrderState::default();
        // Pick slot 0, move down, pick slot 1: they swap.
        order.press(pad::CONFIRM, &mut world);
        order.press(pad::DOWN, &mut world);
        order.press(pad::CONFIRM, &mut world);
        assert_eq!(world.party, vec![1, 0, 2]);
        // The same slot twice: the member goes to the back row, then returns.
        order.press(pad::CONFIRM, &mut world);
        order.press(pad::CONFIRM, &mut world);
        assert!(world.back_row.contains(&0));
        order.press(pad::CONFIRM, &mut world);
        order.press(pad::CONFIRM, &mut world);
        assert!(world.back_row.is_empty());
        // Cancel drops a pick first, then leaves.
        order.press(pad::CONFIRM, &mut world);
        assert_eq!(order.press(pad::CANCEL, &mut world), Exit::Stay);
        assert_eq!(order.press(pad::CANCEL, &mut world), Exit::Back);
    }

    #[test]
    fn config_changes_save_with_the_world() {
        let mut world = world();
        let mut config = ConfigState::default();
        // Music volume steps down by 5 per left press.
        config.press(pad::LEFT, &mut world);
        config.press(pad::LEFT, &mut world);
        assert_eq!(world.settings.music_volume, 90);
        // Down to Cursor (row 3), toggle with confirm.
        for _ in 0..3 {
            config.press(pad::DOWN, &mut world);
        }
        config.press(pad::CONFIRM, &mut world);
        assert_eq!(world.settings.cursor, 1);
        // Battle Speed (row 6) goes from Normal to Fast.
        for _ in 0..3 {
            config.press(pad::DOWN, &mut world);
        }
        config.press(pad::RIGHT, &mut world);
        assert_eq!(world.settings.btl_speed, 2);
        // ATB Mode (row 5) steps to Fast.
        config.press(pad::UP, &mut world);
        config.press(pad::RIGHT, &mut world);
        assert_eq!(world.settings.atb_mode, 1);
        let text = serde_json::to_string(&world).unwrap();
        let back: FieldWorld = serde_json::from_str(&text).unwrap();
        assert_eq!(back.settings, world.settings);
        assert_eq!(config.press(pad::CANCEL, &mut world), Exit::Back);
    }

    #[test]
    fn the_card_list_counts_kinds_and_copies() {
        let mut world = world();
        world.cards = vec![3, 3, 7];
        let mut tables = MenuTables::default();
        tables.card_names.insert(3, "Goblin".to_owned());
        let mut card = CardState::default();
        for _ in 0..3 {
            card.press(pad::DOWN, &world);
        }
        card.press(pad::RIGHT, &world);
        let mut frame = MenuFrame::default();
        card.frame(&mut frame, &world, &tables);
        let all: Vec<&str> = frame
            .windows
            .iter()
            .flat_map(|w| w.texts.iter().map(|t| t.text.as_str()))
            .collect();
        assert!(all.iter().any(|t| t.contains("Goblin")));
        assert!(all.contains(&"x2"));
        assert!(all.contains(&"2 / 2"));
        assert_eq!(collector_points(&world), 20);
        assert_eq!(collector_level(0), 0);
        assert_eq!(collector_level(300), 1);
    }

    /// Runs only with `FFIX_GAME_DATA` (the install's `FF9_Data`) and `FFIX_MEMORIA` set.
    #[test]
    fn the_real_card_names_and_classes_load() {
        let (Some(data), Some(memoria)) = (
            std::env::var_os("FFIX_GAME_DATA"),
            std::env::var_os("FFIX_MEMORIA"),
        ) else {
            return;
        };
        let source = std::fs::read_to_string(
            std::path::Path::new(&memoria)
                .join("Assembly-CSharp/Global/Event/Engine/EventEngineUtils.cs"),
        )
        .unwrap();
        let text = crate::field_data::FieldText::open(
            std::path::Path::new(&data),
            &source,
            "us".to_owned(),
        )
        .unwrap();
        let names = text.names("etc/minista").unwrap();
        let levels = text.names("etc/card").unwrap();
        println!(
            "CARDS {} {:?} {:?}",
            names.len(),
            names.get(&0),
            names.get(&99)
        );
        println!("LEVELS {} {:?}", levels.len(), levels.get(&0));
        assert!(names.len() >= 100);
        assert!(!levels.is_empty());
    }
}
