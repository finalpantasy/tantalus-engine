//! The 4x4 board, placement, battle and flip resolution, turn flow.
//! Sources: `QuadMistGame.cs` (QMG), `Board.cs`.

use super::card::*;
use super::{QuadError, QuadRng, Rules};

/// `Board.SIZE_X`, `Board.SIZE_Y` (Board.cs:241-242), default mode.
pub const BOARD_W: usize = 4;
pub const CELLS: usize = BOARD_W * BOARD_W;
/// Placed cards at which the match ends: `EnemyScore + PlayerScore == 10`
/// (QMG.cs:475).
pub const CARDS_IN_PLAY: i32 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    Empty,
    /// A block: occupies the cell, never flips, never counts, never adjacent.
    Block(Card),
    /// A played card. `slot` is the `matchCards` index (QMG.cs:322-323):
    /// 0..5 the player's hand, 5..10 the enemy's.
    Placed {
        card: Card,
        slot: u8,
    },
}

/// `BattleCalculation` (QMG.cs:990-1056).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BattleCalc {
    pub atk_start: i32,
    pub def_start: i32,
    pub atk_finish: i32,
    pub def_finish: i32,
}

/// `QuadMistGame.Calculate` (QMG.cs:990-1056), default mode. Draws: atk then
/// def, both `Range(0, start)` (or the `ReduceRandom` ranges). A zero start is
/// raised to 1 first. The attacker wins only when `atk_finish > def_finish`
/// (QMG.cs:423), so equal finishes go to the defender.
pub fn calculate(
    rules: Rules,
    attacker: &Card,
    defender: &Card,
    rng: &mut dyn QuadRng,
) -> BattleCalc {
    let (mut atk_start, mut def_start) = match attacker.kind {
        CardType::Physical => (attacker.atk as i32, defender.pdef as i32),
        CardType::Magic => (attacker.atk as i32, defender.mdef as i32),
        CardType::Flexible => (attacker.atk as i32, defender.pdef.min(defender.mdef) as i32),
        CardType::Assault => (
            attacker.atk.max(attacker.pdef).max(attacker.mdef) as i32,
            defender.atk.min(defender.pdef).min(defender.mdef) as i32,
        ),
    };
    if atk_start == 0 {
        atk_start = 1;
    }
    if def_start == 0 {
        def_start = 1;
    }
    let (atk_finish, def_finish);
    if rules.easy_win {
        if attacker.side == 0 {
            atk_finish = 100;
            def_finish = 0;
        } else {
            atk_finish = 0;
            def_finish = 100;
        }
    } else if rules.reduce_random {
        let ak = 1.0f32 - attacker.arrow_number() as f32 / 10.0;
        let dk = 1.0f32 - defender.arrow_number() as f32 / 8.0;
        let lowest_attack = (atk_start as f32 * ak) as i32;
        let lowest_defense = (def_start as f32 * dk) as i32;
        let mut a = rng.range(lowest_attack, atk_start);
        let mut d = rng.range(lowest_defense, def_start);
        if a > d {
            d = 0;
        } else {
            a = 0;
        }
        atk_finish = a;
        def_finish = d;
    } else {
        atk_finish = rng.range(0, atk_start);
        def_finish = rng.range(0, def_start);
    }
    BattleCalc {
        atk_start,
        def_start,
        atk_finish,
        def_finish,
    }
}

/// `START_STATE.SETUP` block count (QMG.cs:310-311): one `Range(0, 100)`;
/// 0-2 none, 3-6 one, 7-11 two, 12-16 three, 17-26 four, 27-96 five, 97-99 six.
pub fn roll_block_count(rng: &mut dyn QuadRng) -> usize {
    let r = rng.range(0, 100);
    match r {
        i32::MIN..=2 => 0,
        3..=6 => 1,
        7..=11 => 2,
        12..=16 => 3,
        17..=26 => 4,
        27..=96 => 5,
        _ => 6,
    }
}

