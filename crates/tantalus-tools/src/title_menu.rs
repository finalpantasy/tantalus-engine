//! The title screen's menu (`TitleUI`): Continue, New Game and Load Game.
//!
//! `TitleUI.OnKeyConfirm`: Continue loads the autosave when there is one
//! (`OnContinueButtonClick`, sound 1261, else the refusal 102), New Game starts
//! the opening field (`OnNewGameButtonClick`, sound 3096) and Load Game opens
//! the load screen (`OnLoadGameButtonClick`, sound 103). The play window has no
//! autosave; Continue loads the save slot written last.
//!
//! The PS1 skin shows the disc's own title menu instead: the NEW GAME and CONTINUE words of the
//! title file (resident file 60) under the logo, and CONTINUE opens the list of saves (the
//! PS1 manual: "A menu of saved game data will appear").

use crate::field_menu::{MenuFrame, MenuText, MenuWindow, ROW, Tint};
use crate::play::pad;

/// The commands, in `TitleUI`'s order (the cloud and language buttons are left out).
pub const COMMANDS: [&str; 3] = ["Continue", "New Game", "Load Game"];

/// The PS1 title's commands, top to bottom.
pub const PS1_COMMANDS: [&str; 2] = ["NEW GAME", "CONTINUE"];

/// Where the PS1 title draws its words on the 320 x 224 screen: the top-left corner and the
/// size in pixels of NEW GAME, CONTINUE and the copyright line (the title file's TIMs 11, 12
/// and 13). Placed by eye, centered under the logo; not measured on a PS1.
pub const PS1_WORDS: [(f32, f32, f32, f32); 3] = [
    (127.0, 150.0, 66.0, 10.0),
    (128.0, 166.0, 64.0, 10.0),
    (119.0, 198.0, 82.0, 18.0),
];

/// What a press on the title asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleChoice {
    Continue,
    NewGame,
    LoadGame,
}

/// The open title screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleScreen {
    pub cursor: usize,
    /// Whether Continue has a save to load (`TitleUI.canContinue`).
    pub can_continue: bool,
    /// Counts presses; a client redraws when it changes.
    pub revision: u64,
    /// The PS1 title (NEW GAME and CONTINUE, drawn from the disc's art) instead of `TitleUI`.
    pub ps1: bool,
}

impl TitleScreen {
    /// The title with the cursor on Continue when there is a save, else on New Game.
    pub fn new(can_continue: bool) -> Self {
        Self {
            cursor: usize::from(!can_continue),
            can_continue,
            revision: 0,
            ps1: false,
        }
    }

    /// The PS1 title: the cursor on CONTINUE when there is a save, else on NEW GAME.
    pub fn ps1(can_continue: bool) -> Self {
        Self {
            cursor: usize::from(can_continue),
            can_continue,
            revision: 0,
            ps1: true,
        }
    }

    /// One pad press (edge bits): the choice confirmed, if any.
    pub fn press(&mut self, pressed: i32) -> Option<TitleChoice> {
        self.revision += 1;
        let count = if self.ps1 {
            PS1_COMMANDS.len()
        } else {
            COMMANDS.len()
        };
        self.cursor = crate::field_menu::step(self.cursor, count, pressed);
        if pressed & (pad::CONFIRM | pad::CIRCLE) == 0 {
            return None;
        }
        if self.ps1 {
            // CONTINUE lists the saves.
            return Some(match self.cursor {
                0 => TitleChoice::NewGame,
                _ => TitleChoice::LoadGame,
            });
        }
        Some(match self.cursor {
            0 => TitleChoice::Continue,
            1 => TitleChoice::NewGame,
            _ => TitleChoice::LoadGame,
        })
    }

    /// The menu window, centered low on the screen as `TitleUI` lays it out.
    pub fn frame(&self) -> MenuFrame {
        if self.ps1 {
            // The words are the title picture's (the client draws them); only the pointer
            // here, left of the word under the cursor, in the picture's scale (224 lines shown
            // as 240).
            let (x, y, _, h) = PS1_WORDS[self.cursor.min(PS1_COMMANDS.len() - 1)];
            let scale = 240.0 / 224.0;
            let (px, py) = (x - 8.0, y + h / 2.0);
            return MenuFrame {
                windows: Vec::new(),
                pointer: Some((160.0 + (px - 160.0) * scale, 112.0 - (112.0 - py) * scale)),
            };
        }
        let (x, y, w) = (116.0, 150.0, 88.0);
        let texts = COMMANDS
            .iter()
            .enumerate()
            .map(|(row, name)| MenuText {
                x: 16.0,
                y: row as f32 * ROW,
                text: (*name).to_owned(),
                tint: if row == 0 && !self.can_continue {
                    Tint::Dim
                } else {
                    Tint::White
                },
                right: false,
            })
            .collect();
        MenuFrame {
            windows: vec![MenuWindow {
                x,
                y,
                w,
                h: COMMANDS.len() as f32 * ROW + 6.0,
                texts,
            }],
            pointer: Some((x + 4.0, y + 3.0 + (self.cursor as f32 + 0.5) * ROW)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cursor_starts_on_continue_only_with_a_save() {
        assert_eq!(TitleScreen::new(true).cursor, 0);
        assert_eq!(TitleScreen::new(false).cursor, 1);
    }

    #[test]
    fn up_down_and_confirm_pick_a_command() {
        let mut title = TitleScreen::new(true);
        assert_eq!(title.press(pad::DOWN), None);
        assert_eq!(title.press(pad::CONFIRM), Some(TitleChoice::NewGame));
        assert_eq!(title.press(pad::DOWN), None);
        assert_eq!(title.press(pad::CONFIRM), Some(TitleChoice::LoadGame));
        assert_eq!(title.press(pad::DOWN), None);
        assert_eq!(title.press(pad::CONFIRM), Some(TitleChoice::Continue));
        let frame = title.frame();
        assert_eq!(frame.windows[0].texts.len(), 3);
        assert_eq!(frame.windows[0].texts[1].text, "New Game");
    }

    #[test]
    fn the_ps1_title_has_new_game_and_continue_which_lists_the_saves() {
        let mut title = TitleScreen::ps1(true);
        assert_eq!(title.cursor, 1, "on CONTINUE with a save");
        assert_eq!(TitleScreen::ps1(false).cursor, 0);
        assert_eq!(title.press(pad::CONFIRM), Some(TitleChoice::LoadGame));
        assert_eq!(title.press(pad::UP), None);
        assert_eq!(title.press(pad::CONFIRM), Some(TitleChoice::NewGame));
        assert_eq!(title.press(pad::UP), None);
        assert_eq!(title.cursor, 1, "wraps to CONTINUE");
        let frame = title.frame();
        assert!(frame.windows.is_empty(), "the words are the picture's");
        let (_, y) = frame.pointer.expect("pointer");
        let (_, top, _, h) = PS1_WORDS[1];
        assert!((y - (112.0 - (112.0 - (top + h / 2.0)) * 240.0 / 224.0)).abs() < 1e-3);
    }
}
