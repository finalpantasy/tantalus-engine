//! One Tetra Master game against an NPC, as a state machine.
//!
//! [`CardGame`] holds no clock or window: the play loop feeds it [`Input`] presses and
//! [`CardGame::advance`] ticks (60 a second), and draws what [`CardGame::view`] describes. It
//! follows `QuadMistGame`:
//!
//! 1. Card choice (`QuadMistCardUI`): the player picks five cards of the collection.
//! 2. Setup (`START_STATE.SETUP`): the opponent's hand from its stage (`EnemyData`), the lost
//!    card handed back, random blocks, the coin toss for who starts.
//! 3. Turns: the player picks a hand card, a cell and, when one placement can fight several
//!    neighbors, the target; the opponent thinks (`EnemyAI`). Each placement resolves round by
//!    round: the battle numbers show, then the loser flips, combos and one-way arrows follow.
//! 4. Result (`END_STATE`): win, lose or draw (perfect when one side owns the board), the prize
//!    card choice on a win, and the settlement of the collection.
//!
//! Stand-ins: when the opponent's placement can fight several neighbors the first is taken (the
//! reference hands later choices to the player's input handler); the opponent's thinking pause
//! and the animation lengths are approximations.

use std::collections::VecDeque;

use tantalus_legacy::quad::{
    Card, Cell, Counters, Game, Outcome, Round, Rules, Settlement, TurnReport, apply_stacked_card,
    choose_enemy_move, coin_toss, enemy_think_draws, match_outcome, roll_block_count, settle,
};

use super::{CardRecord, CardRng, CardTables, collection, store_collection};
use crate::field::world::FieldWorld;

/// Cards a hand holds.
pub const HAND: usize = 5;
/// Ticks the coin shows.
const COIN_TICKS: u32 = 70;
/// Ticks the opponent "thinks".
const THINK_TICKS: u32 = 50;
/// Ticks the two battle numbers stay up.
const NUMBER_TICKS: u32 = 70;
/// Ticks a flip (or the combo that follows it) takes.
const FLIP_TICKS: u32 = 30;
/// Ticks the result banner holds.
const BANNER_TICKS: u32 = 110;
/// Ticks a card takes to change hands.
const TAKE_TICKS: u32 = 50;

/// A pad press the game understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Cancel,
}

/// A thing that happened, for sound (`QuadMistSoundID`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardEvent {
    /// The game opens (its music starts).
    Started,
    /// The game is over and the field returns (its music stops).
    Ended,
    Cursor,
    Cancel,
    /// A refused press (`MINI_SE_WARNING`).
    Warning,
    Coin,
    /// A block lands on the board (`MINI_SE_WALL`).
    Wall,
    /// A card is placed (`MINI_SE_CARD_MOVE`).
    Place,
    /// A card flips (`MINI_SE_FLASH`).
    Flash,
    /// A combo flips a neighbor (`MINI_SE_COMB`).
    Combo,
    /// The battle numbers show (`MINI_SE_BOMB`).
    Bomb,
    Win,
    Perfect,
    Lose,
    Draw,
    /// A card moves from the opponent to the player (`MINI_SE_CARD_GET`).
    CardGet,
    /// A card moves from the player to the opponent (`MINI_SE_WINDOW`).
    CardLost,
}

/// Where the game is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// The player picks five cards; `cursor` may sit one past the list, on the start row.
    Select,
    /// The coin shows who starts.
    Coin { ticks: u32 },
    /// The opponent is about to move.
    Think { ticks: u32 },
    /// The player picks a hand card.
    Hand,
    /// The player picks a cell for the card.
    Cell,
    /// One placement can fight several neighbors: the player picks one.
    Target { candidates: Vec<usize> },
    /// A placement is playing out.
    Resolve,
    /// The result banner.
    Banner { ticks: u32 },
    /// The player picks which of the opponent's cards to take.
    Prize,
    /// The card changes hands.
    Take { ticks: u32 },
    /// Over: [`CardGame::finish`] has the settlement.
    Done,
}

/// How a placement is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    /// The two battle numbers of the round are up.
    Numbers,
    /// The round's flips show.
    Flips,
}

/// A placement being shown, round by round.
#[derive(Debug, Clone)]
struct Resolving {
    cell: usize,
    side: u8,
    rounds: VecDeque<Round>,
    step: Step,
    ticks: u32,
}

