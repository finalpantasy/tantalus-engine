//! Tetra Master (the card game) as the field plays it.
//!
//! The rules are the legacy core in `tantalus_legacy::quad` (board, battle math, enemy AI,
//! settlement). This module keeps what the game saves ([`CardRecord`], [`CardCounts`]) and
//! [`session::CardGame`], the state machine a play loop drives: card choice, coin toss, the
//! turns, the result and the prize card.

pub mod session;
pub use session::{CardEvent, CardGame, Input, Phase, Popup, Slot, View};

use serde::{Deserialize, Serialize};
use tantalus_legacy::quad::{
    Card, CardTable, CardType, EnemyTables, QuadRng, TOTAL_CARDS, create_card,
};

use crate::field::world::FieldWorld;

/// A card in the player's collection (`QuadMistCard`): the stats a game plays with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardRecord {
    pub id: u8,
    pub atk: u8,
    /// `CardType` byte: 0 physical, 1 magic, 2 flexible, 3 assault.
    pub kind: u8,
    pub pdef: u8,
    pub mdef: u8,
    pub cpoint: u8,
    pub arrow: u8,
}

impl CardRecord {
    pub fn from_card(card: &Card) -> Self {
        Self {
            id: card.id,
            atk: card.atk,
            kind: card.kind as u8,
            pdef: card.pdef,
            mdef: card.mdef,
            cpoint: card.cpoint,
            arrow: card.arrow,
        }
    }

    /// The card on the player's side.
    pub fn to_card(&self) -> Card {
        Card {
            id: self.id,
            side: 0,
            atk: self.atk,
            kind: CardType::from_byte(self.kind).unwrap_or(CardType::Physical),
            pdef: self.pdef,
            mdef: self.mdef,
            cpoint: self.cpoint,
            arrow: self.arrow,
        }
    }
}

/// Wins, losses and draws (`QuadMistDatabase.MiniGame_GetWinCount` and the others).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardCounts {
    pub win: u32,
    pub lose: u32,
    pub draw: u32,
}

/// The static tables a game needs: card maximums, and opponent decks by stage.
#[derive(Debug, Clone)]
pub struct CardTables {
    pub cards: CardTable,
    pub enemies: EnemyTables,
}

/// `UnityEngine.Random` stand-in: a deterministic stream (splitmix64).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CardRng(pub u64);

impl QuadRng for CardRng {
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            return lo;
        }
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        lo + (z % (hi - lo) as u64) as i32
    }
}

/// The player's cards as playable cards. A card the collection holds without stats (won from a
/// script, `MiniGame_SetCard`) gets them now, as `CardPool.CreateQuadMistCard` makes them.
pub fn collection(world: &mut FieldWorld, tables: &CardTables) -> Vec<Card> {
    let mut rng = CardRng(world.card_seed);
    world.card_records.truncate(world.cards.len());
    while world.card_records.len() < world.cards.len() {
        let id = world.cards[world.card_records.len()];
        let id = usize::try_from(id).unwrap_or(0).min(TOTAL_CARDS - 1) as u8;
        let card = create_card(tables.cards.max_stat_card(id), &mut rng);
        world.card_records.push(CardRecord::from_card(&card));
    }
    world.card_seed = rng.0;
    world.card_records.iter().map(CardRecord::to_card).collect()
}

/// Stores `cards` as the collection.
pub fn store_collection(world: &mut FieldWorld, cards: &[Card]) {
    world.cards = cards.iter().map(|card| u32::from(card.id)).collect();
    world.card_records = cards.iter().map(CardRecord::from_card).collect();
}

/// `QuadMistDatabase.MiniGame_GetPlayerPoints`: 10 per kind owned, 1 per flexible and 2 per
/// assault card, and 5 for each different arrow pattern. Without card records (cards that
/// never met a game) only the kinds count.
pub fn collector_points(world: &FieldWorld) -> i32 {
    let kinds: std::collections::BTreeSet<u32> = world.cards.iter().copied().collect();
    let mut points = kinds.len() as i32 * 10;
    let mut arrows = std::collections::BTreeSet::new();
    for record in &world.card_records {
        if arrows.insert(record.arrow) {
            points += 5;
        }
        points += [0, 0, 1, 2][usize::from(record.kind.min(3))];
    }
    points
}

/// `QuadMistDatabase.MiniGame_GetCollectorLevel` thresholds.
const COLLECTOR_LEVELS: [i32; 32] = [
    0, 300, 400, 500, 600, 700, 800, 900, 1000, 1100, 1200, 1250, 1300, 1320, 1330, 1340, 1350,
    1360, 1370, 1380, 1390, 1400, 1420, 1470, 1510, 1550, 1600, 1650, 1680, 1690, 1698, 1700,
];

/// `MiniGame_GetCollectorLevel`.
pub fn collector_level(points: i32) -> usize {
    (1..COLLECTOR_LEVELS.len())
        .find(|i| points < COLLECTOR_LEVELS[*i])
        .map_or(COLLECTOR_LEVELS.len() - 1, |i| i - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rng_stays_in_range_and_is_repeatable() {
        let mut a = CardRng(7);
        let mut b = CardRng(7);
        for _ in 0..200 {
            let x = a.range(0, 16);
            assert!((0..16).contains(&x));
            assert_eq!(x, b.range(0, 16));
        }
        assert_eq!(a.range(5, 5), 5);
    }

    #[test]
    fn collector_levels_follow_the_thresholds() {
        assert_eq!(collector_level(0), 0);
        assert_eq!(collector_level(299), 0);
        assert_eq!(collector_level(300), 1);
        assert_eq!(collector_level(5000), 31);
    }
}
