#![allow(clippy::doc_lazy_continuation)]
//! Hand-derived tick-by-tick scenarios for the B5 battle state machine
//! (`tantalus_legacy::battle::{atb, queue, targeting, flee, machine}`).
//!
//! No reference trace was captured: every expected number below is derived by
//! hand from the pinned Memoria source (c716e1fa). Tick numbers are 1-based
//! and count `Battle::tick` calls; `Timing` is left at its default (0), so
//! animation and SFX waits do not appear.
//!
//! Shared numbers (battle speed config 1, `ATBSpeedFactor` 4.0):
//!   max ATB = (60 - dex) * 40 << 2: dex 40 -> 3200, dex 20 -> 6400, dex 0 -> 9600
//!   coef 10 (haste 15, slow 6), add per tick = coef * 4 (40, 60, 24)
//!   dex 40 from 0: 3200 / 40 = 80 ticks; dex 20: 160 ticks
//!   haste dex 40: ceil(3200 / 60) = 54; slow dex 40: ceil(3200 / 24) = 134
//! Command timeline for a command dequeued on tick T: inspection T,
//! select-VFX T + 1, effect and finish T + 2 (zero injected waits).

use std::collections::BTreeMap;
use std::sync::Arc;
use tantalus_legacy::battle::atb::{self, StartType};
use tantalus_legacy::battle::flee;
use tantalus_legacy::battle::machine::{
    Battle, BattleConfig, BattleEvent, BattleHooks, EndSeq, EnemyAction, MenuOffSeq, Phase, result,
};
use tantalus_legacy::battle::queue::{CommandQueue, CommandSpec, cmd, is_declarable};
use tantalus_legacy::battle::rng::Draw;
use tantalus_legacy::battle::status as st;
use tantalus_legacy::battle::targeting::{self, TargetCheck, TargetView, target_type};
use tantalus_legacy::battle::{
    AlterResult, BattleCommand, BattleUnit, Elements, SequenceRandom, StatusDataRow, StatusTable,
    WeaponStats,
};

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

fn unit(id: u16, player: bool, dex: u8) -> BattleUnit {
    let mut u = BattleUnit::new(id, player);
    u.level = if player { 20 } else { 15 };
    u.strength = 30;
    u.magic = 30;
    u.dexterity = dex;
    u.will = if player { 20 } else { 10 };
    u.row = if player { 1 } else { 2 };
    u.cur_hp = 500;
    u.max_hp = 500;
    u.cur_mp = 100;
    u.max_mp = 100;
    if player {
        u.weapon = Some(WeaponStats {
            power: 25,
            rate: 0,
            elements: Elements::NONE,
            category: WeaponStats::SHORT_RANGE,
            status: 0,
        });
    } else {
        u.physical_defence = 10;
        u.magic_defence = 20;
    }
    u
}

/// Catalog stand-in: Attack (script 1), Defend (script 103), and the same
/// Attack data for `EnemyAtk`.
struct Hooks {
    enemy: Option<EnemyAction>,
}

impl BattleHooks for Hooks {
    fn spec(&mut self, cmd_no: u32, _sub: i32, _c: &BattleUnit) -> Option<CommandSpec> {
        match cmd_no {
            cmd::ATTACK | cmd::ENEMY_ATK => {
                let mut c = BattleCommand::new(1);
                c.ability_category = 8;
                c.target_type = target_type::SINGLE_ENEMY;
                Some(CommandSpec {
                    command: c,
                    default_ally: false,
                    for_dead: false,
                })
            }
            cmd::SUMMON_GARNET => {
                let mut c = BattleCommand::new(103);
                c.hit_rate = 255;
                c.target_type = target_type::SELF;
                c.mp_cost = 4;
                Some(CommandSpec {
                    command: c,
                    default_ally: true,
                    for_dead: false,
                })
            }
            cmd::DEFEND => {
                let mut c = BattleCommand::new(103);
                c.hit_rate = 255;
                c.ability_status = st::DEFEND;
                c.target_type = target_type::SELF;
                Some(CommandSpec {
                    command: c,
                    default_ally: true,
                    for_dead: false,
                })
            }
            _ => None,
        }
    }

    fn enemy_atb(&mut self, _b: &Battle, _unit: usize) -> Option<EnemyAction> {
        self.enemy.take()
    }
}

fn hooks() -> Hooks {
    Hooks { enemy: None }
}

fn battle(units: Vec<BattleUnit>) -> Battle {
    Battle::new(units, BattleConfig::default(), table())
}

fn no_draws() -> SequenceRandom {
    SequenceRandom::r16(&[])
}

fn run(b: &mut Battle, h: &mut Hooks, rng: &mut SequenceRandom, n: u32) {
    for _ in 0..n {
        b.tick(h, rng).expect("tick");
    }
}

fn first_tick(b: &Battle, f: impl Fn(&BattleEvent) -> bool) -> Option<u32> {
    b.log.iter().find(|(_, e)| f(e)).map(|(t, _)| *t)
}

fn ticks_of(b: &Battle, f: impl Fn(&BattleEvent) -> bool) -> Vec<u32> {
    b.log
        .iter()
        .filter(|(_, e)| f(e))
        .map(|(t, _)| *t)
        .collect()
}

/// Draws of one weapon hit that connects, never crits and deals 495
/// (see `battle_formulas.rs::s1_basic_hit`): ctor, hit, evade, base, crit pair.
const HIT: [i32; 6] = [7, 1234, 5678, 9999, 7, 50];

// ---------------------------------------------------------------------------
// ATB gauge
// ---------------------------------------------------------------------------

