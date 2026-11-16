//! Hand-derived cases for the Tetra Master core. Every expected value is
//! worked out from the source arithmetic in the comments; arrows are bit masks
//! (bit n = direction n: 0 up, 1 right-up, 2 right, 3 right-down, 4 down,
//! 5 left-down, 6 left, 7 left-up). Cell = x + 4 * y.

use tantalus_legacy::quad::*;

fn card(id: u8, side: u8, atk: u8, pdef: u8, mdef: u8, arrow: u8) -> Card {
    Card {
        id,
        side,
        atk,
        kind: CardType::Physical,
        pdef,
        mdef,
        cpoint: 0,
        arrow,
    }
}

/// Table with 100 records; `set` writes `atk, type, pdef, mdef, cpoint`.
fn table(rows: &[(u8, [u8; 5])]) -> CardTable {
    let mut bytes = vec![0u8; 500];
    for (id, r) in rows {
        bytes[*id as usize * 5..*id as usize * 5 + 5].copy_from_slice(r);
    }
    CardTable::from_bytes(&bytes).unwrap()
}

/// Enemy-AI and cosmetic draws for scripted games: reset roll 99 (above
/// every wise chance), Range(0, 1000) = 999 (never swaps a best), everything
/// else the low bound.
struct Policy;
impl QuadRng for Policy {
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        match (lo, hi) {
            (0, 100) => 99,
            (0, 1000) => 999,
            _ => lo,
        }
    }
}

fn picture(g: &Game) -> String {
    let mut s = String::new();
    for (i, c) in g.cells.iter().enumerate() {
        if i > 0 && i % 4 == 0 {
            s.push('/');
        }
        s.push(match c {
            Cell::Empty => '.',
            Cell::Block(_) => '#',
            Cell::Placed { card, .. } => {
                if card.side == 0 {
                    'p'
                } else {
                    'e'
                }
            }
        });
    }
    s
}

fn place_raw(g: &mut Game, cell: usize, slot: u8, c: Card) {
    g.cells[cell] = Cell::Placed { card: c, slot };
}

fn filler_game() -> Game {
    let c = |i: u8| card(50 + i, 0, 0x10, 0x10, 0x10, 0);
    let e = |i: u8| card(60 + i, 1, 0x10, 0x10, 0x10, 0);
    Game::new(
        [c(0), c(1), c(2), c(3), c(4)],
        [e(0), e(1), e(2), e(3), e(4)],
        Rules::default(),
    )
}

// ---------------------------------------------------------------- arrows

#[test]
fn arrow_reverse_and_direction_check() {
    // Reverse swaps nibbles: bit 0 (up) becomes bit 4 (down).
    assert_eq!(reverse_arrow(0b0000_0001), 0b0001_0000);
    assert_eq!(reverse_arrow(0b1000_0100), 0b0100_1000);
    // Up(0) vs a neighbor pointing down(4): both point at each other.
    assert_eq!(check_direction(0b0000_0001, 0b0001_0000, 0), 2);
    // Only the attacker points: 1. Only the neighbor points back: -1. None: 0.
    assert_eq!(check_direction(0b0000_0001, 0, 0), 1);
    assert_eq!(check_direction(0, 0b0001_0000, 0), -1);
    assert_eq!(check_direction(0, 0, 0), 0);
    // Offsets, y grows down.
    assert_eq!(direction_offset(0), (0, -1));
    assert_eq!(direction_offset(3), (1, 1));
    assert_eq!(direction_offset(6), (-1, 0));
}

// ---------------------------------------------------------------- tables

#[test]
fn card_table_record_layout_and_checks() {
    // Record 0 of the install: 07 00 09 04 0a.
    let t = table(&[(0, [7, 0, 9, 4, 10]), (1, [9, 1, 10, 4, 10])]);
    assert_eq!(t.len(), 100);
    let c = t.max_stat_card(0);
    assert_eq!(
        (c.atk, c.kind, c.pdef, c.mdef, c.cpoint),
        (7, CardType::Physical, 9, 4, 10)
    );
    assert_eq!(t.max_stat_card(1).kind, CardType::Magic);
    assert!(CardTable::from_bytes(&[0u8; 499]).is_err());
    let mut bad = vec![0u8; 500];
    bad[1] = 4;
    assert!(CardTable::from_bytes(&bad).is_err());
    // Digits shown on the card are value >> 4.
    assert_eq!(0xc3u8 >> 4, 12);
}