/// The battle numbers on screen (`BattleAnimation`): the attacker and defender cells with the
/// starting and finishing value each side rolled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Popup {
    pub attacker: usize,
    pub defender: usize,
    pub attack: i32,
    pub defense: i32,
    pub attack_finish: i32,
    pub defense_finish: i32,
    pub attacker_won: bool,
}

/// One board cell as drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Empty,
    Block(u8),
    Card { card: Card, side: u8 },
}

/// Everything a view draws.
#[derive(Debug, Clone)]
pub struct View<'a> {
    pub phase: &'a Phase,
    pub board: [Slot; 16],
    /// Player hand, then opponent hand (cards still unplayed).
    pub hands: [Vec<Card>; 2],
    pub scores: (i32, i32),
    /// The collection while the cards are being chosen.
    pub collection: &'a [Card],
    pub selected: &'a [usize],
    pub cursor: usize,
    pub cell_cursor: usize,
    pub popup: Option<Popup>,
    /// Who moves (0 player, 1 opponent) once the game runs.
    pub turn: u8,
    /// The coin's side while it shows.
    pub coin: Option<u8>,
    /// `(outcome, perfect)` once the match is over.
    pub result: Option<(Outcome, bool)>,
    /// Cards the player may take on a win (opponent hand indices).
    pub prize: &'a [usize],
    /// The opponent cards of a finished match, for the prize choice and the transfer.
    pub enemy_cards: Vec<Card>,
    pub transfers: &'a [Card],
    /// The cursor of the target or prize list.
    pub choice: usize,
}

/// One game.
#[derive(Debug, Clone)]
pub struct CardGame {
    tables: CardTables,
    rules: Rules,
    rng: CardRng,
    collection: Vec<Card>,
    selected: Vec<usize>,
    cursor: usize,
    cell_cursor: usize,
    phase: Phase,
    game: Option<Game>,
    /// Stage of the opponent: card level and AI wisdom.
    level: u8,
    wise: u8,
    turn: u8,
    first: u8,
    /// Picks the player made among a placement's targets so far.
    picks: Vec<usize>,
    placing: Option<(usize, usize)>,
    resolving: Option<Resolving>,
    /// The sides the board shows (they trail the game while a placement plays out).
    shown: [Option<u8>; 16],
    popup: Option<Popup>,
    result: Option<(Outcome, bool)>,
    prize: Vec<usize>,
    prize_cursor: usize,
    win_pick: usize,
    settlement: Option<Settlement>,
    transfers: Vec<Card>,
    stack: Option<Card>,
    counters: Counters,
    events: Vec<CardEvent>,
    outcome_code: Option<i32>,
}

