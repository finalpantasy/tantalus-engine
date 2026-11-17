#![allow(clippy::doc_lazy_continuation)]
//! Hand-derived cases for `tantalus_legacy::battle`.
//!
//! Every expected number below was computed by stepping through the pinned
//! Memoria source (c716e1fa) by hand; the derivation sits above each test.
//! Notation: `r(n)` is a `Comn.random16()` draw of value `n`; `n % m` is the
//! modulo the source applies to it. The draw list passed to `run` is in call
//! order and the rig asserts every draw was used.
//!
//! Fixture (unless a test overrides it):
//!   caster (player, idx 0): level 20, STR 30, MAG 30, DEX 24, WIL 20, row 1
//!     (front), weapon power 25 / short range, 100/100 MP, 400/400 HP.
//!   target (enemy, idx 1): level 15, WIL 10, PDEF 10, MDEF 20, no evade,
//!     row 2, 500/500 HP, 50/50 MP.
//! Shared derived numbers:
//!   ctor draw: r(7) % target WIL 10 = 7 (trance increase)
//!   weapon base / low physical / magic spread: 1 + ((20 + 30) >> 3) = 7
//!   normal physical spread: 1 + ((20 + 30) >> 2) = 13
//!   critical: WIL 20 >> 2 = 5
//!   magic hit rate = command rate + (MAG 30 >> 2 = 7) + 20 - 15 = rate + 12

use tantalus_legacy::battle::calc::EatResult;
use tantalus_legacy::battle::command::COMMAND_SUMMON_EIKO;
use tantalus_legacy::battle::rng::Draw;
use tantalus_legacy::battle::status as st;
use tantalus_legacy::battle::status::{
    StatusCtx, StatusEnv, alter_status, check_point_status, remove_status,
};
use tantalus_legacy::battle::{
    BattleCommand, BattleRandom, BattleUnit, CalcEnv, CalcError, CalcFlags, CalcOutcome, Elements,
    ItemEffect, SequenceRandom, SplitMix64Random, StatusDataRow, StatusTable, UnitFlags,
    WeaponStats, calc_main, cost,
};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// `Memoria.Patcher/StreamingAssets/Data/Battle/StatusData.csv` rows 0..=31:
/// (opr_cnt, conti_cnt, clear_on_apply, immunity_provided). These are
/// Memoria's shipped values. The vanilla table in `audit/battle-tables.json`
/// differs in `invalid` for ids 0 (4294967295), 1 (65538) and 31 (2281701440).
fn table() -> StatusTable {
    let rows: [(u8, u16, u64, u64); 32] = [
        (0, 0, 134217728, 4294967294),
        (20, 0, 98304, 65536),
        (0, 0, 0, 0),
        (0, 0, 0, 0),
        (0, 0, 0, 0),
        (0, 0, 0, 0),
        (0, 0, 0, 2147500033),
        (0, 0, 0, 0),
        (0, 0, 4026506810, 4294966975),
        (0, 0, 0, 0),
        (0, 0, 0, 0),
        (0, 0, 0, 0),
        (0, 0, 0, 0),
        (0, 0, 0, 0),
        (0, 0, 2601720890, 64),
        (0, 0, 0, 0),
        (10, 30, 0, 0),
        (0, 25, 32768, 0),
        (10, 45, 0, 0),
        (0, 40, 0, 0),
        (0, 40, 0, 0),
        (0, 30, 0, 0),
        (0, 45, 0, 0),
        (0, 45, 0, 0),
        (0, 20, 33554432, 0),
        (0, 20, 83886080, 67108864),
        (0, 30, 0, 0),
        (0, 25, 0, 2147483648),
        (0, 20, 0, 0),
        (0, 40, 0, 0),
        (0, 10, 0, 3221225471),
        (0, 25, 0, 134217792),
    ];
    StatusTable::new(
        rows.iter()
            .map(|&(opr, conti, clear, invalid)| StatusDataRow {
                opr_cnt: opr,
                conti_cnt: conti,
                clear_on_apply: clear,
                immunity_provided: invalid,
            })
            .collect(),
    )
}

fn player() -> BattleUnit {
    let mut u = BattleUnit::new(1, true);
    u.character_id = 0;
    u.level = 20;
    u.strength = 30;
    u.magic = 30;
    u.dexterity = 24;
    u.will = 20;
    u.row = 1;
    u.cur_hp = 400;
    u.max_hp = 400;
    u.cur_mp = 100;
    u.max_mp = 100;
    u.weapon = Some(WeaponStats {
        power: 25,
        rate: 0,
        elements: Elements::NONE,
        category: WeaponStats::SHORT_RANGE,
        status: 0,
    });
    u
}

fn enemy() -> BattleUnit {
    let mut u = BattleUnit::new(2, false);
    u.level = 15;
    u.will = 10;
    u.physical_defence = 10;
    u.magic_defence = 20;
    u.row = 2;
    u.cur_hp = 500;
    u.max_hp = 500;
    u.cur_mp = 50;
    u.max_mp = 50;
    u
}

struct Rig {
    units: Vec<BattleUnit>,
    cmd: BattleCommand,
    env: CalcEnv,
}

impl Rig {
    /// Player (idx 0) acts on enemy (idx 1).
    fn new(script: i32) -> Self {
        Self {
            units: vec![player(), enemy()],
            cmd: BattleCommand::new(script),
            env: CalcEnv::default(),
        }
    }

    /// Enemy (idx 0, level 20, STR 30, WIL 20, row 2) acts on a player
    /// (idx 1, level 15, WIL 10, PDEF 10, row 1, 500/500 HP).
    fn enemy_attacks(script: i32) -> Self {
        let mut caster = BattleUnit::new(1, false);
        caster.level = 20;
        caster.strength = 30;
        caster.magic = 30;
        caster.will = 20;
        caster.row = 2;
        caster.cur_hp = 400;
        caster.max_hp = 400;
        let mut target = BattleUnit::new(2, true);
        target.level = 15;
        target.will = 10;
        target.physical_defence = 10;
        target.magic_defence = 20;
        target.row = 1;
        target.cur_hp = 500;
        target.max_hp = 500;
        Self {
            units: vec![caster, target],
            cmd: BattleCommand::new(script),
            env: CalcEnv::default(),
        }
    }

    fn c(&mut self) -> &mut BattleUnit {
        &mut self.units[0]
    }

    fn t(&mut self) -> &mut BattleUnit {
        &mut self.units[1]
    }

    fn try_run(&mut self, draws: &[i32]) -> Result<CalcOutcome, CalcError> {
        let mut rng = SequenceRandom::r16(draws);
        let out = calc_main(
            &mut self.units,
            0,
            1,
            self.cmd.clone(),
            self.env,
            &table(),
            &mut rng,
        );
        assert!(rng.is_exhausted(), "unused draws left: {draws:?}");
        out
    }

    fn run(&mut self, draws: &[i32]) -> CalcOutcome {
        self.try_run(draws).expect("calculation failed")
    }

    fn run_kinds(&mut self, draws: &[(Draw, i32)]) -> CalcOutcome {
        let mut rng = SequenceRandom::with_kinds(draws);
        let out = calc_main(
            &mut self.units,
            0,
            1,
            self.cmd.clone(),
            self.env,
            &table(),
            &mut rng,
        )
        .expect("calculation failed");
        assert!(rng.is_exhausted(), "unused draws left");
        out
    }
}

fn miss(o: &CalcOutcome) -> bool {
    o.flags.has(CalcFlags::MISS)
}

fn guard(o: &CalcOutcome) -> bool {
    o.flags.has(CalcFlags::GUARD)
}

/// Draws for a weapon physical hit that always connects and never crits:
/// ctor, hit (34), evade (78), then the caller's attack draws, then crit
/// pair (2 > 50 is false).
fn weapon_draws(base: i32) -> Vec<i32> {
    vec![7, 1234, 5678, base, 7, 50]
}

// ---------------------------------------------------------------------------
// Script 1: Physical Strike (BaseWeaponScript, Simple)
// ---------------------------------------------------------------------------

/// Derivation: ctor 7 % 10 = 7. Hit: rate 100 <= 1234 % 100 = 34? No. Evade:
/// 0 <= 5678 % 100 = 78, hit. base = 9999 % 7 = 3 (7 * 1428 = 9996), attack =
/// STR 30 + 3 = 33, power 25 - PDEF 10 = 15. Crit: 7 % 5 = 2, 2 > 50 % 100
/// is false. Damage 33 * 15 = 495, HP 500 -> 5.
#[test]
fn s1_basic_hit() {
    let mut r = Rig::new(1);
    r.cmd.ability_category = 8;
    let o = r.run(&weapon_draws(9999));
    assert_eq!(o.target.hp_damage, 495);
    assert_eq!(r.t().cur_hp, 5);
    assert_eq!(o.target.flags, UnitFlags(UnitFlags::HP_ALTERATION));
    assert_eq!(o.trance_increase, 7);
    assert!(!miss(&o) && !guard(&o));
}

/// Derivation: as above up to attack 33. Crit draws: 4 % 5 = 4 > 3 % 100 = 3,
/// so a critical: attack 33 * 2 = 66, damage 66 * 15 = 990 (before the
/// damage cap). 990 >= 500 HP, HP becomes 0 and the reported damage stays
/// 990. The end-of-calc check point adds Death.
#[test]
fn s1_critical_kills() {
    let mut r = Rig::new(1);
    r.cmd.ability_category = 8;
    let o = r.run(&[7, 1234, 5678, 9999, 4, 3]);
    assert_eq!(o.target.hp_damage, 990);
    assert_eq!(r.t().cur_hp, 0);
    assert!(o.target.flags.has(UnitFlags::CRITICAL));
    assert_ne!(o.added_statuses & st::DEATH, 0);
}

/// Derivation: Blind caster halves the rate: 100 / 2 = 50. 50 <= 1299 % 100
/// = 99 is true, so Miss after only two draws.
#[test]
fn s1_blind_miss() {
    let mut r = Rig::new(1);
    r.cmd.ability_category = 8;
    r.c().status_current = st::BLIND;
    let o = r.run(&[7, 1299]);
    assert!(miss(&o));
    assert_eq!(r.t().cur_hp, 500);
    assert_eq!(o.target.hp_damage, 0);
}

/// G12 (Memoria CL 2024.04.28; `AbilityFeatures.txt` SA 9 and SA 10).
/// Distract on the target halves the Attack hit rate for scripts <= 8: rate
/// 50 <= draw 1260 % 100 = 60, so it misses after two draws. Accuracy+ on the
/// caster sets rate 100 and evade 0 and disables Distract for the effect: the
/// same draw hits and the target's evade 99 is ignored.
#[test]
fn g12_accuracy_plus_overrides_distract() {
    let attack = |accuracy: bool, distract: bool, hit_draw: i32| {
        let mut r = Rig::new(1);
        r.cmd.ability_category = 8;
        r.cmd.ability_id = 176;
        r.t().physical_evade = 99;
        if accuracy {
            r.c().support_abilities.insert(9);
        }
        if distract {
            r.t().support_abilities.insert(10);
        }
        // ctor, hit, then (if the hit passes) evade draw 98: evade 99 > 98
        // dodges unless Accuracy+ zeroed it.
        let draws: Vec<i32> = if hit_draw % 100 >= 50 && distract && !accuracy {
            vec![7, hit_draw]
        } else {
            vec![7, hit_draw, 98, 9999, 7, 50]
        };
        let o = r.run(&draws);
        miss(&o)
    };
    // Distract alone: rate 50, draw 60 misses at the hit roll (two draws).
    assert!(attack(false, true, 1260));
    // Accuracy+ and Distract together: rate 100, evade 0, so the hit lands.
    assert!(!attack(true, true, 1260));
    // Accuracy+ alone behaves the same.
    assert!(!attack(true, false, 1260));
}

/// Derivation: target evade 30. Hit draw 1234 % 100 = 34 < 100 passes; evade
/// draw 12: 30 <= 12 is false, so Miss; the target is not covering, so it
/// also sets Dodge.
#[test]
fn s1_evade_dodge() {
    let mut r = Rig::new(1);
    r.cmd.ability_category = 8;
    r.t().physical_evade = 30;
    let o = r.run(&[7, 1234, 12]);
    assert!(miss(&o));
    assert!(o.dodged);
}

