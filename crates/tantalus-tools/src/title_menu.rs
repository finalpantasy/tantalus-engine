//! The title screen's menu (`TitleUI`): Continue, New Game and Load Game.
//!
//! `TitleUI.OnKeyConfirm`: Continue loads the autosave when there is one
//! (`OnContinueButtonClick`, sound 1261, else the refusal 102), New Game starts
//! the opening field (`OnNewGameButtonClick`, sound 3096) and Load Game opens
//! the load screen (`OnLoadGameButtonClick`, sound 103). The play window has no
//! autosave; Continue loads the save slot written last.

use crate::field_menu::{MenuFrame, MenuText, MenuWindow, ROW, Tint};
use crate::play::pad;

/// The commands, in `TitleUI`'s order (the cloud and language buttons are left out).
pub const COMMANDS: [&str; 3] = ["Continue", "New Game", "Load Game"];

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
}

impl TitleScreen {
    /// The title with the cursor on Continue when there is a save, else on New Game.
    pub fn new(can_continue: bool) -> Self {
        Self {
            cursor: usize::from(!can_continue),
            can_continue,
            revision: 0,
        }
    }

    /// One pad press (edge bits): the choice confirmed, if any.
    pub fn press(&mut self, pressed: i32) -> Option<TitleChoice> {
        self.revision += 1;
        self.cursor = crate::field_menu::step(self.cursor, COMMANDS.len(), pressed);
        if pressed & (pad::CONFIRM | pad::CIRCLE) == 0 {
            return None;
        }
        Some(match self.cursor {
            0 => TitleChoice::Continue,
            1 => TitleChoice::NewGame,
            _ => TitleChoice::LoadGame,
        })
    }

    /// The menu window, centered low on the screen as `TitleUI` lays it out.
    pub fn frame(&self) -> MenuFrame {
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
}
