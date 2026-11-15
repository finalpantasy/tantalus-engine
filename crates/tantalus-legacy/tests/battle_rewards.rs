#![allow(clippy::doc_lazy_continuation)]
//! Hand-derived cases for the B8 battle results
//! (`tantalus_legacy::battle::rewards`).
//!
//! Source: Memoria pin `c716e1fa`. No reference trace was captured; every
//! number is derived below from the pinned arithmetic.
//!
//! Level table used: the real `Leveling.csv` rows for levels 1-16, filler for
//! rows 17-98 (only keeps the table monotonic), real row 99
//! (`6633167;4524;642`). `exp[n]` is the threshold of level n+1 and
//! `hp[lv-1]`, `mp[lv-1]` are read at level `lv`:
//!   exp: 0, 16, 47, 101, 186, 314, 496, 746, 1078, 1510, ...
//!   hp : 250, 314, 382, 454, 530, 610, 694, ...
//!   mp : 200, 206, 212, 219, 226, 234, 242, ...

use tantalus_legacy::battle::rewards::{
    BaseStats, BattleBonus, BattleReward, EnemyRewardData, LevelBonus, LevelTables, NO_CARD,
    NO_ITEM, PaEntry, PartyMember, PartySnapshot, RewardError, enemies_gone_reward, settle_escape,
};
use tantalus_legacy::battle::rng::{Draw, SequenceRandom};

fn tables() -> LevelTables {
    let real_exp = [
        0u32, 16, 47, 101, 186, 314, 496, 746, 1078, 1510, 2059, 2745, 3588, 4612, 5840, 7298,
    ];
    let real_hp = [
        250u16, 314, 382, 454, 530, 610, 694, 782, 874, 970, 1062, 1150, 1234, 1314, 1390, 1462,
    ];
    let real_mp = [
        200u16, 206, 212, 219, 226, 234, 242, 250, 259, 268, 277, 285, 293, 301, 308, 315,
    ];
    let mut exp = real_exp.to_vec();
    let mut hp = real_hp.to_vec();
    let mut mp = real_mp.to_vec();
    for k in 16..99u32 {
        exp.push(7298 + (k - 15) * 70_000);
        hp.push(1462 + (k as u16 - 15));
        mp.push(315 + (k as u16 - 15));
    }
    exp[98] = 6_633_167;
    hp[98] = 4524;
    mp[98] = 642;
    LevelTables::new(exp, hp, mp).unwrap()
}

fn member(base: BaseStats, hp: u32, mp: u32) -> PartyMember {
    PartyMember {
        in_battle: true,
        cur_hp: hp,
        cur_mp: mp,
        rewards_disabled: false,
        level: 1,
        exp: 0,
        base,
        bonus: LevelBonus::default(),
        basis: [base.dex, base.str_, base.mgc, base.wpr],
        max_hp: 250 * u32::from(base.str_) / 50,
        max_mp: 200 * u32::from(base.mgc) / 100,
        max_capa: u32::from(base.gems),
        cur_capa: u32::from(base.gems),
        elem: [base.dex, base.str_, base.mgc, base.wpr],
        equip_stat_bonus: [0; 4],
        equipped_abilities: vec![],
        learnable: vec![],
        pa: vec![],
    }
}

const ZIDANE: BaseStats = BaseStats {
    dex: 23,
    str_: 21,
    mgc: 18,
    wpr: 23,
    gems: 18,
};
const VIVI: BaseStats = BaseStats {
    dex: 16,
    str_: 12,
    mgc: 24,
    wpr: 19,
    gems: 14,
};

fn enemy(type_id: u32, gil: u32, exp: u32, items: [u32; 4]) -> EnemyRewardData {
    EnemyRewardData {
        type_id,
        present: true,
        gil,
        exp,
        items,
        item_rates: [256, 96, 32, 1],
        card: NO_CARD,
        card_rate: 32,
        bonus_given: false,
    }
}

fn r8(values: &[i32]) -> SequenceRandom {
    let kinds: Vec<(Draw, i32)> = values.iter().map(|v| (Draw::Random8, *v)).collect();
    SequenceRandom::with_kinds(&kinds)
}