/// Derivation: STR 200, level 99, weapon power 99, PDEF 0, WIL 0 (so no
/// crit draws). base spread 1 + ((99 + 200) >> 3) = 1 + 37 = 38, draw 0 -> 0.
/// attack 200, diff 99, 200 * 99 = 19800 > 9999, capped to 9999.
/// HP 20000 -> 10001.
#[test]
fn s1_damage_cap_9999() {
    let mut r = Rig::new(1);
    r.cmd.ability_category = 8;
    {
        let c = r.c();
        c.strength = 200;
        c.level = 99;
        c.will = 0;
        c.weapon.as_mut().unwrap().power = 99;
    }
    {
        let t = r.t();
        t.physical_defence = 0;
        t.cur_hp = 20000;
        t.max_hp = 20000;
    }
    let o = r.run(&[7, 1234, 5678, 0]);
    assert_eq!(o.target.hp_damage, 9999);
    assert_eq!(r.t().cur_hp, 10001);
}

/// Derivation: Defend target: hit rate 100 / 2 = 50, draw 1230 % 100 = 30,
/// 50 <= 30 false, hit continues. Attack 33, Defend: 33 >> 1 = 16. Caster
/// back row 0 vs enemy row 2: |0 - 2| = 2 > 1, no long range weapon, short
/// range command: 16 >> 1 = 8. Damage 8 * 15 = 120, HP 500 -> 380.
#[test]
fn s1_defend_and_back_row_halve_twice() {
    let mut r = Rig::new(1);
    r.cmd.ability_category = 8;
    r.cmd.is_short_range = true;
    r.c().row = 0;
    r.t().status_current = st::DEFEND;
    let mut d = weapon_draws(9999);
    d[1] = 1230;
    let o = r.run(&d);
    assert_eq!(o.target.hp_damage, 120);
    assert_eq!(r.t().cur_hp, 380);
}

/// Derivation: attack 33. Berserk caster: 33 * 3 = 99 >> 1 = 49. Sleeping
/// target: 49 * 3 = 147 >> 1 = 73 (Sleep also zeroes evade; the evade draw
/// is still taken). 73 * 15 = 1095, HP 2000 -> 905. Physical category (8)
/// removes Sleep after the hit.
#[test]
fn s1_berserk_vs_sleeping_target() {
    let mut r = Rig::new(1);
    r.cmd.ability_category = 8;
    r.c().status_current = st::BERSERK;
    r.t().status_current = st::SLEEP;
    r.t().cur_hp = 2000;
    r.t().max_hp = 2000;
    let o = r.run(&weapon_draws(9999));
    assert_eq!(o.target.hp_damage, 1095);
    assert_eq!(r.t().cur_hp, 905);
    assert_eq!(r.t().status_current & st::SLEEP, 0);
}

/// Derivation: Mini caster sets attack to 1 ("DecreaseAttackDrastically").
/// 1 * 15 = 15, HP 500 -> 485.
#[test]
fn s1_mini_caster_attack_is_one() {
    let mut r = Rig::new(1);
    r.cmd.ability_category = 8;
    r.c().status_current = st::MINI;
    let o = r.run(&weapon_draws(9999));
    assert_eq!(o.target.hp_damage, 15);
    assert_eq!(r.t().cur_hp, 485);
}

/// Derivation: Steiner (character 3) in Trance. Player Trance: 33 * 3 >> 1 =
/// 49. Steiner doubling: 49 * 2 = 98. 98 * 15 = 1470, HP 5000 -> 3530.
#[test]
fn s1_steiner_trance_doubles() {
    let mut r = Rig::new(1);
    r.cmd.ability_category = 8;
    r.c().character_id = 3;
    r.c().status_current = st::TRANCE;
    r.t().cur_hp = 5000;
    r.t().max_hp = 5000;
    let o = r.run(&weapon_draws(9999));
    assert_eq!(o.target.hp_damage, 1470);
    assert_eq!(r.t().cur_hp, 3530);
}

/// Derivation: physical category (8) and the target is Frozen: CalcMain's
/// pre-check shatters it (HP 0, no hit rolls). Only the ctor draw is taken.
#[test]
fn s1_frozen_target_shatters() {
    let mut r = Rig::new(1);
    r.cmd.ability_category = 8;
    r.t().status_current = st::FREEZE;
    let o = r.run(&[7]);
    assert_eq!(r.t().cur_hp, 0);
    assert_eq!(o.target.hp_damage, 0);
    assert_ne!(o.added_statuses & st::DEATH, 0);
}

/// Derivation: weapon element Fire, target guards Fire: Guard after the
/// base-damage draw (ctor, hit, evade, base = 4 draws), no crit draws.
#[test]
fn s1_guarded_weapon_element() {
    let mut r = Rig::new(1);
    r.cmd.ability_category = 8;
    r.c().weapon.as_mut().unwrap().elements = Elements(Elements::FIRE);
    r.t().guard_elements = Elements(Elements::FIRE);
    let o = r.run(&[7, 1234, 5678, 9999]);
    assert!(guard(&o));
    assert_eq!(r.t().cur_hp, 500);
}

/// Derivation: weapon Fire vs weak Fire: 33 * 3 >> 1 = 49. 49 * 15 = 735,
/// HP 2000 -> 1265.
#[test]
fn s1_weak_element_bonus() {
    let mut r = Rig::new(1);
    r.cmd.ability_category = 8;
    r.c().weapon.as_mut().unwrap().elements = Elements(Elements::FIRE);
    r.t().weak_elements = Elements(Elements::FIRE);
    r.t().cur_hp = 2000;
    r.t().max_hp = 2000;
    let o = r.run(&weapon_draws(9999));
    assert_eq!(o.target.hp_damage, 735);
    assert_eq!(r.t().cur_hp, 1265);
}

/// Derivation: Ipsen's Castle reverse attack applies after the crit roll:
/// power 60 - 25 = 35, diff 35 - 10 = 25, 33 * 25 = 825.
#[test]
fn s1_reverse_attack() {
    let mut r = Rig::new(1);
    r.cmd.ability_category = 8;
    r.env.reverse_attack = true;
    let o = r.run(&weapon_draws(9999));
    assert_eq!(o.target.hp_damage, 825);
}

/// Derivation: attack is raised to 1 and the power difference to 1 when the
/// weapon is weaker than the defense: power 25, PDEF 40: diff -15 -> 1;
/// 33 * 1 = 33.
#[test]
fn s1_power_difference_floor_is_one() {
    let mut r = Rig::new(1);
    r.cmd.ability_category = 8;
    r.t().physical_defence = 40;
    let o = r.run(&weapon_draws(9999));
    assert_eq!(o.target.hp_damage, 33);
}

/// Behind-the-target bonus and a long-range weapon: same direction adds
/// 33 * 3 >> 1 = 49; a gap of 2 rows with a long-range weapon has no
/// penalty. 49 * 15 = 735.
#[test]
fn s1_back_attack_and_long_range() {
    let mut r = Rig::new(1);
    r.cmd.ability_category = 8;
    r.cmd.is_short_range = true;
    r.env.caster_same_direction_as_target = true;
    r.c().row = 0;
    r.c().weapon.as_mut().unwrap().category = WeaponStats::LONG_RANGE;
    let o = r.run(&weapon_draws(9999));
    assert_eq!(o.target.hp_damage, 735);
}

// ---------------------------------------------------------------------------
// Scripts 2, 3, 4, 5, 7: other weapon attack bonuses
// ---------------------------------------------------------------------------

fn weapon_variant(script: i32, tweak: impl Fn(&mut Rig), draws: &[i32]) -> CalcOutcome {
    let mut r = Rig::new(script);
    r.cmd.ability_category = 8;
    tweak(&mut r);
    r.run(draws)
}

/// Script 2 (Will): attack = ((STR 30 + WIL 20) >> 1) + base = 25 + base,
/// diff 15. base 3 -> 28 * 15 = 420; base 6 -> 31 * 15 = 465; base 0 ->
/// 25 * 15 = 375.
#[test]
fn s2_will_weapon() {
    assert_eq!(
        weapon_variant(2, |_| {}, &weapon_draws(9999))
            .target
            .hp_damage,
        420
    );
    assert_eq!(
        weapon_variant(2, |_| {}, &weapon_draws(6)).target.hp_damage,
        465
    );
    assert_eq!(
        weapon_variant(2, |_| {}, &weapon_draws(0)).target.hp_damage,
        375
    );
}

/// Script 3 (Dexterity): attack = ((30 + DEX 24) >> 1) + base = 27 + base.
/// base 3 -> 30 * 15 = 450; base 6 -> 33 * 15 = 495; base 0 -> 27 * 15 = 405.
#[test]
fn s3_dexterity_weapon() {
    assert_eq!(
        weapon_variant(3, |_| {}, &weapon_draws(9999))
            .target
            .hp_damage,
        450
    );
    assert_eq!(
        weapon_variant(3, |_| {}, &weapon_draws(6)).target.hp_damage,
        495
    );
    assert_eq!(
        weapon_variant(3, |_| {}, &weapon_draws(0)).target.hp_damage,
        405
    );
}

/// Script 4 (Magic), MAG set to 10: attack = ((30 + 10) >> 1) + base = 20 +
/// base. base 3 -> 23 * 15 = 345; base 6 -> 26 * 15 = 390; base 0 -> 20 * 15
/// = 300. A MAG of 11 gives (41 >> 1) = 20 too (truncation).
#[test]
fn s4_magic_weapon() {
    let set = |r: &mut Rig| r.c().magic = 10;
    assert_eq!(
        weapon_variant(4, set, &weapon_draws(9999)).target.hp_damage,
        345
    );
    assert_eq!(
        weapon_variant(4, set, &weapon_draws(6)).target.hp_damage,
        390
    );
    assert_eq!(
        weapon_variant(4, set, &weapon_draws(0)).target.hp_damage,
        300
    );
    let odd = |r: &mut Rig| r.c().magic = 11;
    assert_eq!(
        weapon_variant(4, odd, &weapon_draws(0)).target.hp_damage,
        300
    );
}

/// Script 5 (Random): the second draw is `r % STR` added to base. Draws:
/// ctor, hit, evade, base, r, crit pair.
/// (base 3, r 100 % 30 = 10): 13 * 15 = 195.
/// (base 3, r 29 % 30 = 29): 32 * 15 = 480.
/// (base 6, r 0): 6 * 15 = 90.
/// STR 0: `% Caster.Strength` divides by zero after the draw -> error.
#[test]
fn s5_random_weapon() {
    let run = |base: i32, rnd: i32| {
        weapon_variant(5, |_| {}, &[7, 1234, 5678, base, rnd, 7, 50])
            .target
            .hp_damage
    };
    assert_eq!(run(9999, 100), 195);
    assert_eq!(run(9999, 29), 480);
    assert_eq!(run(6, 0), 90);

    let mut r = Rig::new(5);
    r.cmd.ability_category = 8;
    r.c().strength = 0;
    // base spread 1 + ((20 + 0) >> 3) = 3, draw 2 -> 2.
    let err = r.try_run(&[7, 1234, 5678, 2, 5]).unwrap_err();
    assert_eq!(err, CalcError::DivideByZero("random16() % Caster.Strength"));
}

/// Script 7 (Level): attack power = 25 + level 20 = 45, diff 35; attack = 30
/// + base. base 3 -> 33 * 35 = 1155; base 0 -> 30 * 35 = 1050; base 6 -> 36
/// * 35 = 1260.
#[test]
fn s7_level_weapon() {
    assert_eq!(
        weapon_variant(7, |_| {}, &weapon_draws(9999))
            .target
            .hp_damage,
        1155
    );
    assert_eq!(
        weapon_variant(7, |_| {}, &weapon_draws(0)).target.hp_damage,
        1050
    );
    assert_eq!(
        weapon_variant(7, |_| {}, &weapon_draws(6)).target.hp_damage,
        1260
    );
}

// ---------------------------------------------------------------------------
// Script 6: Blood Sword (drain)
// ---------------------------------------------------------------------------

/// Derivation: draws as script 1 (attack 33, diff 15 -> 495). Drain: the
/// target loses 495 (HP 500 -> 5) and the caster recovers 495: 100 + 495 =
/// 595 >= max 400, so HP 400. Caster flags HP alteration + recovery.
#[test]
fn s6_blood_sword_drains_hp() {
    let mut r = Rig::new(6);
    r.cmd.ability_category = 8;
    r.c().cur_hp = 100;
    let o = r.run(&weapon_draws(9999));
    assert_eq!(o.target.hp_damage, 495);
    assert_eq!(r.t().cur_hp, 5);
    assert_eq!(r.c().cur_hp, 400);
    assert_eq!(o.caster.hp_damage, 495);
    assert!(o.caster.flags.has(UnitFlags::HP_RECOVERY));
}