#[test]
fn atb_formula_constants() {
    assert_eq!(atb::BATTLE_TPS, 15);
    assert_eq!(atb::max_atb(40), 3200);
    assert_eq!(atb::max_atb(20), 6400);
    assert_eq!(atb::max_atb(0), 9600);
    assert_eq!(
        [
            atb::base_atb_coef(0),
            atb::base_atb_coef(1),
            atb::base_atb_coef(2)
        ],
        [8, 10, 14]
    );
    assert_eq!(atb::base_atb_coef(9), 10);
    assert_eq!(atb::hasted_coef(10), 15);
    assert_eq!(atb::slowed_coef(10), 6);
    assert_eq!(atb::atb_step(10, 4.0), 40);
    assert_eq!(atb::atb_step(15, 4.0), 60);
    assert_eq!(atb::atb_step(6, 4.0), 24);
    // Factors under 1.0 read as 1.0.
    assert_eq!(atb::clamp_speed_factor(0.25), 1.0);
}

/// Derivation (see the header): four players, one enemy that never acts.
/// dex 40 normal: tick 80; dex 20: tick 160; dex 40 haste: 54 * 60 = 3240 at
/// tick 54 (53 * 60 = 3180 is short); dex 40 slow: 134 * 24 = 3216 at tick
/// 134 (133 * 24 = 3192 is short). The enemy (dex 0, max 9600) is full at
/// 9600 / 40 = tick 240.
#[test]
fn fast_vs_slow_first_ready_ticks() {
    let mut b = battle(vec![
        unit(1, true, 40),
        unit(2, true, 20),
        unit(4, true, 40),
        unit(8, true, 40),
        unit(16, false, 0),
    ]);
    b.sched[2].atb_coef = atb::hasted_coef(10);
    b.sched[3].atb_coef = atb::slowed_coef(10);
    let (mut h, mut r) = (hooks(), no_draws());
    run(&mut b, &mut h, &mut r, 240);
    let ready = |u: usize| first_tick(&b, |e| *e == BattleEvent::PlayerReady { unit: u });
    assert_eq!(ready(0), Some(80));
    assert_eq!(ready(1), Some(160));
    assert_eq!(ready(2), Some(54));
    assert_eq!(ready(3), Some(134));
    let full = |u: usize| first_tick(&b, |e| *e == BattleEvent::AtbFull { unit: u });
    assert_eq!(full(4), Some(240));
    // The gauge is clamped at max once full.
    assert_eq!(b.units[0].cur_atb, 3200);
    assert_eq!(b.ready_players, vec![2, 0, 3, 1]);
}

/// Derivation: players dex 40 (max 3200) and enemy dex 0 (max 9600). Draws in
/// unit order: 5000 % 3200 = 1800; 20000 % 9600 = 800. The player then needs
/// (3200 - 1800) / 40 = 35 ticks. A first attack sets players to max - 1 =
/// 3199 (ready on tick 1, no draw) and enemies to 0 (the enemy draw is still
/// taken); a back attack sets players to 0 (no draw, the enemy draws).
#[test]
fn initial_gauge_and_start_types() {
    let mut b = battle(vec![unit(1, true, 40), unit(16, false, 0)]);
    let mut r = SequenceRandom::r16(&[5000, 20000]);
    b.roll_initial_atb(StartType::Normal, &mut r).unwrap();
    assert!(r.is_exhausted());
    assert_eq!((b.units[0].cur_atb, b.units[1].cur_atb), (1800, 800));
    let (mut h, mut r) = (hooks(), no_draws());
    run(&mut b, &mut h, &mut r, 35);
    assert_eq!(
        first_tick(&b, |e| matches!(e, BattleEvent::PlayerReady { .. })),
        Some(35)
    );

    let mut b = battle(vec![unit(1, true, 40), unit(16, false, 0)]);
    let mut r = SequenceRandom::r16(&[20000]);
    b.roll_initial_atb(StartType::FirstAttack, &mut r).unwrap();
    assert!(
        r.is_exhausted(),
        "only the enemy draws: the player takes max - 1"
    );
    assert_eq!((b.units[0].cur_atb, b.units[1].cur_atb), (3199, 0));
    let (mut h, mut r) = (hooks(), no_draws());
    run(&mut b, &mut h, &mut r, 1);
    assert_eq!(
        first_tick(&b, |e| matches!(e, BattleEvent::PlayerReady { .. })),
        Some(1)
    );

    let mut b = battle(vec![unit(1, true, 40), unit(16, false, 0)]);
    let mut r = SequenceRandom::r16(&[20000]);
    b.roll_initial_atb(StartType::BackAttack, &mut r).unwrap();
    assert_eq!(b.units[0].cur_atb, 0);
}

/// Derivation: a unit in Stop (`StopAtb`) does not fill; Sleep fills but
/// never becomes ready (`PreventATBConfirm`).
#[test]
fn gauge_pauses_and_prevent_confirm() {
    let mut asleep = unit(2, true, 40);
    asleep.status_current = st::SLEEP;
    let mut b = battle(vec![unit(1, true, 40), asleep, unit(16, false, 0)]);
    let (mut h, mut r) = (hooks(), no_draws());
    b.alter_status_on(0, 12, &mut r);
    run(&mut b, &mut h, &mut r, 100);
    assert_eq!(b.units[0].cur_atb, 0, "Stop freezes the gauge");
    assert_eq!(b.units[1].cur_atb, 3200);
    assert!(first_tick(&b, |e| *e == BattleEvent::AtbFull { unit: 1 }).is_some());
    assert!(first_tick(&b, |e| matches!(e, BattleEvent::PlayerReady { .. })).is_none());
}

// ---------------------------------------------------------------------------
// Command queue
// ---------------------------------------------------------------------------