/// Draw order and same-type blanking in `SetBonus` (btl_sys.cs:215-258).
///
/// Two type-7 enemies (gil 120, exp 300, items [10,11,12,13], rates
/// 256/96/32/1) and one type-9 enemy (gil 30, exp 100, card 5 rate 32).
///   Kill E1: slot 3 draws 0 (< 1) -> item 13; slots 1-2 skipped (else-if);
///            slot 3 blanked on E1 and E2. Slot 0 draws 255 (< 256) -> 10.
///   Kill E2: slot 3 empty (no draw); slot 2 draws 31 (< 32) -> 12, blanked;
///            slot 0 draws 0 (< 256) -> 10.
///   Kill E3: no items; card slot draws 31 (< 32) -> card 5.
///   Totals: gil 120+120+30 = 270, exp 300+300+100 = 700.
fn scripted_bonus() -> BattleBonus {
    let mut enemies = vec![
        enemy(7, 120, 300, [10, 11, 12, 13]),
        enemy(7, 120, 300, [10, 11, 12, 13]),
        {
            let mut e = enemy(9, 30, 100, [NO_ITEM; 4]);
            e.card = 5;
            e
        },
    ];
    let mut bonus = BattleBonus::new(3, 0b1111, false);
    let mut rng = r8(&[0, 255, 31, 0, 31]);
    assert!(bonus.set_bonus(&mut enemies, 0, &mut rng));
    assert!(bonus.set_bonus(&mut enemies, 1, &mut rng));
    assert!(bonus.set_bonus(&mut enemies, 2, &mut rng));
    assert!(rng.is_exhausted());
    assert_eq!(enemies[0].items, [10, 11, NO_ITEM, NO_ITEM]);
    assert_eq!(enemies[1].items, [10, 11, NO_ITEM, NO_ITEM]);
    bonus
}

#[test]
fn set_bonus_draw_order_and_totals() {
    let bonus = scripted_bonus();
    assert_eq!((bonus.gil, bonus.exp, bonus.ap), (270, 700, 3));
    assert_eq!(bonus.items, vec![13, 10, 12, 10]);
    assert_eq!(bonus.card, 5);
}

#[test]
fn set_bonus_skips_a_paid_unit_without_drawing() {
    let mut enemies = vec![enemy(1, 10, 10, [1, NO_ITEM, NO_ITEM, NO_ITEM])];
    enemies[0].bonus_given = true;
    let mut bonus = BattleBonus::new(0, 1, false);
    let mut rng = r8(&[]);
    assert!(!bonus.set_bonus(&mut enemies, 0, &mut rng));
    assert_eq!((bonus.gil, bonus.exp), (0, 0));
}

