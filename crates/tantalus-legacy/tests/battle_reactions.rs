#![allow(clippy::doc_lazy_continuation)]
//! Reflect, Cover, Counter, Item and Double Cast on the B5 machine
//! (`tantalus_legacy::battle::reactions`).
//!
//! Every expectation is derived by hand from the pinned Memoria source
//! (c716e1fa, `Assembly-CSharp/`); each test names the function it pins.
//! `Timing` stays 0, so a command dequeued on tick T inspects on T, runs the
//! select-VFX step on T + 1 and applies its effect on T + 2.
//!
//! Effect scripts: 25 (`PreciseDirectHPDamageScript`) sets the target HP to the
//! command power, which marks exactly who was hit. Script 9 (magic attack) is
//! used for the reflect damage multiplier; script 69 and 13 for items.

use std::collections::VecDeque;

use tantalus_legacy::battle::command::ItemData;
use tantalus_legacy::battle::machine::{
    Battle, BattleConfig, BattleError, BattleEvent, BattleHooks,
};
use tantalus_legacy::battle::queue::{CommandSpec, cmd};
use tantalus_legacy::battle::reactions::{SA_COUNTER, SA_COVER, SA_EYE_4_EYE};
use tantalus_legacy::battle::rng::BattleRandom;
use tantalus_legacy::battle::status as st;
use tantalus_legacy::battle::targeting::target_type;
use tantalus_legacy::battle::{BattleCommand, BattleUnit, ItemEffect, StatusDataRow, StatusTable};

// ---------------------------------------------------------------------------
// Rig
// ---------------------------------------------------------------------------

const P1: u16 = 1;
const P2: u16 = 2;
const P3: u16 = 4;
const E1: u16 = 16;
const E2: u16 = 32;
const E3: u16 = 64;

/// A random source with scripted `random8` and `random_range(0, 100)` draws
/// (the Counter roll, `NCalcUtility.cs:107`). Everything else returns 0 and
/// the range min, so a script that needs no draws is not disturbed.
#[derive(Default)]
struct Rng {
    r8: VecDeque<i32>,
    counter_roll: VecDeque<i32>,
    r8_used: usize,
    rolls_used: usize,
}

impl Rng {
    fn r8(values: &[i32]) -> Self {
        Self {
            r8: values.iter().copied().collect(),
            ..Self::default()
        }
    }
    fn roll(value: i32) -> Self {
        Self {
            counter_roll: VecDeque::from([value]),
            ..Self::default()
        }
    }
}

impl BattleRandom for Rng {
    fn random16(&mut self) -> i32 {
        0
    }
    fn random8(&mut self) -> i32 {
        self.r8_used += 1;
        self.r8.pop_front().unwrap_or(0)
    }
    fn random_range(&mut self, min: i32, max: i32) -> i32 {
        if (min, max) == (0, 100) {
            self.rolls_used += 1;
            return self
                .counter_roll
                .pop_front()
                .expect("unexpected counter roll");
        }
        min
    }
    fn random_unit(&mut self) -> f32 {
        0.0
    }
}

/// Memoria's shipped `StatusData.csv` rows (same values as `battle_atb.rs`).
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
        (0, 45, 0, 0),
        (0, 20, 33554432, 0),
        (0, 20, 83886080, 67108864),
        (0, 30, 0, 0),
        (0, 25, 0, 2147483648),
        (0, 20, 0, 0),
        (0, 40, 0, 0),
        (0, 10, 0, 3221225471),
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

fn unit(id: u16, player: bool, hp: u32) -> BattleUnit {
    let mut u = BattleUnit::new(id, player);
    u.level = 20;
    u.strength = 30;
    u.magic = 30;
    u.will = 20;
    u.cur_hp = hp;
    u.max_hp = hp.max(1000);
    u.cur_mp = 100;
    u.max_mp = 100;
    u
}

fn with_status(mut u: BattleUnit, mask: u64) -> BattleUnit {
    u.status_current = mask;
    u
}

fn with_sa(mut u: BattleUnit, sa: u8) -> BattleUnit {
    u.support_abilities.insert(sa);
    u
}

fn spell(script: i32, power: i32, category: u8, tt: u8) -> CommandSpec {
    let mut c = BattleCommand::new(script);
    c.power = power;
    c.hit_rate = 255;
    c.ability_category = category;
    c.target_type = tt;
    CommandSpec {
        command: c,
        default_ally: false,
        for_dead: false,
    }
}

