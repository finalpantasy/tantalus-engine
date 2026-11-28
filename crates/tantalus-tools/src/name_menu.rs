//! The naming screen a field script opens with `MENU(1, character)`
//! (`EventService.FF9Menu_Command` case 1 -> `EventService.OpenNameMenu`,
//! `Global/Event/EventService.cs`; the screen is `NameSettingUI`,
//! `Global/NameSettingUI.cs`). The rules (12 character limit, allowed
//! characters, trim, empty refusal) are `tantalus_legacy::party_menu::name`.
//!
//! The PC screen is a text field: the player types, Confirm (Enter) takes
//! the name, Menu (Triangle, `OnKeyMenu`) resets it to the default name, and
//! there is no way out without a non-empty name. Keyboard typing comes in
//! through [`NameMenu::type_text`]; the pad drives a character grid with
//! the cursor, so a gamepad can enter a name too.

use tantalus_game::FieldWorld;
use tantalus_legacy::party_menu::name::{
    NAME_LIMIT, NameError, confirm_name, filter_input, init_id, validate_char,
};

use crate::field_menu::{MenuArt, MenuFrame, MenuImage, MenuText, ROW, Tint, at, window};
use crate::play::{character_name, pad};
use crate::play_audio::menu_sound;

/// The grid's character rows; the last row is the three commands.
const GRID: [&str; 5] = [
    "ABCDEFGHIJKLM",
    "NOPQRSTUVWXYZ",
    "abcdefghijklm",
    "nopqrstuvwxyz",
    "0123456789-.'",
];
const COLUMNS: usize = 13;
const COMMANDS: [&str; 3] = ["Space", "Default", "OK"];

/// The name a script-given character starts with: the player's current
/// name (`GetPlayer(SubNo).Name`), or the default name for the stock cast
/// (`FF9TextTool.CharacterDefaultName`, `IsDefaultName`).
pub fn initial_name(world: &FieldWorld, id: i32) -> String {
    let (is_default, character) = init_id(u32::try_from(id).unwrap_or(0));
    let character = i32::try_from(character).unwrap_or(0);
    if is_default {
        character_name(character).to_owned()
    } else {
        display_name(world, character)
    }
}

/// The name shown for a character: the one the player chose, else the
/// default.
pub fn display_name(world: &FieldWorld, id: i32) -> String {
    world
        .character_names
        .get(&id)
        .cloned()
        .unwrap_or_else(|| character_name(id).to_owned())
}

/// What a press did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NamePress {
    /// The screen closed with a stored name.
    pub closed: bool,
    /// The sound effect to play.
    pub sound: Option<i32>,
}

/// An open naming screen.
#[derive(Debug, Clone, PartialEq)]
pub struct NameMenu {
    /// `CharacterId` whose name is being set (`NameSettingUI._subNumber`).
    pub character: i32,
    default_name: String,
    pub text: String,
    /// Grid cursor, row then column; row 5 holds the commands.
    pub row: usize,
    pub col: usize,
    /// `Warning`: the empty name was refused.
    pub warning: bool,
    pub revision: u64,
}

fn sound(id: i32) -> NamePress {
    NamePress {
        closed: false,
        sound: Some(id),
    }
}

impl NameMenu {
    /// `NameSettingUI.InitID` + `SetData`: the field starts with the default
    /// or current name.
    pub fn new(world: &FieldWorld, id: i32) -> Self {
        let (_, character) = init_id(u32::try_from(id).unwrap_or(0));
        let character = i32::try_from(character).unwrap_or(0);
        Self {
            character,
            default_name: character_name(character).to_owned(),
            text: initial_name(world, id),
            row: 0,
            col: 0,
            warning: false,
            revision: 0,
        }
    }

    /// Characters typed on the keyboard, filtered as the input field does
    /// (`ValidateInput`, `characterLimit`).
    pub fn type_text(&mut self, typed: &str) {
        for c in typed.chars() {
            self.push(c);
        }
    }

