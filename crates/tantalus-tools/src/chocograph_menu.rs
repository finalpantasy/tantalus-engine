//! The Chocograph list the world script opens with `OpenMenu 5`
//! (`ChocographUI`, `Global/ChocographUI.cs`): a Select / Cancel sub menu
//! over a list of the pictures the player found. Confirm on a picture
//! stores `hintmap_id` (`OnKeyConfirm`, the item group); Cancel on the sub
//! menu clears it; the cancel key leaves the list, or the item group. The
//! rules are `tantalus_legacy::chocobo`; this file is the cursor and the
//! frame the window draws.

use std::collections::BTreeMap;

use tantalus_legacy::chocobo::chocograph::OUT_OF_REACH_TEXT;
use tantalus_legacy::chocobo::{ChocographBook, HINT_MAP_MAX};

use crate::field_menu::{MenuFrame, MenuText, ROW, Tint, step, window};
use crate::play::pad;

/// Rows of the picture list in view.
const LIST_ROWS: usize = 8;
const SUB_MENU: [&str; 2] = ["Select", "Cancel"];
/// Characters to a line of the hint text.
const HINT_WIDTH: usize = 46;
const TOP: f32 = 40.0;

/// Where the cursor is (`ChocographUI.SubMenuGroupButton` or
/// `ItemGroupButton`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    Sub(usize),
    /// A row of [`ChocographScreen::rows`].
    Items(usize),
}

/// What a key press did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChocographEvent {
    Nothing,
    Moved,
    Confirmed,
    Refused,
    Cancelled,
    /// The list closed.
    Closed,
}

/// The open list: the book (found pictures, reach, pick) and the cursor.
#[derive(Debug, Clone)]
pub struct ChocographScreen {
    pub book: ChocographBook,
    group: Group,
    /// `FF9TextTool.ChocoboUIText` by id (`etc/ff9choco`).
    texts: BTreeMap<i32, String>,
    /// Changes whenever the frame does.
    pub revision: u64,
}

impl ChocographScreen {
    pub fn new(book: ChocographBook, texts: BTreeMap<i32, String>) -> Self {
        Self {
            book,
            group: Group::Sub(0),
            texts,
            revision: 0,
        }
    }

    /// The pictures shown: the ones the player found (`hasMap`), in order.
    pub fn rows(&self) -> Vec<usize> {
        (0..HINT_MAP_MAX)
            .filter(|index| self.book.has_map(*index))
            .collect()
    }

    fn text(&self, id: usize, fallback: String) -> String {
        self.texts
            .get(&(id as i32))
            .filter(|text| !text.is_empty())
            .cloned()
            .unwrap_or(fallback)
    }

    fn name(&self, picture: usize) -> String {
        let id = self.book.entry(picture).map_or(0, |entry| entry.name_text);
        self.text(id, format!("Chocograph {}", picture + 1))
    }

    /// One key press (`OnKeyConfirm`, `OnKeyCancel`, the item select).
    pub fn press(&mut self, pressed: i32) -> ChocographEvent {
        self.revision += 1;
        let confirm = pressed & (pad::CONFIRM | pad::CIRCLE) != 0;
        let cancel = pressed & (pad::CANCEL | pad::CROSS) != 0;
        let rows = self.rows();
        match self.group {
            Group::Sub(cursor) => {
                let next = step(cursor, SUB_MENU.len(), pressed);
                self.group = Group::Sub(next);
                if cancel {
                    return ChocographEvent::Closed;
                }
                if !confirm {
                    return moved(next != cursor);
                }
                if next == 0 {
                    if rows.is_empty() {
                        return ChocographEvent::Refused;
                    }
                    let row = rows
                        .iter()
                        .position(|picture| *picture as i32 == self.book.selected)
                        .unwrap_or(0);
                    self.group = Group::Items(row);
                    ChocographEvent::Confirmed
                } else if self.book.cancel_selection() {
                    ChocographEvent::Cancelled
                } else {
                    ChocographEvent::Refused
                }
            }
            Group::Items(row) => {
                let next = step(row, rows.len(), pressed);
                self.group = Group::Items(next);
                if cancel {
                    self.group = Group::Sub(0);
                    return ChocographEvent::Cancelled;
                }
                if !confirm {
                    return moved(next != row);
                }
                match rows.get(next).map(|picture| self.book.select(*picture)) {
                    Some(Ok(())) => ChocographEvent::Confirmed,
                    _ => ChocographEvent::Refused,
                }
            }
        }
    }