fn put(q: &mut CommandQueue, unit: usize, k: usize, cmd_no: u32) -> usize {
    let id = q.slot_of(unit, k);
    q.slots[id].cmd_no = cmd_no;
    q.slots[id].priority = !is_declarable(cmd_no);
    q.enqueue(id);
    id
}

/// Derivation (`EnqueueCommand`, `btl_cmd.cs:831-874`): non-priority commands
/// append. SysTrans (59, priority) is inserted before the first queued id
/// outside 59..=99 (the two attacks), so it goes to the front. SysDead (60)
/// goes before the first id < 60 (SysTrans), so it passes SysTrans.
/// EnemyDying (54) goes before the first id that is not 60..=99 and not 54:
/// SysDead (60) is passed, SysTrans (59) is the first stop, so it lands
/// between them.
#[test]
fn queue_priority_insertion() {
    let mut q = CommandQueue::new(4, Some(0));
    let a0 = put(&mut q, 0, 0, cmd::ATTACK);
    let a1 = put(&mut q, 1, 0, cmd::ATTACK);
    assert_eq!(q.queue, vec![a0, a1]);
    let trans = put(&mut q, 2, 4, cmd::SYS_TRANS);
    assert_eq!(q.queue, vec![trans, a0, a1]);
    let dead = put(&mut q, 3, 2, cmd::SYS_DEAD);
    assert_eq!(q.queue, vec![dead, trans, a0, a1]);
    let dying = put(&mut q, 2, 1, cmd::ENEMY_DYING);
    assert_eq!(q.queue, vec![dead, dying, trans, a0, a1]);
}

/// Derivation: `KillStandardCommands(unit 0)` removes queued commands of that
/// unit with id < EnemyDying or > 99 (its Attack) and reports that the main
/// command was cancelled; the SysTrans of unit 0 stays. `KillAllCommand`
/// removes id < SysTrans and > 99 only, so the remaining Attack and a queued
/// SysEscape go and SysTrans stays. Killing a SysEscape clears `cmd_status`
/// bit 0.
#[test]
fn queue_cancel_and_kill() {
    let mut q = CommandQueue::new(2, Some(0));
    let a0 = put(&mut q, 0, 0, cmd::ATTACK);
    let t0 = put(&mut q, 0, 4, cmd::SYS_TRANS);
    let a1 = put(&mut q, 1, 0, cmd::ATTACK);
    assert_eq!(q.queue, vec![t0, a0, a1]);
    assert!(q.kill_standard_commands(0));
    assert_eq!(q.queue, vec![t0, a1]);
    let esc = q.escape_slot();
    q.cmd_status |= 1;
    q.slots[esc].cmd_no = cmd::SYS_ESCAPE;
    q.slots[esc].priority = true;
    q.enqueue(esc);
    let removed = q.kill_all();
    assert_eq!(q.queue, vec![t0]);
    assert_eq!(removed.len(), 2);
    assert_eq!(q.cmd_status & 1, 0, "escape bit cleared by the kill");
}

// ---------------------------------------------------------------------------
// Targeting
// ---------------------------------------------------------------------------

fn view(id: u16, player: bool, dead: bool) -> TargetView {
    TargetView {
        btl_id: id,
        is_player: player,
        dead,
        targetable: true,
        out_of_reach: false,
    }
}

/// Derivation: `Comn.randomID` index = count * random8 >> 8. Mask 0x30 has
/// two bits: random8 200 -> 400 >> 8 = 1 -> 0x20; 127 -> 254 >> 8 = 0 ->
/// 0x10. An empty mask draws nothing.
#[test]
fn random_id_picks_by_count_times_draw() {
    let mut r = SequenceRandom::with_kinds(&[(Draw::Random8, 200), (Draw::Random8, 127)]);
    assert_eq!(targeting::random_id(0x30, &mut r), 0x20);
    assert_eq!(targeting::random_id(0x30, &mut r), 0x10);
    assert_eq!(targeting::random_id(0, &mut r), 0);
    assert!(r.is_exhausted());
}

/// Derivation: enemies 0x10, 0x20 (dead), 0x40. `RandomEnemy` with an enemy
/// default: mask 0xF0 minus the dead 0x20 = 0xD0 (0xF0 also holds 0x80,
/// absent here, so the mask keeps bits 4, 6, 7): three bits, random8 130 ->
/// 390 >> 8 = 1 -> 0x40. With `for_dead` the mask stays 0xF0 (four bits):
/// 130 * 4 >> 8 = 2 -> 0x40. All-target returns 0xFF with no draw; Self
/// returns the caster.
#[test]
fn random_target_for_command_rules() {
    let units = [
        view(1, true, false),
        view(0x10, false, false),
        view(0x20, false, true),
        view(0x40, false, false),
    ];
    let mut r = SequenceRandom::with_kinds(&[(Draw::Random8, 130), (Draw::Random8, 130)]);
    let t = targeting::random_target_for_command(
        &units,
        1,
        target_type::RANDOM_ENEMY,
        false,
        false,
        &mut r,
    );
    assert_eq!(t, 0x40);
    let t = targeting::random_target_for_command(
        &units,
        1,
        target_type::RANDOM_ENEMY,
        false,
        true,
        &mut r,
    );
    assert_eq!(t, 0x40);
    assert!(r.is_exhausted());
    let mut r = no_draws();
    assert_eq!(
        targeting::random_target_for_command(&units, 1, target_type::ALL, false, false, &mut r),
        0xFF
    );
    assert_eq!(
        targeting::random_target_for_command(&units, 1, target_type::SELF, true, false, &mut r),
        1
    );
}

