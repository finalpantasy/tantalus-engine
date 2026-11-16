//! Card model, arrow mask, card creation. Sources: `QuadMistCard.cs`,
//! `Card/CardArrow.cs`, `Card/CardPool.cs`.

use super::{QuadError, QuadRng};

/// `CardPool.TOTAL_CARDS` (CardPool.cs:98). Ids at or above it are blocks.
pub const TOTAL_CARDS: usize = 100;
/// `CardArrow.MAX_ARROWNUM` (CardArrow.cs:110).
pub const MAX_ARROWS: usize = 8;

/// `QuadMistCard.Type` (QuadMistCard.cs:168). The byte value is the table byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardType {
    Physical = 0,
    Magic = 1,
    Flexible = 2,
    Assault = 3,
}

impl CardType {
    pub fn from_byte(b: u8) -> Result<Self, QuadError> {
        match b {
            0 => Ok(Self::Physical),
            1 => Ok(Self::Magic),
            2 => Ok(Self::Flexible),
            3 => Ok(Self::Assault),
            _ => Err(QuadError::BadTable(format!("card type byte {b}"))),
        }
    }
}

/// `QuadMistCard` (QuadMistCard.cs:158-166). `atk`, `pdef`, `mdef` are full
/// bytes used by the battle formula; the digit shown on the card is `value >> 4`
/// (QuadMistCard.cs:100-103). `arrow` bit `n` is direction `n` (see
/// [`direction_offset`]). `side` 0 is the player, 1 the enemy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Card {
    pub id: u8,
    pub side: u8,
    pub atk: u8,
    pub kind: CardType,
    pub pdef: u8,
    pub mdef: u8,
    pub cpoint: u8,
    pub arrow: u8,
}

impl Card {
    /// `new QuadMistCard()` defaults.
    pub fn blank() -> Self {
        Self {
            id: 0,
            side: 0,
            atk: 0,
            kind: CardType::Physical,
            pdef: 0,
            mdef: 0,
            cpoint: 0,
            arrow: 0,
        }
    }

    /// `CardPool.GetBlockCard(id)` (CardPool.cs:90): only the id is set.
    pub fn block(block_id: u8) -> Self {
        Self {
            id: TOTAL_CARDS as u8 + block_id,
            ..Self::blank()
        }
    }

    /// `QuadMistCard.IsBlock` (QuadMistCard.cs:110).
    pub fn is_block(&self) -> bool {
        self.id as usize >= TOTAL_CARDS
    }

    /// `ArrowNumber` (QuadMistCard.cs:108), a bit count.
    pub fn arrow_number(&self) -> u32 {
        self.arrow.count_ones()
    }

    /// `isTheSameCard` (QuadMistCard.cs:112-151): id, atk, arrow, type, pdef, mdef.
    pub fn is_same_card(&self, o: &Card) -> bool {
        self.id == o.id
            && self.atk == o.atk
            && self.arrow == o.arrow
            && self.kind == o.kind
            && self.pdef == o.pdef
            && self.mdef == o.mdef
    }

    /// `LevelUpInMatch` (QuadMistCard.cs:27-44). One `Range(0, 3)` draw; the
    /// stat rises by one unless it already equals the table maximum.
    pub fn level_up_in_match(&mut self, max: &Card, rng: &mut dyn QuadRng) {
        match rng.range(0, 3) {
            0 if self.atk != max.atk => self.atk = self.atk.wrapping_add(1),
            1 if self.pdef != max.pdef => self.pdef = self.pdef.wrapping_add(1),
            2 if self.mdef != max.mdef => self.mdef = self.mdef.wrapping_add(1),
            _ => {}
        }
    }

    /// `LevelUpInBattle` (QuadMistCard.cs:46-67). Physical and Magic draw
    /// `Range(0, 64)`, Flexible draws `Range(0, 128)`, Assault draws nothing.
    pub fn level_up_in_battle(&mut self, rng: &mut dyn QuadRng) {
        match self.kind {
            CardType::Physical | CardType::Magic => {
                if rng.range(0, 64) == 0 {
                    self.kind = CardType::Flexible;
                }
            }
            CardType::Flexible => {
                if rng.range(0, 128) == 0 {
                    self.kind = CardType::Assault;
                }
            }
            CardType::Assault => {}
        }
    }
}