    /// The windows: the title with the counts, the sub menu, the list and
    /// the hint of the picture under the cursor.
    pub fn frame(&self) -> MenuFrame {
        let mut frame = MenuFrame::default();
        let text = |x: f32, row: f32, value: String, tint: Tint, right: bool| MenuText {
            x,
            y: row * ROW,
            text: value,
            tint,
            right,
        };
        let rows = self.rows();
        let selected = self.book.selected;
        let title = if selected >= 0 {
            format!("Chocograph  {}", self.name(selected as usize))
        } else {
            "Chocograph".to_owned()
        };
        let counts = format!(
            "Maps {}   Treasures {}",
            self.book.map_count(),
            self.book.gem_count()
        );
        frame.windows.push(window(
            8.0,
            8.0,
            304.0,
            1.0,
            vec![
                text(8.0, 0.0, title, Tint::White, false),
                text(296.0, 0.0, counts, Tint::Yellow, true),
            ],
        ));
        let sub = SUB_MENU
            .iter()
            .enumerate()
            .map(|(index, label)| {
                // Cancel is grey while nothing is picked (`SetCancelButton`).
                let dim = index == 1 && selected < 0;
                let tint = if dim { Tint::Dim } else { Tint::White };
                text(12.0, index as f32, (*label).to_owned(), tint, false)
            })
            .collect();
        frame.windows.push(window(8.0, TOP, 76.0, 2.0, sub));
        let cursor = match self.group {
            Group::Items(row) => row,
            Group::Sub(_) => rows
                .iter()
                .position(|picture| *picture as i32 == selected)
                .unwrap_or(0),
        };
        let first = cursor
            .saturating_sub(LIST_ROWS - 1)
            .min(rows.len().saturating_sub(LIST_ROWS));
        let mut texts = Vec::new();
        for (line, row) in (first..rows.len().min(first + LIST_ROWS)).enumerate() {
            let picture = rows[row];
            let tint = if picture as i32 == selected {
                Tint::Green
            } else if self.book.can_reach(picture) {
                Tint::White
            } else {
                Tint::Dim
            };
            texts.push(text(14.0, line as f32, self.name(picture), tint, false));
            if self.book.has_gem(picture) {
                let opened = "Opened".to_owned();
                texts.push(text(206.0, line as f32, opened, Tint::Dim, true));
            }
        }
        if rows.is_empty() {
            let none = "No Chocograph found".to_owned();
            texts.push(text(14.0, 0.0, none, Tint::Dim, false));
        }
        let list = window(92.0, TOP, 220.0, LIST_ROWS as f32, texts);
        frame.windows.push(list);
        // `DisplayHint`: the picture the cursor is on, while in the list.
        let mut lines = Vec::new();
        if let (Group::Items(_), Some(picture)) = (self.group, rows.get(cursor)) {
            let id = match self.book.entry(*picture) {
                Some(entry) if entry.can_reach => entry.detail_text,
                _ => OUT_OF_REACH_TEXT,
            };
            lines = wrap(&self.text(id, String::new()), HINT_WIDTH);
        }
        let hint = lines
            .into_iter()
            .take(3)
            .enumerate()
            .map(|(index, line)| text(8.0, index as f32, line, Tint::White, false))
            .collect();
        frame.windows.push(window(8.0, 164.0, 304.0, 3.0, hint));
        frame.pointer = Some(match self.group {
            Group::Sub(index) => (10.0, TOP + 3.0 + (index as f32 + 0.5) * ROW),
            Group::Items(row) => (
                94.0,
                TOP + 3.0 + (row.saturating_sub(first) as f32 + 0.5) * ROW,
            ),
        });
        frame
    }
}

fn moved(did: bool) -> ChocographEvent {
    if did {
        ChocographEvent::Moved
    } else {
        ChocographEvent::Nothing
    }
}

/// Breaks `text` at spaces into lines of at most `width` characters.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}