/// Catalog stand-in.
///
/// * `EnemyAtk` sub 0: script 25 power 7, category 8 (physical).
/// * `EnemyAtk` sub 1: script 25 power 7, category 17 (magic, reflectable).
/// * `EnemyAtk` sub 2: script 9 power 20, category 17.
/// * `EnemyAtk` sub 3: as sub 1 but `ReflectNull`.
/// * `EnemyAtk` sub 4: script 25 power 7, category 16 (no reflect bit).
/// * `Attack` 176: script 25 power 5, category 8. `Counter` 176: power 3.
/// * `Item` base: the `Void` row with reflectable category 17.
/// * `DoubleWhiteMagic`: subs 10-14 are spells, 253-255 Double Cast (script 80).
struct Hooks {
    items: Vec<(i32, ItemData)>,
}

impl Hooks {
    fn new() -> Self {
        Self { items: Vec::new() }
    }
}

impl BattleHooks for Hooks {
    fn spec(&mut self, cmd_no: u32, sub: i32, _c: &BattleUnit) -> Option<CommandSpec> {
        let single = target_type::SINGLE_ENEMY;
        match (cmd_no, sub) {
            (cmd::ENEMY_ATK, 0) => Some(spell(25, 7, 8, single)),
            (cmd::ENEMY_ATK, 1) => Some(spell(25, 7, 17, single)),
            (cmd::ENEMY_ATK, 2) => Some(spell(9, 20, 17, single)),
            (cmd::ENEMY_ATK, 3) => {
                let mut s = spell(25, 7, 17, single);
                s.command.is_reflect_null = true;
                Some(s)
            }
            (cmd::ENEMY_ATK, 4) => Some(spell(25, 7, 16, single)),
            (cmd::ATTACK, 176) => Some(spell(25, 5, 8, single)),
            (cmd::COUNTER, 176) => Some(spell(25, 3, 8, single)),
            (cmd::ITEM, _) => {
                let mut s = spell(0, 0, 17, target_type::SINGLE_ALLY);
                s.default_ally = true;
                Some(s)
            }
            (cmd::DOUBLE_WHITE_MAGIC, 10) => {
                let mut s = spell(25, 7, 17, single);
                s.command.mp_cost = 4;
                Some(s)
            }
            (cmd::DOUBLE_WHITE_MAGIC, 11) => {
                let mut s = spell(25, 9, 17, single);
                s.command.mp_cost = 6;
                Some(s)
            }
            (cmd::DOUBLE_WHITE_MAGIC, 12) => Some(spell(25, 5, 17, target_type::ALL_ENEMY)),
            (cmd::DOUBLE_WHITE_MAGIC, 14) => Some(spell(25, 5, 17, target_type::SINGLE_ANY)),
            (cmd::DOUBLE_WHITE_MAGIC, 253..=255) => {
                let mut s = spell(80, 0, 0, target_type::SINGLE_ANY);
                // Power = first ability id, Rate = second (`btl_cmd.cs:164-165`).
                s.command.power = match sub {
                    255 => 10,
                    254 => 12,
                    _ => 14,
                };
                s.command.hit_rate = 11;
                s.command.mp_cost = 20;
                Some(s)
            }
            _ => None,
        }
    }

    fn item(&mut self, item_id: i32) -> Option<ItemData> {
        self.items
            .iter()
            .find(|(i, _)| *i == item_id)
            .map(|(_, d)| *d)
    }
}

fn battle(units: Vec<BattleUnit>) -> Battle {
    Battle::new(units, BattleConfig::default(), table())
}

fn enemy_cast(b: &mut Battle, h: &mut Hooks, caster: usize, sub: i32, tar: u16) {
    let action = tantalus_legacy::battle::machine::EnemyAction {
        cmd_no: cmd::ENEMY_ATK,
        sub_no: sub,
        tar_id: tar,
    };
    b.set_enemy_command(caster, action, h)
        .expect("enemy command");
}

fn run_idle(b: &mut Battle, h: &mut Hooks, rng: &mut Rng) {
    for _ in 0..40 {
        b.tick(h, rng).expect("tick");
        if b.running.is_empty() && b.cmds.queue.is_empty() {
            return;
        }
    }
    panic!("battle did not go idle");
}

fn events(b: &Battle) -> Vec<BattleEvent> {
    b.log.iter().map(|(_, e)| e.clone()).collect()
}

fn effect_targets(b: &Battle) -> Vec<usize> {
    events(b)
        .into_iter()
        .filter_map(|e| match e {
            BattleEvent::EffectApplied { target, .. } => Some(target),
            _ => None,
        })
        .collect()
}

fn reflected(b: &Battle) -> Vec<(u16, [u16; 4], u8)> {
    events(b)
        .into_iter()
        .filter_map(|e| match e {
            BattleEvent::Reflected {
                reflecting,
                targets,
                mode,
                ..
            } => Some((reflecting, targets, mode)),
            _ => None,
        })
        .collect()
}

