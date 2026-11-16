//! Match result and card win/lose bookkeeping. Source: `QuadMistGame.cs`
//! `EndGame` (481-555), `PostGame` (571-688), `ChangeCardToHand` (1188),
//! `ChangeCardToCenter` (1210), `RestoreCards` (1257), `SaveCardToDatabase`
//! (799), `SaveReservedCardToDatabase` (793).

use super::card::*;
use super::data::CardTable;
use super::game::*;
use super::{QuadError, QuadRng};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Win,
    Lose,
    Draw,
}

/// `QuadMistDatabase.WinCount/LoseCount/DrawCount` (Int16 save fields),
/// incremented at `END_STATE.RESULT` (QMG.cs:505, 522, 528).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    pub win: i16,
    pub lose: i16,
    pub draw: i16,
}

/// Result and the perfect flag (QMG.cs:487-528): perfect means one side owns
/// all 10 board cards. A draw is never perfect.
pub fn match_outcome(game: &Game) -> (Outcome, bool) {
    let (p, e) = game.scores();
    let perfect = p == 10 || e == 10;
    if p > e {
        (Outcome::Win, perfect)
    } else if e > p {
        (Outcome::Lose, perfect)
    } else {
        (Outcome::Draw, false)
    }
}

/// A card that changed owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transfer {
    pub card: Card,
    /// Side that received it (0 player, 1 enemy).
    pub to_side: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settlement {
    pub outcome: Outcome,
    pub perfect: bool,
    /// Win: indices into the enemy hand the player may take (QMG.cs:495-503).
    /// Lose: indices into the player hand the enemy may take (QMG.cs:512-520).
    pub selectable: Vec<usize>,
    /// Each card moved, in order. Every card appears once.
    pub transfers: Vec<Transfer>,
    /// The player's saved list after the match.
    pub collection: Vec<Card>,
}

/// `RestoreCards` (QMG.cs:1257-1273): every card left in a hand returns to its
/// owner; on a win (default mode) each player-hand card also runs
/// `LevelUpInMatch`, player hand first, in hand order.
fn restore_cards(
    player: &mut [Card],
    enemy: &mut [Card],
    win: bool,
    table: &CardTable,
    rng: &mut dyn QuadRng,
) {
    for c in player.iter_mut() {
        c.side = 0;
        if win {
            let max = *table.max_stat_card(c.id);
            c.level_up_in_match(&max, rng);
        }
    }
    for c in enemy.iter_mut() {
        c.side = 1;
    }
}

/// `CheckAndClearStackCard` (QMG.cs:557-569).
fn check_clear_stack(stack: &mut Option<Card>, taken: &Card) {
    if stack.is_some_and(|s| s.is_same_card(taken)) {
        *stack = None;
    }
}

/// Settles a finished match.
///
/// `collection` is the saved card list and `selected` the five indices the
/// player chose into it, in hand order. `win_pick` indexes `selectable` on a
/// non-perfect win (the player's choice; a perfect win takes all five enemy
/// cards in hand order, QMG.cs:628-639). On a non-perfect loss the enemy takes
/// `selectable[0]` (EnemyInputHandler.cs:41-45); a perfect loss takes the
/// player hand front to back (QMG.cs:641-657).
///
/// `stack` is `StackCardInfo`/`StackCardCount` (QMG.cs:49-50, 157-158): the
/// last card the player lost, set on every loss, cleared when the same card is
/// won back.
///
/// Result list (QMG.cs:799-823): the five hand cards (after the changes
/// above) first, then the unselected collection cards in their original
/// order. The source order of `QuadMistUI.allCardList` is not proven; original
/// order is assumed. A draw restores the original list (QMG.cs:793-797) with
/// in-match changes kept, since the cards are shared objects.
#[allow(clippy::too_many_arguments)]
pub fn settle(
    game: &Game,
    table: &CardTable,
    collection: &[Card],
    selected: [usize; 5],
    win_pick: usize,
    stack: &mut Option<Card>,
    counters: &mut Counters,
    rng: &mut dyn QuadRng,
) -> Result<Settlement, QuadError> {
    let m = game.match_cards()?;
    let (outcome, perfect) = match_outcome(game);
    for s in selected {
        if s >= collection.len() {
            return Err(QuadError::BadSelection(format!("selected index {s}")));
        }
    }
    let mut player: Vec<Card> = m[0..5].to_vec();
    let mut enemy: Vec<Card> = m[5..10].to_vec();
    let mut transfers = Vec::new();
    let mut selectable = Vec::new();

    let rest = || -> Vec<Card> {
        collection
            .iter()
            .enumerate()
            .filter(|(i, _)| !selected.contains(i))
            .map(|(_, c)| *c)
            .collect()
    };

    let out_collection = match outcome {
        Outcome::Win => {
            counters.win += 1;
            for (i, c) in enemy.iter().enumerate() {
                if c.side == 0 {
                    selectable.push(i);
                }
            }
            if perfect {
                while !enemy.is_empty() {
                    let chosen = enemy.remove(0);
                    restore_cards(&mut player, &mut enemy, true, table, rng);
                    check_clear_stack(stack, &chosen);
                    player.push(chosen);
                    transfers.push(Transfer {
                        card: chosen,
                        to_side: 0,
                    });
                }
            } else {
                let idx = *selectable
                    .get(win_pick)
                    .ok_or_else(|| QuadError::BadSelection(format!("win pick {win_pick}")))?;
                let chosen = enemy.remove(idx);
                restore_cards(&mut player, &mut enemy, true, table, rng);
                check_clear_stack(stack, &chosen);
                player.push(chosen);
                transfers.push(Transfer {
                    card: chosen,
                    to_side: 0,
                });
            }
            let mut v = player.clone();
            v.extend(rest());
            v
        }
        Outcome::Lose => {
            counters.lose += 1;
            for (i, c) in player.iter().enumerate() {
                if c.side == 1 {
                    selectable.push(i);
                }
            }
            if perfect {
                while !player.is_empty() {
                    let chosen = player.remove(0);
                    *stack = Some(chosen);
                    restore_cards(&mut player, &mut enemy, false, table, rng);
                    enemy.push(chosen);
                    transfers.push(Transfer {
                        card: chosen,
                        to_side: 1,
                    });
                }
            } else {
                let idx = *selectable
                    .first()
                    .ok_or_else(|| QuadError::BadSelection("no card to take".into()))?;
                let chosen = player.remove(idx);
                *stack = Some(chosen);
                restore_cards(&mut player, &mut enemy, false, table, rng);
                enemy.push(chosen);
                transfers.push(Transfer {
                    card: chosen,
                    to_side: 1,
                });
            }
            let mut v = player.clone();
            v.extend(rest());
            v
        }
        Outcome::Draw => {
            counters.draw += 1;
            let mut v = collection.to_vec();
            for (k, s) in selected.iter().enumerate() {
                v[*s] = m[k];
            }
            v
        }
    };
    Ok(Settlement {
        outcome,
        perfect,
        selectable,
        transfers,
        collection: out_collection,
    })
}

/// `START_STATE.SETUP` rematch hook (QMG.cs:313-314): when a lost card is
/// stacked, one enemy hand card is overwritten with it.
pub fn apply_stacked_card(hand: &mut [Card; 5], stack: &Option<Card>, rng: &mut dyn QuadRng) {
    if let Some(lost) = stack {
        super::data::restore_player_lost_card(hand, lost, rng);
    }
}