impl CardGame {
    /// A game against opponent `opponent` (the `MINIGAME` argument). `None` when the player has
    /// fewer than five cards, as the scripts require.
    pub fn start(world: &mut FieldWorld, tables: &CardTables, opponent: u16) -> Option<Self> {
        let cards = collection(world, tables);
        if cards.len() < HAND {
            return None;
        }
        let (level, wise) = tables.enemies.stage(usize::from(opponent).min(255));
        let counters = Counters {
            win: world.card_stats.win as i16,
            lose: world.card_stats.lose as i16,
            draw: world.card_stats.draw as i16,
        };
        Some(Self {
            tables: tables.clone(),
            rules: Rules::default(),
            rng: CardRng(world.card_seed),
            collection: cards,
            selected: Vec::new(),
            cursor: 0,
            cell_cursor: 5,
            phase: Phase::Select,
            game: None,
            level,
            wise,
            turn: 0,
            first: 0,
            picks: Vec::new(),
            placing: None,
            resolving: None,
            shown: [None; 16],
            popup: None,
            result: None,
            prize: Vec::new(),
            prize_cursor: 0,
            win_pick: 0,
            settlement: None,
            transfers: Vec::new(),
            stack: world.card_stack.map(|record| record.to_card()),
            counters,
            events: vec![CardEvent::Started],
            outcome_code: None,
        })
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    pub fn is_done(&self) -> bool {
        self.phase == Phase::Done
    }

    /// Sound events since the last call.
    pub fn take_events(&mut self) -> Vec<CardEvent> {
        std::mem::take(&mut self.events)
    }

    /// Hands the result back to the world: the collection after the match, the record, the lost
    /// card, and `GetTetraMasterResult`. Call once, when [`Self::is_done`].
    pub fn finish(&mut self, world: &mut FieldWorld) {
        if let Some(settlement) = &self.settlement {
            store_collection(world, &settlement.collection);
            world.card_stats.win = self.counters.win.max(0) as u32;
            world.card_stats.lose = self.counters.lose.max(0) as u32;
            world.card_stats.draw = self.counters.draw.max(0) as u32;
            world.card_stack = self.stack.as_ref().map(CardRecord::from_card);
            if let Some(code) = self.outcome_code {
                world.last_card_result = code;
            }
        }
        world.card_seed = self.rng.0;
        self.events.push(CardEvent::Ended);
    }

    /// What to draw.
    pub fn view(&self) -> View<'_> {
        let mut board = [Slot::Empty; 16];
        let mut hands: [Vec<Card>; 2] = [Vec::new(), Vec::new()];
        let mut scores = (0, 0);
        if let Some(game) = &self.game {
            for (cell, slot) in board.iter_mut().enumerate() {
                *slot = match game.cells[cell] {
                    Cell::Empty => Slot::Empty,
                    Cell::Block(card) => Slot::Block(card.id),
                    Cell::Placed { card, .. } => Slot::Card {
                        card,
                        side: self.shown[cell].unwrap_or(card.side),
                    },
                };
            }
            for (side, hand) in game.hands.iter().enumerate() {
                hands[side] = hand.iter().map(|(card, _)| *card).collect();
            }
            for slot in &board {
                if let Slot::Card { side, .. } = slot {
                    if *side == 0 {
                        scores.0 += 1;
                    } else {
                        scores.1 += 1;
                    }
                }
            }
        }
        let enemy_cards = self
            .game
            .as_ref()
            .and_then(|game| game.match_cards().ok())
            .map(|cards| cards[5..10].to_vec())
            .unwrap_or_default();
        View {
            phase: &self.phase,
            board,
            hands,
            scores,
            collection: &self.collection,
            selected: &self.selected,
            cursor: self.cursor,
            cell_cursor: self.cell_cursor,
            popup: self.popup,
            turn: self.turn,
            coin: matches!(self.phase, Phase::Coin { .. }).then_some(self.first),
            result: self.result,
            prize: &self.prize,
            enemy_cards,
            transfers: &self.transfers,
            choice: self.prize_cursor,
        }
    }

    /// The cursor's place in the active list: the card choice or the prize choice.
    pub fn prize_cursor(&self) -> usize {
        self.prize_cursor
    }

    /// One press.
    pub fn press(&mut self, input: Input) {
        match self.phase.clone() {
            Phase::Select => self.press_select(input),
            Phase::Hand => self.press_hand(input),
            Phase::Cell => self.press_cell(input),
            Phase::Target { candidates } => self.press_target(input, &candidates),
            Phase::Prize => self.press_prize(input),
            _ => {}
        }
    }

    fn press_select(&mut self, input: Input) {
        // The list, then one row to start.
        let rows = self.collection.len() + 1;
        match input {
            Input::Up => {
                self.cursor = (self.cursor + rows - 1) % rows;
                self.events.push(CardEvent::Cursor);
            }
            Input::Down => {
                self.cursor = (self.cursor + 1) % rows;
                self.events.push(CardEvent::Cursor);
            }
            Input::Left | Input::Right => {}
            Input::Confirm => {
                if self.cursor == self.collection.len() {
                    if self.selected.len() == HAND {
                        self.begin();
                    } else {
                        self.events.push(CardEvent::Warning);
                    }
                } else if let Some(at) = self.selected.iter().position(|s| *s == self.cursor) {
                    self.selected.remove(at);
                    self.events.push(CardEvent::Cancel);
                } else if self.selected.len() < HAND {
                    self.selected.push(self.cursor);
                    self.events.push(CardEvent::Cursor);
                } else {
                    self.events.push(CardEvent::Warning);
                }
            }
            Input::Cancel => {
                if self.selected.pop().is_some() {
                    self.events.push(CardEvent::Cancel);
                } else {
                    // Walking away: no game, nothing changes.
                    self.events.push(CardEvent::Cancel);
                    self.phase = Phase::Done;
                }
            }
        }
    }