/// Derivation: a zombie target is healed by the drain and the caster takes
/// the damage: target 100 + 495 = 595 > max 500 -> 500; caster 400 - 495 ->
/// 0 (400 > 495 false).
#[test]
fn s6_blood_sword_zombie_target_heals() {
    let mut r = Rig::new(6);
    r.cmd.ability_category = 8;
    r.t().status_current = st::ZOMBIE;
    r.t().cur_hp = 100;
    let o = r.run(&weapon_draws(9999));
    assert_eq!(r.t().cur_hp, 500);
    assert_eq!(r.c().cur_hp, 0);
    assert!(o.target.flags.has(UnitFlags::HP_RECOVERY));
    assert!(!o.caster.flags.has(UnitFlags::HP_RECOVERY));
}

/// Derivation: a dead target gives Miss before any hit roll (only the ctor
/// draw); the caster hitting itself gives Miss as well.
#[test]
fn s6_blood_sword_dead_or_self_target_misses() {
    let mut r = Rig::new(6);
    r.cmd.ability_category = 8;
    r.t().status_current = st::DEATH;
    r.t().cur_hp = 0;
    assert!(miss(&r.run(&[7])));

    let mut units = vec![player(), enemy()];
    let mut cmd = BattleCommand::new(6);
    cmd.ability_category = 8;
    let mut rng = SequenceRandom::r16(&[7]);
    // Self target: target index = caster index; ctor uses caster WIL 20.
    let o = calc_main(
        &mut units,
        0,
        0,
        cmd,
        CalcEnv::default(),
        &table(),
        &mut rng,
    )
    .unwrap();
    assert!(miss(&o));
    assert!(rng.is_exhausted());
}

// ---------------------------------------------------------------------------
// Script 8 and 100: enemy physical attacks
// ---------------------------------------------------------------------------

/// Enemy draws: ctor, hit, evade, attack spread, crit pair, status draw.
fn enemy_draws(spread: i32) -> Vec<i32> {
    vec![7, 1234, 5678, spread, 7, 50, 5]
}

/// Derivation: STR 30 + (9999 % 13 = 2) = 32 (spread 1 + ((20 + 30) >> 2) =
/// 13; 13 * 769 = 9997). Command power 20 - PDEF 10 = 10. 32 * 10 = 320, HP
/// 500 -> 180. The status draw: command rate 0 > 5 false.
#[test]
fn s8_enemy_attack_base() {
    let mut r = Rig::enemy_attacks(8);
    r.cmd.ability_category = 8;
    r.cmd.power = 20;
    let o = r.run(&enemy_draws(9999));
    assert_eq!(o.target.hp_damage, 320);
    assert_eq!(r.t().cur_hp, 180);
}

/// Derivation: target in the back row 0 vs enemy row 2: gap 2 > 1 and the
/// command is short range: 32 >> 1 = 16, 16 * 10 = 160.
#[test]
fn s8_enemy_attack_back_row() {
    let mut r = Rig::enemy_attacks(8);
    r.cmd.ability_category = 8;
    r.cmd.power = 20;
    r.cmd.is_short_range = true;
    r.t().row = 0;
    let o = r.run(&enemy_draws(9999));
    assert_eq!(o.target.hp_damage, 160);
}

/// Derivation: critical draws 4 % 5 = 4 > 3: attack 32 * 2 = 64, 64 * 10 =
/// 640 >= 500, HP 0.
#[test]
fn s8_enemy_attack_critical() {
    let mut r = Rig::enemy_attacks(8);
    r.cmd.ability_category = 8;
    r.cmd.power = 20;
    let o = r.run(&[7, 1234, 5678, 9999, 4, 3, 5]);
    assert_eq!(o.target.hp_damage, 640);
    assert_eq!(r.t().cur_hp, 0);
}

/// Derivation: an enemy in Trance gets the +50% enemy bonus (32 * 3 >> 1 =
/// 48), and Trance also zeroes its target's evade chance. 48 * 10 = 480.
#[test]
fn s8_enemy_trance_bonus() {
    let mut r = Rig::enemy_attacks(8);
    r.cmd.ability_category = 8;
    r.cmd.power = 20;
    r.c().status_current = st::TRANCE;
    let o = r.run(&enemy_draws(9999));
    assert_eq!(o.target.hp_damage, 480);
}

/// Derivation: element Fire guarded by the target: Guard after the attack
/// spread draw, no crit or status draws.
#[test]
fn s8_enemy_attack_guarded_element() {
    let mut r = Rig::enemy_attacks(8);
    r.cmd.ability_category = 8;
    r.cmd.power = 20;
    r.cmd.element = Elements(Elements::FIRE);
    r.cmd.element_for_bonus = Elements(Elements::FIRE);
    r.t().guard_elements = Elements(Elements::FIRE);
    let o = r.run(&[7, 1234, 5678, 9999]);
    assert!(guard(&o));
}

/// Derivation: attached status. Rate 100 > 5, so Poison (CSV: conti 30, opr
/// 10) is added to the player target with WIL 10. Poison is a negative
/// status: default factor (60 - 10) << 3 = 400, conti = 30 * 400 = 12000.
/// Poison tick: (WIL 10 << 2) = 40 as u16, opr = 10 * 40 = 400.
#[test]
fn s8_enemy_attack_adds_poison() {
    let mut r = Rig::enemy_attacks(8);
    r.cmd.ability_category = 8;
    r.cmd.power = 20;
    r.cmd.hit_rate = 100;
    r.cmd.ability_status = st::POISON;
    let o = r.run(&enemy_draws(9999));
    assert_eq!(o.target.hp_damage, 320);
    assert_ne!(r.t().status_current & st::POISON, 0);
    assert_eq!(r.t().conti[&16], 12000);
    assert_eq!(r.t().opr[&16], 400);
    assert_ne!(o.added_statuses & st::POISON, 0);
}

/// Script 100: no hit rolls. Draws: ctor, spread, status draw. 32 * 10 = 320.
/// Defend halves first: 32 >> 1 = 16 -> 160. Weak Fire: 32 * 3 >> 1 = 48 ->
/// 480.
#[test]
fn s100_precise_enemy_attack() {
    let mut r = Rig::enemy_attacks(100);
    r.cmd.power = 20;
    assert_eq!(r.run(&[7, 9999, 5]).target.hp_damage, 320);

    let mut r = Rig::enemy_attacks(100);
    r.cmd.power = 20;
    r.t().status_current = st::DEFEND;
    assert_eq!(r.run(&[7, 9999, 5]).target.hp_damage, 160);

    let mut r = Rig::enemy_attacks(100);
    r.cmd.power = 20;
    r.cmd.element = Elements(Elements::FIRE);
    r.t().weak_elements = Elements(Elements::FIRE);
    assert_eq!(r.run(&[7, 9999, 5]).target.hp_damage, 480);
}

// ---------------------------------------------------------------------------
// Script 9: basic magic attack
// ---------------------------------------------------------------------------

fn magic_rig() -> Rig {
    // Fire spell: power 50, no status, category magical (16).
    let mut r = Rig::new(9);
    r.cmd.power = 50;
    r.cmd.ability_category = 16;
    r.cmd.element = Elements(Elements::FIRE);
    r.cmd.element_for_bonus = Elements(Elements::FIRE);
    r.t().cur_hp = 2000;
    r.t().max_hp = 2000;
    r
}

/// Magic draws: ctor, attack spread, status draw.
const MAGIC_DRAWS: [i32; 3] = [7, 9999, 5];

/// Derivation: spread 1 + ((20 + 30) >> 3) = 7; 9999 % 7 = 3; attack = MAG
/// 30 + 3 = 33; power 50 - MDEF 20 = 30; 33 * 30 = 990; HP 2000 -> 1010.
#[test]
fn s9_magic_base() {
    let mut r = magic_rig();
    let o = r.run(&MAGIC_DRAWS);
    assert_eq!(o.target.hp_damage, 990);
    assert_eq!(r.t().cur_hp, 1010);
}

/// Derivation: weak: 33 * 3 >> 1 = 49; 49 * 30 = 1470.
/// Half: 33 >> 1 = 16; 16 * 30 = 480.
/// Shell and Half: 33 >> 1 = 16 (Shell), then 16 >> 1 = 8 (Half); 240.
#[test]
fn s9_element_modifiers() {
    let mut r = magic_rig();
    r.t().weak_elements = Elements(Elements::FIRE);
    assert_eq!(r.run(&MAGIC_DRAWS).target.hp_damage, 1470);

    let mut r = magic_rig();
    r.t().half_elements = Elements(Elements::FIRE);
    assert_eq!(r.run(&MAGIC_DRAWS).target.hp_damage, 480);

    let mut r = magic_rig();
    r.t().half_elements = Elements(Elements::FIRE);
    r.t().status_current = st::SHELL;
    assert_eq!(r.run(&MAGIC_DRAWS).target.hp_damage, 240);
}

/// Derivation: absorb Fire: defense becomes 0, diff 50, 33 * 50 = 1650 and
/// the hit heals: 500 + 1650 >= max 600 -> 600. Reported 1650.
#[test]
fn s9_absorb_heals() {
    let mut r = magic_rig();
    r.t().absorb_elements = Elements(Elements::FIRE);
    r.t().cur_hp = 500;
    r.t().max_hp = 600;
    let o = r.run(&MAGIC_DRAWS);
    assert_eq!(o.target.hp_damage, 1650);
    assert!(o.target.flags.has(UnitFlags::HP_RECOVERY));
    assert_eq!(r.t().cur_hp, 600);
}

/// Derivation: guarded Fire: Guard after the spread draw (2 draws).
#[test]
fn s9_guard() {
    let mut r = magic_rig();
    r.t().guard_elements = Elements(Elements::FIRE);
    let o = r.run(&[7, 9999]);
    assert!(guard(&o));
    assert_eq!(r.t().cur_hp, 2000);
}

/// Derivation: multi-target (ManyEnemy = 5) is "divided": 33 >> 1 = 16;
/// 16 * 30 = 480. Caster Mini: 33 >> 1 = 16 -> 480 as well. Caster element
/// bonus: 33 * 3 >> 1 = 49 -> 1470.
#[test]
fn s9_divided_mini_and_bonus() {
    let mut r = magic_rig();
    r.cmd.is_many_target = true;
    r.cmd.target_type = 5;
    assert_eq!(r.run(&MAGIC_DRAWS).target.hp_damage, 480);

    let mut r = magic_rig();
    r.c().status_current = st::MINI;
    assert_eq!(r.run(&MAGIC_DRAWS).target.hp_damage, 480);

    let mut r = magic_rig();
    r.c().bonus_elements = Elements(Elements::FIRE);
    assert_eq!(r.run(&MAGIC_DRAWS).target.hp_damage, 1470);
}

/// Derivation: Earth magic against a floating target: Miss, after the
/// spread draw (2 draws).
#[test]
fn s9_earth_misses_floating_target() {
    let mut r = magic_rig();
    r.cmd.element = Elements(Elements::EARTH);
    r.cmd.element_for_bonus = Elements(Elements::EARTH);
    r.t().status_current = st::FLOAT;
    let o = r.run(&[7, 9999]);
    assert!(miss(&o));
    assert_eq!(r.t().cur_hp, 2000);
}

/// Derivation: MAG 120: spread 1 + ((20 + 120) >> 3) = 18, draw 0 -> 0;
/// attack 120; power 99, MDEF 0: 120 * 99 = 11880 -> capped 9999. HP 20000
/// -> 10001.
#[test]
fn s9_magic_cap_9999() {
    let mut r = magic_rig();
    r.cmd.power = 99;
    r.c().magic = 120;
    r.t().magic_defence = 0;
    r.t().cur_hp = 20000;
    r.t().max_hp = 20000;
    let o = r.run(&[7, 0, 5]);
    assert_eq!(o.target.hp_damage, 9999);
    assert_eq!(r.t().cur_hp, 10001);
}

/// Derivation: short summon: 990 * 2 / 3 = 660; HP 2000 -> 1340.
#[test]
fn s9_short_summon_two_thirds() {
    let mut r = magic_rig();
    r.cmd.is_short_summon = true;
    let o = r.run(&MAGIC_DRAWS);
    assert_eq!(o.target.hp_damage, 660);
    assert_eq!(r.t().cur_hp, 1340);
}