#[test]
fn enemy_card_roll_thresholds() {
    // Row 0 of the level table: ids 0,0,0,1,1,1,2,2,2,3,3,3,7,15,20,30.
    let mut level = vec![0u8; 1024];
    level[..16].copy_from_slice(&[0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, 7, 15, 20, 30]);
    let t = EnemyTables::from_bytes(&level, &vec![0u8; 512]).unwrap();
    // Walk down from threshold 254 (index 14): stop at the first threshold the
    // roll reaches; the index is the one above it. 0 -> 0, 19 -> 0, 20 -> 1,
    // 100 -> 5 (100 >= thr[4]=100), 239 -> 11, 254 -> 14, 255 -> 15.
    for (roll, idx) in [
        (0, 0),
        (19, 0),
        (20, 1),
        (100, 5),
        (239, 11),
        (254, 14),
        (255, 15),
    ] {
        let mut rng = ScriptedRng::new(&[roll]);
        assert_eq!(t.card_id(0, &mut rng), level[idx], "roll {roll}");
        assert_eq!(rng.log, vec![(0, 256, roll)]);
    }
    assert!(EnemyTables::from_bytes(&level, &[0u8; 511]).is_err());
}

#[test]
fn create_card_and_arrow_draws() {
    // Max card: atk 0x40 (64), pdef 0x20 (32), mdef 0x10 (16).
    let t = table(&[(7, [0x40, 0, 0x20, 0x10, 3])]);
    // atk: 32 + Range(0,32)=5 -> 37; pdef: 16 + Range(0,16)=3 -> 19;
    // mdef: 8 + Range(0,8)=7 -> 15; Range(0,127)=0 -> Flexible; arrow roll 60
    // lands in the 3-arrow bucket ([34,65)); eight shuffle draws of 5 turn
    // [0..8) into [5,0,1,2,3,7,4,6] (worked swap by swap), first three set:
    // bits 5, 0, 1 = 0b0010_0011.
    let mut rng = ScriptedRng::new(&[5, 3, 7, 0, 60, 5, 5, 5, 5, 5, 5, 5, 5]);
    let c = create_card(t.max_stat_card(7), &mut rng);
    assert_eq!((c.atk, c.pdef, c.mdef), (37, 19, 15));
    assert_eq!(c.kind, CardType::Flexible);
    assert_eq!(c.arrow, 0b0010_0011);
    assert_eq!(c.cpoint, 3);
    assert_eq!(rng.remaining(), 0);
    assert_eq!(rng.log[0], (0, 32, 5));
    assert_eq!(rng.log[1], (0, 16, 3));
    assert_eq!(rng.log[2], (0, 8, 7));
    assert_eq!(rng.log[3], (0, 127, 0));
    assert_eq!(rng.log[4], (0, 100, 60));
}

#[test]
fn block_count_and_block_placement() {
    for (roll, n) in [
        (0, 0),
        (2, 0),
        (3, 1),
        (6, 1),
        (7, 2),
        (11, 2),
        (12, 3),
        (16, 3),
        (17, 4),
        (26, 4),
        (27, 5),
        (96, 5),
        (97, 6),
        (99, 6),
    ] {
        assert_eq!(
            roll_block_count(&mut ScriptedRng::new(&[roll])),
            n,
            "roll {roll}"
        );
    }
    // Block 1: pos 5, id 1 (card 101). Block 2: pos 5 taken, redraw 9, id 0.
    let mut g = filler_game();
    let mut rng = ScriptedRng::new(&[5, 1, 5, 9, 0]);
    assert_eq!(g.place_blocks(2, &mut rng), vec![5, 9]);
    assert_eq!(picture(&g), "..../.#../.#../....");
    match g.cells[5] {
        Cell::Block(b) => assert_eq!(b.id, 101),
        _ => panic!(),
    }
    assert!(!g.is_free(5));
    assert_eq!(rng.remaining(), 0);
    // Blocks are not adjacent targets and do not count.
    assert_eq!(g.scores(), (0, 0));
    assert_eq!(g.adjacent(4), [None; 8]);
    assert_eq!(coin_toss(&mut ScriptedRng::new(&[1])), 1);
}

// ---------------------------------------------------------------- battle formula

