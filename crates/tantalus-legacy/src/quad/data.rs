//! Static tables and enemy deck generation. Sources: `CardPool.cs`,
//! `ENEMY/EnemyData.cs`.

use super::card::{Card, CardType, TOTAL_CARDS, create_card};
use super::{QuadError, QuadRng};

/// `MINIGAME_CARD_DATA_ADDRESS`: 100 records of 5 bytes (CardPool.cs:14-26):
/// `atk, type, pdef, mdef, cpoint`.
#[derive(Debug, Clone)]
pub struct CardTable {
    cards: Vec<Card>,
}

impl CardTable {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, QuadError> {
        if bytes.len() != TOTAL_CARDS * 5 {
            return Err(QuadError::BadTable(format!(
                "card data is {} bytes, expected {}",
                bytes.len(),
                TOTAL_CARDS * 5
            )));
        }
        let mut cards = Vec::with_capacity(TOTAL_CARDS);
        for (i, r) in bytes.chunks_exact(5).enumerate() {
            cards.push(Card {
                id: i as u8,
                side: 0,
                atk: r[0],
                kind: CardType::from_byte(r[1])?,
                pdef: r[2],
                mdef: r[3],
                cpoint: r[4],
                arrow: 0,
            });
        }
        Ok(Self { cards })
    }

    /// `CardPool.GetMaxStatCard` (CardPool.cs:84).
    pub fn max_stat_card(&self, id: u8) -> &Card {
        &self.cards[id as usize]
    }

    pub fn len(&self) -> usize {
        self.cards.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }
}

/// `EnemyData.probability` (EnemyData.cs:68-84).
const ENEMY_CARD_THRESHOLDS: [i32; 15] = [
    20, 40, 60, 80, 100, 120, 140, 160, 180, 200, 220, 240, 252, 254, 255,
];

/// `MINIGAME_CARD_LEVEL_ADDRESS` (64 levels x 16 card ids) and
/// `MINIGAME_STAGE_ADDRESS` (256 stages x `[card level, wise]`), EnemyData.cs:14-21.
#[derive(Debug, Clone)]
pub struct EnemyTables {
    level: Vec<u8>,
    stage: Vec<u8>,
}

impl EnemyTables {
    pub fn from_bytes(level: &[u8], stage: &[u8]) -> Result<Self, QuadError> {
        if level.len() != 1024 || stage.len() != 512 {
            return Err(QuadError::BadTable(format!(
                "level {} bytes (want 1024), stage {} bytes (want 512)",
                level.len(),
                stage.len()
            )));
        }
        Ok(Self {
            level: level.to_vec(),
            stage: stage.to_vec(),
        })
    }

    /// `(cardLevel, wise)` for `miniGameArg` (EnemyData.cs:18-19).
    pub fn stage(&self, mini_game_arg: usize) -> (u8, u8) {
        (
            self.stage[mini_game_arg * 2],
            self.stage[mini_game_arg * 2 + 1],
        )
    }

    /// `EnemyData.GetCardID` (EnemyData.cs:22-37): one `Range(0, 256)` draw;
    /// the index is the first threshold above the roll, else 15.
    pub fn card_id(&self, card_level: u8, rng: &mut dyn QuadRng) -> u8 {
        let rnd = rng.range(0, 256);
        let mut index = 15usize;
        for i in (0..ENEMY_CARD_THRESHOLDS.len()).rev() {
            if rnd >= ENEMY_CARD_THRESHOLDS[i] {
                break;
            }
            index = i;
        }
        self.level[card_level as usize * 16 + index]
    }

    /// `EnemyData.Initialize` (EnemyData.cs:39-47): five cards, each a
    /// `GetCardID` draw then `CreateQuadMistCard` draws, side 1.
    pub fn enemy_hand(
        &self,
        card_level: u8,
        table: &CardTable,
        rng: &mut dyn QuadRng,
    ) -> [Card; 5] {
        let mut hand = [Card::blank(); 5];
        for slot in hand.iter_mut() {
            let id = self.card_id(card_level, rng);
            let mut c = create_card(table.max_stat_card(id), rng);
            c.side = 1;
            *slot = c;
        }
        hand
    }
}

/// `EnemyData.RestorePlayerLostCard` -> `Hand.ReplaceCard` (Hand.cs:620-629):
/// overwrites id, atk, arrow, type, pdef, mdef of one enemy hand card; side
/// and cpoint stay. QMG.cs:313-314 draws `Range(0, 4)` for the index, after the
/// enemy hand is built and only when a lost card is stacked.
pub fn restore_player_lost_card(hand: &mut [Card; 5], lost: &Card, rng: &mut dyn QuadRng) {
    let i = rng.range(0, 4) as usize;
    let h = &mut hand[i];
    h.id = lost.id;
    h.atk = lost.atk;
    h.arrow = lost.arrow;
    h.kind = lost.kind;
    h.pdef = lost.pdef;
    h.mdef = lost.mdef;
}