/// Derivation (`CheckTargetCondition`): a single target that died is
/// re-picked with `GetRandomBtlID` on the same side (`Range(0, count)`):
/// enemies 0x10 (dead, the target) and 0x20 (alive): one candidate, draw 0
/// -> 0x20. An all-target command (cursor bit 0) with no live target is
/// invalid, and a live single target is kept without a draw.
#[test]
fn dead_target_is_repicked_or_invalid() {
    let units = [
        view(1, true, false),
        view(0x10, false, true),
        view(0x20, false, false),
    ];
    let mut r = SequenceRandom::with_kinds(&[(Draw::Range, 0)]);
    assert_eq!(
        targeting::check_target_condition(&units, 0x10, 0, false, false, &mut r),
        TargetCheck::Valid(0x20)
    );
    assert!(r.is_exhausted());
    let mut r = no_draws();
    assert_eq!(
        targeting::check_target_condition(&units, 0x10, 1, false, false, &mut r),
        TargetCheck::Invalid
    );
    assert_eq!(
        targeting::check_target_condition(&units, 0x20, 0, false, false, &mut r),
        TargetCheck::Valid(0x20)
    );
    // An all-enemy mask keeps every live enemy.
    let units = [
        view(1, true, false),
        view(0x10, false, false),
        view(0x20, false, false),
    ];
    assert_eq!(
        targeting::check_target_condition(&units, 0xF0, 1, false, false, &mut r),
        TargetCheck::Valid(0x30)
    );
}

// ---------------------------------------------------------------------------
// Defend
// ---------------------------------------------------------------------------

/// Derivation: the player is ready on tick 80. The Defend command (command
/// 4, script 103, rate 255, status Defend, self target) is queued after
/// tick 80, dequeued on tick 81, so inspection 81, select-VFX 82, effect and
/// finish 83. The gauge resets to 0 on finish and the same tick's ATB add
/// brings it to 40. The next main command removes Defend
/// (`RemoveOnMainCommand`), immediately on being set.
#[test]
fn defend_timeline_and_removal() {
    let mut b = battle(vec![unit(1, true, 40), unit(16, false, 0)]);
    let (mut h, mut r) = (hooks(), SequenceRandom::r16(&[7, 7, 7, 7]));
    run(&mut b, &mut h, &mut r, 80);
    assert_eq!(
        first_tick(&b, |e| matches!(e, BattleEvent::PlayerReady { .. })),
        Some(80)
    );
    assert!(b.player_command(0, cmd::DEFEND, 179, 1, 0, &mut h).unwrap());
    run(&mut b, &mut h, &mut r, 2);
    assert_eq!(
        b.units[0].status_current & st::DEFEND,
        0,
        "not yet, tick 82"
    );
    run(&mut b, &mut h, &mut r, 1);
    assert_ne!(
        b.units[0].status_current & st::DEFEND,
        0,
        "applied on tick 83"
    );
    assert_eq!(
        first_tick(&b, |e| matches!(e, BattleEvent::CommandStarted { .. })),
        Some(81)
    );
    assert_eq!(
        first_tick(&b, |e| matches!(e, BattleEvent::CommandFinished { .. })),
        Some(83)
    );
    assert_eq!(b.units[0].cur_atb, 40);
    assert_eq!(b.sched[0].sel_mode, 0);
    assert!(
        b.player_command(0, cmd::ATTACK, 176, 16, 0, &mut h)
            .unwrap()
    );
    assert_eq!(b.units[0].status_current & st::DEFEND, 0);
}

/// A player command waits for its own effect (`BattleConfig::command_timing`):
/// Defend with (10, 20) applies 10 ticks into `CMD_MODE_LOOP` (tick 93, not
/// 83) and finishes 10 later.
#[test]
fn command_timing_delays_the_effect_and_the_end() {
    let timing = BTreeMap::from([((false, 179, 1u8), (10u32, 20u32))]);
    let mut b = Battle::new(
        vec![unit(1, true, 40), unit(16, false, 0)],
        BattleConfig {
            command_timing: Arc::new(timing),
            ..BattleConfig::default()
        },
        table(),
    );
    let (mut h, mut r) = (hooks(), SequenceRandom::r16(&[7, 7, 7, 7]));
    run(&mut b, &mut h, &mut r, 80);
    assert!(b.player_command(0, cmd::DEFEND, 179, 1, 0, &mut h).unwrap());
    run(&mut b, &mut h, &mut r, 30);
    assert_eq!(
        first_tick(&b, |e| matches!(e, BattleEvent::EffectApplied { .. })),
        Some(93)
    );
    assert_eq!(
        first_tick(&b, |e| matches!(e, BattleEvent::CommandFinished { .. })),
        Some(103)
    );
}

/// `DecideSummonType`: Shiva's first cast plays in full and draws nothing;
/// once seen, `random8() < 230` (more than twice its MP left) makes it
/// short and picks the short effect's timing (variant bit 1).
#[test]
fn summon_first_cast_full_then_short() {
    let timing = BTreeMap::from([
        ((false, 49, 1u8), (300u32, 380u32)),
        ((false, 49, 3u8), (60u32, 90u32)),
    ]);
    let mut p = unit(1, true, 40);
    p.cur_mp = 50;
    let mut b = Battle::new(
        vec![p, unit(16, false, 0)],
        BattleConfig {
            command_timing: Arc::new(timing),
            ..BattleConfig::default()
        },
        table(),
    );
    let (mut h, mut r) = (hooks(), SequenceRandom::r16(&[7, 7, 7, 7]));
    run(&mut b, &mut h, &mut r, 80);
    assert!(
        b.player_command(0, cmd::SUMMON_GARNET, 49, 1, 0, &mut h)
            .unwrap()
    );
    run(&mut b, &mut h, &mut r, 400);
    let applied = ticks_of(&b, |e| matches!(e, BattleEvent::EffectApplied { .. }));
    assert_eq!(applied.first(), Some(&(83 + 300)));
    assert_eq!(b.config.summons_seen, vec![49]);

    let mut b2 = Battle::new(
        vec![
            {
                let mut p = unit(1, true, 40);
                p.cur_mp = 50;
                p
            },
            unit(16, false, 0),
        ],
        BattleConfig {
            command_timing: b.config.command_timing.clone(),
            summons_seen: vec![49],
            ..BattleConfig::default()
        },
        table(),
    );
    let mut r2 = SequenceRandom::with_kinds(&[
        (Draw::Random8, 229),
        (Draw::Random16, 7),
        (Draw::Random16, 7),
        (Draw::Random16, 7),
        (Draw::Random16, 7),
    ]);
    let mut h2 = hooks();
    run(&mut b2, &mut h2, &mut r2, 80);
    assert!(
        b2.player_command(0, cmd::SUMMON_GARNET, 49, 1, 0, &mut h2)
            .unwrap()
    );
    run(&mut b2, &mut h2, &mut r2, 100);
    assert_eq!(
        first_tick(&b2, |e| matches!(e, BattleEvent::EffectApplied { .. })),
        Some(83 + 60)
    );
}