#[test]
fn battle_starts_per_card_type() {
    let rules = Rules::default();
    let mut def = card(1, 1, 0x30, 0x08, 0x40, 0);
    // Physical: atk vs pdef.
    let mut a = card(2, 0, 0x50, 0x60, 0x20, 0);
    let c = calculate(rules, &a, &def, &mut ScriptedRng::new(&[0, 0]));
    assert_eq!((c.atk_start, c.def_start), (0x50, 0x08));
    // Magic: atk vs mdef.
    a.kind = CardType::Magic;
    let c = calculate(rules, &a, &def, &mut ScriptedRng::new(&[0, 0]));
    assert_eq!((c.atk_start, c.def_start), (0x50, 0x40));
    // Flexible: atk vs min(pdef, mdef).
    a.kind = CardType::Flexible;
    let c = calculate(rules, &a, &def, &mut ScriptedRng::new(&[0, 0]));
    assert_eq!((c.atk_start, c.def_start), (0x50, 0x08));
    // Assault: max(atk,pdef,mdef) = 0x60 vs min(atk,pdef,mdef) = 0x08.
    a.kind = CardType::Assault;
    let mut rng = ScriptedRng::new(&[95, 7]);
    let c = calculate(rules, &a, &def, &mut rng);
    assert_eq!(
        (c.atk_start, c.def_start, c.atk_finish, c.def_finish),
        (96, 8, 95, 7)
    );
    assert_eq!(rng.log, vec![(0, 96, 95), (0, 8, 7)]);
    // A zero start is raised to 1, so Range(0, 1): finishes 0 and 0.
    a.kind = CardType::Magic;
    a.atk = 0;
    def.mdef = 0;
    let mut rng = ScriptedRng::new(&[0, 0]);
    let c = calculate(rules, &a, &def, &mut rng);
    assert_eq!((c.atk_start, c.def_start), (1, 1));
    assert_eq!(rng.log, vec![(0, 1, 0), (0, 1, 0)]);
}

#[test]
fn easy_win_and_reduce_random() {
    let a = card(1, 0, 50, 0, 0, 0b0001_1111); // 5 arrows -> ak = 1 - 5/10 = 0.5
    let d = card(2, 1, 0, 30, 0, 0b0000_1111); // 4 arrows -> dk = 1 - 4/8 = 0.5
    let easy = Rules {
        easy_win: true,
        reduce_random: false,
    };
    let c = calculate(easy, &a, &d, &mut ScriptedRng::new(&[]));
    assert_eq!((c.atk_finish, c.def_finish), (100, 0));
    let mut a1 = a;
    a1.side = 1;
    let c = calculate(easy, &a1, &d, &mut ScriptedRng::new(&[]));
    assert_eq!((c.atk_finish, c.def_finish), (0, 100));
    let reduce = Rules {
        easy_win: false,
        reduce_random: true,
    };
    // Ranges (25, 50) and (15, 30). 40 vs 20: attacker higher, defender zeroed.
    let mut rng = ScriptedRng::new(&[40, 20]);
    let c = calculate(reduce, &a, &d, &mut rng);
    assert_eq!((c.atk_finish, c.def_finish), (40, 0));
    assert_eq!(rng.log, vec![(25, 50, 40), (15, 30, 20)]);
    // 26 vs 28: attacker not higher, attacker zeroed.
    let c = calculate(reduce, &a, &d, &mut ScriptedRng::new(&[26, 28]));
    assert_eq!((c.atk_finish, c.def_finish), (0, 28));
}

// ---------------------------------------------------------------- single turns

#[test]
fn equal_finish_goes_to_the_defender_and_flips_the_attacker() {
    // Player A (cell 5, arrow left|right) attacks D (cell 4, arrow right).
    // Equal finishes 30/30 -> atk <= def -> attacker loses (QMG.cs:423).
    // A flips to side 1. Combo of a lost fight: neighbors of A on A's old
    // side that A points at, here F at cell 6 (A's right arrow) -> flips too.
    // Defender D (Physical) levels: Range(0,64)=9, no change.
    let mut g = filler_game();
    let mut a = card(1, 0, 60, 0, 0, 0b0100_0100);
    a.kind = CardType::Physical;
    g.hands[0][0] = (a, 0);
    place_raw(&mut g, 4, 5, card(2, 1, 0, 64, 0, 0b0000_0100));
    place_raw(&mut g, 6, 6, card(3, 0, 0, 0, 0, 0));
    let mut rng = ScriptedRng::new(&[30, 30, 9]);
    let r = g
        .play_turn(
            0,
            0,
            5,
            &mut |_, _| panic!("one target, no choice"),
            &mut rng,
        )
        .unwrap();
    assert_eq!(r.rounds.len(), 1);
    assert!(!r.rounds[0].attacker_won);
    assert_eq!(r.rounds[0].defender, Some(4));
    assert_eq!(r.rounds[0].combos, vec![6]);
    assert_eq!(picture(&g), "..../eee./..../....");
    assert_eq!(rng.remaining(), 0);
}