/// `CardArrow.ToOffset` (CardArrow.cs:43-79): `(dx, dy)` with y growing down.
/// Directions: 0 up, 1 right-up, 2 right, 3 right-down, 4 down, 5 left-down,
/// 6 left, 7 left-up.
pub fn direction_offset(dir: usize) -> (i32, i32) {
    match dir {
        0 => (0, -1),
        1 => (1, -1),
        2 => (1, 0),
        3 => (1, 1),
        4 => (0, 1),
        5 => (-1, 1),
        6 => (-1, 0),
        7 => (-1, -1),
        _ => (0, 0),
    }
}

/// `CardArrow.Reverse` (CardArrow.cs:81): swap nibbles, so bit `n` becomes bit
/// `(n + 4) % 8` (the opposite direction).
pub fn reverse_arrow(arrow: u8) -> u8 {
    arrow.rotate_right(4)
}

/// `CardArrow.HasDirection` (CardArrow.cs:86).
pub fn has_direction(arrow: u8, dir: usize) -> bool {
    (arrow as i32 & (1 << dir)) > 0
}

/// `CardArrow.CheckDirection` (CardArrow.cs:91-108): 2 both point at each
/// other, 1 only `arrow1` points at the neighbor, -1 only the neighbor points
/// back, 0 neither.
pub fn check_direction(arrow1: u8, arrow2: u8, dir: usize) -> i32 {
    let a = has_direction(arrow1, dir);
    let b = has_direction(reverse_arrow(arrow2), dir);
    match (a, b) {
        (true, true) => 2,
        (true, false) => 1,
        (false, true) => -1,
        (false, false) => 0,
    }
}

/// `CardPool.probability` (CardPool.cs:106): percent weight per arrow count 0..=8.
const ARROW_COUNT_WEIGHT: [i32; 9] = [1, 8, 25, 31, 18, 9, 5, 2, 1];

/// `CardPool.RandomCardArrow` (CardPool.cs:43-82). Draws: one `Range(0,100)`,
/// then eight `Range(0,8)` swaps of the direction list.
pub fn random_card_arrow(rng: &mut dyn QuadRng) -> u8 {
    let num = rng.range(0, 100);
    let mut acc = 0;
    let mut count = 0usize;
    while count < ARROW_COUNT_WEIGHT.len() {
        if num >= acc && num < acc + ARROW_COUNT_WEIGHT[count] {
            break;
        }
        acc += ARROW_COUNT_WEIGHT[count];
        count += 1;
    }
    let mut dirs = [0usize, 1, 2, 3, 4, 5, 6, 7];
    for j in 0..8 {
        let k = rng.range(0, 8) as usize;
        dirs.swap(j, k);
    }
    let mut arrow = 0u8;
    for d in dirs.iter().take(count) {
        arrow |= 1 << d;
    }
    arrow
}

/// `CardPool.CreateQuadMistCard` (CardPool.cs:30-41), from the table maximum.
/// Draw order: atk, pdef, mdef (`Range(0, stat / 2)` each), `Range(0, 127)`
/// (zero makes the card Flexible), then the arrow draws.
pub fn create_card(max: &Card, rng: &mut dyn QuadRng) -> Card {
    let mut c = *max;
    c.atk = (c.atk / 2).wrapping_add(rng.range(0, (c.atk / 2) as i32) as u8);
    c.pdef = (c.pdef / 2).wrapping_add(rng.range(0, (c.pdef / 2) as i32) as u8);
    c.mdef = (c.mdef / 2).wrapping_add(rng.range(0, (c.mdef / 2) as i32) as u8);
    if rng.range(0, 127) == 0 {
        c.kind = CardType::Flexible;
    }
    c.arrow = random_card_arrow(rng);
    c
}