// ---------------------------------------------------------------------------
// Flee
// ---------------------------------------------------------------------------

/// Derivation: two level 20 players (sum 40, count 2 -> 20) and one level 15
/// enemy: 200 / 15 = 13, * 20 = 260, / 16 = 16. The roll succeeds when
/// 16 > random16 % 100: draw 115 -> 15 succeeds, 116 -> 16 fails. A zero
/// level average is an error, not a guess.
#[test]
fn flee_rate_and_roll() {
    let s = flee::EscapeSides {
        player_count: 2,
        enemy_count: 1,
        player_levels: 40,
        enemy_levels: 15,
    };
    assert_eq!(flee::escape_rate(s), Ok(16));
    assert!(flee::try_escape_roll(16, &mut SequenceRandom::r16(&[115])));
    assert!(!flee::try_escape_roll(16, &mut SequenceRandom::r16(&[116])));
    let zero = flee::EscapeSides {
        player_count: 1,
        enemy_count: 3,
        player_levels: 20,
        enemy_levels: 2,
    };
    assert!(flee::escape_rate(zero).is_err());
    // Counts and sums of 0 read as 1: (200 / 1) * 1 / 16 = 12.
    let none = flee::EscapeSides {
        player_count: 0,
        enemy_count: 0,
        player_levels: 0,
        enemy_levels: 0,
    };
    assert_eq!(flee::escape_rate(none), Ok(12));
    // Flee ability gil cost: a tenth of 1000 + 500, capped by the party gil.
    assert_eq!(flee::flee_gil_loss(1000, 500, 10_000), (150, 9_850));
    assert_eq!(flee::flee_gil_loss(1000, 500, 100), (100, 0));
}

/// Derivation: the roll is taken between ticks (the UI hold is the caller's).
/// Success queues SysEscape; dequeued on tick 1 (T = 1). Inspection sets
/// MenuOff/Escape (tick 1) and the command goes idle; it finishes on tick 2.
/// `btl_escape_fade` is 32: in the trailing loop of tick 2 it becomes 30 at
/// the first player and the after-loop step makes it 28; each later tick
/// takes 2 more, so it is 0 at the end of tick 16 and the tick 17 loop sees
/// `<= 0` (T + 16). Result ESCAPE and phase Close are set on tick 17; tick 18
/// turns FadeOut into Wait; `btl_fade_time++ > 32` first holds on the 34th
/// call (tick 19 is call 1): tick 52 = 17 + 35.
#[test]
fn flee_key_timeline() {
    let mut b = battle(vec![
        unit(1, true, 40),
        unit(2, true, 40),
        unit(16, false, 0),
    ]);
    let (mut h, mut r) = (hooks(), SequenceRandom::r16(&[115]));
    assert!(b.flee_attempt(&mut h, &mut r).unwrap());
    assert!(r.is_exhausted());
    assert_eq!(b.cmds.cmd_status & 1, 1);
    run(&mut b, &mut h, &mut r, 1);
    assert_eq!(
        (b.phase, b.menu_off_seq),
        (Phase::MenuOff, MenuOffSeq::Escape)
    );
    assert_eq!(b.escape_count, 1);
    assert!(!b.ability_flee, "party flee key is sub 1");
    run(&mut b, &mut h, &mut r, 15);
    assert_eq!(b.result, None, "tick 16");
    assert_eq!(b.escape_fade, 0);
    run(&mut b, &mut h, &mut r, 1);
    assert_eq!(b.result, Some(result::ESCAPE));
    assert_eq!(b.phase, Phase::Close);
    assert_eq!(
        first_tick(&b, |e| matches!(e, BattleEvent::Result { .. })),
        Some(17)
    );
    run(&mut b, &mut h, &mut r, 1);
    assert_eq!(b.end_seq, EndSeq::Wait);
    run(&mut b, &mut h, &mut r, 33);
    assert!(!b.over, "tick 51");
    run(&mut b, &mut h, &mut r, 1);
    assert!(b.over);
    assert_eq!(first_tick(&b, |e| *e == BattleEvent::BattleOver), Some(52));
}

/// A failed roll queues nothing; the flee ability always queues the escape
/// with sub 0 (gil cost flag) and needs no roll.
#[test]
fn flee_failure_and_ability() {
    let mut b = battle(vec![
        unit(1, true, 40),
        unit(2, true, 40),
        unit(16, false, 0),
    ]);
    let (mut h, mut r) = (hooks(), SequenceRandom::r16(&[116]));
    assert!(!b.flee_attempt(&mut h, &mut r).unwrap());
    assert!(b.cmds.queue.is_empty());
    assert_eq!(b.cmds.cmd_status & 1, 0);
    assert!(b.flee_ability(&mut h).unwrap());
    let mut r = no_draws();
    run(&mut b, &mut h, &mut r, 1);
    assert!(b.ability_flee);
    run(&mut b, &mut h, &mut r, 16);
    assert_eq!(b.result, Some(result::ESCAPE));
}