#[test]
fn arrow_away_neighbor_is_ignored_and_one_way_arrow_flips_without_a_fight() {
    // Attacker (side 0, cell 5) arrow = up (0). Neighbors:
    // cell 1 (up): arrow 0 -> one-way, beat, flips with no draws.
    // cell 4 (left): points right at the attacker, attacker does not point
    // left -> check = -1, ignored, stays side 1.
    let mut g = filler_game();
    g.hands[0][0] = (card(1, 0, 10, 10, 10, 0b0000_0001), 0);
    place_raw(&mut g, 1, 5, card(2, 1, 0, 0, 0, 0));
    place_raw(&mut g, 4, 6, card(3, 1, 0, 0, 0, 0b0000_0100));
    let mut rng = ScriptedRng::new(&[]);
    let r = g
        .play_turn(0, 0, 5, &mut |_, _| panic!(), &mut rng)
        .unwrap();
    assert_eq!(r.rounds.len(), 1);
    assert_eq!(r.rounds[0].defender, None);
    assert_eq!(r.rounds[0].beats, vec![1]);
    assert_eq!(picture(&g), ".p../ep../..../....");
}

#[test]
fn enemy_first_target_is_index_zero_and_a_won_fight_continues() {
    // Enemy attacker (side 1) at cell 5, arrow up|left. Player cards at cell 1
    // (arrow down) and cell 4 (arrow right): both mutual. Candidate order is
    // direction order [1, 4]. The enemy takes index 0 without asking
    // (EnemyInputHandler.cs:35-39); the first fight is won, one target is left
    // (no choice), second fight is won.
    let mut g = filler_game();
    g.hands[1][0] = (card(1, 1, 0x20, 0, 0, 0b0100_0001), 5);
    place_raw(&mut g, 1, 0, card(2, 0, 0, 0x10, 0, 0b0001_0000));
    place_raw(&mut g, 4, 1, card(3, 0, 0, 0x10, 0, 0b0000_0100));
    // Fight 1: atk Range(0,32)=5, def Range(0,16)=0 -> win, level draw 9.
    // Fight 2: same.
    let mut rng = ScriptedRng::new(&[5, 0, 9, 5, 0, 9]);
    let r = g
        .play_turn(1, 0, 5, &mut |_, _| panic!("enemy never chooses"), &mut rng)
        .unwrap();
    assert_eq!(
        r.rounds.iter().map(|x| x.defender).collect::<Vec<_>>(),
        vec![Some(1), Some(4)]
    );
    assert!(r.rounds.iter().all(|x| x.attacker_won));
    assert_eq!(picture(&g), ".e../ee../..../....");
}

// ---------------------------------------------------------------- full scripted game

/// Player hand P0..P4 and enemy hand E0..E4 (slots 0..9).
fn scripted_cards() -> ([Card; 5], [Card; 5]) {
    let p0 = card(10, 0, 0x30, 0x30, 0x20, 17);
    let p1 = card(11, 0, 0x60, 0x20, 0x20, 13);
    let p2 = card(12, 0, 0x50, 0x28, 0x30, 193);
    let p3 = card(13, 0, 0x40, 0x20, 0x20, 64);
    let p4 = card(14, 0, 0x40, 0x20, 0x20, 96);
    let e0 = card(20, 1, 0x30, 0x40, 0x10, 24);
    let e1 = card(21, 1, 0x30, 0x20, 0x10, 4);
    let e2 = card(22, 1, 0x30, 0x10, 0x10, 1);
    let e3 = card(23, 1, 0x40, 0x10, 0x10, 24);
    let e4 = card(24, 1, 0x20, 0x10, 0x10, 0);
    ([p0, p1, p2, p3, p4], [e0, e1, e2, e3, e4])
}

