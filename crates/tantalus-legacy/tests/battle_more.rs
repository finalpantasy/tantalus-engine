#![allow(clippy::doc_lazy_continuation)]
//! Magic Sword, Protect Girls, Auto-Potion, Throw, the remaining item effect
//! scripts and the enemy counter and dying requests on the B5 machine
//! (`tantalus_legacy::battle::reactions`, `scripts`, `machine`).
//!
//! Every expectation is derived by hand from the pinned Memoria source
//! (c716e1fa, `Assembly-CSharp/` and `Memoria.Scripts/Sources/Battle/`); each
//! test names the function it pins. `Timing` stays 0, so a command dequeued on
//! tick T inspects on T, runs the select-VFX step on T + 1 and applies its
//! effect on T + 2. The rig mirrors `battle_reactions.rs`: script 25 sets the
//! target HP to the command power and marks who was hit; `Rng` returns 0 for
//! `random16`, so a script's caster draw adds nothing unless a test queues one.

use std::collections::VecDeque;

use tantalus_legacy::battle::command::ItemData;
use tantalus_legacy::battle::machine::{
    Battle, BattleConfig, BattleEvent, BattleHooks, EnemyAction, EnemyReactionKind,
    EnemyReactionRequest,
};
use tantalus_legacy::battle::queue::{CommandSpec, cmd};
use tantalus_legacy::battle::reactions::{
    ITEM_HI_POTION, ITEM_POTION, MAGIC_SWORD_BENEFICIARY_BLOCKING, MAGIC_SWORD_SUPPORTER_BLOCKING,
    MagicSwordSet, SA_AUTO_POTION, SA_COUNTER, SA_PROTECT_GIRLS, ability_id_from_active,
};
use tantalus_legacy::battle::rng::BattleRandom;
use tantalus_legacy::battle::status as st;
use tantalus_legacy::battle::targeting::target_type;
use tantalus_legacy::battle::unit::{CATEGORY_FEMALE, default_player_category};
use tantalus_legacy::battle::{BattleCommand, BattleUnit, ItemEffect, StatusDataRow, StatusTable};

// ---------------------------------------------------------------------------
// Rig
// ---------------------------------------------------------------------------

const P1: u16 = 1;
const P2: u16 = 2;
const P3: u16 = 4;
const E1: u16 = 16;
const E2: u16 = 32;

/// Scripted `random8`, `random16` and `random_range(0, 100)` draws; every
/// other draw returns 0 or the range minimum.
#[derive(Default)]
struct Rng {
    r8: VecDeque<i32>,
    r16: VecDeque<i32>,
    counter_roll: VecDeque<i32>,
}