/// Fleeing is refused when the scene forbids it or no player can leave.
#[test]
fn flee_blocked() {
    let cfg = BattleConfig {
        runaway: false,
        ..BattleConfig::default()
    };
    let mut b = Battle::new(vec![unit(1, true, 40), unit(16, false, 0)], cfg, table());
    let mut h = hooks();
    assert!(!b.flee_ability(&mut h).unwrap());
    let mut p = unit(1, true, 40);
    p.status_current = st::SLEEP;
    let mut b = battle(vec![p, unit(16, false, 0)]);
    assert!(!b.flee_ability(&mut h).unwrap());
}

// ---------------------------------------------------------------------------
// Trance
// ---------------------------------------------------------------------------

/// Derivation: entering Trance queues SysTrans on slot 4 at once: dequeued on
/// tick 1, inspection 1, select 2, finish 3 (no decay, it is not a
/// declarable command). With trance 255, level 20 and will 20 a main command
/// decays by (300 - 20) / 20 * 10 = 140: the attack finishing on tick 83
/// leaves 115; the player's gauge restarts at 40 on tick 83 (the add follows
/// the reset) and is full on tick 162; the second attack is dequeued on 163
/// and finishes on 165, where 115 <= 140 removes Trance and queues a second
/// SysTrans (dequeued on 166, finished on 168). Each attack uses one hit
/// draw set. A player in Trance hits harder (`modifier_up`): attack 33 + (33
/// >> 1) = 49, 49 * 15 = 735, so 5000 -> 4265 (not 495).
#[test]
fn trance_entry_decay_and_exit() {
    let mut p = unit(1, true, 40);
    p.has_trance_gauge = true;
    let mut e = unit(16, false, 0);
    e.cur_hp = 5000;
    e.max_hp = 5000;
    let mut b = battle(vec![p, e]);
    let mut draws = Vec::new();
    draws.extend_from_slice(&HIT);
    draws.extend_from_slice(&HIT);
    let (mut h, mut r) = (hooks(), SequenceRandom::r16(&draws));
    let mut none = no_draws();
    assert_eq!(b.alter_status_on(0, 14, &mut none), AlterResult::Success);
    b.units[0].trance = 255;
    assert!(b.cmds.is_queued(b.slot(0, 4)));
    run(&mut b, &mut h, &mut r, 80);
    assert_eq!(
        first_tick(&b, |e| matches!(e, BattleEvent::CommandFinished { .. })),
        Some(3)
    );
    assert!(
        b.player_command(0, cmd::ATTACK, 176, 16, 0, &mut h)
            .unwrap()
    );
    run(&mut b, &mut h, &mut r, 3);
    assert_eq!(b.units[0].trance, 115);
    assert_eq!(b.units[1].cur_hp, 4265);
    assert_eq!(b.units[0].cur_atb, 40);
    run(&mut b, &mut h, &mut r, 79);
    assert_eq!(
        ticks_of(&b, |e| matches!(e, BattleEvent::PlayerReady { .. })),
        vec![80, 162]
    );
    assert!(
        b.player_command(0, cmd::ATTACK, 176, 16, 0, &mut h)
            .unwrap()
    );
    run(&mut b, &mut h, &mut r, 3);
    assert_eq!(
        first_tick(&b, |e| *e == BattleEvent::TranceEnded { unit: 0 }),
        Some(165)
    );
    assert_eq!(b.units[0].status_current & st::TRANCE, 0);
    assert_eq!(b.units[0].trance, 0);
    run(&mut b, &mut h, &mut r, 3);
    let finished = ticks_of(&b, |e| matches!(e, BattleEvent::CommandFinished { .. }));
    assert_eq!(finished, vec![3, 83, 165, 168]);
    assert!(r.is_exhausted());
}

// ---------------------------------------------------------------------------
// Status ticks
// ---------------------------------------------------------------------------

/// Derivation (will 20, max HP 1600, dex 40 so the status loop and ATB run
/// every tick): Poison has OprCnt 10 and ContiCnt 30 in `StatusData.csv`.
/// opr = 10 * (will << 2 = 80) = 800; conti = 30 * ((60 - 20) << 3 = 320) =
/// 9600. Each tick subtracts the ATB coefficient (10): opr is 0 after tick
/// 80, so the first damage lands on tick 81 (opr reset to 800), then every
/// 81 ticks (162, 243, ...). Damage is max HP >> 4 = 100. conti reaches -10
/// on tick 961 and Poison expires; the damage ticks up to then are
/// 81 * k <= 961, k = 1..=11.
#[test]
fn poison_ticks_and_expires() {
    let mut p = unit(1, true, 40);
    p.max_hp = 1600;
    p.cur_hp = 1600;
    let mut b = battle(vec![p, unit(2, true, 40), unit(16, false, 0)]);
    let mut none = no_draws();
    b.alter_status_on(0, 16, &mut none);
    assert_eq!((b.units[0].opr[&16], b.units[0].conti[&16]), (800, 9600));
    let (mut h, mut r) = (hooks(), no_draws());
    run(&mut b, &mut h, &mut r, 80);
    assert_eq!(b.units[0].cur_hp, 1600);
    run(&mut b, &mut h, &mut r, 1);
    assert_eq!(b.units[0].cur_hp, 1500);
    run(&mut b, &mut h, &mut r, 879);
    let ticks = ticks_of(&b, |e| {
        matches!(
            e,
            BattleEvent::StatusTick {
                unit: 0,
                status: 16
            }
        )
    });
    assert_eq!(ticks, (1..=11).map(|k| 81 * k).collect::<Vec<u32>>());
    assert_eq!(b.units[0].cur_hp, 500);
    assert_ne!(b.units[0].status_current & st::POISON, 0);
    run(&mut b, &mut h, &mut r, 1);
    assert_eq!(
        first_tick(&b, |e| *e
            == BattleEvent::StatusExpired {
                unit: 0,
                status: 16
            }),
        Some(961)
    );
    assert_eq!(b.units[0].status_current & st::POISON, 0);
}