/// `START_STATE.COIN` (QMG.cs:349-351): `Range(0, 2)`; 0 means the player moves first.
pub fn coin_toss(rng: &mut dyn QuadRng) -> u8 {
    rng.range(0, 2) as u8
}

/// One resolved fight inside a turn (QMG.cs:417-444 and 1345-1391).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Round {
    /// Defender cell, `None` when nothing was fightable (type NOTHING).
    pub defender: Option<usize>,
    pub calc: Option<BattleCalc>,
    pub attacker_won: bool,
    /// Combo cells flipped to the round winner (single level, not recursive).
    pub combos: Vec<usize>,
    /// One-way (arrow only) targets that changed side.
    pub beats: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnReport {
    pub cell: usize,
    pub rounds: Vec<Round>,
}

/// Choices the human player makes. The enemy side never calls this; it uses
/// [`super::ai`].
pub trait PlayerInput {
    /// `(hand index, cell)` for the player's placement (QMG.cs:841-851).
    fn place(&mut self, game: &Game) -> (usize, usize);
    /// Index into `candidates` (cells) when more than one fight is possible.
    fn pick_target(&mut self, game: &Game, candidates: &[usize]) -> usize;
}

#[derive(Debug, Clone)]
pub struct Game {
    pub rules: Rules,
    pub cells: [Cell; CELLS],
    /// `[player, enemy]` hands: `(card, slot)`.
    pub hands: [Vec<(Card, u8)>; 2],
}

impl Game {
    /// Player cards take slots 0..5, enemy cards 5..10 (QMG.cs:321-322). Card
    /// `side` fields are used as given.
    pub fn new(player: [Card; 5], enemy: [Card; 5], rules: Rules) -> Self {
        let p = player
            .iter()
            .enumerate()
            .map(|(i, c)| (*c, i as u8))
            .collect();
        let e = enemy
            .iter()
            .enumerate()
            .map(|(i, c)| (*c, 5 + i as u8))
            .collect();
        Self {
            rules,
            cells: [Cell::Empty; CELLS],
            hands: [p, e],
        }
    }

    /// `Board.RandomBlock` loop (Board.cs:211-218, QMG.cs ScaleInBlocks call at
    /// Board.cs:186-188): per block `Range(0,16)` until a free cell, then
    /// `Range(0,2)` for the block id. Returns the cells used.
    ///
    /// Draw-order caveat: the source runs block 1 inside the SETUP_FIELD frame
    /// and later blocks across animation frames, so the coin draw can fall
    /// before the later blocks. The caller picks the order; this is not proven.
    pub fn place_blocks(&mut self, n: usize, rng: &mut dyn QuadRng) -> Vec<usize> {
        let mut used = Vec::new();
        for _ in 0..n {
            let mut pos = rng.range(0, CELLS as i32) as usize;
            while self.cells[pos] != Cell::Empty {
                pos = rng.range(0, CELLS as i32) as usize;
            }
            let id = rng.range(0, 2) as u8;
            self.cells[pos] = Cell::Block(Card::block(id));
            used.push(pos);
        }
        used
    }

    pub fn is_free(&self, cell: usize) -> bool {
        self.cells[cell] == Cell::Empty
    }

    pub fn card_at(&self, cell: usize) -> Option<&Card> {
        match &self.cells[cell] {
            Cell::Placed { card, .. } => Some(card),
            _ => None,
        }
    }

    fn card_mut(&mut self, cell: usize) -> &mut Card {
        match &mut self.cells[cell] {
            Cell::Placed { card, .. } => card,
            other => panic!("cell {cell} holds {other:?}, not a card"),
        }
    }

    /// `Board.GetAdjacentCards(x, y)` (Board.cs:130-142): cell index per
    /// direction, `None` off-board, empty or block.
    pub fn adjacent(&self, cell: usize) -> [Option<usize>; MAX_ARROWS] {
        let (x, y) = ((cell % BOARD_W) as i32, (cell / BOARD_W) as i32);
        let mut out = [None; MAX_ARROWS];
        for (d, slot) in out.iter_mut().enumerate() {
            let (dx, dy) = direction_offset(d);
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= BOARD_W as i32 || ny >= BOARD_W as i32 {
                continue;
            }
            let n = ny as usize * BOARD_W + nx as usize;
            if matches!(self.cells[n], Cell::Placed { .. }) {
                *slot = Some(n);
            }
        }
        out
    }