    /// `START_STATE.SETUP`: the opponent's hand, the lost card, the blocks and the coin.
    fn begin(&mut self) {
        let mut hand: [Card; HAND] = [Card::blank(); HAND];
        for (slot, index) in hand.iter_mut().zip(&self.selected) {
            *slot = self.collection[*index];
            slot.side = 0;
        }
        let mut enemy =
            self.tables
                .enemies
                .enemy_hand(self.level, &self.tables.cards, &mut self.rng);
        apply_stacked_card(&mut enemy, &self.stack, &mut self.rng);
        let mut game = Game::new(hand, enemy, self.rules);
        let count = roll_block_count(&mut self.rng);
        game.place_blocks(count, &mut self.rng);
        for _ in 0..count {
            self.events.push(CardEvent::Wall);
        }
        self.first = coin_toss(&mut self.rng);
        self.turn = self.first;
        self.game = Some(game);
        self.events.push(CardEvent::Coin);
        self.phase = Phase::Coin { ticks: COIN_TICKS };
    }

    fn press_hand(&mut self, input: Input) {
        let Some(game) = &self.game else { return };
        let len = game.hands[0].len();
        match input {
            Input::Up if len > 0 => {
                self.cursor = (self.cursor + len - 1) % len;
                self.events.push(CardEvent::Cursor);
            }
            Input::Down if len > 0 => {
                self.cursor = (self.cursor + 1) % len;
                self.events.push(CardEvent::Cursor);
            }
            Input::Confirm if len > 0 => {
                self.cursor = self.cursor.min(len - 1);
                self.events.push(CardEvent::Cursor);
                self.phase = Phase::Cell;
            }
            _ => {}
        }
    }

    fn press_cell(&mut self, input: Input) {
        let Some(game) = &self.game else { return };
        let (x, y) = (self.cell_cursor % 4, self.cell_cursor / 4);
        let moved = match input {
            Input::Left => Some((x + 3) % 4 + y * 4),
            Input::Right => Some((x + 1) % 4 + y * 4),
            Input::Up => Some(x + (y + 3) % 4 * 4),
            Input::Down => Some(x + (y + 1) % 4 * 4),
            _ => None,
        };
        if let Some(cell) = moved {
            self.cell_cursor = cell;
            self.events.push(CardEvent::Cursor);
            return;
        }
        match input {
            Input::Cancel => {
                self.events.push(CardEvent::Cancel);
                self.phase = Phase::Hand;
            }
            Input::Confirm => {
                if game.is_free(self.cell_cursor) {
                    self.picks.clear();
                    self.placing = Some((self.cursor, self.cell_cursor));
                    self.place(0);
                } else {
                    self.events.push(CardEvent::Warning);
                }
            }
            _ => {}
        }
    }

    fn press_target(&mut self, input: Input, candidates: &[usize]) {
        let len = candidates.len();
        // The cursor walks the candidates; it is kept in `prize_cursor`.
        match input {
            Input::Up | Input::Left => {
                self.prize_cursor = (self.prize_cursor + len - 1) % len;
                self.events.push(CardEvent::Cursor);
            }
            Input::Down | Input::Right => {
                self.prize_cursor = (self.prize_cursor + 1) % len;
                self.events.push(CardEvent::Cursor);
            }
            Input::Confirm => {
                self.picks.push(self.prize_cursor);
                self.prize_cursor = 0;
                self.events.push(CardEvent::Cursor);
                self.place(0);
            }
            Input::Cancel => {}
        }
    }

    fn press_prize(&mut self, input: Input) {
        let len = self.prize.len();
        if len == 0 {
            return;
        }
        match input {
            Input::Up | Input::Left => {
                self.prize_cursor = (self.prize_cursor + len - 1) % len;
                self.events.push(CardEvent::Cursor);
            }
            Input::Down | Input::Right => {
                self.prize_cursor = (self.prize_cursor + 1) % len;
                self.events.push(CardEvent::Cursor);
            }
            Input::Confirm => {
                self.win_pick = self.prize_cursor;
                self.events.push(CardEvent::Cursor);
                self.settle();
            }
            Input::Cancel => {}
        }
    }