/// Scripted win, full chain.
///
/// Party lines: A (Zidane, alive), B (dead), C (Petrify), D (Vivi, alive).
/// Counted for EXP: A and D -> 700 / 2 = 350 each. AP 3 each for A and D
/// (they have AP rows); B and C get nothing.
///
/// A: level 1, exp 0 -> 350. Thresholds 16, 47, 101, 186, 314 are <= 350,
///    496 is not: 5 level-ups, level 6. Bonuses after 5 ups: str 15, mgc 15,
///    wpr 5, cap 25 (all >> 5 = 0). lv = 6:
///      dex 23 + 6/10 = 23; str 21 + 18/10 = 22; mgc 18 + 1 = 19;
///      wpr 23 + 18/20 = 23.
///      max HP = hp[5] 610 * 22 / 50 = 13420 / 50 = 268; cur HP 100 stays.
///      max MP = mp[5] 234 * 19 / 100 = 4446 / 100 = 44.
///      stones: 18 + 24/10 + 0 = 20; usage 4 kept -> cur 16.
/// D: same 5 level-ups. Level 2: str 12, mgc 24, HP 314 * 12 / 50 = 75
///    (75.36) so cur HP 100 -> 75; MP 206 * 24 / 100 = 49 (49.44) so cur MP
///    50 -> 49. Level 6: str 12 + 1 = 13, mgc 24 + 1 = 25;
///    HP 610 * 13 / 50 = 158, MP 234 * 25 / 100 = 58; cur stays 75 / 49.
///      dex 16 + 0 = 16; wpr 19 + 0 = 19; stones 14 + 2 = 16 (usage 0).
/// AP A: ability 200 (max 2): tick 1: 2 <= 0+1 no -> 1; tick 2: 2 <= 2 ->
///    learned; tick 3 no-op. Ability 201 (max 5): 3. Slot of id 0 skipped.
/// AP D: ability 300 on two items (max 4): per tick it rises twice:
///    tick 1 -> 1 then 2; tick 2 -> 3 then (4 <= 3+1) learned at 4.
/// Gil 1000 + 270 = 1270. Items [13,10,12,10] -> stacks (13,1) (10,2) (12,1);
///    inventory had (10,98): the 10 stack takes 1 of 2 (overflow 1).
#[test]
fn scripted_win_credits_exact_rewards_once() {
    let bonus = scripted_bonus();
    let mut a = member(ZIDANE, 100, 30);
    a.max_capa = 18;
    a.cur_capa = 14;
    a.equipped_abilities = vec![200, 201, 0];
    a.learnable = vec![
        PaEntry { id: 200, max_ap: 2 },
        PaEntry { id: 201, max_ap: 5 },
    ];
    a.pa = vec![0, 0];
    let mut b = member(ZIDANE, 0, 0);
    b.level = 3;
    let mut c = member(ZIDANE, 50, 0);
    c.rewards_disabled = true;
    c.learnable = vec![PaEntry { id: 1, max_ap: 1 }];
    c.pa = vec![0];
    let mut d = member(VIVI, 100, 50);
    d.equipped_abilities = vec![300, 300];
    d.learnable = vec![PaEntry { id: 300, max_ap: 4 }];
    d.pa = vec![0];
    let mut party = PartySnapshot {
        gil: 1000,
        members: [Some(a), Some(b.clone()), Some(c.clone()), Some(d)],
        items: vec![(10, 98)],
        cards: vec![],
    };

    let mut reward = BattleReward::from_bonus(&bonus, &party);
    assert_eq!(reward.shares[0].exp, Some(350));
    assert_eq!(reward.shares[1].exp, None);
    assert_eq!(reward.shares[2].exp, None);
    assert_eq!(reward.shares[3].exp, Some(350));
    assert_eq!(reward.shares[0].ap, Some(3));
    assert_eq!(reward.shares[2].ap, None);
    assert_eq!(reward.gil, 270);
    assert_eq!(reward.items, vec![(13, 1), (10, 2), (12, 1)]);
    assert_eq!(reward.card, Some(5));

    let report = reward.apply(&mut party, &tables()).unwrap();
    assert!(reward.is_applied());
    assert_eq!(report.levels_gained, [5, 0, 0, 5]);
    assert_eq!(report.learned[0], vec![200]);
    assert_eq!(report.learned[3], vec![300]);
    assert_eq!(report.items_overflow, 1);
    assert_eq!(report.gil_added, 270);

    let a = party.members[0].as_ref().unwrap();
    assert_eq!((a.level, a.exp), (6, 350));
    assert_eq!(a.basis, [23, 22, 19, 23]);
    assert_eq!((a.max_hp, a.cur_hp, a.max_mp), (268, 100, 44));
    assert_eq!((a.max_capa, a.cur_capa), (20, 16));
    assert_eq!(a.pa, vec![2, 3]);
    assert_eq!(
        a.bonus,
        LevelBonus {
            dex: 0,
            str_: 15,
            mgc: 15,
            wpr: 5,
            cap: 25
        }
    );
    let d = party.members[3].as_ref().unwrap();
    assert_eq!((d.level, d.exp), (6, 350));
    assert_eq!(d.basis, [16, 13, 25, 19]);
    assert_eq!((d.max_hp, d.cur_hp), (158, 75));
    assert_eq!((d.max_mp, d.cur_mp), (58, 49));
    assert_eq!((d.max_capa, d.cur_capa), (16, 16));
    assert_eq!(d.pa, vec![4]);
    assert_eq!(party.members[1].as_ref().unwrap(), &b);
    assert_eq!(party.members[2].as_ref().unwrap(), &c);
    assert_eq!(party.gil, 1270);
    assert_eq!(party.items, vec![(10, 99), (13, 1), (12, 1)]);
    assert_eq!(party.cards, vec![5]);

    // Second application is refused and changes nothing.
    let before = party.clone();
    assert_eq!(
        reward.apply(&mut party, &tables()),
        Err(RewardError::AlreadyApplied)
    );
    assert_eq!(party, before);
}