/// Derivation: Regen (OprCnt 10, ContiCnt 45): opr = 10 * ((60 - 20) << 2 =
/// 160) = 1600; first heal on tick 161 then every 161 ticks (322), each
/// max HP >> 4 = 100, capped at max HP. A Slow unit counts down at coef 6:
/// 266 ticks leave 1600 - 1596 = 4, tick 267 leaves -2, so it heals on
/// tick 268.
#[test]
fn regen_ticks_and_slow_interval() {
    let mut fast = unit(1, true, 40);
    fast.max_hp = 1600;
    fast.cur_hp = 1000;
    let mut slow = unit(2, true, 40);
    slow.max_hp = 1600;
    slow.cur_hp = 1000;
    let mut b = battle(vec![fast, slow, unit(16, false, 0)]);
    b.sched[1].atb_coef = atb::slowed_coef(10);
    let mut none = no_draws();
    b.alter_status_on(0, 18, &mut none);
    b.alter_status_on(1, 18, &mut none);
    let (mut h, mut r) = (hooks(), no_draws());
    run(&mut b, &mut h, &mut r, 322);
    assert_eq!(
        ticks_of(&b, |e| matches!(
            e,
            BattleEvent::StatusTick {
                unit: 0,
                status: 18
            }
        )),
        vec![161, 322]
    );
    assert_eq!(b.units[0].cur_hp, 1200);
    assert_eq!(
        ticks_of(&b, |e| matches!(
            e,
            BattleEvent::StatusTick {
                unit: 1,
                status: 18
            }
        )),
        vec![268]
    );
    assert_eq!(b.units[1].cur_hp, 1100);
}

/// Derivation: Doom opr = 1.0 * ContiCnt 25 * ((60 - 20) << 3 = 320) / 10 =
/// 800 and the counter starts at 10. Ticks at 81 * k decrement it; the tenth,
/// tick 810, reaches 0 and applies Death.
#[test]
fn doom_kills_on_the_tenth_tick() {
    let mut b = battle(vec![
        unit(1, true, 40),
        unit(2, true, 40),
        unit(16, false, 0),
    ]);
    let mut none = no_draws();
    b.alter_status_on(0, 27, &mut none);
    assert_eq!((b.units[0].opr[&27], b.units[0].doom_counter), (800, 10));
    let (mut h, mut r) = (hooks(), no_draws());
    run(&mut b, &mut h, &mut r, 809);
    assert_eq!(b.units[0].status_current & st::DEATH, 0);
    run(&mut b, &mut h, &mut r, 1);
    assert_ne!(b.units[0].status_current & st::DEATH, 0);
    assert_eq!(
        first_tick(&b, |e| *e == BattleEvent::UnitDied { unit: 0 }),
        Some(810)
    );
}

// ---------------------------------------------------------------------------
// Scripted battles
// ---------------------------------------------------------------------------