    /// True when `c` was added.
    fn push(&mut self, c: char) -> bool {
        let at = self.text.chars().count();
        if at >= NAME_LIMIT || validate_char(at, c).is_none() {
            return false;
        }
        self.text.push(c);
        self.warning = false;
        self.revision += 1;
        true
    }

    /// Deletes the last character (the input field's Backspace).
    pub fn backspace(&mut self) {
        if self.text.pop().is_some() {
            self.revision += 1;
        }
    }

    /// `OnResetButtonClick`: back to the default name.
    pub fn reset(&mut self) {
        self.text = filter_input(&self.default_name);
        self.warning = false;
        self.revision += 1;
    }

    /// `OnConfirmButtonClick`: trim and store, or show the warning when the
    /// name is empty. `SetCharacterName` then closes the screen.
    pub fn confirm(&mut self, world: &mut FieldWorld) -> NamePress {
        match confirm_name(&self.text) {
            Ok(name) => {
                world.character_names.insert(self.character, name);
                NamePress {
                    closed: true,
                    sound: Some(menu_sound::CONFIRM),
                }
            }
            Err(NameError::Empty) => {
                self.warning = true;
                self.revision += 1;
                sound(menu_sound::REFUSE)
            }
        }
    }

    fn cell_count(row: usize) -> usize {
        if row < GRID.len() {
            COLUMNS
        } else {
            COMMANDS.len()
        }
    }

    /// One tick's pad edges. Menu (Triangle) resets (`OnKeyMenu`); Cancel
    /// deletes a character (the grid's stand-in for Backspace).
    pub fn press(&mut self, pressed: i32, world: &mut FieldWorld) -> NamePress {
        let rows = GRID.len() + 1;
        let (row, col) = (self.row, self.col);
        if pressed & pad::UP != 0 {
            self.row = (self.row + rows - 1) % rows;
        }
        if pressed & pad::DOWN != 0 {
            self.row = (self.row + 1) % rows;
        }
        let cells = Self::cell_count(self.row);
        self.col = self.col.min(cells - 1);
        if pressed & pad::LEFT != 0 {
            self.col = (self.col + cells - 1) % cells;
        }
        if pressed & pad::RIGHT != 0 {
            self.col = (self.col + 1) % cells;
        }
        if (row, col) != (self.row, self.col) {
            self.revision += 1;
            return sound(menu_sound::CURSOR);
        }
        if pressed & pad::TRIANGLE != 0 {
            self.reset();
            return sound(menu_sound::CONFIRM);
        }
        if pressed & (pad::CANCEL | pad::CROSS) != 0 {
            self.backspace();
            return sound(menu_sound::CANCEL);
        }
        if pressed & (pad::CONFIRM | pad::CIRCLE) != 0 {
            if self.row < GRID.len() {
                let c = GRID[self.row].chars().nth(self.col).unwrap_or(' ');
                return sound(if self.push(c) {
                    menu_sound::CONFIRM
                } else {
                    menu_sound::REFUSE
                });
            }
            return match self.col {
                0 => sound(if self.push(' ') {
                    menu_sound::CONFIRM
                } else {
                    menu_sound::REFUSE
                }),
                1 => {
                    self.reset();
                    sound(menu_sound::CONFIRM)
                }
                _ => self.confirm(world),
            };
        }
        NamePress::default()
    }