/// A level-up that crosses the `bonus >> 5` boundary: Zidane with
/// bonus.str 30 gains 3 -> 33, 33 >> 5 = 1. Level 2: str = 21 + 6/10 + 1 = 22;
/// HP = 314 * 22 / 50 = 6908 / 50 = 138. EXP 16 is exactly the threshold
/// (`exp <= player.exp`).
#[test]
fn level_up_at_exact_threshold_with_bonus_carry() {
    let mut a = member(ZIDANE, 100, 10);
    a.bonus.str_ = 30;
    let mut party = PartySnapshot {
        members: [Some(a), None, None, None],
        ..Default::default()
    };
    let bonus = {
        let mut b = BattleBonus::new(0, 1, false);
        b.exp = 16;
        b
    };
    let mut reward = BattleReward::from_bonus(&bonus, &party);
    let report = reward.apply(&mut party, &tables()).unwrap();
    assert_eq!(report.levels_gained[0], 1);
    let a = party.members[0].as_ref().unwrap();
    assert_eq!((a.level, a.bonus.str_), (2, 33));
    assert_eq!((a.basis[1], a.max_hp), (22, 138));
}

/// Equipment stat bonus is added to the level bonus on each level-up
/// (`bonus.str += 3 + equipBonus`, ff9level.cs:97-99): equip str 29 gives
/// 0 + 3 + 29 = 32, 32 >> 5 = 1 at level 2, so str = 21 + 0 + 1 = 22; elem
/// = basis 22 + equip 29 = 51.
#[test]
fn equipment_bonus_accumulates_into_level_bonus() {
    let mut a = member(ZIDANE, 100, 10);
    a.equip_stat_bonus = [0, 29, 0, 0];
    let mut party = PartySnapshot {
        members: [Some(a), None, None, None],
        ..Default::default()
    };
    let mut bonus = BattleBonus::new(0, 1, false);
    bonus.exp = 16;
    BattleReward::from_bonus(&bonus, &party)
        .apply(&mut party, &tables())
        .unwrap();
    let a = party.members[0].as_ref().unwrap();
    assert_eq!(a.bonus.str_, 32);
    assert_eq!(a.basis[1], 22);
    assert_eq!(a.elem[1], 51);
}

/// Level 98 with exp 6,000,000 gains 700,000 -> 6,700,000 >= 6,633,167
/// (threshold of level 99). One level-up, then the `level < 99` check stops
/// it. lv 99: str = 21 + 297/10 = 29 + 0 = 50, HP = 4524 * 50 / 50 = 4524.
#[test]
fn level_99_stops_further_level_ups() {
    let mut a = member(ZIDANE, 100, 10);
    a.level = 98;
    a.exp = 6_000_000;
    let mut party = PartySnapshot {
        members: [Some(a), None, None, None],
        ..Default::default()
    };
    let mut bonus = BattleBonus::new(0, 1, false);
    bonus.exp = 700_000;
    let report = BattleReward::from_bonus(&bonus, &party)
        .apply(&mut party, &tables())
        .unwrap();
    let a = party.members[0].as_ref().unwrap();
    assert_eq!((report.levels_gained[0], a.level), (1, 99));
    assert_eq!((a.basis[1], a.max_hp), (50, 4524));
    // Another win at level 99 only adds EXP.
    let mut bonus = BattleBonus::new(0, 1, false);
    bonus.exp = 5;
    let r = BattleReward::from_bonus(&bonus, &party)
        .apply(&mut party, &tables())
        .unwrap();
    assert_eq!(r.levels_gained[0], 0);
    assert_eq!(party.members[0].as_ref().unwrap().exp, 6_700_005);
}

/// EXP split: 1000 over 3 counted lines = 333 each (remainder 1 lost); a
/// line at the cap does not count; EXP caps at 9,999,999
/// (9,999,990 + 333 -> 9,999,999).
#[test]
fn exp_split_cap_and_remainder() {
    let mut capped = member(ZIDANE, 10, 0);
    capped.exp = 9_999_999;
    let mut near = member(ZIDANE, 10, 0);
    near.exp = 9_999_990;
    near.level = 99;
    let mut party = PartySnapshot {
        members: [
            Some(member(ZIDANE, 10, 0)),
            Some(near),
            Some(member(VIVI, 10, 0)),
            Some(capped),
        ],
        ..Default::default()
    };
    let mut bonus = BattleBonus::new(0, 0b1111, false);
    bonus.exp = 1000;
    let mut reward = BattleReward::from_bonus(&bonus, &party);
    assert_eq!(
        reward.shares.map(|s| s.exp),
        [Some(333), Some(333), Some(333), None]
    );
    reward.apply(&mut party, &tables()).unwrap();
    assert_eq!(party.members[1].as_ref().unwrap().exp, 9_999_999);
    assert_eq!(party.members[3].as_ref().unwrap().exp, 9_999_999);
}