    /// Plays hand card `index` of `side` onto `cell` (the player's choice is in `placing`).
    /// A placement that needs the player's pick between targets stops in [`Phase::Target`]; the
    /// pick is added and the placement is run again from the same random state.
    fn place(&mut self, enemy: u8) {
        let (hand_index, cell) = if enemy == 0 {
            match self.placing {
                Some(placing) => placing,
                None => return,
            }
        } else {
            match self.placing.take() {
                Some(placing) => placing,
                None => return,
            }
        };
        let side = if enemy == 0 { self.turn } else { 1 };
        let Some(game) = &self.game else { return };
        let mut trial = game.clone();
        let mut rng = self.rng;
        let mut needed: Option<Vec<usize>> = None;
        let mut used = 0usize;
        let picks = self.picks.clone();
        let before_sides = sides_of(game);
        let report = trial.play_turn(
            side,
            hand_index,
            cell,
            &mut |_, candidates| {
                if side == 1 {
                    return 0;
                }
                if let Some(pick) = picks.get(used) {
                    used += 1;
                    return (*pick).min(candidates.len() - 1);
                }
                needed = Some(candidates.to_vec());
                0
            },
            &mut rng,
        );
        if let Some(candidates) = needed {
            self.prize_cursor = 0;
            self.phase = Phase::Target { candidates };
            return;
        }
        let Ok(report) = report else {
            self.events.push(CardEvent::Warning);
            self.phase = Phase::Cell;
            return;
        };
        self.rng = rng;
        self.game = Some(trial);
        self.placing = None;
        self.shown = before_sides;
        let mover = self
            .game
            .as_ref()
            .and_then(|g| g.card_at(cell).map(|_| side))
            .unwrap_or(side);
        self.shown[cell] = Some(mover);
        self.events.push(CardEvent::Place);
        self.begin_resolve(report, side);
    }

    fn begin_resolve(&mut self, report: TurnReport, side: u8) {
        self.resolving = Some(Resolving {
            cell: report.cell,
            side,
            rounds: report.rounds.into_iter().collect(),
            step: Step::Flips,
            ticks: 20,
        });
        self.phase = Phase::Resolve;
    }