/// Two players (dex 40) against one enemy (500 HP, dex 0, so it never acts
/// inside this window). Derivation, 1-based ticks:
///   80   both players ready (gauge 3200)
///   --   both Attacks queued after tick 80 (player 0 first)
///   81   attack 1 dequeued, inspection; 82 select-VFX; 83 effect: 495
///        damage (draws `HIT`), HP 500 -> 5; finish, gauge reset
///   84-86 attack 2: dequeued 84, effect on 86 takes HP to 0
///   87   `CheckPointData` at the top of the tick adds Death:
///        `CheckForecastMenuOff` stops the ATB and queued commands, die_seq
///        0 -> 1, and the same tick's status loop steps it 1 -> 2
///   88: 2 -> 3   89: 3 -> 4 (fade rate 32 -> 30)
///   90-104 fade rate 28 .. 0 (15 steps of -2)
///   105  die_seq 4 -> 5   106: 5 -> 6, `CheckBattlePhase` sets MenuOff/Victory
///   107  trailing loop: queue empty, enemy gone: result VICTORY, phase Victory
///   108  FadeOut -> Wait
///   109..142 `btl_fade_time` 0..33: `> 32` first true on tick 142 = 107 + 35
#[test]
fn two_vs_one_reaches_victory() {
    let mut b = battle(vec![
        unit(1, true, 40),
        unit(2, true, 40),
        unit(16, false, 0),
    ]);
    b.sched[2].bonus_exp = 120;
    let mut draws = Vec::new();
    draws.extend_from_slice(&HIT);
    draws.extend_from_slice(&HIT);
    let (mut h, mut r) = (hooks(), SequenceRandom::r16(&draws));
    run(&mut b, &mut h, &mut r, 80);
    assert_eq!(b.ready_players, vec![0, 1]);
    assert!(
        b.player_command(0, cmd::ATTACK, 176, 16, 0, &mut h)
            .unwrap()
    );
    assert!(
        b.player_command(1, cmd::ATTACK, 176, 16, 0, &mut h)
            .unwrap()
    );
    run(&mut b, &mut h, &mut r, 3);
    assert_eq!(b.units[2].cur_hp, 5, "tick 83");
    run(&mut b, &mut h, &mut r, 3);
    assert_eq!(b.units[2].cur_hp, 0, "tick 86");
    assert_eq!(b.units[2].status_current & st::DEATH, 0, "not yet, tick 86");
    assert!(b.menu_enabled);
    run(&mut b, &mut h, &mut r, 1);
    assert!(!b.menu_enabled, "tick 87");
    assert_eq!(
        first_tick(&b, |e| *e == BattleEvent::UnitDied { unit: 2 }),
        Some(87)
    );
    assert_eq!(b.sched[2].die_seq, 2, "tick 87");
    run(&mut b, &mut h, &mut r, 2);
    assert_eq!(
        (b.sched[2].die_seq, b.sched[2].die_fade_rate),
        (4, 30),
        "tick 89"
    );
    run(&mut b, &mut h, &mut r, 15);
    assert_eq!(
        (b.sched[2].die_seq, b.sched[2].die_fade_rate),
        (4, 0),
        "tick 104"
    );
    run(&mut b, &mut h, &mut r, 1);
    assert_eq!(b.sched[2].die_seq, 5, "tick 105");
    assert_eq!(b.phase, Phase::Normal);
    run(&mut b, &mut h, &mut r, 1);
    assert_eq!(b.sched[2].die_seq, 6, "tick 106");
    assert_eq!(
        (b.phase, b.menu_off_seq),
        (Phase::MenuOff, MenuOffSeq::Victory)
    );
    assert_eq!(b.bonus_exp, 120);
    assert_eq!(b.result, None);
    run(&mut b, &mut h, &mut r, 1);
    assert_eq!((b.result, b.phase), (Some(result::VICTORY), Phase::Victory));
    assert_eq!(
        first_tick(&b, |e| matches!(e, BattleEvent::Result { .. })),
        Some(107)
    );
    run(&mut b, &mut h, &mut r, 34);
    assert!(!b.over, "tick 141");
    run(&mut b, &mut h, &mut r, 1);
    assert!(b.over);
    assert_eq!(first_tick(&b, |e| *e == BattleEvent::BattleOver), Some(142));
    assert!(r.is_exhausted());
    // The attacks finished in the order they were queued.
    assert_eq!(
        ticks_of(&b, |e| matches!(e, BattleEvent::CommandFinished { .. })),
        vec![83, 86]
    );
}

/// Derivation: the only player is killed (HP 0, then Death) on tick 5. Its
/// `die_seq` goes to 1 at once and to 6 on the next tick (zero injected
/// wait), where `CheckBattlePhase` sets MenuOff / Defeat; the trailing loop
/// then sets result DEFEAT on the tick after. The defeat fade-out is not
/// ported, so the battle never reports `over`.
#[test]
fn defeat_when_all_players_die() {
    let mut b = battle(vec![unit(1, true, 40), unit(16, false, 0)]);
    let (mut h, mut r) = (hooks(), no_draws());
    run(&mut b, &mut h, &mut r, 5);
    b.units[0].cur_hp = 0;
    let mut none = no_draws();
    b.alter_status_on(0, 8, &mut none);
    assert_ne!(b.units[0].status_current & st::DEATH, 0);
    run(&mut b, &mut h, &mut r, 1);
    assert_eq!(b.sched[0].die_seq, 6);
    assert_eq!(
        (b.phase, b.menu_off_seq),
        (Phase::MenuOff, MenuOffSeq::Defeat)
    );
    run(&mut b, &mut h, &mut r, 1);
    assert_eq!((b.result, b.phase), (Some(result::DEFEAT), Phase::Defeat));
    run(&mut b, &mut h, &mut r, 100);
    assert!(!b.over);
}

/// An enemy whose gauge fills asks the hook for an action: it is queued at
/// once and the unit waits (`sel_mode 1`). The enemy (dex 0, max 9600) fills
/// on tick 240, the command is dequeued on 241, and the player is hit on 243.
/// The player (dex 0 as well) is also full on tick 240 and waits in the ready
/// list.
#[test]
fn enemy_action_through_hook() {
    let mut p = unit(1, true, 0);
    p.cur_hp = 5000;
    p.max_hp = 5000;
    let mut b = battle(vec![p, unit(16, false, 0)]);
    let mut h = Hooks {
        enemy: Some(EnemyAction {
            cmd_no: cmd::ENEMY_ATK,
            sub_no: 0,
            tar_id: 1,
        }),
    };
    let mut r = SequenceRandom::r16(&HIT);
    run(&mut b, &mut h, &mut r, 243);
    assert_eq!(
        first_tick(&b, |e| *e == BattleEvent::AtbFull { unit: 1 }),
        Some(240)
    );
    assert_eq!(
        first_tick(&b, |e| matches!(
            e,
            BattleEvent::CommandStarted { unit: 1, .. }
        )),
        Some(241)
    );
    assert_eq!(
        first_tick(&b, |e| matches!(
            e,
            BattleEvent::EffectApplied {
                caster: 1,
                target: 0,
                ..
            }
        )),
        Some(243)
    );
    assert_eq!(b.ready_players, vec![0]);
}

#[test]
fn a_battle_speed_set_after_construction_matches_one_built_with_it() {
    let units = || vec![unit(1, true, 40), unit(16, false, 0)];
    let mut late = battle(units());
    late.set_btl_speed(2);
    let built = Battle::new(
        units(),
        BattleConfig {
            btl_speed_config: 2,
            ..BattleConfig::default()
        },
        table(),
    );
    let coefs = |b: &Battle| b.sched.iter().map(|s| s.atb_coef).collect::<Vec<_>>();
    assert_eq!(coefs(&late), coefs(&built));
    assert_eq!(late.config.btl_speed_config, 2);
}