struct Script {
    cells: Vec<usize>,
    picks: Vec<usize>,
    seen_candidates: Vec<Vec<usize>>,
}
impl PlayerInput for Script {
    fn place(&mut self, _g: &Game) -> (usize, usize) {
        (0, self.cells.remove(0))
    }
    fn pick_target(&mut self, _g: &Game, c: &[usize]) -> usize {
        self.seen_candidates.push(c.to_vec());
        self.picks.remove(0)
    }
}

#[test]
#[allow(clippy::needless_range_loop)]
fn scripted_full_game_player_wins_seven_to_three() {
    // Board cells: 0 1 2 3 / 4 5 6 7 / 8 9 10 11 / 12 13 14 15. Player moves
    // first. The enemy runs the real AI with wise 0 and reset roll 99
    // (> 28), so both score factors are 0: every legal placement scores 0 and
    // the Range(0,1000) = 999 draws never swap the best, so it plays hand
    // card 0 on the first free cell in x-major order (0,4,8,12,1,5,9,13,2,...).
    // Combat draws come from a separate script.
    let (pc, ec) = scripted_cards();
    let mut g = Game::new(pc, ec, Rules::default());
    let mut player = Script {
        cells: vec![5, 4, 9, 13, 6],
        picks: vec![1, 0],
        seen_candidates: vec![],
    };
    // Battle draws in order of use:
    // T3 P1(96) vs E0 pdef 64: 30,30 tie -> P1 loses; level draw E0 = 5.
    // T5 P2(80) vs E1 pdef 32: 40,10 win, level 7; vs P0 pdef 48: 40,10 win,
    //    level 8; vs P1 pdef 32: 40,10 win, level 9.
    // T8 E3(64) vs P0 pdef 48: 50,47 win, level 11.
    let mut battle = ScriptedRng::new(&[30, 30, 5, 40, 10, 7, 40, 10, 8, 40, 10, 9, 50, 47, 11]);
    let mut ai = Policy;
    let enemy_cells = [0usize, 8, 12, 1, 2];
    let pictures = [
        "..../.p../..../....",
        "e.../.e../..../....",
        "e.../ee../..../....",
        "e.../ee../e.../....",
        "p.../pp../pp../....",
        "p.../pp../ep../e...",
        "p.../pp../ep../pp..",
        "pe../pe../ee../pp..",
        "pe../ppp./ep../pp..",
        "pee./ppp./ep../pp..",
    ];
    let mut enemy_turn = 0;
    for turn in 0..10 {
        if turn % 2 == 0 {
            let (h, c) = player.place(&g);
            g.play_turn(
                0,
                h,
                c,
                &mut |gg, cands| player.pick_target(gg, cands),
                &mut battle,
            )
            .unwrap();
        } else {
            enemy_think_draws(&mut ai);
            let (h, c) = choose_enemy_move(&g, 0, &mut ai).unwrap();
            // T2 cell 0, T4 cell 8 (0 and 4 are taken), T6 cell 12, T8 cell 1,
            // T10 cell 2 (see the x-major walk in the comment above).
            assert_eq!((h, c), (0, enemy_cells[enemy_turn]), "turn {turn}");
            enemy_turn += 1;
            g.play_turn(
                1,
                h,
                c,
                &mut |gg, cands| player.pick_target(gg, cands),
                &mut battle,
            )
            .unwrap();
        }
        assert_eq!(picture(&g), pictures[turn], "after turn {turn}");
    }
    assert_eq!(battle.remaining(), 0);
    assert!(g.is_over());
    assert_eq!(g.scores(), (7, 3));
    // T5 offered [cell 5, cell 8, cell 4] first, then [5, 4] after the first
    // win removed cell 8; the picks were 1 then 0.
    assert_eq!(player.seen_candidates, vec![vec![5, 8, 4], vec![5, 4]]);

    // ---- bookkeeping: win, not perfect
    assert_eq!(match_outcome(&g), (Outcome::Win, false));
    let t = table(&[
        (10, [0x70, 0, 0x70, 0x70, 0]),
        (11, [0x70, 0, 0x70, 0x70, 0]),
        (12, [0x90, 0, 0x90, 0x30, 0]), // mdef equals the card: no gain
        (13, [0x70, 0, 0x70, 0x70, 0]),
        (14, [0x70, 0, 0x70, 0x70, 0]),
    ]);
    let x = card(30, 0, 1, 1, 1, 0);
    let y = card(31, 0, 2, 2, 2, 0);
    let collection = [pc[0], pc[1], pc[2], pc[3], pc[4], x, y];
    // Enemy cards on the player's side at the end: E0 (index 0) and E2
    // (index 2). The player takes E2: win_pick 1.
    // Level draws for hand P0..P4: 0,1,2,0,1 -> P0 atk+1, P1 pdef+1,
    // P2 mdef capped, P3 atk+1, P4 pdef+1.
    let mut stack = Some(g.match_cards().unwrap()[7]); // E2 itself was lost before
    let mut counters = Counters::default();
    let mut srng = ScriptedRng::new(&[0, 1, 2, 0, 1]);
    let s = settle(
        &g,
        &t,
        &collection,
        [0, 1, 2, 3, 4],
        1,
        &mut stack,
        &mut counters,
        &mut srng,
    )
    .unwrap();
    assert_eq!(s.outcome, Outcome::Win);
    assert!(!s.perfect);
    assert_eq!(s.selectable, vec![0, 2]);
    assert_eq!(s.transfers.len(), 1);
    assert_eq!(s.transfers[0].card.id, 22);
    assert_eq!(s.transfers[0].to_side, 0);
    assert_eq!(stack, None, "won the stacked card back");
    assert_eq!(
        counters,
        Counters {
            win: 1,
            lose: 0,
            draw: 0
        }
    );
    assert_eq!(s.collection.len(), 8);
    let ids: Vec<u8> = s.collection.iter().map(|c| c.id).collect();
    assert_eq!(ids, vec![10, 11, 12, 13, 14, 22, 30, 31]);
    assert_eq!(s.collection[0].atk, 0x31);
    assert_eq!(s.collection[1].pdef, 0x21);
    assert_eq!(s.collection[2].mdef, 0x30);
    assert_eq!(s.collection[3].atk, 0x41);
    assert_eq!(s.collection[4].pdef, 0x21);
    assert!(s.collection[..6].iter().all(|c| c.side == 0));
    assert_eq!(srng.remaining(), 0);
}