#[test]
fn dead_member_and_member_flag_gate_exp() {
    let party = PartySnapshot {
        members: [
            Some(member(ZIDANE, 0, 0)),
            Some(member(VIVI, 10, 0)),
            None,
            None,
        ],
        ..Default::default()
    };
    let mut bonus = BattleBonus::new(0, 0b0001, false);
    bonus.exp = 500;
    let reward = BattleReward::from_bonus(&bonus, &party);
    assert_eq!(reward.shares[0].exp, None); // dead
    assert_eq!(reward.shares[1].exp, None); // member_flag bit 1 not set
}

/// Gil caps at 9,999,999: 9,999,800 + 500 adds 199.
#[test]
fn gil_caps() {
    let mut party = PartySnapshot {
        gil: 9_999_800,
        ..Default::default()
    };
    let mut bonus = BattleBonus::new(0, 0, false);
    bonus.gil = 500;
    let mut reward = BattleReward::from_bonus(&bonus, &party);
    let report = reward.apply(&mut party, &tables()).unwrap();
    assert_eq!((party.gil, report.gil_added), (9_999_999, 199));
}

/// `AnalyzeBonusItem`: 6 distinct ids fit; a 7th new id stops the scan, so
/// the trailing repeat of id 1 is not counted either.
#[test]
fn item_list_limit_stops_scan() {
    let mut bonus = BattleBonus::new(0, 0, false);
    bonus.items = vec![1, 2, 3, 4, 5, 6, 7, 1, NO_ITEM];
    let reward = BattleReward::from_bonus(&bonus, &PartySnapshot::default());
    assert_eq!(
        reward.items,
        vec![(1, 1), (2, 1), (3, 1), (4, 1), (5, 1), (6, 1)]
    );
}

#[test]
fn item_stack_caps_at_99() {
    let mut party = PartySnapshot {
        items: vec![(4, 90)],
        ..Default::default()
    };
    let mut bonus = BattleBonus::new(0, 0, false);
    bonus.items = vec![4; 12];
    let mut reward = BattleReward::from_bonus(&bonus, &party);
    let report = reward.apply(&mut party, &tables()).unwrap();
    assert_eq!(party.items, vec![(4, 99)]);
    assert_eq!(report.items_overflow, 3);
}

/// Escape: Flee ability costs (bonus.gil + enemy BonusGil sum) / 10:
/// (100 + 400) / 10 = 50 from 1000 -> 950. Party flee key costs nothing.
/// Gil 20 with loss 50 -> lost 20, left 0. Rewards are cleared either way.
#[test]
fn flee_bookkeeping() {
    let mut bonus = BattleBonus::new(5, 1, false);
    bonus.gil = 100;
    bonus.exp = 999;
    let mut party = PartySnapshot {
        gil: 1000,
        ..Default::default()
    };
    let out = settle_escape(bonus.clone(), true, 400, &mut party);
    assert_eq!((out.gil_lost, party.gil), (50, 950));
    assert_eq!(out.reward, BattleReward::none());

    let out = settle_escape(bonus.clone(), false, 400, &mut party);
    assert_eq!((out.gil_lost, party.gil), (0, 950));

    party.gil = 20;
    let out = settle_escape(bonus, true, 400, &mut party);
    assert_eq!((out.gil_lost, party.gil), (20, 0));
}

/// Enemies gone (battle.cs:193): exp 0 pays nothing; exp > 0 pays as a win.
/// An after-event scene (BattleResultUI.cs:35) pays nothing either.
#[test]
fn enemies_gone_and_event_battles() {
    let mut party = PartySnapshot::default();
    party.members[0] = Some(member(ZIDANE, 10, 0));
    let mut bonus = BattleBonus::new(0, 1, false);
    bonus.gil = 7;
    assert_eq!(enemies_gone_reward(&bonus, &party), BattleReward::none());
    bonus.exp = 5;
    assert_eq!(enemies_gone_reward(&bonus, &party).gil, 7);
    bonus.event = true;
    assert_eq!(
        BattleReward::from_bonus(&bonus, &party),
        BattleReward::none()
    );
}