/// Derivation: status rate 100 > 5 % 100: Silence is added through
/// `TryAlterMagicStatuses` (no Miss/Guard flag since changeContext is false).
#[test]
fn s9_magic_adds_status() {
    let mut r = magic_rig();
    r.cmd.hit_rate = 100;
    r.cmd.ability_status = st::SILENCE;
    let o = r.run(&MAGIC_DRAWS);
    assert_ne!(r.t().status_current & st::SILENCE, 0);
    assert_eq!(o.target.hp_damage, 990);
}

/// Derivation: Cold magic removes Heat from the target; Fire removes
/// Freeze.
#[test]
fn s9_element_status_side_effects() {
    let mut r = magic_rig();
    r.cmd.element = Elements(Elements::COLD);
    r.cmd.element_for_bonus = Elements(Elements::COLD);
    r.t().status_current = st::HEAT;
    r.run(&MAGIC_DRAWS);
    assert_eq!(r.t().status_current & st::HEAT, 0);

    let mut r = magic_rig();
    r.t().status_current = st::FREEZE;
    r.run(&MAGIC_DRAWS);
    assert_eq!(r.t().status_current & st::FREEZE, 0);
}

// ---------------------------------------------------------------------------
// Script 10: Cure family
// ---------------------------------------------------------------------------

fn cure_rig() -> Rig {
    let mut r = Rig::new(10);
    r.cmd.power = 16;
    r.cmd.ability_category = 16;
    r.t().cur_hp = 100;
    r.t().max_hp = 1000;
    r
}

/// Derivation: attack = 30 + (9999 % 7 = 3) = 33; recovery = power 16 * 33 =
/// 528 (no defense, no hit roll); HP 100 -> 628.
#[test]
fn s10_cure_base() {
    let mut r = cure_rig();
    let o = r.run(&[7, 9999]);
    assert_eq!(o.target.hp_damage, 528);
    assert_eq!(r.t().cur_hp, 628);
    assert!(o.target.flags.has(UnitFlags::HP_RECOVERY));
}

/// Derivation: HP 900 + 528 >= 1000 -> HP 1000; the reported amount stays
/// 528.
#[test]
fn s10_cure_clamps_to_max_hp() {
    let mut r = cure_rig();
    r.t().cur_hp = 900;
    let o = r.run(&[7, 9999]);
    assert_eq!(o.target.hp_damage, 528);
    assert_eq!(r.t().cur_hp, 1000);
}

/// Derivation: a zombie is not healed: no recovery flag, so 528 damage: HP
/// 600 -> 72.
#[test]
fn s10_cure_hurts_zombie() {
    let mut r = cure_rig();
    r.t().status_current = st::ZOMBIE;
    r.t().cur_hp = 600;
    let o = r.run(&[7, 9999]);
    assert!(!o.target.flags.has(UnitFlags::HP_RECOVERY));
    assert_eq!(r.t().cur_hp, 72);
}

/// Derivation: a target under Death gets nothing (SetRecover returns 0).
#[test]
fn s10_cure_on_dead_target() {
    let mut r = cure_rig();
    r.t().status_current = st::DEATH;
    r.t().cur_hp = 0;
    let o = r.run(&[7, 9999]);
    assert_eq!(o.target.hp_damage, 0);
    assert_eq!(r.t().cur_hp, 0);
}

/// Derivation: MAG 255, level 20: spread 1 + ((20 + 255) >> 3) = 35, draw 0
/// -> 0, attack 255; power 60 * 255 = 15300 -> capped 9999. HP 100 -> 10099.
#[test]
fn s10_cure_cap_9999() {
    let mut r = cure_rig();
    r.cmd.power = 60;
    r.c().magic = 255;
    r.t().max_hp = 20000;
    let o = r.run(&[7, 0]);
    assert_eq!(o.target.hp_damage, 9999);
    assert_eq!(r.t().cur_hp, 10099);
}

/// Derivation: Mini caster 33 >> 1 = 16, 16 * 16 = 256. Multi-target (type
/// 4) also halves: 256. An enemy caster in Trance: 33 * 3 >> 1 = 49, 16 * 49
/// = 784.
#[test]
fn s10_cure_modifiers() {
    let mut r = cure_rig();
    r.c().status_current = st::MINI;
    assert_eq!(r.run(&[7, 9999]).target.hp_damage, 256);

    let mut r = cure_rig();
    r.cmd.is_many_target = true;
    r.cmd.target_type = 4;
    assert_eq!(r.run(&[7, 9999]).target.hp_damage, 256);

    let mut r = cure_rig();
    r.c().is_player = false;
    r.c().status_current = st::TRANCE;
    assert_eq!(r.run(&[7, 9999]).target.hp_damage, 784);
}

// ---------------------------------------------------------------------------
// Script 11 / 103 / 109: status spells
// ---------------------------------------------------------------------------

fn status_rig(script: i32, status: u64) -> Rig {
    // rate 60: hit rate 60 + 7 + 20 - 15 = 72.
    let mut r = Rig::new(script);
    r.cmd.hit_rate = 60;
    r.cmd.ability_status = status;
    r.cmd.ability_category = 16;
    r.t().magic_evade = 10;
    r
}

/// Draws: ctor, hit (1010 % 100 = 10, 72 <= 10 false), evade (1950 % 100 =
/// 50, 10 > 50 false).
const HIT: [i32; 3] = [7, 1010, 1950];

/// Derivation: hit passes both rolls; Silence (id 3) has no resist, no
/// immunity, so it is added and no flag is raised.
#[test]
fn s11_status_applied() {
    let mut r = status_rig(11, st::SILENCE);
    let o = r.run(&HIT);
    assert_ne!(r.t().status_current & st::SILENCE, 0);
    assert!(!miss(&o) && !guard(&o));
    assert_ne!(o.added_statuses & st::SILENCE, 0);
}

/// Derivation: hit draw 7172 % 100 = 72; 72 <= 72 is true: Miss (rate equal
/// to the roll fails). Evade draw 5: 10 > 5: Miss.
#[test]
fn s11_miss_boundaries() {
    let mut r = status_rig(11, st::SILENCE);
    assert!(miss(&r.run(&[7, 7172])));
    assert_eq!(r.t().status_current, 0);

    let mut r = status_rig(11, st::SILENCE);
    assert!(miss(&r.run(&[7, 1010, 5])));
}

/// Derivation: the target resists Silence: result Resist -> Guard flag.
/// Empty status set: best result starts at Resist -> Guard too.
#[test]
fn s11_resist_and_empty_set_guard() {
    let mut r = status_rig(11, st::SILENCE);
    r.t().status_resist = st::SILENCE;
    assert!(guard(&r.run(&HIT)));

    let mut r = status_rig(11, 0);
    assert!(guard(&r.run(&HIT)));
}

/// Derivation: Shell halves the rate: 72 >> 1 = 36; draw 40: 36 <= 40 Miss.
/// Multi-target (type 3) halves: 72 / 2 = 36; draw 40 Miss. Both together
/// 72 >> 1 = 36, / 2 = 18.
#[test]
fn s11_shell_and_divided_hit_rate() {
    let mut r = status_rig(11, st::SILENCE);
    r.t().status_current = st::SHELL;
    assert!(miss(&r.run(&[7, 40])));

    let mut r = status_rig(11, st::SILENCE);
    r.cmd.is_many_target = true;
    r.cmd.target_type = 3;
    assert!(miss(&r.run(&[7, 40])));

    // Rate 18 < draw 20 -> the roll passes (18 <= 20 is true -> Miss).
    let mut r = status_rig(11, st::SILENCE);
    r.t().status_current = st::SHELL;
    r.cmd.is_many_target = true;
    r.cmd.target_type = 3;
    assert!(miss(&r.run(&[7, 18])));
    // 18 <= 17 is false: the roll passes, evade 10 > 50 false: hit.
    let mut r = status_rig(11, st::SILENCE);
    r.t().status_current = st::SHELL;
    r.cmd.is_many_target = true;
    r.cmd.target_type = 3;
    let o = r.run(&[7, 17, 1950]);
    assert!(!miss(&o));
    assert_ne!(r.t().status_current & st::SILENCE, 0);
}

/// Derivation: Sleep (CSV conti 25) on a target (WIL 10) that is Defending:
/// Sleep clears Defend (clear 32768). Negative status factor (60 - 10) << 3 =
/// 400; conti = 25 * 400 = 10000.
#[test]
fn s11_sleep_clears_defend_and_sets_duration() {
    let mut r = status_rig(11, st::SLEEP);
    r.t().status_current = st::DEFEND;
    r.run(&HIT);
    assert_eq!(r.t().status_current, st::SLEEP);
    assert_eq!(r.t().conti[&17], 10000);
}

/// Derivation: Poison on the enemy target: conti 30 * 400 = 12000, opr =
/// 10 * ((10 << 2) as u16 = 40) = 400.
#[test]
fn s11_poison_duration_and_tick() {
    let mut r = status_rig(11, st::POISON);
    r.run(&HIT);
    assert_eq!(r.t().conti[&16], 12000);
    assert_eq!(r.t().opr[&16], 400);
}

/// Derivation: a Petrified target blocks every other status (Petrify
/// immunity covers ids 1..=31): result Invalid -> Miss.
#[test]
fn s11_petrified_target_is_immune() {
    let mut r = status_rig(11, st::SILENCE);
    r.t().status_current = st::PETRIFY;
    let o = r.run(&HIT);
    assert!(miss(&o));
    assert_eq!(r.t().status_current, st::PETRIFY);
}

/// Derivation: Slow on a Hasted target removes Haste and does not set Slow
/// (result SuccessNoSet, so no flag).
#[test]
fn s11_slow_cancels_haste() {
    let mut r = status_rig(11, st::SLOW);
    r.t().status_current = st::HASTE;
    let o = r.run(&HIT);
    assert_eq!(r.t().status_current, 0);
    assert!(!miss(&o) && !guard(&o));
}

/// Script 103: no hit roll, only the ctor draw. Haste on a Slowed target
/// cancels Slow; Protect (positive, CSV conti 45) with WIL 10: factor 10 << 3
/// = 80, conti = 45 * 80 = 3600; a resisted status is a Guard; Trance on an
/// enemy without a trance gauge is Invalid -> Miss.
#[test]
fn s103_positive_status() {
    let mut r = status_rig(103, st::HASTE);
    r.t().status_current = st::SLOW;
    let o = r.run(&[7]);
    assert_eq!(r.t().status_current, 0);
    assert!(!miss(&o) && !guard(&o));

    let mut r = status_rig(103, st::PROTECT);
    r.run(&[7]);
    assert_eq!(r.t().conti[&23], 3600);

    let mut r = status_rig(103, st::SHELL);
    r.t().status_resist = st::SHELL;
    assert!(guard(&r.run(&[7])));

    let mut r = status_rig(103, st::TRANCE);
    assert!(miss(&r.run(&[7])));
}

/// Derivation: Eiko's summon (command 20, not short) adds Protect to the
/// spell's own status: Shell (conti 45 * 80 = 3600) and Protect (3600). A
/// short summon adds nothing.
#[test]
fn s103_eiko_summon_adds_protect() {
    let mut r = status_rig(103, st::SHELL);
    r.cmd.command_id = COMMAND_SUMMON_EIKO;
    r.run(&[7]);
    assert_eq!(r.t().status_current, st::SHELL | st::PROTECT);
    assert_eq!(r.t().conti[&22], 3600);
    assert_eq!(r.t().conti[&23], 3600);

    let mut r = status_rig(103, st::SHELL);
    r.cmd.command_id = COMMAND_SUMMON_EIKO;
    r.cmd.is_short_summon = true;
    r.run(&[7]);
    assert_eq!(r.t().status_current, st::SHELL);
}

/// Script 109 (Mini): a Mini target skips the hit roll and the Mini toggles
/// off (Apply removes it, result SuccessNoSet). Otherwise normal hit roll;
/// Mini is not a ContiCount status, so it gets no duration counter.
#[test]
fn s109_mini_toggle() {
    let mut r = status_rig(109, st::MINI);
    r.t().status_current = st::MINI;
    let o = r.run(&[7]);
    assert_eq!(r.t().status_current, 0);
    assert!(!miss(&o) && !guard(&o));

    let mut r = status_rig(109, st::MINI);
    r.run(&HIT);
    assert_eq!(r.t().status_current, st::MINI);
    // Mini is not in BattleStatus.ContiCount (BattleStatus.cs:122): no counter.
    assert!(!r.t().conti.contains_key(&28));

    let mut r = status_rig(109, st::MINI);
    assert!(miss(&r.run(&[7, 7172])));
}