    /// Moves the match along by `ticks` (1/60 s each).
    pub fn advance(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.tick();
        }
    }

    fn tick(&mut self) {
        match self.phase.clone() {
            Phase::Coin { ticks } => {
                if ticks <= 1 {
                    self.next_turn();
                } else {
                    self.phase = Phase::Coin { ticks: ticks - 1 };
                }
            }
            Phase::Think { ticks } => {
                if ticks <= 1 {
                    self.enemy_move();
                } else {
                    self.phase = Phase::Think { ticks: ticks - 1 };
                }
            }
            Phase::Resolve => self.tick_resolve(),
            Phase::Banner { ticks } => {
                if ticks <= 1 {
                    self.after_banner();
                } else {
                    self.phase = Phase::Banner { ticks: ticks - 1 };
                }
            }
            Phase::Take { ticks } => {
                if ticks <= 1 {
                    self.phase = Phase::Done;
                } else {
                    self.phase = Phase::Take { ticks: ticks - 1 };
                }
            }
            _ => {}
        }
    }

    fn next_turn(&mut self) {
        if self.game.as_ref().is_some_and(Game::is_over) {
            self.finish_match();
        } else if self.turn == 0 {
            self.cursor = 0;
            self.phase = Phase::Hand;
        } else {
            self.phase = Phase::Think { ticks: THINK_TICKS };
        }
    }

    fn enemy_move(&mut self) {
        let Some(game) = &self.game else { return };
        enemy_think_draws(&mut self.rng);
        match choose_enemy_move(game, self.wise, &mut self.rng) {
            Ok((hand_index, cell)) => {
                self.placing = Some((hand_index, cell));
                self.picks.clear();
                self.place(1);
            }
            Err(_) => self.finish_match(),
        }
    }

    fn tick_resolve(&mut self) {
        let Some(mut resolving) = self.resolving.take() else {
            self.phase = Phase::Done;
            return;
        };
        if resolving.ticks > 1 {
            resolving.ticks -= 1;
            self.resolving = Some(resolving);
            return;
        }
        match resolving.step {
            Step::Flips => {
                // The previous round's flips are done; show the next round's numbers.
                match resolving.rounds.front().cloned() {
                    None => {
                        self.popup = None;
                        self.turn = 1 - resolving.side;
                        self.next_turn();
                        return;
                    }
                    Some(round) => {
                        if let (Some(defender), Some(calc)) = (round.defender, round.calc) {
                            self.popup = Some(Popup {
                                attacker: resolving.cell,
                                defender,
                                attack: calc.atk_start,
                                defense: calc.def_start,
                                attack_finish: calc.atk_finish,
                                defense_finish: calc.def_finish,
                                attacker_won: round.attacker_won,
                            });
                            self.events.push(CardEvent::Bomb);
                            resolving.step = Step::Numbers;
                            resolving.ticks = NUMBER_TICKS;
                        } else {
                            // No fight: only one-way arrows flip.
                            self.apply_round(&resolving);
                            resolving.rounds.pop_front();
                            resolving.ticks = FLIP_TICKS;
                        }
                    }
                }
            }
            Step::Numbers => {
                self.popup = None;
                self.apply_round(&resolving);
                resolving.rounds.pop_front();
                resolving.step = Step::Flips;
                resolving.ticks = FLIP_TICKS;
            }
        }
        self.resolving = Some(resolving);
    }

    /// Shows the flips of the first queued round.
    fn apply_round(&mut self, resolving: &Resolving) {
        let Some(round) = resolving.rounds.front() else {
            return;
        };
        let Some(game) = &self.game else { return };
        let attacker_side = game.card_at(resolving.cell).map(|c| c.side);
        let mut flashed = false;
        if let (Some(defender), Some(_)) = (round.defender, round.calc) {
            // The winner's side: the attacker keeps its side on a win; on a loss the
            // attacker's cell and its combos go to the defender's side.
            let winner = if round.attacker_won {
                resolving.side
            } else {
                1 - resolving.side
            };
            if round.attacker_won {
                self.shown[defender] = Some(winner);
            } else {
                self.shown[resolving.cell] = Some(winner);
            }
            for combo in &round.combos {
                self.shown[*combo] = Some(winner);
                self.events.push(CardEvent::Combo);
            }
            flashed = true;
        }
        let win_side = attacker_side.unwrap_or(resolving.side);
        for beat in &round.beats {
            self.shown[*beat] = Some(win_side);
            flashed = true;
        }
        if flashed {
            self.events.push(CardEvent::Flash);
        }
    }

    /// Ten cards are down: the result.
    fn finish_match(&mut self) {
        let Some(game) = &self.game else {
            self.phase = Phase::Done;
            return;
        };
        let (outcome, perfect) = match_outcome(game);
        self.result = Some((outcome, perfect));
        self.events.push(match (outcome, perfect) {
            (Outcome::Win, true) => CardEvent::Perfect,
            (Outcome::Win, false) => CardEvent::Win,
            (Outcome::Lose, _) => CardEvent::Lose,
            (Outcome::Draw, _) => CardEvent::Draw,
        });
        self.phase = Phase::Banner {
            ticks: BANNER_TICKS,
        };
    }

    fn after_banner(&mut self) {
        let Some((outcome, perfect)) = self.result else {
            self.phase = Phase::Done;
            return;
        };
        if outcome == Outcome::Win && !perfect {
            let cards = self
                .game
                .as_ref()
                .and_then(|game| game.match_cards().ok())
                .map(|cards| cards[5..10].to_vec())
                .unwrap_or_default();
            self.prize = cards
                .iter()
                .enumerate()
                .filter(|(_, card)| card.side == 0)
                .map(|(index, _)| index)
                .collect();
            if self.prize.len() > 1 {
                self.prize_cursor = 0;
                self.phase = Phase::Prize;
                return;
            }
        }
        self.win_pick = 0;
        self.settle();
    }

    fn settle(&mut self) {
        let Some(game) = &self.game else {
            self.phase = Phase::Done;
            return;
        };
        let mut selected = [0usize; HAND];
        for (slot, index) in selected.iter_mut().zip(&self.selected) {
            *slot = *index;
        }
        match settle(
            game,
            &self.tables.cards,
            &self.collection,
            selected,
            self.win_pick,
            &mut self.stack,
            &mut self.counters,
            &mut self.rng,
        ) {
            Ok(settlement) => {
                self.outcome_code = Some(match settlement.outcome {
                    Outcome::Win => 0,
                    Outcome::Lose => 1,
                    Outcome::Draw => 2,
                });
                self.transfers = settlement
                    .transfers
                    .iter()
                    .map(|t| Card {
                        side: t.to_side,
                        ..t.card
                    })
                    .collect();
                if let Some(transfer) = settlement.transfers.first() {
                    self.events.push(if transfer.to_side == 0 {
                        CardEvent::CardGet
                    } else {
                        CardEvent::CardLost
                    });
                    self.phase = Phase::Take { ticks: TAKE_TICKS };
                } else {
                    self.phase = Phase::Done;
                }
                self.settlement = Some(settlement);
            }
            Err(_) => self.phase = Phase::Done,
        }
    }
}