impl Rng {
    fn r8(values: &[i32]) -> Self {
        Self {
            r8: values.iter().copied().collect(),
            ..Self::default()
        }
    }
    fn r16(values: &[i32]) -> Self {
        Self {
            r16: values.iter().copied().collect(),
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
        self.r16.pop_front().unwrap_or(0)
    }
    fn random8(&mut self) -> i32 {
        self.r8.pop_front().unwrap_or(0)
    }
    fn random_range(&mut self, min: i32, max: i32) -> i32 {
        if (min, max) == (0, 100) {
            return self.counter_roll.pop_front().unwrap_or(0);
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

fn item_effect(power: i32, hit_rate: i32, status: u64) -> ItemEffect {
    ItemEffect {
        power,
        hit_rate,
        status,
        ..ItemEffect::default()
    }
}

/// Catalog stand-in.
///
/// * `EnemyAtk` sub 0: script 25 power 7, category 8 (physical).
/// * `Attack` 176: script 25 power 5, category 8; 177: power 0 (kills);
///   200: script 10 power 10, category 16, single ally.
/// * `Counter` 176: script 25 power 3, category 8.
/// * `MagicSword` 51 and 52: script 25 power 9, category 16.
/// * `Item` and `AutoPotion`: the `Void` row, script and effect from `items`.
/// * `Throw`: script 42, category 8, single enemy.
/// * `Jump` 178: no script; `Spear` 185: script 25 power 6.
/// * `EnemyCounter` and `EnemyDying` sub 0: script 25 power 11, category 8.
struct Hooks {
    items: Vec<(i32, ItemData)>,
    weapon_power: Vec<(i32, i32)>,
    requests: Vec<EnemyReactionRequest>,
    reaction_cmds: Vec<(u16, EnemyAction)>,
}

impl Hooks {
    fn new() -> Self {
        Self {
            items: Vec::new(),
            weapon_power: Vec::new(),
            requests: Vec::new(),
            reaction_cmds: Vec::new(),
        }
    }
}

impl BattleHooks for Hooks {
    fn spec(&mut self, cmd_no: u32, sub: i32, _c: &BattleUnit) -> Option<CommandSpec> {
        let single = target_type::SINGLE_ENEMY;
        match (cmd_no, sub) {
            (cmd::ENEMY_ATK, 0) => Some(spell(25, 7, 8, single)),
            (cmd::ATTACK, 176) => Some(spell(25, 5, 8, single)),
            (cmd::ATTACK, 177) => Some(spell(25, 0, 8, single)),
            (cmd::ATTACK, 200) => Some(spell(10, 10, 16, target_type::SINGLE_ALLY)),
            (cmd::COUNTER, 176) => Some(spell(25, 3, 8, single)),
            (cmd::MAGIC_SWORD, 51 | 52) => Some(spell(25, 9, 16, single)),
            (cmd::ITEM | cmd::AUTO_POTION, _) => {
                let mut s = spell(0, 0, 0, target_type::SINGLE_ALLY);
                s.default_ally = true;
                Some(s)
            }
            (cmd::THROW, _) => Some(spell(42, 0, 8, single)),
            (cmd::JUMP, 178) => Some(spell(0, 0, 8, single)),
            (cmd::SPEAR, 185) => Some(spell(25, 6, 8, single)),
            (cmd::ENEMY_COUNTER | cmd::ENEMY_DYING, 0) => {
                Some(spell(25, 11, 8, target_type::SINGLE_ENEMY))
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

    fn item_weapon_power(&mut self, item_id: i32) -> Option<i32> {
        self.weapon_power
            .iter()
            .find(|(i, _)| *i == item_id)
            .map(|(_, p)| *p)
    }

    fn enemy_reaction(&mut self, _battle: &Battle, request: EnemyReactionRequest) {
        self.requests.push(request);
    }

    fn enemy_reaction_commands(&mut self, _battle: &Battle) -> Vec<(u16, EnemyAction)> {
        std::mem::take(&mut self.reaction_cmds)
    }
}

fn battle(units: Vec<BattleUnit>) -> Battle {
    Battle::new(units, BattleConfig::default(), table())
}

fn enemy_cast(b: &mut Battle, h: &mut Hooks, caster: usize, sub: i32, tar: u16) {
    let action = EnemyAction {
        cmd_no: cmd::ENEMY_ATK,
        sub_no: sub,
        tar_id: tar,
    };
    b.set_enemy_command(caster, action, h)
        .expect("enemy command");
}

fn run_idle(b: &mut Battle, h: &mut Hooks, rng: &mut Rng) {
    for _ in 0..60 {
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

fn queued(b: &Battle, cmd_no: u32) -> Vec<(usize, u16)> {
    events(b)
        .into_iter()
        .filter_map(|e| match e {
            BattleEvent::CommandQueued {
                unit,
                cmd_no: c,
                tar_id,
            } if c == cmd_no => Some((unit, tar_id)),
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
// Data helpers
// ---------------------------------------------------------------------------

/// `CharacterParameters.csv` `DefaultCategory` column (`CharacterParameters.csv`
/// rows 0 to 11): Zidane 9, Vivi 5, Garnet 6, Steiner 5, Freya 6, Quina 5,
/// Eiko 6, Amarant 5, Cinna 21, Marcus 21, Blank 21, Beatrix 22. Female is bit
/// 2 (`CharacterCategory.cs`: Male 1, Female 2), so Garnet, Freya, Eiko and
/// Beatrix (22 = 16 + 4 + 2) are female and the others are not.
#[test]
fn default_categories_mark_exactly_the_female_characters() {
    let female: Vec<u8> = (0..12)
        .filter(|&c| default_player_category(c).unwrap() & CATEGORY_FEMALE != 0)
        .collect();
    assert_eq!(female, vec![2, 4, 6, 11]);
    assert_eq!(default_player_category(0), Some(9));
    assert_eq!(default_player_category(11), Some(22));
    assert_eq!(default_player_category(12), None);
}

/// `ff9abil.GetAbilityIdFromActiveAbility` (`ff9abil.cs:346`):
/// `(a / 192) * 256 + a % 192`. Active ids below 192 map to themselves; 192 and
/// above move to the 256 block: 192 -> 256, 197 -> 261, 200 -> 264.
#[test]
fn ability_id_from_active_splits_at_192() {
    assert_eq!(ability_id_from_active(51), 51);
    assert_eq!(ability_id_from_active(191), 191);
    assert_eq!(ability_id_from_active(192), 256);
    assert_eq!(ability_id_from_active(197), 261);
    assert_eq!(ability_id_from_active(200), 264);
}

// ---------------------------------------------------------------------------
// Magic Sword (`btl_cmd.SetCommand`, `DecideMagicSword`, `IsBtlBusy`)
// ---------------------------------------------------------------------------

fn sword_set(unlocked: Vec<i32>) -> MagicSwordSet {
    MagicSwordSet {
        supporter: 1,
        beneficiary: 0,
        unlocked_abilities: unlocked,
        supporter_blocking: MAGIC_SWORD_SUPPORTER_BLOCKING,
        beneficiary_blocking: MAGIC_SWORD_BENEFICIARY_BLOCKING,
    }
}

fn sword_battle(supporter: BattleUnit, beneficiary: BattleUnit, unlocked: Vec<i32>) -> Battle {
    let mut b = battle(vec![beneficiary, supporter, unit(E1, false, 500)]);
    b.magic_swords.push(sword_set(unlocked));
    b
}

/// Derivation (`btl_cmd.SetCommand`, `btl_cmd.cs:262-266`): the command takes
/// `magic_caster_id` = the supporter's id only when the caster is the
/// beneficiary and `GetAbilityIdFromActiveAbility(aaIndex)` is in the set's
/// unlocked list. Beneficiary P1 (index 0) with set {51} and supporter P2:
/// sub 51 -> P2's id (2); sub 52 (locked) -> 0. The supporter casting the
/// same sub 51 has no set as a beneficiary -> 0. Throw uses `aaIndex` 190
/// (`GetCommandMainActionIndex`, `btl_util.cs:359-393`), so unlocking 190
/// carries a Throw, and Item maps to ability 0.
#[test]
fn magic_caster_id_is_set_only_for_the_beneficiary_and_an_unlocked_ability() {
    let mut b = sword_battle(unit(P2, true, 500), unit(P1, true, 500), vec![51, 190, 0]);
    let mut h = Hooks::new();
    h.items.push((7, ItemData::default()));
    let slot = b.cmds.slot_of(0, 0);
    b.set_command(slot, cmd::MAGIC_SWORD, 51, E1, 0, false, &mut h)
        .unwrap();
    assert_eq!(b.cmds.slots[slot].magic_caster_id, P2);

    let mut b = sword_battle(unit(P2, true, 500), unit(P1, true, 500), vec![51]);
    let slot = b.cmds.slot_of(0, 0);
    b.set_command(slot, cmd::MAGIC_SWORD, 52, E1, 0, false, &mut h)
        .unwrap();
    assert_eq!(b.cmds.slots[slot].magic_caster_id, 0);

    // The supporter (index 1) is not the beneficiary.
    let slot = b.cmds.slot_of(1, 0);
    b.set_command(slot, cmd::MAGIC_SWORD, 51, E1, 0, false, &mut h)
        .unwrap();
    assert_eq!(b.cmds.slots[slot].magic_caster_id, 0);

    // Throw -> ability 190; Item -> ability 0.
    let mut b = sword_battle(unit(P2, true, 500), unit(P1, true, 500), vec![190]);
    let slot = b.cmds.slot_of(0, 0);
    h.weapon_power.push((7, 10));
    b.set_command(slot, cmd::THROW, 7, E1, 0, false, &mut h)
        .unwrap();
    assert_eq!(b.cmds.slots[slot].magic_caster_id, P2);
    let mut b = sword_battle(unit(P2, true, 500), unit(P1, true, 500), vec![190]);
    let slot = b.cmds.slot_of(0, 0);
    b.set_command(slot, cmd::ITEM, 7, P3, 0, false, &mut h)
        .unwrap();
    assert_eq!(b.cmds.slots[slot].magic_caster_id, 0);
}

/// Derivation (`btl_cmd.DecideMagicSword`, `btl_cmd.cs:1629-1642`; blockers
/// `BattleMagicSwordSet.cs:35-39`): the command is refused with
/// `CombinationFailed` when the caster is under `Immobilized` (Death, Petrify,
/// Stop, Sleep and the like) or a beneficiary blocker (Sleep, Mini), or when the
/// supporter is under `Immobilized` or a supporter blocker (Silence, Confuse,
/// Berserk, Sleep, Heat, Mini). Poison blocks neither side and Silence on the
/// beneficiary is not a blocker (the sword spell has no Silence category bit).
/// An accepted command runs script 25 and sets E1's HP to the power 9.
#[test]
fn decide_magic_sword_rejects_blocked_supporters_and_beneficiaries() {
    let run = |supporter: u64, beneficiary: u64| -> (bool, u32) {
        let mut b = sword_battle(
            with_status(unit(P2, true, 500), supporter),
            with_status(unit(P1, true, 500), beneficiary),
            vec![51],
        );
        let (mut h, mut r) = (Hooks::new(), Rng::default());
        assert!(
            b.player_command(0, cmd::MAGIC_SWORD, 51, E1, 0, &mut h)
                .unwrap()
        );
        run_idle(&mut b, &mut h, &mut r);
        let failed = events(&b)
            .iter()
            .any(|e| matches!(e, BattleEvent::CombinationFailed { unit: 0 }));
        (failed, b.units[2].cur_hp)
    };
    // Accepted.
    assert_eq!(run(0, 0), (false, 9));
    assert_eq!(run(st::POISON, 0), (false, 9));
    assert_eq!(run(0, st::POISON), (false, 9));
    assert_eq!(run(0, st::SILENCE), (false, 9));
    // Supporter blocked.
    for status in [
        st::SILENCE,
        st::CONFUSE,
        st::BERSERK,
        st::SLEEP,
        st::HEAT,
        st::MINI,
        st::STOP,
        st::DEATH,
    ] {
        assert_eq!(run(status, 0), (true, 500), "supporter status {status:#x}");
    }
    // Beneficiary blocked.
    for status in [st::SLEEP, st::MINI] {
        assert_eq!(
            run(0, status),
            (true, 500),
            "beneficiary status {status:#x}"
        );
    }
}

/// Derivation (`btl_abil.CheckCoverAbility`, `btl_abil.cs:191-218`;
/// `btl_util.IsBtlBusy(CASTER | MAGIC_CASTER)`, `btl_util.cs:178-188`): a
/// unit named by a running command's `magic_caster_id` is busy and cannot cover.
/// Cover candidates P2 (HP 300) and P3 (HP 200) for target P1: the strongest
/// defender is P2. With a running command whose `magic_caster_id` is P2 (and
/// whose caster is P1), P2 is skipped and P3 covers. A running command whose
/// caster is P3 skips P3 the same way.
#[test]
fn a_magic_sword_supporter_is_busy_for_cover() {
    let make = || {
        battle(vec![
            unit(P1, true, 100),
            unit(P2, true, 300),
            unit(P3, true, 200),
            unit(E1, false, 500),
        ])
    };
    let mut b = make();
    assert_eq!(b.check_cover_ability(0, P2 | P3), P2);

    let mut b = make();
    let id = b.cmds.slot_of(0, 0);
    b.cmds.slots[id].regist = Some(0);
    b.cmds.slots[id].magic_caster_id = P2;
    b.running.push(id);
    assert_eq!(b.check_cover_ability(0, P2 | P3), P3);

    let mut b = make();
    let id = b.cmds.slot_of(2, 0);
    b.cmds.slots[id].regist = Some(2);
    b.running.push(id);
    assert_eq!(b.check_cover_ability(0, P2 | P3), P2);
    let mut b = make();
    let id = b.cmds.slot_of(1, 0);
    b.cmds.slots[id].regist = Some(1);
    b.running.push(id);
    assert_eq!(b.check_cover_ability(0, P2 | P3), P3);
}

// ---------------------------------------------------------------------------
// Protect Girls (SA 38, `CharacterAbilityGems.TriggerOnCommand`)
// ---------------------------------------------------------------------------

fn girl(id: u16, hp: u32, category: u8) -> BattleUnit {
    let mut u = unit(id, true, hp);
    u.player_category = category;
    u
}

/// Derivation (`AbilityFeatures.txt` SA 38 `TryCover`: ally of the cover unit,
/// target alive, `HasCategory(Female)`, `HP < MaxHP / 2`; condition shared with
/// Cover: opposite-side physical `EnemyAtk` at one target, cover unit not Heat
/// and not Immobilized). P1 is female (category 6) with HP 400 of max 1000, so
/// `400 < 500`: P2 (SA 38) takes the hit. HP 500 -> `500 < 500` false, no
/// cover. A male (category 9) at HP 400 is not covered, and neither is a
/// female whose protector is under Heat. Cover (SA 37) is not involved: P1 is
/// not under Low HP.
#[test]
fn protect_girls_covers_a_wounded_female_ally_only() {
    let run = |hp: u32, category: u8, protector: u64| -> (Vec<(u16, u16)>, u32, u32) {
        let mut b = battle(vec![
            girl(P1, hp, category),
            with_status(with_sa(unit(P2, true, 300), SA_PROTECT_GIRLS), protector),
            unit(E1, false, 500),
        ]);
        let (mut h, mut r) = (Hooks::new(), Rng::default());
        enemy_cast(&mut b, &mut h, 2, 0, P1);
        run_idle(&mut b, &mut h, &mut r);
        (covered(&b), b.units[0].cur_hp, b.units[1].cur_hp)
    };
    assert_eq!(run(400, 6, 0), (vec![(P1, P2)], 400, 7));
    assert_eq!(run(499, 22, 0), (vec![(P1, P2)], 499, 7));
    assert_eq!(run(500, 6, 0), (vec![], 7, 300));
    assert_eq!(run(400, 9, 0), (vec![], 7, 300));
    assert_eq!(run(400, 6, st::HEAT), (vec![], 7, 300));
}

// ---------------------------------------------------------------------------
// Item effect scripts
// ---------------------------------------------------------------------------

/// P1 uses item 7 (copies `count`) on `target`; returns the battle.
#[allow(clippy::too_many_arguments)]
fn use_item(
    script: i32,
    effect: ItemEffect,
    for_dead: bool,
    target: BattleUnit,
    tar: u16,
    count: u32,
    caster: BattleUnit,
    rng: &mut Rng,
) -> Battle {
    let mut b = battle(vec![caster, target, unit(E1, false, 500)]);
    let mut h = Hooks::new();
    h.items.push((
        7,
        ItemData {
            effect,
            script_id: script,
            for_dead,
        },
    ));
    b.inventory.insert(7, count);
    assert!(b.player_command(0, cmd::ITEM, 7, tar, 0, &mut h).unwrap());
    run_idle(&mut b, &mut h, rng);
    b
}

fn hurt(id: u16, hp: u32) -> BattleUnit {
    unit(id, true, hp)
}

/// Derivation (`0062_ItemSoftScript.cs:19`): on a target without the Stone
/// category the item's statuses are removed (`TryRemoveItemStatuses`); a
/// Stone-category enemy is killed instead. P2 under Petrify and an item status
/// of Petrify ends clean; an enemy flagged `is_stone` with HP 500 ends at 0
/// with the death flag set (`Kill` leaves the Death status to `CheckPointData`).
#[test]
fn script_62_soft_cures_petrify_and_kills_stone_enemies() {
    let soft = item_effect(0, 255, st::PETRIFY);
    let b = use_item(
        62,
        soft,
        false,
        with_status(hurt(P2, 100), st::PETRIFY),
        P2,
        1,
        unit(P1, true, 500),
        &mut Rng::default(),
    );
    assert_eq!(b.units[1].status_current & st::PETRIFY, 0);

    let mut stone = unit(E1, false, 500);
    stone.is_stone = true;
    let mut b = battle(vec![unit(P1, true, 500), hurt(P2, 100), stone]);
    let mut h = Hooks::new();
    h.items.push((
        7,
        ItemData {
            effect: soft,
            script_id: 62,
            for_dead: false,
        },
    ));
    b.inventory.insert(7, 1);
    let mut r = Rng::default();
    assert!(b.player_command(0, cmd::ITEM, 7, E1, 0, &mut h).unwrap());
    run_idle(&mut b, &mut h, &mut r);
    assert_eq!(b.units[2].cur_hp, 0);
    assert!(b.units[2].death_flag);
}

/// Derivation (`0070_ItemEtherScript.cs:20`): `Attack = 15`,
/// `AttackPower = item power`, `DefensePower = 0`, `CalcMpMagicRecovery`:
/// `MpDamage = AttackPower * Attack`. Power 2 -> 30; P2 MP 10 -> 40. With the
/// caster's Chemist (SA 27, script 70), `Attack = 2 * Attack` first (SA text
/// `AbilityFeatures.txt` Chemist): 2 * 2 * 15 = 60 -> MP 70.
#[test]
fn script_70_ether_restores_mp_and_chemist_doubles_it() {
    let ether = item_effect(2, 255, 0);
    let mut target = hurt(P2, 100);
    target.cur_mp = 10;
    let b = use_item(
        70,
        ether,
        false,
        target.clone(),
        P2,
        1,
        unit(P1, true, 500),
        &mut Rng::default(),
    );
    assert_eq!(b.units[1].cur_mp, 40);
    let b = use_item(
        70,
        ether,
        false,
        target,
        P2,
        1,
        with_sa(unit(P1, true, 500), 27),
        &mut Rng::default(),
    );
    assert_eq!(b.units[1].cur_mp, 70);
}

/// Derivation (`0071_ItemElixirScript.cs:19`): a living non-zombie target gets
/// `HP = MaxHP` and `MP = MaxMP`; a Zombie target loses its MP and dies.
/// P2: HP 100 of 1000, MP 10 of 100 -> 1000 and 100. Zombie P2 -> HP 0, MP 0 and the death flag.
#[test]
fn script_71_elixir_fills_hp_and_mp_but_kills_zombies() {
    let mut target = hurt(P2, 100);
    target.cur_mp = 10;
    let b = use_item(
        71,
        item_effect(0, 255, 0),
        false,
        target.clone(),
        P2,
        1,
        unit(P1, true, 500),
        &mut Rng::default(),
    );
    assert_eq!((b.units[1].cur_hp, b.units[1].cur_mp), (1000, 100));
    let b = use_item(
        71,
        item_effect(0, 255, 0),
        false,
        with_status(target, st::ZOMBIE),
        P2,
        1,
        unit(P1, true, 500),
        &mut Rng::default(),
    );
    assert_eq!((b.units[1].cur_hp, b.units[1].cur_mp), (0, 0));
    assert!(b.units[1].death_flag);
}

/// Derivation (`0072_ItemPhoenixScript.cs:19`): a dead player (item status
/// Death, `ForDead`) is revived with `1 + Next8() % 10` HP: draw 27 -> 1 + 7
/// = 8. A living Zombie that is not Easy-Kill-guarded gets `Next8() % 10` HP:
/// draw 13 -> 3; draw 20 -> 0, which then dies.
#[test]
fn script_72_phoenix_revives_and_zombies_roll_their_hp() {
    let phoenix = item_effect(0, 255, st::DEATH);
    let dead = with_status(hurt(P2, 0), st::DEATH);
    let b = use_item(
        72,
        phoenix,
        true,
        dead,
        P2,
        1,
        unit(P1, true, 500),
        &mut Rng::r8(&[27]),
    );
    assert_eq!(b.units[1].cur_hp, 8);
    assert_eq!(b.units[1].status_current & st::DEATH, 0);

    let zombie = with_status(hurt(P2, 200), st::ZOMBIE);
    let b = use_item(
        72,
        phoenix,
        false,
        zombie.clone(),
        P2,
        1,
        unit(P1, true, 500),
        &mut Rng::r8(&[13]),
    );
    assert_eq!(b.units[1].cur_hp, 3);
    let b = use_item(
        72,
        phoenix,
        false,
        zombie,
        P2,
        1,
        unit(P1, true, 500),
        &mut Rng::r8(&[20]),
    );
    assert_eq!(b.units[1].cur_hp, 0);
    assert!(b.units[1].death_flag);
}

/// Derivation (`0074_ItemGemScript.cs:19`): `HpAlteration | HpRecovery` and
/// `HpDamage = Power * (count + 1)`, where the count is read when the effect
/// applies, after the `CMD_MODE_SELECT_VFX` step removed one copy. Power 10 with
/// 3 copies: 2 left -> 10 * 3 = 30; P2 HP 100 -> 130.
#[test]
fn script_74_gem_heals_power_times_remaining_plus_one() {
    let b = use_item(
        74,
        item_effect(10, 255, 0),
        false,
        hurt(P2, 100),
        P2,
        3,
        unit(P1, true, 500),
        &mut Rng::default(),
    );
    assert_eq!(b.inventory[&7], 2);
    assert_eq!(b.units[1].cur_hp, 130);
}

/// Derivation (`0075_DeadPepperScript.cs:19`): `HpAlteration` only and the same
/// `Power * (count + 1)`. Power 100 with 2 copies: 1 left -> 100 * 2 = 200
/// damage; P2 HP 500 -> 300.
#[test]
fn script_75_dead_pepper_damages_power_times_remaining_plus_one() {
    let b = use_item(
        75,
        item_effect(100, 255, 0),
        false,
        hurt(P2, 500),
        P2,
        2,
        unit(P1, true, 500),
        &mut Rng::default(),
    );
    assert_eq!(b.units[1].cur_hp, 300);
}

/// Derivation (`0076_TentScript.cs:19`): HP and MP recover by half of the
/// maximum (`MaxHP >> 1`, `MaxMP >> 1`); the item's statuses are added when
/// `hit_rate < Next16() % 100`. P2 HP 100 of 1000, MP 10 of 100 -> 600 and 60.
/// The calculator's constructor first spends one `random16` on the trance
/// increase (`BattleCalculator.cs:195`), so the queue is [0, draw]. Item rate 50
/// and draw 199 (199 % 100 = 99): 50 < 99, so Protect is added.
/// Draw 30: 50 < 30 is false, no status.
#[test]
fn script_76_tent_recovers_half_and_rolls_the_status() {
    let tent = item_effect(0, 50, st::PROTECT);
    let mut target = hurt(P2, 100);
    target.cur_mp = 10;
    let b = use_item(
        76,
        tent,
        false,
        target.clone(),
        P2,
        1,
        unit(P1, true, 500),
        &mut Rng::r16(&[0, 199]),
    );
    assert_eq!((b.units[1].cur_hp, b.units[1].cur_mp), (600, 60));
    assert_ne!(b.units[1].status_current & st::PROTECT, 0);
    let b = use_item(
        76,
        tent,
        false,
        target,
        P2,
        1,
        unit(P1, true, 500),
        &mut Rng::r16(&[0, 30]),
    );
    assert_eq!((b.units[1].cur_hp, b.units[1].cur_mp), (600, 60));
    assert_eq!(b.units[1].status_current & st::PROTECT, 0);
}

/// Derivation (`0077_DarkMatterScript.cs:18`): a target that can be attacked
/// takes 9999 HP damage. P2 HP 500 -> 0 (the Death status itself is added later by
/// `CheckPointData`); a Petrified P2 is untouched.
#[test]
fn script_77_dark_matter_does_9999_damage() {
    let b = use_item(
        77,
        item_effect(0, 255, 0),
        false,
        hurt(P2, 500),
        P2,
        1,
        unit(P1, true, 500),
        &mut Rng::default(),
    );
    assert_eq!(b.units[1].cur_hp, 0);

    let b = use_item(
        77,
        item_effect(0, 255, 0),
        false,
        with_status(hurt(P2, 500), st::PETRIFY),
        P2,
        1,
        unit(P1, true, 500),
        &mut Rng::default(),
    );
    assert_eq!(b.units[1].cur_hp, 500);
}

/// Derivation (`0069_ItemPotionScript.cs`, Chemist `AbilityFeatures.txt`
/// SA 27): `Attack = 15` doubled by Chemist for script 69, so
/// `HpDamage = 10 * 30 = 300`: P2 HP 100 -> 400 (plain: 10 * 15 = 150 -> 250).
#[test]
fn chemist_doubles_the_potion_script() {
    let potion = item_effect(10, 255, 0);
    let b = use_item(
        69,
        potion,
        false,
        hurt(P2, 100),
        P2,
        1,
        unit(P1, true, 500),
        &mut Rng::default(),
    );
    assert_eq!(b.units[1].cur_hp, 250);
    let b = use_item(
        69,
        potion,
        false,
        hurt(P2, 100),
        P2,
        1,
        with_sa(unit(P1, true, 500), 27),
        &mut Rng::default(),
    );
    assert_eq!(b.units[1].cur_hp, 400);
}

/// Derivation (`0010_MagicRecoveryScript.cs`, Concentrate `AbilityFeatures.txt`
/// SA 33): script 10 with Magic 30, draw 0 gives `Attack = 30`, power 10 ->
/// `HpDamage = 10 * 30 = 300`: P2 HP 100 -> 400. SA 33 adds one
/// `DamageModifierCount` before the figure: `Attack = 30 * 3 >> 1 = 45`,
/// 10 * 45 = 450 -> 550.
#[test]
fn concentrate_raises_the_cure_script() {
    let run = |sa: bool| {
        let caster = if sa {
            with_sa(unit(P1, true, 500), 33)
        } else {
            unit(P1, true, 500)
        };
        let mut b = battle(vec![caster, hurt(P2, 100), unit(E1, false, 500)]);
        let (mut h, mut r) = (Hooks::new(), Rng::default());
        assert!(
            b.player_command(0, cmd::ATTACK, 200, P2, 0, &mut h)
                .unwrap()
        );
        run_idle(&mut b, &mut h, &mut r);
        b.units[1].cur_hp
    };
    assert_eq!(run(false), 400);
    assert_eq!(run(true), 550);
}

// ---------------------------------------------------------------------------
// Throw (`0042_ThrowScript.cs`)
// ---------------------------------------------------------------------------

fn throw_run(caster: BattleUnit, mut enemy: BattleUnit, power: Option<i32>) -> (Battle, Hooks) {
    enemy.physical_defence = 5;
    let mut b = battle(vec![caster, enemy]);
    let mut h = Hooks::new();
    if let Some(p) = power {
        h.weapon_power.push((9, p));
    }
    b.inventory.insert(9, 2);
    let mut r = Rng::default();
    assert!(b.player_command(0, cmd::THROW, 9, E1, 0, &mut h).unwrap());
    run_idle(&mut b, &mut h, &mut r);
    (b, h)
}

/// Derivation (`0042_ThrowScript.cs:19`): `SetLowPhysicalAttack`:
/// `Attack = Strength + Next16 % (1 + ((Level + Strength) >> 3))` = 30 with
/// draw 0 (Level 20, Strength 30); `AttackPower = Weapon.Power << 1` = 20;
/// `DefensePower = 5`; `HpDamage = Attack * (AttackPower - DefensePower)` =
/// 30 * 15 = 450. E1 HP 2000 -> 1550; the thrown item loses one copy (Throw is
/// an item command: `GetCommandItem`).
#[test]
fn throw_uses_the_weapon_power_and_consumes_the_item() {
    let (b, _) = throw_run(unit(P1, true, 500), unit(E1, false, 2000), Some(10));
    assert_eq!(b.units[1].cur_hp, 1550);
    assert_eq!(b.inventory[&9], 1);
    assert!(events(&b).contains(&BattleEvent::ItemUsed {
        unit: 0,
        item: 9,
        remaining: 1,
    }));
}

/// Derivation: Power Throw (SA 28, `AbilityFeatures.txt` script 42) adds one
/// `DamageModifierCount`: `Attack = 30 * 3 >> 1 = 45`, 45 * 15 = 675 ->
/// E1 HP 1325. A flying enemy adds the same single step (`HasCategory(Flight)`
/// -> `ModifierUp`): also 675. Both together: `45 * 3 >> 1 = 67`, 67 * 15 =
/// 1005 -> 995. A Protected target steps down once instead: `30 >> 1 = 15`,
/// 15 * 15 = 225 -> 1775.
#[test]
fn throw_modifiers_follow_power_throw_flight_and_protect() {
    let enemy = || unit(E1, false, 2000);
    let (b, _) = throw_run(with_sa(unit(P1, true, 500), 28), enemy(), Some(10));
    assert_eq!(b.units[1].cur_hp, 1325);
    let mut flier = enemy();
    flier.is_flight = true;
    let (b, _) = throw_run(unit(P1, true, 500), flier.clone(), Some(10));
    assert_eq!(b.units[1].cur_hp, 1325);
    let (b, _) = throw_run(with_sa(unit(P1, true, 500), 28), flier, Some(10));
    assert_eq!(b.units[1].cur_hp, 995);
    let (b, _) = throw_run(
        unit(P1, true, 500),
        with_status(enemy(), st::PROTECT),
        Some(10),
    );
    assert_eq!(b.units[1].cur_hp, 1775);
}

/// Derivation (`CheckCommandCondition`, `btl_cmd.cs:1001-1007`): an empty
/// stack rejects the Throw and nothing is thrown. An item with no weapon row
/// has no power for the script (`Weapon.Find(item)`), which is an error here,
/// never a guessed number.
#[test]
fn throw_needs_a_copy_and_a_weapon_row() {
    let mut b = battle(vec![unit(P1, true, 500), unit(E1, false, 2000)]);
    let (mut h, mut r) = (Hooks::new(), Rng::default());
    h.weapon_power.push((9, 10));
    assert!(b.player_command(0, cmd::THROW, 9, E1, 0, &mut h).unwrap());
    run_idle(&mut b, &mut h, &mut r);
    assert_eq!(b.units[1].cur_hp, 2000);
    assert!(events(&b).contains(&BattleEvent::CommandRejected {
        unit: 0,
        cmd_no: cmd::THROW,
    }));

    let mut b = battle(vec![unit(P1, true, 500), unit(E1, false, 2000)]);
    let mut h = Hooks::new();
    b.inventory.insert(9, 1);
    assert!(b.player_command(0, cmd::THROW, 9, E1, 0, &mut h).unwrap());
    let result = (0..10).try_for_each(|_| b.tick(&mut h, &mut r));
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Auto-Potion (SA 56, `CharacterAbilityGems` `AutoItem`)
// ---------------------------------------------------------------------------

fn auto_potion_battle(extra_sa: &[u8], inventory: &[(i32, u32)]) -> (Battle, Hooks) {
    let mut p2 = with_sa(unit(P2, true, 100), SA_AUTO_POTION);
    p2.will = 90;
    for &sa in extra_sa {
        p2 = with_sa(p2, sa);
    }
    let mut b = battle(vec![unit(P1, true, 500), p2, unit(E1, false, 500)]);
    let mut h = Hooks::new();
    h.items.push((
        ITEM_POTION,
        ItemData {
            effect: item_effect(10, 255, 0),
            script_id: 69,
            for_dead: false,
        },
    ));
    h.items.push((
        ITEM_HI_POTION,
        ItemData {
            effect: item_effect(20, 255, 0),
            script_id: 69,
            for_dead: false,
        },
    ));
    for &(item, n) in inventory {
        b.inventory.insert(item, n);
    }
    (b, h)
}

/// Derivation (`AbilityFeatures.txt:831-834`, `AutoItem`): P2 (SA 56) takes a
/// direct HP hit (script 25, `DirectHP`) from an enemy, so `AutoItem` picks
/// Potion (236) while a copy is left and queues `AutoPotion` on itself. The
/// hit leaves HP 7; the Potion script (10 * 15 = 150) heals to 157 and the
/// copy is spent. With no Potion the Hi-Potion (237) goes: 20 * 15 = 300 ->
/// 307. With neither nothing is queued.
#[test]
fn auto_potion_drinks_a_potion_else_a_hi_potion() {
    let (mut b, mut h) = auto_potion_battle(&[], &[(ITEM_POTION, 1), (ITEM_HI_POTION, 1)]);
    let mut r = Rng::default();
    enemy_cast(&mut b, &mut h, 2, 0, P2);
    run_idle(&mut b, &mut h, &mut r);
    assert_eq!(queued(&b, cmd::AUTO_POTION), vec![(1, P2)]);
    assert_eq!(b.units[1].cur_hp, 157);
    assert_eq!(b.inventory[&ITEM_POTION], 0);
    assert_eq!(b.inventory[&ITEM_HI_POTION], 1);

    let (mut b, mut h) = auto_potion_battle(&[], &[(ITEM_HI_POTION, 1)]);
    enemy_cast(&mut b, &mut h, 2, 0, P2);
    run_idle(&mut b, &mut h, &mut r);
    assert_eq!(b.units[1].cur_hp, 307);
    assert_eq!(b.inventory[&ITEM_HI_POTION], 0);

    let (mut b, mut h) = auto_potion_battle(&[], &[]);
    enemy_cast(&mut b, &mut h, 2, 0, P2);
    run_idle(&mut b, &mut h, &mut r);
    assert!(queued(&b, cmd::AUTO_POTION).is_empty());
    assert_eq!(b.units[1].cur_hp, 7);
}

/// Derivation (`DisableSA 36 54 56`, `CharacterAbilityGems.cs:548`): the
/// features run in ascending id order, so with Counter (36) and Auto-Potion
/// (56) on P2 a passing Counter condition (Spirit 90 > roll 0) queues the
/// Counter and disables 56: no AutoPotion, the Potion stays. A failing Counter
/// condition (roll 99) disables nothing, so 56 then fires.
#[test]
fn counter_disables_auto_potion_only_when_it_passes() {
    let (mut b, mut h) = auto_potion_battle(&[SA_COUNTER], &[(ITEM_POTION, 1)]);
    let mut r = Rng::roll(0);
    enemy_cast(&mut b, &mut h, 2, 0, P2);
    run_idle(&mut b, &mut h, &mut r);
    assert_eq!(queued(&b, cmd::COUNTER).len(), 1);
    assert!(queued(&b, cmd::AUTO_POTION).is_empty());
    assert_eq!(b.inventory[&ITEM_POTION], 1);

    let (mut b, mut h) = auto_potion_battle(&[SA_COUNTER], &[(ITEM_POTION, 1)]);
    let mut r = Rng::roll(99);
    enemy_cast(&mut b, &mut h, 2, 0, P2);
    run_idle(&mut b, &mut h, &mut r);
    assert!(queued(&b, cmd::COUNTER).is_empty());
    assert_eq!(queued(&b, cmd::AUTO_POTION).len(), 1);
    assert_eq!(b.inventory[&ITEM_POTION], 0);
}

// ---------------------------------------------------------------------------
// Enemy reactions (`SBattleCalculator.CalcResult`, `btl_cmd.SetEnemyCommand`)
// ---------------------------------------------------------------------------

fn player_hits_enemy(sub: i32, enemy: BattleUnit) -> Vec<(EnemyReactionKind, u16, u16, u32)> {
    let mut b = battle(vec![unit(P1, true, 500), enemy, unit(E2, false, 500)]);
    let (mut h, mut r) = (Hooks::new(), Rng::default());
    assert!(
        b.player_command(0, cmd::ATTACK, sub, E1, 0, &mut h)
            .unwrap()
    );
    run_idle(&mut b, &mut h, &mut r);
    h.requests
        .iter()
        .map(|q| (q.kind, q.target_id, q.caster_id, q.cmd_no))
        .collect()
}

/// Derivation (`SBattleCalculator.cs:315-325`): after a player's declarable
/// command hits an enemy that is not under `PreventCounter` (Death,
/// `CannotAct`, Sleep): `EnemyDying` when `attack_on_death` and HP is 0,
/// `EnemyCounter` when HP is above 0; `EnemyReaction` is always raised. Attack
/// 176 sets HP to 5 (alive): Counter, Reaction. Attack 177 sets HP to 0: with
/// `attack_on_death` Dying, Reaction; without it only Reaction. A stopped
/// enemy (`CannotAct`) only gets Reaction (Sleep would be cleared by the hit
/// itself, so it is not used here). No request goes out when the target is a player.
#[test]
fn a_player_hit_raises_dying_counter_and_reaction_requests() {
    use EnemyReactionKind::{Counter, Dying, Reaction};
    let alive = player_hits_enemy(176, unit(E1, false, 500));
    assert_eq!(
        alive,
        vec![
            (Counter, E1, P1, cmd::ATTACK),
            (Reaction, E1, P1, cmd::ATTACK)
        ]
    );
    let mut dying = unit(E1, false, 500);
    dying.attack_on_death = true;
    assert_eq!(
        player_hits_enemy(177, dying),
        vec![
            (Dying, E1, P1, cmd::ATTACK),
            (Reaction, E1, P1, cmd::ATTACK)
        ]
    );
    assert_eq!(
        player_hits_enemy(177, unit(E1, false, 500)),
        vec![(Reaction, E1, P1, cmd::ATTACK)]
    );
    assert_eq!(
        player_hits_enemy(176, with_status(unit(E1, false, 500), st::STOP)),
        vec![(Reaction, E1, P1, cmd::ATTACK)]
    );

    // An enemy hitting a player raises nothing.
    let mut b = battle(vec![unit(P1, true, 500), unit(E1, false, 500)]);
    let (mut h, mut r) = (Hooks::new(), Rng::default());
    enemy_cast(&mut b, &mut h, 1, 0, P1);
    run_idle(&mut b, &mut h, &mut r);
    assert!(h.requests.is_empty());
}

/// Derivation (`btl_cmd.SetEnemyCommand`, `btl_cmd.cs:346-438`): the AI's
/// `EnemyCounter` and `EnemyDying` land in slot 1 and `ScriptCounter1..3` in
/// slots 3, 4 and 5, all queued as `EnemyCounter`. A busy slot drops the new
/// request (an enemy attack in slot 0 does not block them). Queueing goes
/// through the same path the tick uses for the AI's commands.
#[test]
fn enemy_counter_dying_and_script_counters_use_their_slots() {
    let mut b = battle(vec![unit(P1, true, 500), unit(E1, false, 500)]);
    let mut h = Hooks::new();
    let act = |cmd_no| EnemyAction {
        cmd_no,
        sub_no: 0,
        tar_id: P1,
    };
    b.set_enemy_command(1, act(cmd::ENEMY_COUNTER), &mut h)
        .unwrap();
    assert_eq!(b.cmds.queue.to_vec(), vec![6 + 1]);
    // Slot 1 is busy: a dying request is dropped (no seventh command).
    b.set_enemy_command(1, act(cmd::ENEMY_DYING), &mut h)
        .unwrap();
    assert_eq!(b.cmds.queue.len(), 1);
    for (cmd_no, slot) in [
        (cmd::SCRIPT_COUNTER_1, 3),
        (cmd::SCRIPT_COUNTER_2, 4),
        (cmd::SCRIPT_COUNTER_3, 5),
    ] {
        b.set_enemy_command(1, act(cmd_no), &mut h).unwrap();
        assert!(b.cmds.queue.contains(&(6 + slot)), "slot {slot}");
        assert_eq!(b.cmds.slots[6 + slot].cmd_no, cmd::ENEMY_COUNTER);
    }
    assert_eq!(queued(&b, cmd::ENEMY_COUNTER).len(), 4);

    let mut b = battle(vec![unit(P1, true, 500), unit(E1, false, 500)]);
    b.set_enemy_command(1, act(cmd::ENEMY_DYING), &mut h)
        .unwrap();
    assert_eq!(b.cmds.slots[6 + 1].cmd_no, cmd::ENEMY_DYING);
    assert_eq!(queued(&b, cmd::ENEMY_DYING), vec![(1, P1)]);
}

/// Derivation (`btl_para.IsSpecialHPInDyingState`, `btl_para.cs:350`, read by
/// `SetEnemyCommand`): a non-dying boss under 10000 HP and alone on its side
/// queues no `EnemyAtk` (or counter), but a Dying command still goes through.
/// With a second enemy in the list the boss attacks.
#[test]
fn a_lone_non_dying_boss_does_not_queue_attacks() {
    let boss = |hp: u32| {
        let mut u = unit(E1, false, hp);
        u.non_dying_boss = true;
        u
    };
    let mut h = Hooks::new();
    let atk = EnemyAction {
        cmd_no: cmd::ENEMY_ATK,
        sub_no: 0,
        tar_id: P1,
    };
    let mut b = battle(vec![unit(P1, true, 500), boss(5000)]);
    b.set_enemy_command(1, atk, &mut h).unwrap();
    assert!(b.cmds.queue.is_empty());
    b.set_enemy_command(
        1,
        EnemyAction {
            cmd_no: cmd::ENEMY_DYING,
            ..atk
        },
        &mut h,
    )
    .unwrap();
    assert_eq!(b.cmds.queue.len(), 1);

    let mut b = battle(vec![unit(P1, true, 500), boss(5000), unit(E2, false, 500)]);
    b.set_enemy_command(1, atk, &mut h).unwrap();
    assert_eq!(b.cmds.queue.len(), 1);
    let mut b = battle(vec![unit(P1, true, 500), boss(20000)]);
    b.set_enemy_command(1, atk, &mut h).unwrap();
    assert_eq!(b.cmds.queue.len(), 1);
}

/// Derivation (`DoEventCode.cs:1177-1215`, `SetEnemyCommand`): commands the AI
/// host reports are queued by `tick` before the ATB pass. A dying command from
/// E1 shows up as `CommandQueued` with command `EnemyDying`, then runs: script
/// 25 power 11 sets P1's HP to 11.
#[test]
fn ai_reaction_commands_are_queued_on_the_next_tick_and_run() {
    let mut b = battle(vec![unit(P1, true, 500), unit(E1, false, 500)]);
    let (mut h, mut r) = (Hooks::new(), Rng::default());
    h.reaction_cmds.push((
        E1,
        EnemyAction {
            cmd_no: cmd::ENEMY_DYING,
            sub_no: 0,
            tar_id: P1,
        },
    ));
    run_idle(&mut b, &mut h, &mut r);
    assert_eq!(queued(&b, cmd::ENEMY_DYING), vec![(1, P1)]);
    assert_eq!(b.units[0].cur_hp, 11);
}

/// Derivation (`btl_cmd.cs:1025`, `JumpStatusScript`, `ProcessJumpSpear`):
/// Jump gives the caster the Jump status and makes it untargetable; the
/// countdown `10 * 4 * (60 - Will 20)` = 1600 runs on the ATB clock, then
/// Spear is queued on the Jump's target (script 25 power 6 sets E1's HP to
/// 6), and its finish removes the status and empties the gauge, which the
/// rest of that tick refills by one ATB step.
#[test]
fn jump_leaves_the_field_and_lands_with_spear() {
    let mut b = battle(vec![unit(P1, true, 500), unit(E1, false, 500)]);
    let mut h = Hooks::new();
    let mut rng = Rng::default();
    let slot = b.cmds.slot_of(0, 0);
    b.set_command(slot, cmd::JUMP, 178, E1, 0, false, &mut h)
        .unwrap();
    run_idle(&mut b, &mut h, &mut rng);
    assert!(events(&b).contains(&BattleEvent::Jumped { unit: 0 }));
    assert!(b.units[0].status_current & st::JUMP != 0);
    assert_eq!(b.jumps[&0].countdown, 1600);
    let mut ticks = 0;
    while !events(&b).contains(&BattleEvent::Landed { unit: 0 }) {
        b.tick(&mut h, &mut rng).expect("tick");
        ticks += 1;
        assert!(ticks < 20_000, "never landed");
    }
    assert_eq!(b.units[1].cur_hp, 6);
    assert_eq!(b.units[0].status_current & st::JUMP, 0);
    // Emptied, then one ATB step of the same tick.
    assert!(b.units[0].cur_atb > 0 && b.units[0].cur_atb < b.sched[0].max_atb / 4);
    assert!(b.jumps.is_empty());
    assert_eq!(queued(&b, cmd::SPEAR), vec![(0, E1)]);
}
