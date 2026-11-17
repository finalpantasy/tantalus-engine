//! Drawing a Tetra Master game on a CPU frame (see [`crate::card_view`] for the art).

use tantalus_game::cards::{Phase, Slot, View};
use tantalus_legacy::quad::{Card, CardType, Outcome};

use crate::battle_raster::Frame;
use crate::card_view::CardArt;

/// Size of the frame (`battle_view::LIVE_SIZE`).
pub const SIZE: (u32, u32) = (640, 480);
/// The board image's scale: its 1080 rows fill the 480-row frame.
const BOARD_SCALE: f32 = 480.0 / 1080.0;
/// A card's size on the board, in frame pixels (the art is 204x248).
const CELL: (i32, i32) = (90, 110);
const BOARD_ORIGIN: (i32, i32) = (135, 18);
/// A card in a hand.
const HAND_CARD: (i32, i32) = (80, 98);
const WHITE: [u8; 3] = [255, 255, 255];
const YELLOW: [u8; 3] = [255, 224, 96];
const BLUE: [u8; 3] = [120, 170, 255];
const RED: [u8; 3] = [255, 120, 110];
const GRAY: [u8; 3] = [170, 170, 190];

/// Top-left of board cell `cell` (0 to 15, row-major).
fn cell_origin(cell: usize) -> (i32, i32) {
    (
        BOARD_ORIGIN.0 + (cell % 4) as i32 * CELL.0,
        BOARD_ORIGIN.1 + (cell / 4) as i32 * (CELL.1 + 1),
    )
}

/// Top-left of hand card `index` of `side`.
fn hand_origin(side: usize, index: usize, lifted: bool) -> (i32, i32) {
    let x = if side == 0 {
        20
    } else {
        640 - 20 - HAND_CARD.0
    };
    let shift = match (lifted, side) {
        (false, _) => 0,
        (true, 0) => 14,
        (true, _) => -14,
    };
    (x + shift, 4 + index as i32 * 86)
}

/// The arrow sprite of direction `dir` (0 up, then clockwise) and where it sits in the
/// 204x248 card.
fn arrow_slot(dir: usize) -> (&'static str, i32, i32) {
    match dir {
        0 => ("card_arrow_top.png", 82, 2),
        1 => ("card_arrow_righttop.png", 162, 2),
        2 => ("card_arrow_right.png", 162, 104),
        3 => ("card_arrow_rightbuttom.png", 162, 206),
        4 => ("card_arrow_buttom.png", 82, 206),
        5 => ("card_arrow_leftbuttom.png", 2, 206),
        6 => ("card_arrow_left.png", 2, 104),
        _ => ("card_arrow_lefttop.png", 2, 2),
    }
}

/// The digit a stat shows on a card: the byte's high nibble in hex.
fn digit(value: u8) -> char {
    char::from_digit(u32::from(value >> 4), 16)
        .unwrap_or('0')
        .to_ascii_uppercase()
}

fn kind_letter(kind: CardType) -> char {
    match kind {
        CardType::Physical => 'P',
        CardType::Magic => 'M',
        CardType::Flexible => 'X',
        CardType::Assault => 'A',
    }
}

fn outline(frame: &mut Frame, x: i32, y: i32, width: i32, height: i32, color: [u8; 3]) {
    frame.rect(x, y, width, 1, color, 1.0);
    frame.rect(x, y + height - 1, width, 1, color, 1.0);
    frame.rect(x, y, 1, height, color, 1.0);
    frame.rect(x + width - 1, y, 1, height, color, 1.0);
}

/// `text` centered across the frame at row `y`.
fn banner(frame: &mut Frame, text: &str, scale: i32, y: i32, color: [u8; 3]) {
    let width = text.chars().count() as i32 * 6 * scale;
    frame.text((640 - width) / 2, y, text, scale, color);
}