fn covered(b: &Battle) -> Vec<(u16, u16)> {
    events(b)
        .into_iter()
        .filter_map(|e| match e {
            BattleEvent::Covered {
                target_id,
                cover_id,
            } => Some((target_id, cover_id)),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Reflect (`btl_cmd.CheckReflec`, `btl_cmd.cs:876-936`)
// ---------------------------------------------------------------------------

/// Derivation: P1 reflects a magic spell aimed at it. Targets outside `tar_id`
/// that are alive and targetable: players [P2]; enemies [E1 (the caster), E2].
/// `partyReflectCount = 1`, so one draw `random8() % 2`: draw 1 -> E2. The
/// reflecting mask equals `tar_id` -> `info.reflec = 1`,
/// `tar_id = MergeReflecTargetID`. Script 25 then sets E2's HP to the power 7;
/// P1 is untouched.
#[test]
fn reflect_sends_magic_back_at_an_enemy() {
    let mut b = battle(vec![
        with_status(unit(P1, true, 500), st::REFLECT),
        unit(P2, true, 500),
        unit(E1, false, 500),
        unit(E2, false, 500),
    ]);
    let (mut h, mut r) = (Hooks::new(), Rng::r8(&[1]));
    enemy_cast(&mut b, &mut h, 2, 1, P1);
    run_idle(&mut b, &mut h, &mut r);
    assert_eq!(reflected(&b), vec![(P1, [E2, 0, 0, 0], 1)]);
    assert_eq!(effect_targets(&b), vec![3]);
    assert_eq!(b.units[3].cur_hp, 7);
    assert_eq!(b.units[0].cur_hp, 500);
    assert_eq!(r.r8_used, 1);
}

/// Derivation: the reflected hit repeats per matching reflect slot
/// (`GetReflectMultiplierOnTarget`, `BattleCommand.cs:204`, applied in
/// `DamageFinalChanges`, `SBattleCalculator.cs:378`). One reflecting target
/// drawing E2 does damage X to E2; two reflecting targets that both draw E2
/// (draws 1, 1 over [E1, E2]) do 2 * X to the single remaining target E2.
#[test]
fn reflect_damage_multiplies_by_reflect_slots() {
    let loss = |tar: u16, reflectors: [bool; 2], draws: &[i32]| {
        let mut p1 = unit(P1, true, 500);
        let mut p2 = unit(P2, true, 500);
        if reflectors[0] {
            p1.status_current = st::REFLECT;
        }
        if reflectors[1] {
            p2.status_current = st::REFLECT;
        }
        let mut e2 = unit(E2, false, 5000);
        e2.max_hp = 5000;
        let mut b = battle(vec![p1, p2, unit(E1, false, 500), e2]);
        let (mut h, mut r) = (Hooks::new(), Rng::r8(draws));
        enemy_cast(&mut b, &mut h, 2, 2, tar);
        run_idle(&mut b, &mut h, &mut r);
        assert_eq!(effect_targets(&b), vec![3]);
        5000 - b.units[3].cur_hp as i32
    };
    let one = loss(P1, [true, false], &[1]);
    let two = loss(P1 | P2, [true, true], &[1, 1]);
    assert!(one > 0, "the reflected spell must hurt");
    assert_eq!(two, 2 * one);
}

/// Derivation: tar = P1 (reflects) | P2 (does not). `reflectingBtl != tar_id`
/// -> `info.reflec = 2`, `tar_id &= ~reflectingBtl` = P2. P2 is hit first; when
/// the loop ends `ReqFinishCommand` (`btl_cmd.cs:1276`) runs
/// `LoopBattleVfxForReflect`: `tar_id` = the reflect targets, `reflec = 1`,
/// and the loop plays again. The command finishes once.
#[test]
fn partial_reflect_hits_the_rest_first_then_the_reflected_targets() {
    let mut b = battle(vec![
        with_status(unit(P1, true, 500), st::REFLECT),
        unit(P2, true, 500),
        unit(E1, false, 500),
        unit(E2, false, 500),
    ]);
    let (mut h, mut r) = (Hooks::new(), Rng::r8(&[1]));
    enemy_cast(&mut b, &mut h, 2, 1, P1 | P2);
    run_idle(&mut b, &mut h, &mut r);
    assert_eq!(reflected(&b), vec![(P1, [E2, 0, 0, 0], 2)]);
    assert_eq!(effect_targets(&b), vec![1, 3]);
    assert_eq!(b.units[1].cur_hp, 7);
    assert_eq!(b.units[3].cur_hp, 7);
    assert_eq!(b.units[0].cur_hp, 500);
    let finished = events(&b)
        .iter()
        .filter(|e| matches!(e, BattleEvent::CommandFinished { unit: 2, .. }))
        .count();
    assert_eq!(finished, 1);
}

/// Derivation: draws are made party reflections first (`for i < partyReflectCount`
/// from the enemy list), then enemy reflections (from the player list). P1 and
/// E1 both reflect. Outside `tar_id`: players [P2, P3], enemies [E2, E3].
/// Draw 1 -> enemies[1] = E3; draw 0 -> players[0] = P2. `reflec.tar_id` =
/// [E3, P2, 0, 0], mode 1 because both targets reflect.
#[test]
fn reflect_draw_order_is_party_reflects_then_enemy_reflects() {
    let mut b = battle(vec![
        with_status(unit(P1, true, 500), st::REFLECT),
        unit(P2, true, 500),
        unit(P3, true, 500),
        with_status(unit(E1, false, 500), st::REFLECT),
        unit(E2, false, 500),
        unit(E3, false, 500),
    ]);
    let (mut h, mut r) = (Hooks::new(), Rng::r8(&[1, 0]));
    enemy_cast(&mut b, &mut h, 5, 1, P1 | E1);
    run_idle(&mut b, &mut h, &mut r);
    assert_eq!(reflected(&b), vec![(P1 | E1, [E3, P2, 0, 0], 1)]);
    assert_eq!(r.r8_used, 2);
    assert_eq!(effect_targets(&b), vec![1, 5]);
}

/// Derivation: no redirect when `ReflectNull` is set, when the category lacks
/// bit 1, or for an Item command (`CheckReflec` guard, `btl_cmd.cs:880`). No
/// draw is made in any of the three.
#[test]
fn reflect_is_skipped_for_null_non_reflectable_and_items() {
    for sub in [3, 4] {
        let mut b = battle(vec![
            with_status(unit(P1, true, 500), st::REFLECT),
            unit(E1, false, 500),
        ]);
        let (mut h, mut r) = (Hooks::new(), Rng::default());
        enemy_cast(&mut b, &mut h, 1, sub, P1);
        run_idle(&mut b, &mut h, &mut r);
        assert!(reflected(&b).is_empty(), "sub {sub}");
        assert_eq!(b.units[0].cur_hp, 7, "sub {sub}");
        assert_eq!(r.r8_used, 0);
    }
    // Item with a reflectable base category (17) on a Reflect target.
    let mut hurt = with_status(unit(P2, true, 100), st::REFLECT);
    hurt.max_hp = 1000;
    let mut b = battle(vec![unit(P1, true, 500), hurt, unit(E1, false, 500)]);
    let mut h = Hooks::new();
    h.items.push((
        1,
        ItemData {
            effect: ItemEffect {
                power: 10,
                ..ItemEffect::default()
            },
            script_id: 69,
            for_dead: false,
        },
    ));
    b.inventory.insert(1, 3);
    let mut r = Rng::default();
    assert!(b.player_command(0, cmd::ITEM, 1, P2, 0, &mut h).unwrap());
    run_idle(&mut b, &mut h, &mut r);
    assert!(reflected(&b).is_empty());
    assert_eq!(effect_targets(&b), vec![1]);
    assert_eq!(b.units[1].cur_hp, 250);
}

// ---------------------------------------------------------------------------
// Cover (SA 37, `TriggerOnCommand` + `btl_abil.CheckCoverAbility`)
// ---------------------------------------------------------------------------

fn cover_battle(target: BattleUnit, others: Vec<BattleUnit>) -> Battle {
    let mut units = vec![target];
    units.extend(others);
    units.push(unit(E1, false, 500));
    battle(units)
}

fn low(mut u: BattleUnit) -> BattleUnit {
    u.status_current |= st::LOW_HP;
    u
}

/// Derivation: P1 is under Low HP, P2 has SA 37 and can move. The enemy's
/// physical `EnemyAtk` at the single target P1 satisfies the feature
/// condition; `TryCover` is true (not Death/Petrify, Low HP). `CheckCoverAbility`
/// picks P2: `cmd.tar_id = (tar & ~P1) | P2`, `info.cover = 1`. During the
/// command P2 `bi.cover_unit` is P1; `FinishCommand` clears it.
#[test]
fn cover_takes_the_hit_for_a_low_hp_ally_and_clears_after() {
    let mut b = cover_battle(
        low(unit(P1, true, 100)),
        vec![with_sa(unit(P2, true, 300), SA_COVER)],
    );
    let (mut h, mut r) = (Hooks::new(), Rng::default());
    enemy_cast(&mut b, &mut h, 2, 0, P1);
    // Tick until the cover happens (dequeue, then select VFX).
    for _ in 0..2 {
        b.tick(&mut h, &mut r).unwrap();
    }
    assert_eq!(covered(&b), vec![(P1, P2)]);
    assert_eq!(b.units[1].cover_unit, Some(P1));
    assert!(b.units[1].covering);
    run_idle(&mut b, &mut h, &mut r);
    assert_eq!(effect_targets(&b), vec![1]);
    assert_eq!(b.units[1].cur_hp, 7);
    assert_eq!(b.units[0].cur_hp, 100);
    assert_eq!(b.units[1].cover_unit, None);
    assert!(!b.units[1].covering);
}

/// Derivation: each row breaks one term of the feature text and no cover
/// happens (the ally P1 is hit).
#[test]
fn cover_conditions_each_block_the_cover() {
    // (label, setup, command sub, target mask, caster index)
    type Setup = fn(&mut Vec<BattleUnit>);
    let cases: Vec<(&str, Setup, i32, u16)> = vec![
        ("target not under Low HP", |u| u[0].cur_hp = 800, 0, P1),
        (
            "cover user under Heat",
            |u| u[1].status_current |= st::HEAT,
            0,
            P1,
        ),
        (
            "cover user is dead",
            |u| u[1].status_current |= st::DEATH,
            0,
            P1,
        ),
        (
            "cover user is immobilized",
            |u| u[1].status_current |= st::STOP,
            0,
            P1,
        ),
        ("magic is not category 8", |_| {}, 4, P1),
        ("two targets are not single", |_| {}, 0, P1 | P3),
        (
            "target is petrified",
            |u| u[0].status_current |= st::PETRIFY,
            0,
            P1,
        ),
    ];
    // The same rig with no term broken does cover, so each row below fails
    // for its own reason.
    let mut base = battle(vec![
        low(unit(P1, true, 100)),
        with_sa(unit(P2, true, 300), SA_COVER),
        unit(P3, true, 300),
        unit(E1, false, 500),
    ]);
    let (mut h, mut r) = (Hooks::new(), Rng::default());
    enemy_cast(&mut base, &mut h, 3, 0, P1);
    run_idle(&mut base, &mut h, &mut r);
    assert_eq!(covered(&base), vec![(P1, P2)]);
    for (label, setup, sub, tar) in cases {
        let mut units = vec![
            low(unit(P1, true, 100)),
            with_sa(unit(P2, true, 300), SA_COVER),
            unit(P3, true, 300),
            unit(E1, false, 500),
        ];
        setup(&mut units);
        let mut b = battle(units);
        let (mut h, mut r) = (Hooks::new(), Rng::default());
        enemy_cast(&mut b, &mut h, 3, sub, tar);
        run_idle(&mut b, &mut h, &mut r);
        assert!(covered(&b).is_empty(), "{label}");
        assert_eq!(b.units[1].cover_unit, None, "{label}");
    }
}

/// Derivation: the feature requires `CommandId == EnemyAtk`, so a player's
/// Attack on an ally never draws a cover.
#[test]
fn cover_only_answers_enemy_attacks() {
    let mut b = battle(vec![
        low(unit(P1, true, 100)),
        with_sa(unit(P2, true, 300), SA_COVER),
        unit(P3, true, 300),
        unit(E1, false, 500),
    ]);
    let (mut h, mut r) = (Hooks::new(), Rng::default());
    assert!(
        b.player_command(2, cmd::ATTACK, 176, P1, 0, &mut h)
            .unwrap()
    );
    run_idle(&mut b, &mut h, &mut r);
    assert!(covered(&b).is_empty());
}

/// Derivation: `FindStrongestDefender` keeps the candidate with the highest
/// current HP and replaces it only when strictly lower (`coverBy.CurrentHp <
/// unit.CurrentHp`), so the first unit wins a tie.
#[test]
fn cover_picks_the_strongest_defender_first_on_ties() {
    let run = |hp2: u32, hp3: u32| {
        let mut b = cover_battle(
            low(unit(P1, true, 100)),
            vec![
                with_sa(unit(P2, true, hp2), SA_COVER),
                with_sa(unit(P3, true, hp3), SA_COVER),
            ],
        );
        let (mut h, mut r) = (Hooks::new(), Rng::default());
        enemy_cast(&mut b, &mut h, 3, 0, P1);
        run_idle(&mut b, &mut h, &mut r);
        covered(&b)
    };
    assert_eq!(run(100, 300), vec![(P1, P3)]);
    assert_eq!(run(300, 300), vec![(P1, P2)]);
    assert_eq!(run(400, 300), vec![(P1, P2)]);
}

/// Derivation: a unit that already covers the target keeps covering it even
/// when a stronger candidate exists (`CheckCoverAbility` first loop,
/// `btl_abil.cs:193-196`).
#[test]
fn an_existing_cover_is_kept() {
    let mut p3 = unit(P3, true, 50);
    p3.cover_unit = Some(P1);
    p3.covering = true;
    let mut b = cover_battle(
        low(unit(P1, true, 100)),
        vec![with_sa(unit(P2, true, 900), SA_COVER), p3],
    );
    let (mut h, mut r) = (Hooks::new(), Rng::default());
    enemy_cast(&mut b, &mut h, 3, 0, P1);
    run_idle(&mut b, &mut h, &mut r);
    assert_eq!(covered(&b), vec![(P1, P3)]);
    assert_eq!(b.units[2].cur_hp, 7);
}

// ---------------------------------------------------------------------------
// Counter (SA 36, `TriggerOnAbility` EffectDone + `btl_cmd.SetCounter`)
// ---------------------------------------------------------------------------

fn counter_battle(will: u8, sa39: bool) -> Battle {
    let mut p1 = with_sa(unit(P1, true, 500), SA_COUNTER);
    p1.will = will;
    if sa39 {
        p1 = with_sa(p1, SA_EYE_4_EYE);
    }
    battle(vec![p1, unit(E1, false, 500)])
}

fn counters(b: &Battle) -> usize {
    events(b)
        .iter()
        .filter(|e| matches!(e, BattleEvent::CommandQueued { unit: 0, cmd_no, .. } if *cmd_no == cmd::COUNTER))
        .count()
}

/// Derivation: P1 (Spirit 60) is hit by a physical enemy attack. Condition:
/// `IsCounterableCommand` (EnemyAtk 47 < 48), opposite sides, category & 8,
/// `60 > GetRandom(0, 100)`. Roll 59 -> true: `SetCounter(P1, Counter,
/// 176, E1)` queues command 49 targeting E1 in slot 1; it then runs and the
/// `Counter` Attack (power 3) sets E1's HP to 3. Roll 60 -> `60 > 60` false.
#[test]
fn counter_fires_when_spirit_beats_the_roll() {
    let mut b = counter_battle(60, false);
    let (mut h, mut r) = (Hooks::new(), Rng::roll(59));
    enemy_cast(&mut b, &mut h, 1, 0, P1);
    run_idle(&mut b, &mut h, &mut r);
    assert_eq!(counters(&b), 1);
    assert!(events(&b).contains(&BattleEvent::CommandQueued {
        unit: 0,
        cmd_no: cmd::COUNTER,
        tar_id: E1,
    }));
    assert_eq!(effect_targets(&b), vec![0, 1]);
    assert_eq!(b.units[1].cur_hp, 3);
    assert_eq!(r.rolls_used, 1);

    let mut b = counter_battle(60, false);
    let (mut h, mut r) = (Hooks::new(), Rng::roll(60));
    enemy_cast(&mut b, &mut h, 1, 0, P1);
    run_idle(&mut b, &mut h, &mut r);
    assert_eq!(counters(&b), 0);
    assert_eq!(b.units[1].cur_hp, 500);
    assert_eq!(r.rolls_used, 1);
}

/// Derivation: with SA 39 the left side is `2 * TargetSpirit`: 2 * 40 = 80 > 79.
#[test]
fn eye_4_eye_doubles_the_spirit_term() {
    let mut b = counter_battle(40, true);
    let (mut h, mut r) = (Hooks::new(), Rng::roll(79));
    enemy_cast(&mut b, &mut h, 1, 0, P1);
    run_idle(&mut b, &mut h, &mut r);
    assert_eq!(counters(&b), 1);
    let mut b = counter_battle(40, false);
    let (mut h, mut r) = (Hooks::new(), Rng::roll(39));
    enemy_cast(&mut b, &mut h, 1, 0, P1);
    run_idle(&mut b, &mut h, &mut r);
    assert_eq!(counters(&b), 1);
    let mut b = counter_battle(40, false);
    let (mut h, mut r) = (Hooks::new(), Rng::roll(40));
    enemy_cast(&mut b, &mut h, 1, 0, P1);
    run_idle(&mut b, &mut h, &mut r);
    assert_eq!(counters(&b), 0);
}

/// Derivation: NCalc `&&` short-circuits, so the roll is not drawn when an
/// earlier term fails: a magic hit (no category bit 8), a target that cannot
/// move (Stop is in `Immobilized`), or an attack from the same side.
#[test]
fn counter_draws_no_roll_when_an_earlier_term_fails() {
    // Magic (category 16): no bit 8.
    let mut b = counter_battle(90, false);
    let (mut h, mut r) = (Hooks::new(), Rng::default());
    enemy_cast(&mut b, &mut h, 1, 4, P1);
    run_idle(&mut b, &mut h, &mut r);
    assert_eq!((counters(&b), r.rolls_used), (0, 0));

    // Immobilized target (a second player keeps the battle running).
    let mut p1 = with_sa(unit(P1, true, 500), SA_COUNTER);
    p1.will = 90;
    p1.status_current = st::STOP;
    let mut b = battle(vec![p1, unit(P2, true, 500), unit(E1, false, 500)]);
    let (mut h, mut r) = (Hooks::new(), Rng::default());
    enemy_cast(&mut b, &mut h, 2, 0, P1);
    run_idle(&mut b, &mut h, &mut r);
    assert_eq!((counters(&b), r.rolls_used), (0, 0));

    // Ally hit by an ally: `CasterIsPlayer != TargetIsPlayer` fails.
    let mut p2 = unit(P2, true, 500);
    p2.status_current = 0;
    let mut b = battle(vec![
        with_sa(unit(P1, true, 500), SA_COUNTER),
        p2,
        unit(E1, false, 500),
    ]);
    let (mut h, mut r) = (Hooks::new(), Rng::default());
    assert!(
        b.player_command(1, cmd::ATTACK, 176, P1, 0, &mut h)
            .unwrap()
    );
    run_idle(&mut b, &mut h, &mut r);
    assert_eq!((counters(&b), r.rolls_used), (0, 0));
}

/// Derivation: `SetCounter` refuses a unit under `PreventCounter` (Sleep,
/// Death or a `CannotAct` status, `btl_cmd.cs:338`) and any phase other than
/// normal, and uses `cursor = 1` only for several targets.
#[test]
fn set_counter_is_refused_under_prevent_counter() {
    let mut b = counter_battle(60, false);
    let mut h = Hooks::new();
    b.units[0].status_current = st::SLEEP;
    assert!(!b.set_counter(0, cmd::COUNTER, 176, E1, &mut h).unwrap());
    b.units[0].status_current = 0;
    assert!(b.set_counter(0, cmd::COUNTER, 176, E1, &mut h).unwrap());
    let slot = b.cmds.slot_of(0, 1);
    assert_eq!(b.cmds.slots[slot].cmd_no, cmd::COUNTER);
    assert_eq!(b.cmds.slots[slot].cursor, 0);
    assert!(b.cmds.slots[slot].priority);
    // The slot is in use, so a second counter is not queued.
    assert!(!b.set_counter(0, cmd::COUNTER, 176, E1, &mut h).unwrap());
}

// ---------------------------------------------------------------------------
// Item commands
// ---------------------------------------------------------------------------

fn potion() -> ItemData {
    ItemData {
        effect: ItemEffect {
            power: 10,
            ..ItemEffect::default()
        },
        script_id: 69,
        for_dead: false,
    }
}

/// Derivation (`GetCommandScriptId`, `btl_util.cs:445`; `ItemUse`,
/// `btl_cmd.cs:643-648`): the Item command runs the item's script id (69, the
/// B4 Potion script: 10 * 15 = 150) and removes one copy when it reaches
/// `CMD_MODE_SELECT_VFX`. P2 HP 100 -> 250; the stack goes 3 -> 2.
#[test]
fn item_command_runs_the_item_script_and_consumes_one() {
    let mut hurt = unit(P2, true, 100);
    hurt.max_hp = 1000;
    let mut b = battle(vec![unit(P1, true, 500), hurt, unit(E1, false, 500)]);
    let mut h = Hooks::new();
    h.items.push((7, potion()));
    b.inventory.insert(7, 3);
    let mut r = Rng::default();
    assert!(b.player_command(0, cmd::ITEM, 7, P2, 0, &mut h).unwrap());
    run_idle(&mut b, &mut h, &mut r);
    assert_eq!(b.units[1].cur_hp, 250);
    assert_eq!(b.inventory[&7], 2);
    assert!(events(&b).contains(&BattleEvent::ItemUsed {
        unit: 0,
        item: 7,
        remaining: 2,
    }));
}

/// Derivation (`CheckCommandCondition`, `btl_cmd.cs:1001-1007`): an empty
/// stack rejects the command in inspection and nothing is consumed; an item id
/// with no table row is refused at queue time.
#[test]
fn item_command_needs_a_copy_and_a_table_row() {
    let mut hurt = unit(P2, true, 100);
    hurt.max_hp = 1000;
    let mut b = battle(vec![unit(P1, true, 500), hurt, unit(E1, false, 500)]);
    let mut h = Hooks::new();
    h.items.push((7, potion()));
    let mut r = Rng::default();
    assert!(b.player_command(0, cmd::ITEM, 7, P2, 0, &mut h).unwrap());
    run_idle(&mut b, &mut h, &mut r);
    assert!(events(&b).contains(&BattleEvent::CommandRejected {
        unit: 0,
        cmd_no: cmd::ITEM,
    }));
    assert_eq!(b.units[1].cur_hp, 100);
    assert!(!b.inventory.contains_key(&7));
    assert_eq!(
        b.player_command(0, cmd::ITEM, 99, P2, 0, &mut h),
        Err(BattleError::UnknownCommand {
            cmd_no: cmd::ITEM,
            sub_no: 99,
        })
    );
}

/// Derivation (`CheckTargetCondition`, `btl_cmd.cs:1231-1232`): the item's
/// `ForDead` flag decides whether a dead target is valid. With
/// `for_dead = false` a dead single target is replaced by a random living
/// player (`GetRandomBtlID`, the `(cursor & 1) == 0` arm), so the effect lands
/// on P1 (index 0) and the copy is used; with `for_dead = true` the dead P2
/// (index 1) is accepted (script 64, no formula, only to show who was
/// targeted).
#[test]
fn item_for_dead_flag_decides_dead_targets() {
    let run = |for_dead: bool| {
        let dead = with_status(unit(P2, true, 0), st::DEATH);
        let mut b = battle(vec![unit(P1, true, 500), dead, unit(E1, false, 500)]);
        let mut h = Hooks::new();
        h.items.push((
            9,
            ItemData {
                effect: ItemEffect::default(),
                script_id: 64,
                for_dead,
            },
        ));
        b.inventory.insert(9, 1);
        let mut r = Rng::default();
        assert!(b.player_command(0, cmd::ITEM, 9, P2, 0, &mut h).unwrap());
        run_idle(&mut b, &mut h, &mut r);
        (effect_targets(&b), b.inventory[&9])
    };
    assert_eq!(run(false), (vec![0], 0));
    assert_eq!(run(true), (vec![1], 0));
}

// ---------------------------------------------------------------------------
// Double Cast (`btl_cmd.SetCommand`, script 80 branch, `btl_cmd.cs:162-241`)
// ---------------------------------------------------------------------------

fn dc_battle() -> Battle {
    battle(vec![
        unit(P1, true, 500),
        unit(E1, false, 500),
        unit(E2, false, 500),
    ])
}

/// Derivation: Double Cast (sub 255: Power 10, Rate 11, MP 20) at E1. Both
/// spells are single-target, non-ally: the `else` arm with
/// `!first.DefaultAlly && (tar & 0xF0) == tar` gives both the same target.
/// The first goes to slot 3 with `CustomMPCost = 20`, the second to slot 0
/// with 0. Queue order [slot 3, slot 0]; MP 100 -> 80; the spells set E1's HP
/// to 7, then 9.
#[test]
fn double_cast_queues_two_spells_and_charges_once() {
    let mut b = dc_battle();
    let (mut h, mut r) = (Hooks::new(), Rng::default());
    assert!(
        b.player_command(0, cmd::DOUBLE_WHITE_MAGIC, 255, E1, 0, &mut h)
            .unwrap()
    );
    let (s3, s0) = (b.cmds.slot_of(0, 3), b.cmds.slot_of(0, 0));
    assert_eq!(b.cmds.queue, vec![s3, s0]);
    assert_eq!((b.cmds.slots[s3].sub_no, b.cmds.slots[s3].tar_id), (10, E1));
    assert_eq!((b.cmds.slots[s0].sub_no, b.cmds.slots[s0].tar_id), (11, E1));
    run_idle(&mut b, &mut h, &mut r);
    assert_eq!(effect_targets(&b), vec![1, 1]);
    assert_eq!(b.units[1].cur_hp, 9);
    assert_eq!(b.units[0].cur_mp, 80);
}

/// Derivation: a first spell that is `AllEnemy` takes `0xF0` and the second
/// takes the chosen target (`first_aa.Info.Target == AllEnemy` arm); the
/// cursor is 1 for a mask with several bits.
#[test]
fn double_cast_all_enemy_first_spell_takes_the_whole_side() {
    let mut b = dc_battle();
    let mut h = Hooks::new();
    assert!(
        b.player_command(0, cmd::DOUBLE_WHITE_MAGIC, 254, E1, 0, &mut h)
            .unwrap()
    );
    let (s3, s0) = (b.cmds.slot_of(0, 3), b.cmds.slot_of(0, 0));
    assert_eq!(b.cmds.slots[s3].tar_id, 0xF0);
    assert_eq!(b.cmds.slots[s3].cursor, 1);
    assert_eq!(b.cmds.slots[s0].tar_id, E1);
    assert_eq!(b.cmds.slots[s0].cursor, 0);
}

/// Derivation: a `SingleAny` first spell cast at a player falls into the last
/// arm, `Comn.randomID(0xF0)`: with draw 130, index `4 * 130 >> 8 = 2` picks
/// bit 6 (0x40). The plain `set_command` has no random source and refuses;
/// `set_command_with_rng` queues (0x40, P1).
#[test]
fn double_cast_random_branch_needs_a_random_source() {
    let mut b = dc_battle();
    let mut h = Hooks::new();
    let slot0 = b.cmds.slot_of(0, 0);
    assert!(matches!(
        b.set_command(slot0, cmd::DOUBLE_WHITE_MAGIC, 253, P1, 0, false, &mut h),
        Err(BattleError::Unported(_))
    ));
    let mut r = Rng::r8(&[130]);
    assert!(
        b.set_command_with_rng(
            slot0,
            cmd::DOUBLE_WHITE_MAGIC,
            253,
            P1,
            0,
            false,
            &mut h,
            &mut r,
        )
        .unwrap()
    );
    let s3 = b.cmds.slot_of(0, 3);
    assert_eq!(
        (b.cmds.slots[s3].tar_id, b.cmds.slots[slot0].tar_id),
        (0x40, P1)
    );
}
