//! Opponent move choice. Source: `Global/ENEMY/EnemyAI.cs` (AI),
//! `EnemyInputHandler.cs`.

use super::card::*;
use super::game::*;
use super::{QuadError, QuadRng};

/// `wiseResetChance` (EnemyAI.cs:237-243), indexed by the stage's wise 0..=3.
pub const WISE_RESET_CHANCE: [i32; 4] = [28, 62, 92, 100];
/// `MIN_POINTS` (EnemyAI.cs:245).
const MIN_POINTS: i32 = -32767;

/// `EnemyAI.Thinking` (EnemyAI.cs:161-181): two cosmetic draws,
/// `Range(15, 31)` then `Range(-2, 21)`, taken before `SetCard` on every enemy
/// turn. They only drive animation but advance the same Unity random stream.
pub fn enemy_think_draws(rng: &mut dyn QuadRng) {
    rng.range(15, 31);
    rng.range(-2, 21);
}

/// `EnemyAI.Check` (EnemyAI.cs:185-235), default mode. Returns `-fights` when
/// any adjacent opposing card points back (mutual arrows), else `+flips`
/// (one-way arrows), else 0; plus the first mutual target.
fn check(game: &Game, card: &Card, cell: usize) -> (i32, Option<usize>) {
    let mut flips = 0;
    let mut fights = 0;
    let mut target = None;
    for (d, n) in game.adjacent(cell).iter().enumerate() {
        if let Some(n) = n {
            let o = game.card_at(*n).unwrap();
            if o.side != card.side {
                match check_direction(card.arrow, o.arrow, d) {
                    1 => flips += 1,
                    2 => {
                        if target.is_none() {
                            target = Some(*n);
                        }
                        fights += 1;
                    }
                    _ => {}
                }
            }
        }
    }
    if fights != 0 {
        (-fights, target)
    } else if flips != 0 {
        (flips, target)
    } else {
        (0, target)
    }
}

/// `EnemyAI.SetCard` (EnemyAI.cs:10-157), default mode: returns `(hand index,
/// cell)` for the enemy hand (`game.hands[1]`).
///
/// Draw order: one `Range(0, 100)` against `wiseResetChance` (on a roll above
/// the chance both factors become 0); then `Calculate` draws (two per
/// fightable `(card, cell)` pair, in hand, x, y order) while scoring; then
/// `Range(0, 1000)` for every pair whose score equals the running best.
/// Scores: mutual-arrow fights use `winChance = atk*100/(atk+def)`; wise 3
/// scores `winChance - 100` below 60 else `145 - winChance`, other wise values
/// score `winChance * fightFactor`; flips score `flips * flipNoFightFactor`.
#[allow(clippy::needless_range_loop)]
pub fn choose_enemy_move(
    game: &Game,
    wise: u8,
    rng: &mut dyn QuadRng,
) -> Result<(usize, usize), QuadError> {
    if wise as usize >= WISE_RESET_CHANCE.len() {
        return Err(QuadError::BadWise(wise));
    }
    let hand = &game.hands[1];
    if hand.is_empty() {
        return Err(QuadError::BadHandIndex(0));
    }
    let (player_score, enemy_score) = game.scores();
    let (mut flip_factor, mut fight_factor) = if enemy_score == player_score {
        (64, 1)
    } else if enemy_score < player_score {
        (67, 1)
    } else {
        (1, 5)
    };
    if rng.range(0, 100) > WISE_RESET_CHANCE[wise as usize] {
        flip_factor = 0;
        fight_factor = 0;
    }
    let n = hand.len();
    let mut score = vec![0i32; BOARD_W * BOARD_W * n];
    let at = |i: usize, x: usize, y: usize| (i * BOARD_W + x) * BOARD_W + y;
    let mut best = MIN_POINTS;
    let mut best_count = 0i32;
    for i in 0..n {
        for x in 0..BOARD_W {
            for y in 0..BOARD_W {
                let cell = x + y * BOARD_W;
                let k = at(i, x, y);
                score[k] = 0;
                if !game.is_free(cell) {
                    score[k] = MIN_POINTS;
                } else {
                    let (count, defender) = check(game, &hand[i].0, cell);
                    if count > 0 {
                        score[k] += count * flip_factor;
                    } else if count < 0 {
                        let d = *game.card_at(defender.unwrap()).unwrap();
                        let calc = calculate(game.rules, &hand[i].0, &d, rng);
                        let win_chance = calc.atk_start * 100 / (calc.atk_start + calc.def_start);
                        if wise == 3 {
                            score[k] = if win_chance < 60 {
                                win_chance - 100
                            } else {
                                145 - win_chance
                            };
                        } else {
                            score[k] = win_chance * fight_factor;
                        }
                    }
                }
                if score[k] == best {
                    best_count += 1;
                }
                if score[k] > best {
                    best = score[k];
                    best_count = 1;
                }
            }
        }
    }
    best = MIN_POINTS;
    let (mut rx, mut ry, mut ri) = (0usize, 0usize, 0usize);
    for i in 0..n {
        for x in 0..BOARD_W {
            for y in 0..BOARD_W {
                let s = score[at(i, x, y)];
                let mut is_best = false;
                if best < s {
                    is_best = true;
                }
                if best == s && rng.range(0, 1000) < 1000 / best_count {
                    is_best = true;
                }
                if is_best {
                    rx = x;
                    ry = y;
                    ri = i;
                    best = s;
                }
            }
        }
    }
    Ok((ri, rx + ry * BOARD_W))
}