impl CardArt {
    /// One card at `origin` in a `size` box: background by owner, the art, the frame, the
    /// arrows and the stats (attack, type, physical and magical defense).
    pub fn card(
        &self,
        frame: &mut Frame,
        card: &Card,
        side: u8,
        origin: (i32, i32),
        size: (i32, i32),
    ) {
        let (x, y) = origin;
        let (width, height) = size;
        let (bg, rim) = if side == 0 {
            ("card_player_bg.png", "card_player_frame.png")
        } else {
            ("card_opponent_bg.png", "card_opponent_frame.png")
        };
        let kx = width as f32 / 204.0;
        let ky = height as f32 / 248.0;
        let at = |value: i32, scale: f32| (value as f32 * scale).round() as i32;
        self.blit(frame, bg, x, y, width, height);
        self.blit(
            frame,
            &format!("card_{:02}.png", card.id.min(99)),
            x + at(18, kx),
            y + at(20, ky),
            at(168, kx),
            at(208, ky),
        );
        self.blit(frame, rim, x, y, width, height);
        for dir in 0..8 {
            if card.arrow >> dir & 1 != 0 {
                let (name, ax, ay) = arrow_slot(dir);
                self.blit(
                    frame,
                    name,
                    x + at(ax, kx),
                    y + at(ay, ky),
                    at(39, kx),
                    at(39, ky),
                );
            }
        }
        let stats = format!(
            "{}{}{}{}",
            digit(card.atk),
            kind_letter(card.kind),
            digit(card.pdef),
            digit(card.mdef)
        );
        let scale = (width / 40).max(1);
        let text_width = stats.chars().count() as i32 * 6 * scale;
        let left = x + (width - text_width) / 2;
        let top = y + height - 8 * scale - 5;
        frame.rect(
            left - 2,
            top - 1,
            text_width + 4,
            8 * scale + 2,
            [0, 0, 0],
            0.55,
        );
        frame.text(left, top, &stats, scale, WHITE);
    }

    /// The whole game.
    pub fn render(&self, view: &View<'_>) -> Frame {
        let mut frame = Frame::new(SIZE.0, SIZE.1, [8, 8, 24, 255]);
        let wide = (1008.0 * BOARD_SCALE) as i32;
        self.blit(
            &mut frame,
            "card_mg.png",
            (SIZE.0 as i32 - wide) / 2,
            0,
            wide,
            480,
        );
        if matches!(view.phase, Phase::Select) {
            self.render_select(&mut frame, view);
            return frame;
        }
        self.render_board(&mut frame, view);
        self.render_overlay(&mut frame, view);
        frame
    }

    fn render_select(&self, frame: &mut Frame, view: &View<'_>) {
        frame.rect(0, 0, 640, 480, [0, 0, 16], 0.7);
        frame.text(24, 14, "TETRA MASTER - CHOOSE 5 CARDS", 2, YELLOW);
        let columns = 6;
        let per_page = columns * 3;
        let page = view.cursor.min(view.collection.len().saturating_sub(1)) / per_page;
        for slot in 0..per_page {
            let index = page * per_page + slot;
            let Some(card) = view.collection.get(index) else {
                break;
            };
            let origin = (
                24 + (slot % columns) as i32 * 100,
                50 + (slot / columns) as i32 * 118,
            );
            self.card(frame, card, 0, origin, CELL);
            if let Some(order) = view.selected.iter().position(|chosen| *chosen == index) {
                frame.rect(origin.0, origin.1, CELL.0, CELL.1, YELLOW, 0.25);
                frame.text(
                    origin.0 + 4,
                    origin.1 + 4,
                    &(order + 1).to_string(),
                    3,
                    YELLOW,
                );
            }
            if index == view.cursor {
                outline(frame, origin.0 - 2, origin.1 - 2, 94, 114, YELLOW);
                self.blit(
                    frame,
                    "cursor_hand_choice.png",
                    origin.0 + 50,
                    origin.1 + 70,
                    61,
                    35,
                );
            }
        }
        let ready = view.selected.len() == 5;
        frame.text(
            24,
            430,
            &format!("{}/5 CHOSEN", view.selected.len()),
            2,
            if ready { YELLOW } else { GRAY },
        );
        let start = view.cursor == view.collection.len();
        frame.text(
            300,
            430,
            if start { ">> START <<" } else { "START" },
            2,
            if start { YELLOW } else { GRAY },
        );
        frame.text(24, 456, &format!("PAGE {}", page + 1), 1, GRAY);
    }