#[test]
fn whole_match_driver_is_deterministic_and_terminates() {
    let run = || {
        let (pc, ec) = scripted_cards();
        let mut g = Game::new(pc, ec, Rules::default());
        struct First;
        impl PlayerInput for First {
            fn place(&mut self, g: &Game) -> (usize, usize) {
                (0, (0..16).find(|c| g.is_free(*c)).unwrap())
            }
            fn pick_target(&mut self, _g: &Game, _c: &[usize]) -> usize {
                0
            }
        }
        let reports = play_match(&mut g, 0, 1, &mut First, &mut Policy).unwrap();
        (reports.len(), picture(&g), g.scores())
    };
    let a = run();
    assert_eq!(a, run());
    assert_eq!(a.0, 10);
    assert_eq!(a.2.0 + a.2.1, 10);
}

// ---------------------------------------------------------------- bookkeeping

/// Ten placed cards, cell = slot; sides given per slot.
fn finished(player_sides: [u8; 5], enemy_sides: [u8; 5]) -> (Game, [Card; 5]) {
    let (mut pc, mut ec) = scripted_cards();
    for i in 0..5 {
        pc[i].side = player_sides[i];
        ec[i].side = enemy_sides[i];
    }
    let mut g = Game::new(pc, ec, Rules::default());
    for i in 0..5 {
        g.cells[i] = Cell::Placed {
            card: pc[i],
            slot: i as u8,
        };
        g.cells[5 + i] = Cell::Placed {
            card: ec[i],
            slot: 5 + i as u8,
        };
    }
    g.hands = [vec![], vec![]];
    (g, scripted_cards().0)
}

fn big_table() -> CardTable {
    let rows: Vec<(u8, [u8; 5])> = (10..=14).map(|i| (i, [0xf0, 0, 0xf0, 0xf0, 0])).collect();
    let mut rows2 = rows;
    rows2.extend((20..=24).map(|i| (i, [0xf0u8, 0, 0xf0, 0xf0, 0])));
    table(&rows2)
}