    /// `UpdateScore` (QMG.cs:972-988): non-block cards per side on the board.
    pub fn scores(&self) -> (i32, i32) {
        let (mut p, mut e) = (0, 0);
        for c in &self.cells {
            if let Cell::Placed { card, .. } = c {
                if card.side == 0 {
                    p += 1;
                } else {
                    e += 1;
                }
            }
        }
        (p, e)
    }

    /// `EnemyScore + PlayerScore == 10` (QMG.cs:475).
    pub fn is_over(&self) -> bool {
        let (p, e) = self.scores();
        p + e == CARDS_IN_PLAY
    }

    /// `GenerateCombo` (QMG.cs:948-964): neighbors of `cell` that are not on
    /// `side_of` and that `cell`'s arrows point at (`CheckDirection > 0`).
    fn generate_combo(&self, cell: usize, side_of: u8) -> Vec<usize> {
        let card = *self.card_at(cell).expect("combo origin is a card");
        let mut out = Vec::new();
        for (d, n) in self.adjacent(cell).iter().enumerate() {
            if let Some(n) = n {
                let o = self.card_at(*n).unwrap();
                if o.side != side_of && check_direction(card.arrow, o.arrow, d) > 0 {
                    out.push(*n);
                }
            }
        }
        out
    }

    /// Resolves one placement (`InputPlayer`/`InputEnemy` QMG.cs:841-900,
    /// `GenerateTargetable` 902-937, `CALCULATE_BATTLE` 417-444,
    /// `ANIMATE_BATTLE` 445-479, outcome application BattleAnimation
    /// 1345-1391, ComboAnimation 1393-1448, BeatAnimation 1450-1471).
    ///
    /// Source facts reproduced:
    /// - Targets: an adjacent opposing card both sides point at (`CheckDirection`
    ///   2) is a fight target; one only the attacker points at (1) is a beat
    ///   (flipped with no fight); one only it points back (-1) is ignored.
    /// - First fight of an enemy placement takes target index 0
    ///   (EnemyInputHandler.cs:35-39). A player placement with several targets
    ///   asks `pick`.
    /// - After a won fight with more than one remaining target, play returns to
    ///   `INPUT_PLAYER` (QMG.cs:445-452) whoever attacked, so every continued
    ///   choice among several targets goes to `pick` (the player's handler), a
    ///   single remaining target is auto-selected.
    /// - A lost fight ends the turn, flips the attacker (and attacker-side
    ///   combos) to the defender's side and clears beats (QMG.cs:436-437).
    pub fn play_turn(
        &mut self,
        side: u8,
        hand_index: usize,
        cell: usize,
        pick: &mut dyn FnMut(&Game, &[usize]) -> usize,
        rng: &mut dyn QuadRng,
    ) -> Result<TurnReport, QuadError> {
        if cell >= CELLS {
            return Err(QuadError::BadCell(cell));
        }
        if !self.is_free(cell) {
            return Err(QuadError::CellOccupied(cell));
        }
        let hand = &mut self.hands[side as usize];
        if hand_index >= hand.len() {
            return Err(QuadError::BadHandIndex(hand_index));
        }
        let (card, slot) = hand.remove(hand_index);
        self.cells[cell] = Cell::Placed { card, slot };
        let attacker_side = card.side;

        // GenerateTargetable (QMG.cs:902-937), default mode branch.
        let mut selectable: Vec<usize> = Vec::new();
        let mut beatable: Vec<usize> = Vec::new();
        for (d, n) in self.adjacent(cell).iter().enumerate() {
            if let Some(n) = n {
                let o = *self.card_at(*n).unwrap();
                if o.side != attacker_side {
                    match check_direction(card.arrow, o.arrow, d) {
                        2 => selectable.push(*n),
                        1 => beatable.push(*n),
                        _ => {}
                    }
                }
            }
        }

        let mut rounds = Vec::new();
        let mut first = true;
        loop {
            let defender = match selectable.len() {
                0 => None,
                1 => Some(selectable[0]),
                n => {
                    if first && attacker_side == 1 {
                        Some(selectable[0])
                    } else {
                        let i = pick(self, &selectable);
                        if i >= n {
                            return Err(QuadError::BadSelection(format!("target {i} of {n}")));
                        }
                        Some(selectable[i])
                    }
                }
            };
            first = false;

            let mut round = Round {
                defender,
                calc: None,
                attacker_won: false,
                combos: Vec::new(),
                beats: Vec::new(),
            };
            if let Some(d) = defender {
                let attacker = *self.card_at(cell).unwrap();
                let defender_card = *self.card_at(d).unwrap();
                let calc = calculate(self.rules, &attacker, &defender_card, rng);
                let win = calc.atk_finish > calc.def_finish;
                round.calc = Some(calc);
                round.attacker_won = win;
                if win {
                    let combos = self.generate_combo(d, attacker.side);
                    beatable.retain(|b| !combos.contains(b));
                    self.card_mut(d).side = attacker.side;
                    self.card_mut(cell).level_up_in_battle(rng);
                    for c in &combos {
                        self.card_mut(*c).side = attacker.side;
                    }
                    round.combos = combos;
                } else {
                    let combos = self.generate_combo(cell, defender_card.side);
                    beatable.clear();
                    self.card_mut(cell).side = defender_card.side;
                    self.card_mut(d).level_up_in_battle(rng);
                    for c in &combos {
                        self.card_mut(*c).side = defender_card.side;
                    }
                    round.combos = combos;
                }
            }
            // BeatAnimation (QMG.cs:1450-1471): winSide is the attacker's side now.
            let win_side = self.card_at(cell).unwrap().side;
            for b in &beatable {
                if self.card_at(*b).unwrap().side != win_side {
                    self.card_mut(*b).side = win_side;
                    round.beats.push(*b);
                }
            }

            let continue_turn = round.attacker_won && selectable.len() > 1;
            if continue_turn {
                for c in &round.combos {
                    selectable.retain(|s| s != c);
                }
                let d = round.defender.unwrap();
                selectable.retain(|s| *s != d);
            }
            rounds.push(round);
            if !continue_turn {
                break;
            }
        }
        Ok(TurnReport { cell, rounds })
    }