// ---------------------------------------------------------------------------
// Script 12: cure status
// ---------------------------------------------------------------------------

/// Derivation: remove Silence and Blind. The target has Silence: removing
/// at least one status returns 2 (no flag). With none present: result 1 ->
/// Miss. A permanent status cannot be removed: Blind permanent + Silence
/// current: Silence goes, Blind stays, still no Miss.
#[test]
fn s12_cure_status() {
    let mut r = Rig::new(12);
    r.cmd.ability_status = st::SILENCE | st::BLIND;
    r.t().status_current = st::SILENCE;
    let o = r.run(&[7]);
    assert!(!miss(&o));
    assert_eq!(r.t().status_current, 0);

    let mut r = Rig::new(12);
    r.cmd.ability_status = st::SILENCE | st::BLIND;
    assert!(miss(&r.run(&[7])));

    let mut r = Rig::new(12);
    r.cmd.ability_status = st::SILENCE | st::BLIND;
    r.t().status_current = st::SILENCE | st::BLIND;
    r.t().status_permanent = st::BLIND;
    let o = r.run(&[7]);
    assert!(!miss(&o));
    assert_eq!(r.t().status_current, st::BLIND);
}

// ---------------------------------------------------------------------------
// Script 13: Life
// ---------------------------------------------------------------------------

fn revive_rig() -> Rig {
    let mut r = Rig::new(13);
    r.units[1] = {
        let mut p = BattleUnit::new(2, true);
        p.level = 15;
        p.will = 20;
        p.cur_hp = 0;
        p.max_hp = 1000;
        p.status_current = st::DEATH;
        p
    };
    r.cmd.power = 10;
    r.cmd.ability_status = st::DEATH;
    r.cmd.ability_category = 16;
    r
}

/// Derivation: HP = max 1000 * (WIL 20 + power 10) / 100 = 300. Death is
/// removed, then HP 0 + 300. ctor draw: 7 % 20 = 7.
#[test]
fn s13_revive_hp() {
    let mut r = revive_rig();
    let o = r.run(&[7]);
    assert_eq!(o.target.hp_damage, 300);
    assert_eq!(r.t().cur_hp, 300);
    assert_eq!(r.t().status_current & st::DEATH, 0);
}

/// Derivation: truncation: 999 * (7 + 10) / 100 = 16983 / 100 = 169.
#[test]
fn s13_revive_truncates() {
    let mut r = revive_rig();
    r.t().max_hp = 999;
    r.t().will = 7;
    let o = r.run(&[3]);
    assert_eq!(o.target.hp_damage, 169);
    assert_eq!(r.t().cur_hp, 169);
}

/// Derivation: a Petrified target Miss; a dead zombie Miss; an enemy Miss
/// (`CheckIsPlayer`); a living zombie rolls the magic hit and dies.
#[test]
fn s13_revive_refusals_and_zombie() {
    let mut r = revive_rig();
    r.t().status_current = st::PETRIFY;
    assert!(miss(&r.run(&[7])));

    let mut r = revive_rig();
    r.t().status_current = st::DEATH | st::ZOMBIE;
    assert!(miss(&r.run(&[7])));

    let mut r = revive_rig();
    r.t().is_player = false;
    assert!(miss(&r.run(&[7])));

    // Living zombie (player): hit rate 0 + 7 + 20 - 15 = 12 -> clamp ok;
    // draws: ctor 7, hit 1010 % 100 = 10 (12 <= 10 false), evade 0 > ... no
    // evade, 50 >= 0. Then Kill: HP 0.
    let mut r = revive_rig();
    r.t().status_current = st::ZOMBIE;
    r.t().cur_hp = 500;
    let o = r.run(&[7, 1010, 1950]);
    assert!(!miss(&o));
    assert_eq!(r.t().cur_hp, 0);
}

// ---------------------------------------------------------------------------
// Script 14: Death
// ---------------------------------------------------------------------------

fn death_rig() -> Rig {
    // rate 70: 70 + 7 + 20 - 15 = 82.
    let mut r = Rig::new(14);
    r.cmd.hit_rate = 70;
    r.cmd.ability_status = st::DEATH;
    r.cmd.ability_category = 16;
    r
}

/// Derivation: hit rolls pass (82 <= 10 false; evade 0 > 50 false); Death
/// is inflicted: the unit is killed (HP 0) and Death clears Poison, Defend
/// and the other statuses in its clear mask (0xEFFF9E3A).
#[test]
fn s14_death_kills_and_clears() {
    let mut r = death_rig();
    r.t().status_current = st::POISON | st::DEFEND;
    let o = r.run(&HIT);
    assert_eq!(r.t().cur_hp, 0);
    assert_eq!(r.t().status_current, st::DEATH);
    assert_ne!(o.added_statuses & st::DEATH, 0);
}

/// Derivation: boss (EasyKill) -> Guard right away (only the ctor draw). A
/// zombie is healed to max HP (no hit roll). A target that resists Death:
/// Guard.
#[test]
fn s14_death_edge_cases() {
    let mut r = death_rig();
    r.t().status_current = st::EASY_KILL;
    assert!(guard(&r.run(&[7])));
    assert_eq!(r.t().cur_hp, 500);

    let mut r = death_rig();
    r.t().status_current = st::ZOMBIE;
    r.t().cur_hp = 120;
    let o = r.run(&[7]);
    assert_eq!(r.t().cur_hp, 500);
    assert!(!miss(&o));

    let mut r = death_rig();
    r.t().status_resist = st::DEATH;
    assert!(guard(&r.run(&HIT)));
    assert_eq!(r.t().cur_hp, 500);
}

/// Derivation: a non-dying boss keeps 1 HP when killed.
#[test]
fn s14_non_dying_boss_keeps_one_hp() {
    let mut r = death_rig();
    r.t().non_dying_boss = true;
    r.run(&HIT);
    assert_eq!(r.t().cur_hp, 1);
}

// ---------------------------------------------------------------------------
// Scripts 15, 16, 17: drains and proportional damage
// ---------------------------------------------------------------------------

fn drain_rig(script: i32) -> Rig {
    let mut r = Rig::new(script);
    r.cmd.power = 50;
    r.cmd.ability_category = 16;
    r.t().cur_mp = 80;
    r.t().max_mp = 200;
    r.t().cur_hp = 2000;
    r.t().max_hp = 2000;
    r.c().cur_mp = 50;
    r.c().cur_hp = 100;
    r.c().max_hp = 400;
    r
}

/// Script 15 derivation: attack 33 (MAG 30 + 9999 % 7), diff 50 - 20 = 30,
/// MP damage = (30 * 33) >> 2 = 990 >> 2 = 247. The target only has 80 MP,
/// so 80 moves: target MP 0, caster MP 50 + 80 = 130 -> max 100.
#[test]
fn s15_drain_mp_limited_by_target_mp() {
    let mut r = drain_rig(15);
    let o = r.run(&[7, 9999]);
    assert_eq!(o.target.mp_damage, 80);
    assert_eq!(r.t().cur_mp, 0);
    assert_eq!(r.c().cur_mp, 100);
}

/// Derivation: target MP 300 (max 300): full 247 drained: target 300 - 247 =
/// 53; caster 50 + 247 = 297 -> 100.
#[test]
fn s15_drain_mp_full() {
    let mut r = drain_rig(15);
    r.t().cur_mp = 300;
    r.t().max_mp = 300;
    let o = r.run(&[7, 9999]);
    assert_eq!(o.target.mp_damage, 247);
    assert_eq!(r.t().cur_mp, 53);
    assert_eq!(r.c().cur_mp, 100);
}

/// Derivation: zombie target: the caster pays, limited by caster MP 50: the
/// target recovers 50 (80 + 50 = 130 <= 200), the caster loses 50 -> 0.
#[test]
fn s15_drain_mp_zombie_target() {
    let mut r = drain_rig(15);
    r.t().status_current = st::ZOMBIE;
    let o = r.run(&[7, 9999]);
    assert_eq!(o.target.mp_damage, 50);
    assert_eq!(r.t().cur_mp, 130);
    assert_eq!(r.c().cur_mp, 0);
}

/// Script 16 derivation: 33 * 30 = 990 drained: target 2000 -> 1010; caster
/// 100 + 990 -> max 400. Power difference < 1 (power 10 vs MDEF 20) stops
/// early with no damage. Zombie: target heals, caster is hurt.
#[test]
fn s16_drain_hp() {
    let mut r = drain_rig(16);
    let o = r.run(&[7, 9999]);
    assert_eq!(o.target.hp_damage, 990);
    assert_eq!(r.t().cur_hp, 1010);
    assert_eq!(r.c().cur_hp, 400);

    let mut r = drain_rig(16);
    r.cmd.power = 10;
    let o = r.run(&[7, 9999]);
    assert_eq!(o.target.hp_damage, 0);
    assert_eq!(r.t().cur_hp, 2000);

    let mut r = drain_rig(16);
    r.t().status_current = st::ZOMBIE;
    r.t().cur_hp = 1500;
    let o = r.run(&[7, 9999]);
    assert_eq!(r.t().cur_hp, 2000);
    assert_eq!(r.c().cur_hp, 0);
    assert_eq!(o.target.hp_damage, 990);
}

/// Script 17 (proportional): rate 70 -> 82. Attack = power 25; damage = 999
/// * 25 / 100 = 24975 / 100 = 249.
/// Element bonus: 25 * 3 >> 1 = 37 -> 999 * 37 / 100 = 369.
/// Power 150 caps at 100 -> 999.
/// Short summon: 249 * 2 / 3 = 166.
#[test]
fn s17_gravity_proportional() {
    let make = || {
        let mut r = Rig::new(17);
        r.cmd.hit_rate = 70;
        r.cmd.power = 25;
        r.cmd.ability_category = 16;
        r.t().cur_hp = 999;
        r.t().max_hp = 999;
        r.t().magic_evade = 10;
        r
    };
    let mut r = make();
    assert_eq!(r.run(&HIT).target.hp_damage, 249);
    assert_eq!(r.t().cur_hp, 750);

    let mut r = make();
    r.cmd.element_for_bonus = Elements(Elements::EARTH);
    r.c().bonus_elements = Elements(Elements::EARTH);
    assert_eq!(r.run(&HIT).target.hp_damage, 369);

    let mut r = make();
    r.cmd.power = 150;
    assert_eq!(r.run(&HIT).target.hp_damage, 999);
    assert_eq!(r.t().cur_hp, 0);

    let mut r = make();
    r.cmd.is_short_summon = true;
    assert_eq!(r.run(&HIT).target.hp_damage, 166);

    // Boss: Miss right after the ctor draw.
    let mut r = make();
    r.t().status_current = st::EASY_KILL;
    assert!(miss(&r.run(&[7])));
}

// ---------------------------------------------------------------------------
// Scripts 19, 20, 63: weapon-power physical and magic hybrids
// ---------------------------------------------------------------------------

fn weapon_hybrid(script: i32, power: i32) -> Rig {
    let mut r = Rig::new(script);
    r.cmd.power = power;
    r.cmd.ability_category = if script == 19 { 8 } else { 16 };
    r.t().cur_hp = 2000;
    r.t().max_hp = 2000;
    r
}

/// Script 19 derivation: attack power = weapon 25 * 14 / 10 = 35 (350 / 10);
/// attack = STR 30 + (9999 % 7 = 3) = 33; diff 35 - 10 = 25; 33 * 25 = 825.
/// HP 2000 -> 1175. Mini caster: 25. Defend target: 33 >> 1 = 16 -> 400.
/// Weak Fire weapon: 33 * 3 >> 1 = 49 -> 1225. Power 15: 25 * 15 / 10 = 37
/// (truncated) -> diff 27 -> 891.
#[test]
fn s19_physical_attack() {
    let mut r = weapon_hybrid(19, 14);
    assert_eq!(r.run(&[7, 9999, 5]).target.hp_damage, 825);
    assert_eq!(r.t().cur_hp, 1175);

    let mut r = weapon_hybrid(19, 14);
    r.c().status_current = st::MINI;
    assert_eq!(r.run(&[7, 9999, 5]).target.hp_damage, 25);

    let mut r = weapon_hybrid(19, 14);
    r.t().status_current = st::DEFEND;
    assert_eq!(r.run(&[7, 9999, 5]).target.hp_damage, 400);

    let mut r = weapon_hybrid(19, 14);
    r.c().weapon.as_mut().unwrap().elements = Elements(Elements::FIRE);
    r.t().weak_elements = Elements(Elements::FIRE);
    assert_eq!(r.run(&[7, 9999, 5]).target.hp_damage, 1225);

    let mut r = weapon_hybrid(19, 15);
    assert_eq!(r.run(&[7, 9999, 5]).target.hp_damage, 891);
}