    fn render_board(&self, frame: &mut Frame, view: &View<'_>) {
        for cell in 0..16 {
            let origin = cell_origin(cell);
            match view.board[cell] {
                Slot::Empty => {}
                Slot::Block(id) => {
                    let name = if id % 2 == 0 {
                        "block_a.png"
                    } else {
                        "block_b.png"
                    };
                    self.blit(frame, name, origin.0, origin.1, CELL.0, CELL.1);
                }
                Slot::Card { card, side } => self.card(frame, &card, side, origin, CELL),
            }
        }
        let choosing = matches!(view.phase, Phase::Hand | Phase::Cell);
        for side in 0..2 {
            for (index, card) in view.hands[side].iter().enumerate() {
                let lifted = side == 0 && choosing && index == view.cursor;
                let origin = hand_origin(side, index, lifted);
                self.card(frame, card, side as u8, origin, HAND_CARD);
                if lifted && matches!(view.phase, Phase::Hand) {
                    outline(
                        frame,
                        origin.0 - 1,
                        origin.1 - 1,
                        HAND_CARD.0 + 2,
                        HAND_CARD.1 + 2,
                        YELLOW,
                    );
                    self.blit(
                        frame,
                        "cursor_hand_choice.png",
                        origin.0 + HAND_CARD.0 - 30,
                        origin.1 + HAND_CARD.1 / 2 - 8,
                        61,
                        35,
                    );
                }
            }
        }
        frame.text(20, 452, &format!("YOU {}", view.scores.0), 3, BLUE);
        frame.text(
            640 - 20 - 7 * 18,
            452,
            &format!("FOE {}", view.scores.1),
            3,
            RED,
        );
        if matches!(view.phase, Phase::Cell) {
            let origin = cell_origin(view.cell_cursor);
            outline(frame, origin.0, origin.1, CELL.0, CELL.1, YELLOW);
            outline(
                frame,
                origin.0 + 1,
                origin.1 + 1,
                CELL.0 - 2,
                CELL.1 - 2,
                YELLOW,
            );
        }
        if let Phase::Target { candidates } = view.phase {
            for (index, cell) in candidates.iter().enumerate() {
                let origin = cell_origin(*cell);
                let color = if index == view.choice { YELLOW } else { GRAY };
                outline(frame, origin.0, origin.1, CELL.0, CELL.1, color);
                outline(
                    frame,
                    origin.0 + 1,
                    origin.1 + 1,
                    CELL.0 - 2,
                    CELL.1 - 2,
                    color,
                );
            }
        }
        if let Some(popup) = view.popup {
            for (cell, value, start, won) in [
                (
                    popup.attacker,
                    popup.attack_finish,
                    popup.attack,
                    popup.attacker_won,
                ),
                (
                    popup.defender,
                    popup.defense_finish,
                    popup.defense,
                    !popup.attacker_won,
                ),
            ] {
                let origin = cell_origin(cell);
                frame.rect(origin.0, origin.1 + 34, CELL.0, 44, [0, 0, 0], 0.7);
                frame.text(origin.0 + 4, origin.1 + 36, &format!("{start}>"), 1, GRAY);
                let text = value.to_string();
                let width = text.chars().count() as i32 * 18;
                frame.text(
                    origin.0 + (CELL.0 - width) / 2,
                    origin.1 + 48,
                    &text,
                    3,
                    if won { YELLOW } else { WHITE },
                );
            }
        }
    }

    fn render_overlay(&self, frame: &mut Frame, view: &View<'_>) {
        match view.phase {
            Phase::Coin { ticks } => {
                let step = (70_u32.saturating_sub(*ticks) / 9) % 8 + 1;
                self.blit(frame, &format!("coin_{step:02}.png"), 229, 150, 182, 173);
                let text = if view.coin == Some(0) {
                    "YOU GO FIRST"
                } else {
                    "FOE GOES FIRST"
                };
                banner(frame, text, 2, 340, YELLOW);
            }
            Phase::Think { .. } => banner(frame, "FOE IS THINKING", 2, 4, GRAY),
            Phase::Banner { .. } => {
                if let Some((outcome, perfect)) = view.result {
                    let (text, color) = match (outcome, perfect) {
                        (Outcome::Win, true) => ("PERFECT", YELLOW),
                        (Outcome::Win, false) => ("YOU WIN", BLUE),
                        (Outcome::Lose, _) => ("YOU LOSE", RED),
                        (Outcome::Draw, _) => ("DRAW", WHITE),
                    };
                    frame.rect(0, 190, 640, 100, [0, 0, 0], 0.75);
                    banner(frame, text, 8, 206, color);
                }
            }
            Phase::Prize => {
                frame.rect(0, 120, 640, 230, [0, 0, 16], 0.85);
                banner(frame, "TAKE ONE OF THE FOES CARDS", 2, 130, YELLOW);
                for (slot, index) in view.prize.iter().enumerate() {
                    let Some(card) = view.enemy_cards.get(*index) else {
                        continue;
                    };
                    let origin = (320 - view.prize.len() as i32 * 50 + slot as i32 * 100, 180);
                    self.card(frame, card, 1, origin, CELL);
                    if slot == view.choice {
                        outline(frame, origin.0 - 2, origin.1 - 2, 94, 114, YELLOW);
                    }
                }
            }
            Phase::Take { .. } => {
                if let Some(card) = view.transfers.first() {
                    frame.rect(0, 120, 640, 230, [0, 0, 16], 0.85);
                    let gained = card.side == 0;
                    banner(
                        frame,
                        if gained { "CARD GAINED" } else { "CARD LOST" },
                        3,
                        130,
                        if gained { YELLOW } else { RED },
                    );
                    self.card(frame, card, card.side, (275, 190), CELL);
                }
            }
            _ => {}
        }
    }
}