fn sides_of(game: &Game) -> [Option<u8>; 16] {
    let mut out = [None; 16];
    for (cell, side) in out.iter_mut().enumerate() {
        if let Cell::Placed { card, .. } = game.cells[cell] {
            *side = Some(card.side);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tantalus_legacy::quad::{CardTable, EnemyTables};

    /// 100 cards of attack 0x80 physical, one arrow right; a stage table of level 0.
    fn tables() -> CardTables {
        let mut bytes = Vec::new();
        for _ in 0..100 {
            bytes.extend([0x80, 0, 0x60, 0x60, 1]);
        }
        let mut level = vec![0u8; 1024];
        for (i, slot) in level.iter_mut().enumerate().take(16) {
            *slot = i as u8;
        }
        let stage = vec![0u8; 512];
        CardTables {
            cards: CardTable::from_bytes(&bytes).unwrap(),
            enemies: EnemyTables::from_bytes(&level, &stage).unwrap(),
        }
    }

    fn world(cards: usize) -> FieldWorld {
        let mut world = FieldWorld::new();
        world.cards = (0..cards as u32).collect();
        world
    }

    /// Plays the player's turns by taking the first card and first free cell.
    fn drive(game: &mut CardGame) {
        for _ in 0..200_000 {
            match game.phase().clone() {
                Phase::Done => return,
                Phase::Select => {
                    for _ in 0..HAND {
                        game.press(Input::Confirm);
                        game.press(Input::Down);
                    }
                    while game.view().cursor < game.view().collection.len() {
                        game.press(Input::Down);
                    }
                    game.press(Input::Confirm);
                }
                Phase::Hand => game.press(Input::Confirm),
                Phase::Cell => {
                    let free = game
                        .view()
                        .board
                        .iter()
                        .position(|slot| *slot == Slot::Empty)
                        .unwrap();
                    while game.view().cell_cursor % 4 != free % 4 {
                        game.press(Input::Right);
                    }
                    while game.view().cell_cursor / 4 != free / 4 {
                        game.press(Input::Down);
                    }
                    game.press(Input::Confirm);
                }
                Phase::Target { .. } | Phase::Prize => game.press(Input::Confirm),
                _ => game.advance(1),
            }
        }
        panic!("the game did not finish");
    }

    #[test]
    fn fewer_than_five_cards_cannot_play() {
        let mut world = world(4);
        assert!(CardGame::start(&mut world, &tables(), 0).is_none());
    }

    #[test]
    fn a_whole_game_ends_and_the_world_keeps_a_consistent_collection() {
        let mut world = world(8);
        let tables = tables();
        let mut game = CardGame::start(&mut world, &tables, 0).unwrap();
        drive(&mut game);
        game.finish(&mut world);
        let counts = world.card_stats;
        assert_eq!(counts.win + counts.lose + counts.draw, 1);
        assert_eq!(world.cards.len(), world.card_records.len());
        // A loss sets one card aside, a win can add one.
        assert!((7..=9).contains(&world.cards.len()));
        assert!(matches!(world.last_card_result, 0..=2));
        let events = game.take_events();
        assert!(events.contains(&CardEvent::Coin));
        assert_eq!(events.last(), Some(&CardEvent::Ended));
    }

    #[test]
    fn walking_away_from_the_card_choice_changes_nothing() {
        let mut world = world(6);
        let mut game = CardGame::start(&mut world, &tables(), 0).unwrap();
        game.press(Input::Cancel);
        assert!(game.is_done());
        game.finish(&mut world);
        assert_eq!(world.card_stats, Default::default());
        assert_eq!(world.last_card_result, 1);
    }

    #[test]
    fn the_same_seed_plays_the_same_game() {
        let tables = tables();
        let play = || {
            let mut world = world(8);
            let mut game = CardGame::start(&mut world, &tables, 0).unwrap();
            drive(&mut game);
            game.finish(&mut world);
            (world.last_card_result, world.card_records.clone())
        };
        assert_eq!(play(), play());
    }
}