#[test]
fn lose_takes_the_first_flipped_card_once_and_stacks_it() {
    // Player cards 0 and 2 ended on the enemy side; player owns 1,3,4 (3 of
    // 10) vs enemy 7: lose, not perfect. selectable = [0, 2]; the enemy takes
    // index 0 (EnemyInputHandler.cs:41-45).
    let (g, pc) = finished([1, 0, 1, 0, 0], [1, 1, 1, 1, 1]);
    let extra = card(40, 0, 1, 1, 1, 0);
    let collection = [pc[0], pc[1], pc[2], pc[3], pc[4], extra];
    let mut stack = None;
    let mut counters = Counters::default();
    let s = settle(
        &g,
        &big_table(),
        &collection,
        [0, 1, 2, 3, 4],
        0,
        &mut stack,
        &mut counters,
        &mut ScriptedRng::new(&[]),
    )
    .unwrap();
    assert_eq!(s.outcome, Outcome::Lose);
    assert_eq!(s.selectable, vec![0, 2]);
    assert_eq!(s.transfers.len(), 1);
    assert_eq!((s.transfers[0].card.id, s.transfers[0].to_side), (10, 1));
    // 6 cards in, 5 out: P0 gone exactly once; remaining hand sides reset to 0.
    let ids: Vec<u8> = s.collection.iter().map(|c| c.id).collect();
    assert_eq!(ids, vec![11, 12, 13, 14, 40]);
    assert!(s.collection[..4].iter().all(|c| c.side == 0));
    assert_eq!(stack.unwrap().id, 10);
    assert_eq!(
        counters,
        Counters {
            win: 0,
            lose: 1,
            draw: 0
        }
    );

    // Rematch: the stacked card overwrites enemy hand card Range(0,4)=2:
    // id, atk, arrow, type, pdef, mdef replaced; side and cpoint stay.
    let (_, ec) = scripted_cards();
    let mut hand = ec;
    hand[2].cpoint = 9;
    apply_stacked_card(&mut hand, &stack, &mut ScriptedRng::new(&[2]));
    assert_eq!(hand[2].id, 10);
    assert_eq!(hand[2].arrow, 17);
    assert_eq!(hand[2].side, 1);
    assert_eq!(hand[2].cpoint, 9);
    assert_eq!(hand[1].id, 21);
    // No stack, no draw.
    let mut none_rng = ScriptedRng::new(&[]);
    apply_stacked_card(&mut hand, &None, &mut none_rng);
}

#[test]
fn perfect_lose_gives_all_five_and_stacks_the_last() {
    let (g, pc) = finished([1; 5], [1; 5]);
    let collection = [pc[0], pc[1], pc[2], pc[3], pc[4], card(40, 0, 1, 1, 1, 0)];
    let mut stack = None;
    let mut counters = Counters::default();
    let s = settle(
        &g,
        &big_table(),
        &collection,
        [0, 1, 2, 3, 4],
        0,
        &mut stack,
        &mut counters,
        &mut ScriptedRng::new(&[]),
    )
    .unwrap();
    assert!(s.perfect);
    assert_eq!(
        s.transfers.iter().map(|t| t.card.id).collect::<Vec<_>>(),
        vec![10, 11, 12, 13, 14]
    );
    assert!(s.transfers.iter().all(|t| t.to_side == 1));
    assert_eq!(
        s.collection.iter().map(|c| c.id).collect::<Vec<_>>(),
        vec![40]
    );
    assert_eq!(stack.unwrap().id, 14);
    assert_eq!(counters.lose, 1);
}

#[test]
fn perfect_win_takes_five_and_levels_per_iteration() {
    // All ten cards on the player side. Iteration k takes enemy card k-1 and
    // then runs LevelUpInMatch on every player-hand card: 5, 6, 7, 8, 9 draws
    // (35). Draw value 2 raises mdef (max 0xf0) by one per draw, so P0..P4 get
    // +5 and the enemy cards taken at iteration k are in hand for the later
    // 5-k iterations: E0 +4, E1 +3, E2 +2, E3 +1, E4 +0.
    let (g, pc) = finished([0; 5], [0; 5]);
    let collection = [pc[0], pc[1], pc[2], pc[3], pc[4]];
    let mut stack = None;
    let mut counters = Counters::default();
    let mut rng = ScriptedRng::new(&[2; 35]);
    let s = settle(
        &g,
        &big_table(),
        &collection,
        [0, 1, 2, 3, 4],
        0,
        &mut stack,
        &mut counters,
        &mut rng,
    )
    .unwrap();
    assert!(s.perfect);
    assert_eq!(rng.remaining(), 0);
    assert_eq!(s.transfers.len(), 5);
    assert!(s.transfers.iter().all(|t| t.to_side == 0));
    let (_, ec) = scripted_cards();
    let want: Vec<u8> = (0..5)
        .map(|i| pc[i].mdef + 5)
        .chain((0..5).map(|i| ec[i].mdef + (4 - i as u8)))
        .collect();
    assert_eq!(
        s.collection.iter().map(|c| c.mdef).collect::<Vec<_>>(),
        want
    );
    assert_eq!(s.collection.len(), 10);
    assert_eq!(counters.win, 1);
}

