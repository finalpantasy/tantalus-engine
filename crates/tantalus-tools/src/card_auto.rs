//! A Tetra Master player for the trace's story probe (`tantalus-trace chain --play-card-games`).
//!
//! The trace has no pad: when a script starts a card game (`MINIGAME`) this plays the whole
//! game through [`CardGame`] with presses, as a player would, and hands the result back with
//! [`CardGame::finish`]. It picks the five strongest cards, and on each turn tries every hand
//! card on every free cell in a copy of the game, plays the copy on through the opponent's
//! reply, and keeps the move that leaves the best board. The copies share the game's random
//! stream, so the look-ahead sees the real rolls: this is a route aid for the probe, not a
//! fair opponent model, and nothing outside the trace uses it.

use tantalus_game::cards::{CardGame, Input, Phase, Slot};
use tantalus_legacy::quad::{Card, Outcome};

/// Presses and ticks one game may take before the player gives up (walks away is not possible
/// once the game runs, so this only guards against a stuck machine).
const STEP_LIMIT: usize = 200_000;

/// How strong a card looks for the hand choice: its numbers plus its arrows.
fn strength(card: &Card) -> u32 {
    u32::from(card.atk) + u32::from(card.pdef.max(card.mdef)) + 24 * card.arrow.count_ones()
}

/// Board value for the player after a move: owned cards, and the result once the game is over.
fn value(game: &CardGame) -> i32 {
    let view = game.view();
    let board = view.scores.0 - view.scores.1;
    match view.result {
        Some((Outcome::Win, _)) => 1000 + board,
        Some((Outcome::Draw, _)) => board,
        Some((Outcome::Lose, _)) => -1000 + board,
        None => board,
    }
}

/// Plays until the player has a choice of card again (or the game is over), taking the first
/// target and prize offered.
fn run_to_choice(game: &mut CardGame) {
    for _ in 0..STEP_LIMIT {
        match game.phase() {
            Phase::Hand | Phase::Done | Phase::Select | Phase::Cell => return,
            Phase::Target { .. } | Phase::Prize => game.press(Input::Confirm),
            _ => game.advance(1),
        }
    }
}

/// Puts hand card `hand` on `cell` (from the `Hand` phase).
fn place(game: &mut CardGame, hand: usize, cell: usize) {
    for _ in 0..8 {
        if game.view().cursor == hand {
            break;
        }
        game.press(Input::Down);
    }
    game.press(Input::Confirm);
    while game.view().cell_cursor % 4 != cell % 4 {
        game.press(Input::Right);
    }
    while game.view().cell_cursor / 4 != cell / 4 {
        game.press(Input::Down);
    }
    game.press(Input::Confirm);
}

/// The five strongest cards of the collection, then the start row.
fn choose_hand(game: &mut CardGame) {
    let collection = game.view().collection.to_vec();
    let mut order: Vec<usize> = (0..collection.len()).collect();
    order.sort_by_key(|index| std::cmp::Reverse(strength(&collection[*index])));
    for index in order.into_iter().take(tantalus_game::cards::session::HAND) {
        while game.view().cursor != index {
            game.press(Input::Down);
        }
        game.press(Input::Confirm);
    }
    while game.view().cursor != collection.len() {
        game.press(Input::Down);
    }
    game.press(Input::Confirm);
}

/// Plays `game` to its end. The caller still calls [`CardGame::finish`].
pub fn play_out(game: &mut CardGame) {
    for _ in 0..STEP_LIMIT {
        match game.phase().clone() {
            Phase::Done => return,
            Phase::Select => choose_hand(game),
            Phase::Hand => {
                let view = game.view();
                let hand = view.hands[0].len();
                let free: Vec<usize> = (0..16)
                    .filter(|cell| view.board[*cell] == Slot::Empty)
                    .collect();
                let mut best: Option<(i32, usize, usize)> = None;
                for card in 0..hand {
                    for cell in &free {
                        let mut copy = game.clone();
                        place(&mut copy, card, *cell);
                        run_to_choice(&mut copy);
                        let score = value(&copy);
                        if best.is_none_or(|(top, _, _)| score > top) {
                            best = Some((score, card, *cell));
                        }
                    }
                }
                let Some((_, card, cell)) = best else { return };
                place(game, card, cell);
            }
            // A refused cell (it cannot happen for a free one) goes back to the hand.
            Phase::Cell => game.press(Input::Cancel),
            Phase::Target { .. } | Phase::Prize => game.press(Input::Confirm),
            _ => game.advance(1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tantalus_game::cards::CardTables;
    use tantalus_legacy::quad::{CardTable, EnemyTables};

    /// 100 physical cards of rising attack with one arrow right; stage tables of level 0.
    fn tables() -> CardTables {
        let mut bytes = Vec::new();
        for id in 0..100u8 {
            bytes.extend([id, 0, 0x20 + id / 2, 0x20 + id / 2, 1]);
        }
        let mut level = vec![0u8; 1024];
        for (i, slot) in level.iter_mut().enumerate().take(16) {
            *slot = i as u8;
        }
        CardTables {
            cards: CardTable::from_bytes(&bytes).unwrap(),
            enemies: EnemyTables::from_bytes(&level, &[0u8; 512]).unwrap(),
        }
    }

    #[test]
    fn a_game_is_played_to_the_end_and_its_result_reaches_the_world() {
        let tables = tables();
        let mut world = tantalus_game::FieldWorld::new();
        world.cards = (60..70).collect();
        let mut game = CardGame::start(&mut world, &tables, 0).unwrap();
        play_out(&mut game);
        assert!(game.is_done());
        game.finish(&mut world);
        let stats = world.card_stats;
        assert_eq!(stats.win + stats.lose + stats.draw, 1);
        assert!(matches!(world.last_card_result, 0..=2));
    }

    #[test]
    fn strong_cards_against_weak_ones_win() {
        let tables = tables();
        let mut world = tantalus_game::FieldWorld::new();
        // The strongest ten; the stage-0 opponent draws from the weakest cards.
        world.cards = (90..100).collect();
        let mut wins = 0;
        for seed in 0..5u64 {
            world.card_seed = seed * 7919 + 1;
            let mut game = CardGame::start(&mut world, &tables, 0).unwrap();
            play_out(&mut game);
            game.finish(&mut world);
            wins += i32::from(world.last_card_result == 0);
        }
        assert!(wins >= 4, "won {wins} of 5");
    }
}