    /// The ten cards by `matchCards` slot once all are placed (QMG.cs:321-322).
    pub fn match_cards(&self) -> Result<[Card; 10], QuadError> {
        let mut out = [Card::blank(); 10];
        let mut seen = [false; 10];
        for c in &self.cells {
            if let Cell::Placed { card, slot } = c {
                out[*slot as usize] = *card;
                seen[*slot as usize] = true;
            }
        }
        if seen.iter().all(|s| *s) {
            Ok(out)
        } else {
            Err(QuadError::BadSelection("match not finished".into()))
        }
    }
}

/// Runs a whole match from the first mover until ten cards are on the board
/// (QMG.cs:372-479). Enemy turns consume the cosmetic think draws
/// (EnemyAI.cs:161-170) and then the AI draws, in source order.
pub fn play_match(
    game: &mut Game,
    first_side: u8,
    wise: u8,
    player: &mut dyn PlayerInput,
    rng: &mut dyn QuadRng,
) -> Result<Vec<TurnReport>, QuadError> {
    let mut side = first_side;
    let mut reports = Vec::new();
    while !game.is_over() {
        let report = if side == 0 {
            let (h, c) = player.place(game);
            game.play_turn(0, h, c, &mut |g, cands| player.pick_target(g, cands), rng)?
        } else {
            super::ai::enemy_think_draws(rng);
            let (h, c) = super::ai::choose_enemy_move(game, wise, rng)?;
            game.play_turn(1, h, c, &mut |g, cands| player.pick_target(g, cands), rng)?
        };
        reports.push(report);
        side ^= 1;
    }
    Ok(reports)
}