/// Script 20 derivation: attack power 35, attack 33, MDEF 20: diff 15, 33 *
/// 15 = 495. Shell: 33 >> 1 = 16 -> 240. Mini caster: 16 -> 240. Guard Fire:
/// after 2 draws. Earth vs floating: Miss.
#[test]
fn s20_original_magic_attack() {
    let mut r = weapon_hybrid(20, 14);
    assert_eq!(r.run(&[7, 9999, 5]).target.hp_damage, 495);

    let mut r = weapon_hybrid(20, 14);
    r.t().status_current = st::SHELL;
    assert_eq!(r.run(&[7, 9999, 5]).target.hp_damage, 240);

    let mut r = weapon_hybrid(20, 14);
    r.c().status_current = st::MINI;
    assert_eq!(r.run(&[7, 9999, 5]).target.hp_damage, 240);

    let mut r = weapon_hybrid(20, 14);
    r.cmd.element = Elements(Elements::FIRE);
    r.t().guard_elements = Elements(Elements::FIRE);
    assert!(guard(&r.run(&[7, 9999])));

    let mut r = weapon_hybrid(20, 14);
    r.cmd.element = Elements(Elements::EARTH);
    r.t().status_current = st::FLOAT;
    assert!(miss(&r.run(&[7, 9999])));
}

/// Script 63: attack 33, power = weapon 25 + command 15 = 40, PDEF 10: 30;
/// 33 * 30 = 990. Enemy caster (no weapon) with power 15, in Trance: attack
/// 33 -> 49, diff 15 - 10 = 5: 245. Guard and flight Miss as for script 20.
#[test]
fn s63_magic_sword() {
    let mut r = weapon_hybrid(63, 15);
    assert_eq!(r.run(&[7, 9999]).target.hp_damage, 990);

    let mut r = Rig::enemy_attacks(63);
    r.cmd.power = 15;
    r.cmd.ability_category = 16;
    r.c().status_current = st::TRANCE;
    assert_eq!(r.run(&[7, 9999]).target.hp_damage, 245);

    let mut r = weapon_hybrid(63, 15);
    r.cmd.element = Elements(Elements::FIRE);
    r.t().guard_elements = Elements(Elements::FIRE);
    assert!(guard(&r.run(&[7, 9999])));
}

// ---------------------------------------------------------------------------
// Scripts 22, 25, 27: direct HP
// ---------------------------------------------------------------------------

/// Script 22: target level 15. Rate 5: 15 % 5 == 0 -> proceeds; power 0 ->
/// Kill (HP 0). Rate 4: 15 % 4 = 3 -> Miss. Rate 0: division by zero.
/// Power 20: direct HP set to 20 (DirectHP flag).
#[test]
fn s22_level_direct_hp() {
    let mut r = Rig::new(22);
    r.cmd.hit_rate = 5;
    let o = r.run(&[7]);
    assert_eq!(r.t().cur_hp, 0);
    assert!(!miss(&o));

    let mut r = Rig::new(22);
    r.cmd.hit_rate = 4;
    assert!(miss(&r.run(&[7])));
    assert_eq!(r.t().cur_hp, 500);

    let mut r = Rig::new(22);
    r.cmd.hit_rate = 0;
    assert_eq!(
        r.try_run(&[7]).unwrap_err(),
        CalcError::DivideByZero("Target.Level % Command.HitRate")
    );

    let mut r = Rig::new(22);
    r.cmd.hit_rate = 5;
    r.cmd.power = 20;
    let o = r.run(&[7]);
    assert!(o.flags.has(CalcFlags::DIRECT_HP));
    assert_eq!(r.t().cur_hp, 20);
}

/// Script 25: boss -> Miss; power 0 kills; zombie is healed to max.
#[test]
fn s25_precise_direct_hp() {
    let mut r = Rig::new(25);
    r.t().status_current = st::EASY_KILL;
    assert!(miss(&r.run(&[7])));

    let mut r = Rig::new(25);
    r.run(&[7]);
    assert_eq!(r.t().cur_hp, 0);

    let mut r = Rig::new(25);
    r.t().status_current = st::ZOMBIE;
    r.t().cur_hp = 77;
    r.run(&[7]);
    assert_eq!(r.t().cur_hp, 500);
}

/// Script 27: rate 70 -> 82, hit draws pass (10, 50 vs evade 10); power 0
/// kills. Shell: 82 >> 1 = 41, draw 45: 41 <= 45 Miss. Petrified target:
/// Miss from `CanBeAttacked`.
#[test]
fn s27_direct_hp_with_hit_roll() {
    let mut r = Rig::new(27);
    r.cmd.hit_rate = 70;
    r.t().magic_evade = 10;
    r.run(&HIT);
    assert_eq!(r.t().cur_hp, 0);

    let mut r = Rig::new(27);
    r.cmd.hit_rate = 70;
    r.t().status_current = st::SHELL;
    assert!(miss(&r.run(&[7, 45])));
    assert_eq!(r.t().cur_hp, 500);

    let mut r = Rig::new(27);
    r.cmd.hit_rate = 70;
    r.t().status_current = st::PETRIFY;
    assert!(miss(&r.run(&[7])));
}

// ---------------------------------------------------------------------------
// Scripts 64, 69, 73: no-op and items
// ---------------------------------------------------------------------------

/// Script 64: no effect, only the ctor draw.
#[test]
fn s64_no_effect() {
    let mut r = Rig::new(64);
    let o = r.run(&[7]);
    assert_eq!(o.flags, CalcFlags::default());
    assert_eq!(o.target.hp_damage, 0);
}

/// Script 69 (Potion: item power 10): attack fixed 15, defense 0: 10 * 15 =
/// 150, HP 100 -> 250. Over max: 900 + 150 -> 1000. A zombie takes 150 (HP
/// 400 -> 250).
#[test]
fn s69_potion_heals_150() {
    let mut r = Rig::new(69);
    r.cmd.item = Some(ItemEffect {
        power: 10,
        hit_rate: 0,
        status: 0,
        elements: Elements::NONE,
    });
    r.t().cur_hp = 100;
    r.t().max_hp = 1000;
    let o = r.run(&[7]);
    assert_eq!(o.target.hp_damage, 150);
    assert_eq!(r.t().cur_hp, 250);

    r.t().cur_hp = 900;
    r.run(&[7]);
    assert_eq!(r.t().cur_hp, 1000);

    r.t().cur_hp = 400;
    r.t().status_current = st::ZOMBIE;
    r.run(&[7]);
    assert_eq!(r.t().cur_hp, 250);
}

/// Script 69 status path: power 0 + status Protect, rate 100: the range
/// draw 99 < 100 applies it (3600 as in the Protect cases above). Rate 50
/// with draw 50: 50 < 50 false -> status-only effect Miss.
#[test]
fn s69_item_status_roll() {
    let item = |rate| ItemEffect {
        power: 0,
        hit_rate: rate,
        status: st::PROTECT,
        elements: Elements::NONE,
    };
    let mut r = Rig::new(69);
    r.cmd.item = Some(item(100));
    let o = r.run_kinds(&[(Draw::Random16, 7), (Draw::Range, 99)]);
    assert!(!miss(&o));
    assert_eq!(r.t().conti[&23], 3600);

    let mut r = Rig::new(69);
    r.cmd.item = Some(item(50));
    let o = r.run_kinds(&[(Draw::Random16, 7), (Draw::Range, 50)]);
    assert!(miss(&o));
    assert_eq!(r.t().status_current, 0);
}

/// Script 73: item cures Poison + Venom: target Poisoned -> removed, no
/// flag; target clean -> Miss.
#[test]
fn s73_item_cure_status() {
    let item = ItemEffect {
        power: 0,
        hit_rate: 0,
        status: st::POISON | st::VENOM,
        elements: Elements::NONE,
    };
    let mut r = Rig::new(73);
    r.cmd.item = Some(item);
    r.t().status_current = st::POISON;
    let o = r.run(&[7]);
    assert!(!miss(&o));
    assert_eq!(r.t().status_current, 0);

    let mut r = Rig::new(73);
    r.cmd.item = Some(item);
    assert!(miss(&r.run(&[7])));
}

// ---------------------------------------------------------------------------
// Draw order, errors, unported ids
// ---------------------------------------------------------------------------

/// A target with WIL 0: the ctor draws then divides by zero (the source
/// throws and `CalcMain` skips `CalcResult`).
#[test]
fn ctor_zero_will_is_an_error() {
    let mut r = Rig::new(1);
    r.t().will = 0;
    assert_eq!(
        r.try_run(&[7]).unwrap_err(),
        CalcError::DivideByZero("random16() % Target.Will")
    );
}

#[test]
fn unported_script_is_an_error() {
    let mut r = Rig::new(200);
    assert_eq!(r.try_run(&[7]).unwrap_err(), CalcError::UnportedScript(200));
}

/// Script 26 (`0026_ThousandNeedlesScript.cs`): Power * 100 + HitRate.
#[test]
fn s26_thousand_needles_is_fixed_damage() {
    let mut r = Rig::enemy_attacks(26);
    r.cmd.power = 10;
    r.cmd.hit_rate = 3;
    let o = r.run(&[7]);
    assert_eq!(o.target.hp_damage, 1003);
    assert!(o.target.flags.has(UnitFlags::HP_ALTERATION));
}

/// Script 30 (`0030_WhiteWindScript.cs`): a third of the caster's maximum
/// HP at Power 0, `Power` percent otherwise; a zombie is hurt instead.
#[test]
fn s30_white_wind_heals_from_the_casters_max_hp() {
    let mut r = Rig::enemy_attacks(30);
    let o = r.run(&[7]);
    assert_eq!(o.target.hp_damage, 133);
    assert!(o.target.flags.has(UnitFlags::HP_RECOVERY));

    let mut r = Rig::enemy_attacks(30);
    r.cmd.power = 50;
    r.t().status_current = st::ZOMBIE;
    let o = r.run(&[7]);
    assert_eq!(o.target.hp_damage, 200);
    assert!(!o.target.flags.has(UnitFlags::HP_RECOVERY));
}

/// Script 97 (`0097_EnticeScript.cs`): a female character is missed.
#[test]
fn s97_entice_misses_women() {
    let mut r = Rig::enemy_attacks(97);
    r.t().player_category = tantalus_legacy::battle::unit::CATEGORY_FEMALE;
    assert!(miss(&r.run(&[7])));
}

#[test]
fn steal_takes_the_rarest_slot_that_rolls() {
    // Trance draw, the Level + Will vs Level roll (0 % 21 < 0 % 1 fails
    // to block), then slot 3 (rate 1) misses and slot 2 (rate 16) hits.
    let mut r = Rig::new(58);
    r.t().steal_items = [1, 2, 3, 255];
    r.t().steal_rates = [256, 64, 16, 1];
    let outcome = r.run_kinds(&[
        (Draw::Random16, 7),
        (Draw::Random16, 0),
        (Draw::Random16, 0),
        (Draw::Random8, 5),
        (Draw::Random8, 3),
    ]);
    assert_eq!(outcome.stolen, Some(Some(3)));
    assert_eq!(r.t().steal_items, [1, 2, 255, 255]);
    // Nothing left to take.
    let mut r = Rig::new(58);
    let outcome = r.try_run(&[7]).unwrap();
    assert_eq!(outcome.stolen, Some(None));
}

#[test]
fn change_flips_the_row() {
    let mut r = Rig::new(55);
    r.t().row = 1;
    r.try_run(&[7]).unwrap();
    assert_eq!(r.t().row, 0);
}

#[test]
fn same_draws_same_output() {
    let draws = weapon_draws(9999);
    let mut a = Rig::new(1);
    a.cmd.ability_category = 8;
    let mut b = Rig::new(1);
    b.cmd.ability_category = 8;
    assert_eq!(a.run(&draws), b.run(&draws));
    assert_eq!(a.units, b.units);
}