    /// The frame to draw: the portrait and header, the name field, the
    /// character grid.
    pub fn frame(&self) -> MenuFrame {
        let mut frame = MenuFrame::default();
        let shown = format!("{}_", self.text);
        let mut head = vec![
            at(48.0, 0.0, "Name", Tint::Yellow, false),
            at(48.0, ROW, shown, Tint::White, false),
            at(288.0, 0.0, "Max 12", Tint::Dim, true),
        ];
        if self.warning {
            head.push(at(288.0, ROW, "Enter a name.", Tint::Red, true));
        }
        frame.windows.push(window(8.0, 8.0, 304.0, 2.0, head));
        frame.images.push(MenuImage {
            x: 16.0,
            y: 12.0,
            w: 28.0,
            h: 28.0,
            art: MenuArt::Face {
                character: self.character,
                dim: false,
            },
        });
        let mut texts: Vec<MenuText> = Vec::new();
        for (row, line) in GRID.iter().enumerate() {
            for (col, c) in line.chars().enumerate() {
                texts.push(at(
                    14.0 + col as f32 * 22.0,
                    row as f32 * ROW,
                    c.to_string(),
                    Tint::White,
                    false,
                ));
            }
        }
        for (col, label) in COMMANDS.iter().enumerate() {
            let tint = if col == 2 { Tint::Yellow } else { Tint::White };
            texts.push(at(
                14.0 + col as f32 * 70.0,
                GRID.len() as f32 * ROW,
                *label,
                tint,
                false,
            ));
        }
        frame.windows.push(window(8.0, 54.0, 304.0, 6.0, texts));
        let x = if self.row < GRID.len() {
            14.0 + self.col as f32 * 22.0
        } else {
            14.0 + self.col as f32 * 70.0
        };
        frame.pointer = Some((x - 4.0, 54.0 + 3.0 + (self.row as f32 + 0.5) * ROW));
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> FieldWorld {
        FieldWorld::default()
    }

    #[test]
    fn starts_with_the_default_name_and_confirm_stores_it_trimmed() {
        let mut w = world();
        let mut menu = NameMenu::new(&w, 1);
        assert_eq!(menu.text, "Vivi");
        menu.type_text(" X ");
        assert_eq!(menu.text, "Vivi X ");
        let done = menu.confirm(&mut w);
        assert!(done.closed);
        assert_eq!(
            w.character_names.get(&1).map(String::as_str),
            Some("Vivi X")
        );
        assert_eq!(display_name(&w, 1), "Vivi X");
        assert_eq!(display_name(&w, 2), "Dagger");
    }

    #[test]
    fn empty_name_is_refused_with_a_warning() {
        let mut w = world();
        let mut menu = NameMenu::new(&w, 0);
        for _ in 0..10 {
            menu.backspace();
        }
        let done = menu.confirm(&mut w);
        assert!(!done.closed);
        assert!(menu.warning);
        assert_eq!(done.sound, Some(menu_sound::REFUSE));
        assert!(w.character_names.is_empty());
    }

    #[test]
    fn typing_stops_at_twelve_and_rejects_a_leading_space() {
        let w = world();
        let mut menu = NameMenu::new(&w, 0);
        menu.text.clear();
        menu.type_text(" ABCDEFGHIJKLMNOP");
        assert_eq!(menu.text, "ABCDEFGHIJKL");
    }

    #[test]
    fn menu_button_resets_and_grid_confirm_types() {
        let mut w = world();
        let mut menu = NameMenu::new(&w, 4);
        menu.text = "Zzz".into();
        menu.press(pad::TRIANGLE, &mut w);
        assert_eq!(menu.text, "Freya");
        menu.press(pad::CONFIRM, &mut w);
        assert_eq!(menu.text, "FreyaA");
        menu.press(pad::CANCEL, &mut w);
        assert_eq!(menu.text, "Freya");
    }

    #[test]
    fn ok_cell_closes() {
        let mut w = world();
        let mut menu = NameMenu::new(&w, 3);
        menu.press(pad::UP, &mut w);
        menu.press(pad::RIGHT, &mut w);
        menu.press(pad::RIGHT, &mut w);
        assert_eq!((menu.row, menu.col), (5, 2));
        assert!(menu.press(pad::CONFIRM, &mut w).closed);
        assert_eq!(display_name(&w, 3), "Steiner");
    }

    #[test]
    fn a_character_past_the_stock_cast_starts_from_the_current_name() {
        let mut w = world();
        w.character_names.insert(2, "Garnet".into());
        let menu = NameMenu::new(&w, 14);
        assert_eq!(menu.character, 2);
        assert_eq!(menu.text, "Garnet");
    }
}