#[test]
fn draw_restores_the_list_and_keeps_in_match_changes() {
    // 5 v 5. P2 became Flexible in the match (shared object): the saved list
    // keeps that, nothing is transferred, no random draws.
    let (mut g, pc) = finished([0; 5], [1; 5]);
    if let Cell::Placed { card, .. } = &mut g.cells[2] {
        card.kind = CardType::Flexible;
    }
    let extra = card(40, 0, 1, 1, 1, 0);
    let collection = [extra, pc[0], pc[1], pc[2], pc[3], pc[4]];
    let mut stack = None;
    let mut counters = Counters::default();
    let s = settle(
        &g,
        &big_table(),
        &collection,
        [1, 2, 3, 4, 5],
        0,
        &mut stack,
        &mut counters,
        &mut ScriptedRng::new(&[]),
    )
    .unwrap();
    assert_eq!(s.outcome, Outcome::Draw);
    assert!(!s.perfect);
    assert!(s.transfers.is_empty());
    assert_eq!(s.collection.len(), 6);
    assert_eq!(s.collection[0].id, 40);
    assert_eq!(s.collection[3].kind, CardType::Flexible);
    assert_eq!(
        counters,
        Counters {
            win: 0,
            lose: 0,
            draw: 1
        }
    );
}

// ---------------------------------------------------------------- level-up in battle

#[test]
fn level_up_in_battle_draw_ranges() {
    let mut c = card(1, 0, 0, 0, 0, 0);
    let mut rng = ScriptedRng::new(&[0]);
    c.level_up_in_battle(&mut rng);
    assert_eq!(c.kind, CardType::Flexible);
    assert_eq!(rng.log, vec![(0, 64, 0)]);
    let mut rng = ScriptedRng::new(&[127]);
    c.level_up_in_battle(&mut rng);
    assert_eq!(c.kind, CardType::Flexible);
    assert_eq!(rng.log, vec![(0, 128, 127)]);
    let mut rng = ScriptedRng::new(&[0]);
    c.level_up_in_battle(&mut rng);
    assert_eq!(c.kind, CardType::Assault);
    // Assault draws nothing.
    c.level_up_in_battle(&mut ScriptedRng::new(&[]));
}

// ---------------------------------------------------------------- real install

#[test]
fn install_tables_load_when_present() {
    let path = std::path::Path::new(
        "E:/SteamLibrary/steamapps/common/FINAL FANTASY IX/x64/FF9_Data/resources.assets",
    );
    if !path.is_file() {
        eprintln!("skipping: FFIX install not present");
        return;
    }
    use tantalus_assets::pc::audio_decode::read_text_asset;
    let cards = read_text_asset(path, "MINIGAME_CARD_DATA_ADDRESS").unwrap();
    let level = read_text_asset(path, "MINIGAME_CARD_LEVEL_ADDRESS").unwrap();
    let stage = read_text_asset(path, "MINIGAME_STAGE_ADDRESS").unwrap();
    assert_eq!((cards.len(), level.len(), stage.len()), (500, 1024, 512));
    let t = CardTable::from_bytes(&cards).unwrap();
    assert_eq!(t.len(), 100);
    // Record 0 reads 07 00 09 04 0a.
    let c0 = t.max_stat_card(0);
    assert_eq!(
        (c0.atk, c0.kind, c0.pdef, c0.mdef, c0.cpoint),
        (7, CardType::Physical, 9, 4, 10)
    );
    let e = EnemyTables::from_bytes(&level, &stage).unwrap();
    assert_eq!(e.stage(0), (0, 0));
    // Level row 0 begins 00 00 00 01 01 01: roll 0 -> index 0 -> id 0,
    // roll 100 -> index 5 -> id 1.
    assert_eq!(e.card_id(0, &mut ScriptedRng::new(&[0])), 0);
    assert_eq!(e.card_id(0, &mut ScriptedRng::new(&[100])), 1);
    // Every id the level table can produce is a real card.
    assert!(level.iter().all(|id| (*id as usize) < 100));
    assert!((0..256).all(|s| e.stage(s).1 < 4));
}