#[test]
fn seeded_generator_is_repeatable_and_in_range() {
    let mut a = SplitMix64Random::new(42);
    let mut b = SplitMix64Random::new(42);
    for _ in 0..64 {
        let x = a.random16();
        assert_eq!(x, b.random16());
        assert!((0..65536).contains(&x));
        let y = a.random_range(3, 9);
        assert_eq!(y, b.random_range(3, 9));
        assert!((3..9).contains(&y));
        assert!((0..256).contains(&a.random8()));
        b.random8();
    }
    let mut c = SplitMix64Random::new(43);
    assert_ne!(
        (0..8).map(|_| a.random16()).collect::<Vec<_>>(),
        (0..8).map(|_| c.random16()).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Status module
// ---------------------------------------------------------------------------

fn ctx<'a>(t: &'a StatusTable, e: &'a StatusEnv) -> StatusCtx<'a> {
    StatusCtx { table: t, env: e }
}

/// Resist beats everything; permanent and already-present NoReset statuses
/// are Invalid; Sleep (not NoReset) can be re-applied and resets its
/// duration.
#[test]
fn status_resist_permanent_and_reapply() {
    let t = table();
    let e = StatusEnv::default();
    let mut rng = SequenceRandom::r16(&[]);
    let mut u = enemy();
    u.status_resist = st::SILENCE;
    assert_eq!(
        alter_status(&mut u, 3, false, &ctx(&t, &e), &mut rng),
        st::AlterResult::Resist
    );

    let mut u = enemy();
    u.status_permanent = st::BLIND;
    assert_eq!(
        alter_status(&mut u, 4, false, &ctx(&t, &e), &mut rng),
        st::AlterResult::Invalid
    );

    let mut u = enemy();
    assert_eq!(
        alter_status(&mut u, 3, false, &ctx(&t, &e), &mut rng),
        st::AlterResult::Success
    );
    assert_eq!(
        alter_status(&mut u, 3, false, &ctx(&t, &e), &mut rng),
        st::AlterResult::Invalid
    );

    let mut u = enemy();
    assert_eq!(
        alter_status(&mut u, 17, false, &ctx(&t, &e), &mut rng),
        st::AlterResult::Success
    );
    u.conti.insert(17, 1);
    assert_eq!(
        alter_status(&mut u, 17, false, &ctx(&t, &e), &mut rng),
        st::AlterResult::Success
    );
    assert_eq!(u.conti[&17], 10000);
}

/// Partial resistance: a rate above 0 draws one unit random; a draw below
/// the rate blocks (Invalid); at or above lets it through. A rate of 0
/// draws nothing (the unit sequence is empty and would panic).
#[test]
fn status_partial_resist_draws() {
    let t = table();
    let e = StatusEnv::default();
    let mut u = enemy();
    u.partial_resist.insert(3, 0.5);
    let mut rng = SequenceRandom::r16(&[]).with_units(&[0.25]);
    assert_eq!(
        alter_status(&mut u, 3, true, &ctx(&t, &e), &mut rng),
        st::AlterResult::Invalid
    );
    let mut rng = SequenceRandom::r16(&[]).with_units(&[0.75]);
    assert_eq!(
        alter_status(&mut u, 3, true, &ctx(&t, &e), &mut rng),
        st::AlterResult::Success
    );
    let mut u = enemy();
    let mut rng = SequenceRandom::r16(&[]);
    assert_eq!(
        alter_status(&mut u, 3, true, &ctx(&t, &e), &mut rng),
        st::AlterResult::Success
    );
}

/// Death on a unit at 0 HP bypasses resistances; at positive HP the resist
/// applies. Applying Death to a living unit zeroes HP and ATB.
#[test]
fn status_death_bypass_and_kill() {
    let t = table();
    let e = StatusEnv::default();
    let mut rng = SequenceRandom::r16(&[]);
    let mut u = enemy();
    u.status_resist = st::DEATH;
    assert_eq!(
        alter_status(&mut u, 8, false, &ctx(&t, &e), &mut rng),
        st::AlterResult::Resist
    );
    u.cur_hp = 0;
    assert_eq!(
        alter_status(&mut u, 8, false, &ctx(&t, &e), &mut rng),
        st::AlterResult::Success
    );
    assert_ne!(u.status_current & st::DEATH, 0);

    let mut u = enemy();
    u.cur_atb = 99;
    alter_status(&mut u, 8, false, &ctx(&t, &e), &mut rng);
    assert_eq!((u.cur_hp, u.cur_atb, u.death_flag), (0, 0, true));
}

/// Petrify during battle (speed < 3) is deferred through the `SysStone`
/// command (SuccessNoSet, status not set); when the command runs it applies
/// for real, zeroing ATB.
#[test]
fn status_petrify_deferred_then_applied() {
    let t = table();
    let e = StatusEnv {
        phase_after_enter: true,
        battle_speed: 0,
        menu_off_phase: false,
    };
    let mut rng = SequenceRandom::r16(&[]);
    let mut u = enemy();
    u.cur_atb = 50;
    assert_eq!(
        alter_status(&mut u, 0, false, &ctx(&t, &e), &mut rng),
        st::AlterResult::SuccessNoSet
    );
    assert!(u.petrify_command_pending);
    assert_eq!(u.status_current, 0);
    assert_eq!(
        alter_status(&mut u, 0, false, &ctx(&t, &e), &mut rng),
        st::AlterResult::Success
    );
    assert_eq!(u.status_current, st::PETRIFY);
    assert_eq!(u.cur_atb, 0);
    assert!(!u.petrify_command_pending);
}

/// Duration and tick counters (WIL 10): Regen positive: 45 * (10 << 3 = 80)
/// = 3600; opr (60 - 10) << 2 = 200, 10 * 200 = 2000. Doom: counter 10, opr
/// = 1.0 * 25 * ((60 - 10) << 3 = 400) / 10 = 1000.0 -> 1000. Haste (positive,
/// conti 40): 3200.
#[test]
fn status_duration_and_tick_counters() {
    let t = table();
    let e = StatusEnv::default();
    let mut rng = SequenceRandom::r16(&[]);
    let mut u = enemy();
    alter_status(&mut u, 18, false, &ctx(&t, &e), &mut rng);
    assert_eq!(u.conti[&18], 3600);
    assert_eq!(u.opr[&18], 2000);

    let mut u = enemy();
    alter_status(&mut u, 27, false, &ctx(&t, &e), &mut rng);
    assert_eq!(u.doom_counter, 10);
    assert_eq!(u.opr[&27], 1000);

    let mut u = enemy();
    alter_status(&mut u, 19, false, &ctx(&t, &e), &mut rng);
    assert_eq!(u.conti[&19], 3200);

    // Duration factor 0.5 halves it: 3200 * 0.5 = 1600.
    let mut u = enemy();
    u.duration_factor.insert(19, 0.5);
    alter_status(&mut u, 19, false, &ctx(&t, &e), &mut rng);
    assert_eq!(u.conti[&19], 1600);
}

/// Haste and Slow swap, Mini toggles, EasyKill adds Death resistance,
/// Zombie resets a player's trance gauge, Berserk is refused when the unit
/// cannot attack.
#[test]
fn status_special_apply_rules() {
    let t = table();
    let e = StatusEnv::default();
    let mut rng = SequenceRandom::r16(&[]);

    let mut u = enemy();
    u.status_current = st::SLOW;
    assert_eq!(
        alter_status(&mut u, 19, false, &ctx(&t, &e), &mut rng),
        st::AlterResult::SuccessNoSet
    );
    assert_eq!(u.status_current, 0);

    let mut u = enemy();
    u.status_current = st::MINI;
    assert_eq!(
        alter_status(&mut u, 28, false, &ctx(&t, &e), &mut rng),
        st::AlterResult::SuccessNoSet
    );
    assert_eq!(u.status_current, 0);
    u.status_permanent = st::MINI;
    u.status_current = st::MINI;
    assert_eq!(
        alter_status(&mut u, 28, false, &ctx(&t, &e), &mut rng),
        st::AlterResult::Invalid
    );

    let mut u = enemy();
    alter_status(&mut u, 7, false, &ctx(&t, &e), &mut rng);
    assert_ne!(u.status_resist & st::DEATH, 0);

    let mut p = player();
    p.trance = 200;
    alter_status(&mut p, 6, false, &ctx(&t, &e), &mut rng);
    assert_eq!(p.trance, 0);

    let mut p = player();
    p.can_use_attack_command = false;
    assert_eq!(
        alter_status(&mut p, 11, false, &ctx(&t, &e), &mut rng),
        st::AlterResult::Resist
    );
}

/// Removal rules: not present or permanent -> 1 (nothing); present -> 2.
#[test]
fn status_remove_rules() {
    let e = StatusEnv::default();
    let mut u = enemy();
    u.status_current = st::SILENCE | st::BLIND;
    u.status_permanent = st::BLIND;
    assert_eq!(remove_status(&mut u, 4, &e), st::REMOVE_NOTHING);
    assert_eq!(remove_status(&mut u, 3, &e), st::REMOVE_DONE);
    assert_eq!(remove_status(&mut u, 3, &e), st::REMOVE_NOTHING);
    assert_eq!(u.status_current, st::BLIND);
}

/// Low HP check point: a player at 1/6 max HP or less gets the Low HP
/// status: 80 * 6 = 480 <= 500. At 84: 504 > 500 it is removed again. An
/// enemy never gets it.
#[test]
fn status_low_hp_check_point() {
    let t = table();
    let e = StatusEnv::default();
    let mut rng = SequenceRandom::r16(&[]);
    let mut p = player();
    p.max_hp = 500;
    p.cur_hp = 80;
    assert_eq!(
        check_point_status(&mut p, &ctx(&t, &e), &mut rng),
        st::LOW_HP
    );
    assert_ne!(p.status_current & st::LOW_HP, 0);
    p.cur_hp = 84;
    assert_eq!(check_point_status(&mut p, &ctx(&t, &e), &mut rng), 0);
    assert_eq!(p.status_current & st::LOW_HP, 0);
    p.cur_hp = 0;
    assert_eq!(
        check_point_status(&mut p, &ctx(&t, &e), &mut rng),
        st::DEATH
    );

    let mut en = enemy();
    en.cur_hp = 1;
    assert_eq!(check_point_status(&mut en, &ctx(&t, &e), &mut rng), 0);
}

/// A player hit down to 1/6 HP gains Low HP through the end-of-calc check
/// point: HP 500/500, damage 495 -> 5 HP; 5 * 6 = 30 <= 500.
#[test]
fn low_hp_added_after_damage() {
    let mut r = Rig::enemy_attacks(8);
    r.cmd.ability_category = 8;
    r.cmd.power = 20;
    r.t().physical_defence = 10;
    // attack 32 vs power 20 - 10 = 10 -> 320 damage; use power 26: 32 * 16 =
    // 512 >= 500: HP 0 -> Death instead. Use power 25: diff 15 -> 480 ->
    // HP 20, 20 * 6 = 120 <= 500.
    r.cmd.power = 25;
    let o = r.run(&enemy_draws(9999));
    assert_eq!(o.target.hp_damage, 480);
    assert_eq!(r.t().cur_hp, 20);
    assert_ne!(r.t().status_current & st::LOW_HP, 0);
    assert_ne!(o.added_statuses & st::LOW_HP, 0);
}

/// Trance gauge: an enemy damages a player with a gauge while the battle
/// menu is on. Gauge 100 + increase 7 = 107 < 255 -> 107. At 250: 257 >= 255
/// -> 255 and Trance is added (the target has a gauge).
#[test]
fn trance_gauge_reaction() {
    let mut r = Rig::enemy_attacks(8);
    r.cmd.ability_category = 8;
    r.cmd.power = 20;
    r.env.battle_menu_enabled = true;
    r.t().has_trance_gauge = true;
    r.t().trance = 100;
    r.run(&enemy_draws(9999));
    assert_eq!(r.t().trance, 107);

    let mut r = Rig::enemy_attacks(8);
    r.cmd.ability_category = 8;
    r.cmd.power = 20;
    r.env.battle_menu_enabled = true;
    r.t().has_trance_gauge = true;
    r.t().trance = 250;
    r.run(&enemy_draws(9999));
    assert_eq!(r.t().trance, 255);
    assert_ne!(r.t().status_current & st::TRANCE, 0);
}

// ---------------------------------------------------------------------------
// MP cost
// ---------------------------------------------------------------------------

/// Cost 6 for a player with factor 100: 6 * 100 / 100 = 6. Factor 50: 3.
/// Factor 75: 6 * 75 / 100 = 4 (450 / 100 truncated). Garnet summon x4: 24.
#[test]
fn mp_cost_rules() {
    let mut cmd = BattleCommand::new(10);
    cmd.mp_cost = 6;
    let p = player();
    assert_eq!(cost::command_mp_cost(&cmd, &p, false, 100), 6);
    assert_eq!(cost::command_mp_cost(&cmd, &p, false, 50), 3);
    assert_eq!(cost::command_mp_cost(&cmd, &p, false, 75), 4);
    cmd.ability_type = 4;
    assert_eq!(cost::command_mp_cost(&cmd, &p, true, 100), 24);
    assert_eq!(cost::command_mp_cost(&cmd, &p, false, 100), 6);
    let e = enemy();
    assert_eq!(cost::command_mp_cost(&cmd, &e, true, 50), 24);
}

/// Consuming MP: enough MP deducts; too little refuses and leaves MP
/// untouched; Magic Counter is free; a custom cost overrides the table.
#[test]
fn mp_consumption() {
    let mut cmd = BattleCommand::new(10);
    cmd.mp_cost = 30;
    let mut p = player();
    assert!(cost::try_consume_mp(&cmd, &mut p, false, 100));
    assert_eq!(p.cur_mp, 70);
    p.cur_mp = 29;
    assert!(!cost::try_consume_mp(&cmd, &mut p, false, 100));
    assert_eq!(p.cur_mp, 29);
    p.cur_mp = 30;
    assert!(cost::try_consume_mp(&cmd, &mut p, false, 100));
    assert_eq!(p.cur_mp, 0);

    let mut counter = BattleCommand::new(10);
    counter.mp_cost = 30;
    counter.command_id = tantalus_legacy::battle::command::COMMAND_MAGIC_COUNTER;
    let mut p = player();
    assert!(cost::try_consume_mp(&counter, &mut p, false, 100));
    assert_eq!(p.cur_mp, 100);

    let mut custom = BattleCommand::new(10);
    custom.mp_cost = 30;
    custom.custom_mp_cost = Some(0);
    let mut p = player();
    assert!(cost::try_consume_mp(&custom, &mut p, false, 100));
    assert_eq!(p.cur_mp, 100);
}

#[test]
fn eat_needs_low_hp_and_teaches_blue_magic() {
    // Above MaximumHp / Power (500 / 4 = 125): too strong.
    let mut r = Rig::new(65);
    r.cmd.power = 4;
    r.t().blue_magic = 81;
    assert_eq!(r.run(&[7]).eaten, Some(EatResult::Failed));
    assert!(r.t().cur_hp > 0);
    // At 125 HP it dies and teaches its blue magic.
    let mut r = Rig::new(65);
    r.cmd.power = 4;
    r.t().blue_magic = 81;
    r.t().cur_hp = 125;
    assert_eq!(r.run(&[7]).eaten, Some(EatResult::Yummy(81)));
    assert_eq!(r.t().cur_hp, 0);
    // No blue magic: eaten, tastes bad.
    let mut r = Rig::new(65);
    r.cmd.power = 4;
    r.t().cur_hp = 1;
    assert_eq!(r.run(&[7]).eaten, Some(EatResult::TasteBad));
    // A Humanoid cannot be eaten.
    let mut r = Rig::new(65);
    r.cmd.power = 4;
    r.t().cur_hp = 1;
    r.t().enemy_category = 1;
    assert_eq!(r.run(&[7]).eaten, Some(EatResult::CannotEat));
    assert_eq!(r.t().cur_hp, 1);
}

#[test]
fn focus_raises_magic_by_a_power_fraction() {
    let mut r = Rig::new(44);
    r.cmd.power = 4;
    r.t().magic = 30;
    r.run(&[7]);
    assert_eq!(r.t().magic, 37);
    r.t().magic = 95;
    r.run(&[7]);
    assert_eq!(r.t().magic, 99);
}

#[test]
fn spear_is_a_weapon_hit() {
    // No hit or crit roll: the ctor draw and the weapon base only.
    let mut r = Rig::new(48);
    r.cmd.ability_category = 8;
    let o = r.run(&[7, 9999]);
    assert!(o.target.hp_damage > 0, "{o:?}");
}

#[test]
fn might_raises_strength_and_misses_when_nothing_changes() {
    let mut r = Rig::new(43);
    r.cmd.power = 4;
    r.t().strength = 40;
    let o = r.run(&[7]);
    assert_eq!(r.t().strength, 50);
    assert!(!miss(&o));
    r.t().strength = 0;
    let o = r.run(&[7]);
    assert!(miss(&o), "an unchanged stat is Invalid, so Miss");
    r.t().strength = 98;
    r.run(&[7]);
    assert_eq!(r.t().strength, 99);
    r.cmd.power = 0;
    assert!(r.try_run(&[7]).is_err());
}

#[test]
fn absorb_magic_and_strength_halve_the_target_and_double_the_caster() {
    let mut r = Rig::new(94);
    r.t().magic = 41;
    r.c().magic = 30;
    let o = r.run(&[7]);
    assert!(!miss(&o));
    assert_eq!((r.t().magic, r.c().magic), (20, 60));
    let mut r = Rig::new(95);
    r.t().strength = 60;
    r.c().strength = 200;
    r.run(&[7]);
    assert_eq!((r.t().strength, r.c().strength), (30, 255));
    // A target that resists the status guards and the caster gains nothing.
    let mut r = Rig::new(95);
    r.t().strength = 60;
    r.c().strength = 20;
    r.t().status_resist |= tantalus_legacy::battle::status::CHANGE_STAT;
    let o = r.run(&[7]);
    assert!(o.flags.has(CalcFlags::GUARD));
    assert_eq!((r.t().strength, r.c().strength), (60, 20));
}

#[test]
fn scan_shows_libra_unless_the_target_is_easy_kill_protected() {
    let mut r = Rig::new(59);
    assert!(r.run(&[7]).libra);
    let mut r = Rig::new(59);
    r.t().status_current |= tantalus_legacy::battle::status::EASY_KILL;
    let o = r.run(&[7]);
    assert!(!o.libra && miss(&o));
}

#[test]
fn melt_deals_the_casters_hp_and_kills_the_caster() {
    let mut r = Rig::new(88);
    r.c().cur_hp = 321;
    let o = r.run(&[7]);
    assert_eq!(o.target.hp_damage, 321);
    assert!(o.caster_dies);
    let mut r = Rig::new(88);
    r.c().cur_hp = 0;
    assert!(!r.run(&[7]).caster_dies);
}

#[test]
fn swallow_removes_the_target() {
    let mut r = Rig::new(106);
    assert!(r.run(&[7]).target_removed);
}

#[test]
fn comet_misses_on_a_high_roll_and_meteor_never_misses_by_itself() {
    let mut r = Rig::new(18);
    r.cmd.special_effect = 56;
    let o = r.run_kinds(&[(Draw::Random16, 7), (Draw::Random8, 200)]);
    assert!(miss(&o));
    // Not a Comet: no roll, then `Next16() % (Magic + Level)` is the attack.
    let mut r = Rig::new(18);
    r.cmd.power = 56;
    r.cmd.ability_category = 16;
    let o = r.run_kinds(&[
        (Draw::Random16, 7),
        (Draw::Random16, 100),
        (Draw::Random16, 5),
    ]);
    assert!(!miss(&o), "{o:?}");
    assert!(o.target.hp_damage > 0, "{o:?}");
}

// ---------------------------------------------------------------------------
// Attack support abilities (`AbilityFeatures.txt` SA 12, 13-20, 24, 25)
// ---------------------------------------------------------------------------

/// A plain Attack (ability 176, script 1) by the player on the enemy with the
/// given support abilities and enemy category; `extra` are draws taken inside
/// the damage step (after the crit pair), `weapon_rate` the weapon's status rate.
fn sa_attack(
    sas: &[u8],
    category: u8,
    weapon_rate: i32,
    extra: &[(Draw, i32)],
) -> (CalcOutcome, Rig) {
    let mut r = Rig::new(1);
    r.cmd.ability_category = 8;
    r.cmd.ability_id = 176;
    for sa in sas {
        r.c().support_abilities.insert(*sa);
    }
    r.t().enemy_category = category;
    r.t().is_undead = category & 16 != 0;
    r.c().weapon.as_mut().unwrap().rate = weapon_rate;
    let mut draws = vec![
        (Draw::Random16, 7),
        (Draw::Random16, 1234),
        (Draw::Random16, 5678),
        (Draw::Random16, 9999),
        (Draw::Random16, 7),
        (Draw::Random16, 50),
    ];
    draws.extend_from_slice(extra);
    let o = r.run_kinds(&draws);
    (o, r)
}

/// Each Killer (and Man Eater) adds one damage modifier against its category:
/// attack 33 * 3 >> 1 = 49, power difference 15, damage 735 instead of 495.
#[test]
fn sa_killers_raise_damage_against_their_category() {
    for (sa, bit) in [
        (13u8, 128u8),
        (14, 64),
        (15, 32),
        (16, 16),
        (17, 8),
        (18, 4),
        (19, 2),
        (20, 1),
    ] {
        let (o, _) = sa_attack(&[sa], bit, 0, &[]);
        assert_eq!(o.target.hp_damage, 735, "SA {sa}");
        // A different category leaves the damage alone.
        let (o, _) = sa_attack(&[sa], if bit == 1 { 2 } else { 1 }, 0, &[]);
        assert_eq!(o.target.hp_damage, 495, "SA {sa} wrong category");
    }
}

/// `DisableSA 13..20`: two Killers matching one target still add only one
/// modifier.
#[test]
fn sa_killers_do_not_stack() {
    let (o, _) = sa_attack(&[13, 14], 128 | 64, 0, &[]);
    assert_eq!(o.target.hp_damage, 735);
}

/// A Killer only works on Attack, not on another ability.
#[test]
fn sa_killer_needs_the_attack_ability() {
    let mut r = Rig::new(1);
    r.cmd.ability_category = 8;
    r.cmd.ability_id = 5;
    r.c().support_abilities.insert(16);
    r.t().enemy_category = 16;
    let o = r.run(&weapon_draws(9999));
    assert_eq!(o.target.hp_damage, 495);
}

/// SA 12 MP Attack: one modifier up, the MpAttack flag, and 1/8 of max MP is
/// spent (100 -> 88). With no MP it does nothing.
#[test]
fn sa_mp_attack_boosts_and_spends_mp() {
    let (o, r) = sa_attack(&[12], 0, 0, &[]);
    assert_eq!(o.target.hp_damage, 735);
    assert!(o.flags.has(CalcFlags::MP_ATTACK));
    assert_eq!(r.units[0].cur_mp, 88);

    let mut r = Rig::new(1);
    r.cmd.ability_category = 8;
    r.cmd.ability_id = 176;
    r.c().support_abilities.insert(12);
    r.c().cur_mp = 0;
    let o = r.run(&weapon_draws(9999));
    assert_eq!(o.target.hp_damage, 495);
    assert!(!o.flags.has(CalcFlags::MP_ATTACK));
}

/// SA 24 Healer: the Attack heals a living, non-undead target; an undead one
/// is hurt as usual.
#[test]
fn sa_healer_turns_attack_into_recovery() {
    let (o, _) = sa_attack(&[24], 0, 0, &[]);
    assert!(o.target.flags.has(UnitFlags::HP_RECOVERY));
    let (o, _) = sa_attack(&[24], 16, 0, &[]);
    assert!(!o.target.flags.has(UnitFlags::HP_RECOVERY));
}

/// SA 25 Add Status: `GetRandom(0, 100) < weapon rate` sets AddStat. The draw
/// is taken only for Attack; rate 40 with a draw of 39 procs and applies the
/// weapon's Poison, a draw of 40 does not.
#[test]
fn sa_add_status_procs_the_weapon_status() {
    let proc = |roll: i32| {
        let mut r = Rig::new(1);
        r.cmd.ability_category = 8;
        r.cmd.ability_id = 176;
        r.c().support_abilities.insert(25);
        let w = r.c().weapon.as_mut().unwrap();
        w.rate = 40;
        w.status = st::POISON;
        let draws = vec![
            (Draw::Random16, 7),
            (Draw::Random16, 1234),
            (Draw::Random16, 5678),
            (Draw::Random16, 9999),
            (Draw::Random16, 7),
            (Draw::Random16, 50),
            (Draw::Range, roll),
        ];
        let o = r.run_kinds(&draws);
        (
            o.flags.has(CalcFlags::ADD_STAT),
            o.added_statuses & st::POISON != 0,
        )
    };
    assert_eq!(proc(39), (true, true));
    assert_eq!(proc(40), (false, false));
}

/// Without the ability no draw is taken and nothing procs.
#[test]
fn sa_add_status_needs_the_ability() {
    let (o, _) = sa_attack(&[], 0, 100, &[]);
    assert!(!o.flags.has(CalcFlags::ADD_STAT));
}
