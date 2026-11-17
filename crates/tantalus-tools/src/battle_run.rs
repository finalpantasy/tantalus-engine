//! Full battle runner: run a real decoded scene to completion with
//! battle-script enemy AI, player attacks, death-driven rewards, and the
//! reference split — the proof that battles no longer block the chain.
//!
//! Composition: scene enemies (`pattern_machine_units`) + synthetic armed
//! party -> `LegacyBattle` -> `LegacyBattleAdapter` with `EnemyAiHooks`
//! (script AI) -> tick loop with player Attack commands -> `set_bonus` per
//! death -> `BattleReward::from_bonus` totals. Script unit writes apply back
//! through `take_dirty_views`/`apply_unit_views` each tick.

use std::collections::BTreeMap;

use tantalus_assets::pc::battle_scene::BattleScene;
use tantalus_contracts::{ContentId, EntityId, Tick};
use tantalus_game::FieldWorld;
use tantalus_game::battle::GameServices;
use tantalus_game::battle::legacy::NoEnemyAi;
use tantalus_game::battle::legacy::ServicesRandom;
use tantalus_game::battle::{BattleOutcome, WorldGameServices};
use tantalus_legacy::EventBinary;
use tantalus_legacy::battle::atb::StartType;
use tantalus_legacy::battle::host::{apply_unit_views, battle_views};
use tantalus_legacy::battle::machine::{ABILITY_ATTACK, BattleConfig};
use tantalus_legacy::battle::queue::cmd;
use tantalus_legacy::battle::rewards::{
    BattleBonus, BattleReward, EnemyRewardData, PartyMember, PartySnapshot,
};
use tantalus_legacy::battle::rng::{BattleRandom, SplitMix64Random};
use tantalus_legacy::battle::script_host::{
    BattleScriptDriver, BattleScriptState, EnemyAiHooks, SceneOverlay, ScriptTable,
};
use tantalus_legacy::battle::{BattleUnit, StatusDataRow, StatusTable, WeaponStats};

use crate::battle_scene_rewards::assemble_legacy_battle;

const MAX_TICKS: u64 = 20_000;
/// Flee-first stall window. After the single flee press fails, the policy
/// never attacks, so a battle neither side can end would burn the full
/// 20,000-tick budget doing nothing. Slow party deaths are REAL defeats
/// (median ~13k ticks in the flee corpora) and reflect/counter kills are
/// REAL victories, so instead of a fixed cap we watch ALL units' HP: if no
/// unit has lost HP in this many consecutive ticks after the flee press,
/// the battle cannot end and we stop early with the honest no-outcome
/// shape. The machine itself has no such limit.
const FLEE_STALL_WINDOW: u64 = 1_000;
const ATTACK_ACTION: &str = "ffix:action/attack";

/// Four stand-in party members (ids 1, 2, 4, 8). `weak` swaps the armed
/// level-20 default for an unarmed level-8 party so sweeps can exercise
/// mass-defeat paths (decided-defeat gate, reward suppression) instead of
/// only the endgame superboss wipes.
fn demo_players(weak: bool) -> Vec<BattleUnit> {
    [1u16, 2, 4, 8]
        .iter()
        .enumerate()
        .map(|(index, id)| {
            let mut unit = BattleUnit::new(*id, true);
            unit.character_id = index as u8;
            if weak {
                unit.level = 8;
                unit.cur_hp = 120;
                unit.max_hp = 120;
                unit.cur_mp = 50;
                unit.max_mp = 50;
                unit.strength = 8;
                unit.magic = 8;
                unit.dexterity = 10;
                unit.will = 8;
                unit.physical_defence = 4;
                unit.physical_evade = 2;
                unit.magic_defence = 5;
                unit.magic_evade = 2;
            } else {
                // `TANTALUS_BATTLE_STRONG` set: a mid-game party (level 70,
                // 4,000 HP) for sweeps that look for scenes a strong party
                // still loses; the default stays the level-20 corpus party.
                let strong = std::env::var_os("TANTALUS_BATTLE_STRONG").is_some();
                unit.level = if strong { 70 } else { 20 };
                let hp = if strong { 4000 } else { 500 };
                unit.cur_hp = hp;
                unit.max_hp = hp;
                unit.cur_mp = 100;
                unit.max_mp = 100;
                unit.strength = if strong { 55 } else { 30 };
                unit.magic = if strong { 55 } else { 30 };
                unit.dexterity = 30;
                unit.will = if strong { 35 } else { 20 };
                unit.physical_defence = if strong { 30 } else { 10 };
                unit.physical_evade = 5;
                unit.magic_defence = if strong { 30 } else { 12 };
                unit.magic_evade = 6;
                unit.weapon = Some(WeaponStats {
                    power: if strong { 50 } else { 25 },
                    rate: 0,
                    elements: tantalus_legacy::battle::Elements::NONE,
                    category: WeaponStats::SHORT_RANGE,
                    status: 0,
                });
            }
            unit
        })
        .collect()
}

fn demo_member(hp: u32) -> PartyMember {
    demo_member_with_level(hp, 20, [23, 21, 18, 23], 18)
}

/// Weak-party variant: level-8 member whose base stats match the unarmed
/// demo units (`demo_players(true)`).
fn weak_member(hp: u32) -> PartyMember {
    demo_member_with_level(hp, 8, [10, 8, 8, 8], 8)
}

fn demo_member_with_level(hp: u32, level: u8, basis: [u8; 4], gems: u8) -> PartyMember {
    let base = tantalus_legacy::battle::rewards::BaseStats {
        dex: basis[0],
        str_: basis[1],
        mgc: basis[2],
        wpr: basis[3],
        gems,
    };
    PartyMember {
        in_battle: true,
        cur_hp: hp,
        cur_mp: 100,
        rewards_disabled: false,
        level,
        exp: 0,
        base,
        bonus: Default::default(),
        basis,
        max_hp: 500,
        max_mp: 100,
        max_capa: u32::from(gems),
        cur_capa: u32::from(gems),
        elem: basis,
        equip_stat_bonus: [0; 4],
        equipped_abilities: vec![],
        learnable: vec![],
        pa: vec![],
    }
}

/// JSON-serializable outcome of one run.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BattleRunReport {
    pub scene: String,
    pub pattern: usize,
    pub outcome: Option<BattleOutcome>,
    pub ticks: u64,
    pub error: Option<String>,
    pub enemy_deaths: usize,
    pub players_alive: usize,
    pub bonus_gil: u32,
    pub bonus_exp: u32,
    pub bonus_ap: u16,
    pub bonus_items: Vec<(u32, u32)>,
    pub card: Option<u32>,
}

/// Batch-safe wrapper: catches panics (debug-build arithmetic edge cases on
/// untested scenes) and reports them instead of aborting the sweep.
pub fn run_scene_battle_caught(
    scene_name: &str,
    scene: &BattleScene,
    pattern: usize,
    script: &[u8],
    seed: u64,
    weak_party: bool,
    flee_first: bool,
) -> BattleRunReport {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_scene_battle(
            scene_name, scene, pattern, script, seed, weak_party, flee_first,
        )
    }))
    .unwrap_or_else(|_| BattleRunReport {
        scene: scene_name.to_owned(),
        pattern,
        outcome: None,
        ticks: 0,
        error: Some("panic during run".to_owned()),
        enemy_deaths: 0,
        players_alive: 0,
        bonus_gil: 0,
        bonus_exp: 0,
        bonus_ap: 0,
        bonus_items: Vec::new(),
        card: None,
    })
}

/// Runs `scene` pattern `pattern` to completion. `script` is the decoded
/// `EVT_BATTLE_<name>.eb` bytecode driving enemy AI.
pub fn run_scene_battle(
    scene_name: &str,
    scene: &BattleScene,
    pattern: usize,
    script: &[u8],
    seed: u64,
    weak_party: bool,
    flee_first: bool,
) -> BattleRunReport {
    let mut report = BattleRunReport {
        scene: scene_name.to_owned(),
        pattern,
        outcome: None,
        ticks: 0,
        error: None,
        enemy_deaths: 0,
        players_alive: 0,
        bonus_gil: 0,
        bonus_exp: 0,
        bonus_ap: 0,
        bonus_items: Vec::new(),
        card: None,
    };

    let players = demo_players(weak_party);
    let party_ids: Vec<u8> = players.iter().map(|unit| unit.character_id).collect();
    let player_entities: Vec<EntityId> = players
        .iter()
        .map(|unit| EntityId::new(u64::from(unit.id)))
        .collect();
    let Some(setup) = assemble_legacy_battle(
        scene,
        pattern,
        &BattleConfig::default(),
        &StatusTable::new(vec![StatusDataRow::default(); 32]),
        &player_entities
            .iter()
            .copied()
            .zip(players)
            .collect::<Vec<_>>(),
        BTreeMap::from([(
            ContentId::parse(ATTACK_ACTION).unwrap(),
            (cmd::ATTACK, ABILITY_ATTACK),
        )]),
    ) else {
        report.error = Some("scene/pattern did not assemble".to_owned());
        return report;
    };

    // All three random streams answer to the run seed: script AI
    // (SplitMix64(seed)), drop draws (SplitMix64(seed ^ 0xD1AA)), and the
    // calculator/hit-roll stream through GameServices (world RNG here).
    let mut world = FieldWorld::new();
    world.seed_rng(seed ^ 0xC0FF_EE11_DEAD_BEEF);
    // `btl_sys.StartType` (btl_sys.cs:181): the back-attack flag forces
    // BackAttack; on non-special scenes the reference rolls random8 for a
    // back attack (24/256) then a preemptive (16/256). These draws come
    // from the calculator stream, before the initial ATB rolls.
    let (start, start_code) = {
        let mut services = WorldGameServices::new(&mut world);
        if scene.header.flags & 0x2 != 0 {
            (StartType::BackAttack, 0)
        } else if scene.header.flags & 0x1 != 0 {
            (StartType::Normal, 2)
        } else {
            let back = services.random_below(256) < 24;
            let first = !back && services.random_below(256) < 16;
            if back {
                (StartType::BackAttack, 0)
            } else if first {
                (StartType::FirstAttack, 1)
            } else {
                (StartType::Normal, 2)
            }
        }
    };

    // Script AI over the assembled battle state.
    let (data, units) = battle_views(setup.adapter.battle());
    let binary = match EventBinary::parse(script) {
        Ok(binary) => binary,
        Err(error) => {
            report.error = Some(format!("script decode failed: {error}"));
            return report;
        }
    };
    let driver = BattleScriptDriver::new(
        ScriptTable::from_binary(&binary),
        BattleScriptState::new(data, units),
        Box::new(SplitMix64Random::new(seed)),
        &party_ids,
    );
    let hooks = EnemyAiHooks::new(
        NoEnemyAi,
        driver,
        SceneOverlay {
            start_type: start_code,
            ..SceneOverlay::default()
        },
    );
    let mut adapter = setup.adapter.with_hooks(hooks);
    {
        let mut services = WorldGameServices::new(&mut world);
        if let Err(error) = adapter.battle_mut().roll_initial_atb(
            start,
            &mut ServicesRandom {
                services: &mut services,
            },
        ) {
            report.error = Some(format!("initial ATB roll: {error:?}"));
            return report;
        }
    }
    let mut reward_rows: Vec<EnemyRewardData> = setup.reward_rows;
    let pattern_ap = setup.pattern_ap;
    let enemy_entities = setup.enemy_entities;

    // `SB2_HEAD` 0x800 (`after_event`): story battles pay nothing.
    let event = scene.header.flags & 0x800 != 0;
    let mut bonus = BattleBonus::new(pattern_ap, 0x0F, event);
    let mut draw_rng: Box<dyn BattleRandom> = Box::new(SplitMix64Random::new(seed ^ 0xD1AA));
    let attack = ContentId::parse(ATTACK_ACTION).unwrap();

    let mut fled = false;
    let mut party_hp = 0u64;
    let mut last_damage_tick = 0u64;
    for tick in 0..MAX_TICKS {
        let mut input = tantalus_game::battle::CombatInput::new(Tick::new(tick));
        // The machine gates the flee roll on the ATB running (`atb_runs`),
        // false at tick 0: wait for the ATB, then press ONCE and live with
        // the roll. Waiting consumes no draws, so both engine paths roll
        // from the same stream state; retrying until success would make
        // every runaway battle a guaranteed escape.
        if flee_first && !fled && adapter.battle().atb_runs() {
            fled = true;
            let mut services = WorldGameServices::new(&mut world);
            let _ = adapter.flee_attempt(&mut services);
        }
        // Escape policy: once the flee key is tried the party never attacks.
        if !(flee_first && fled) {
            // Player policy: the first ready party member attacks the first
            // living enemy.
            let ready = adapter.battle().ready_players.first().copied();
            if let Some(unit_index) = ready {
                let target = adapter
                    .battle()
                    .units
                    .iter()
                    .enumerate()
                    .find(|(_, unit)| !unit.is_player && unit.cur_hp > 0)
                    .map(|(index, _)| EntityId::new(u64::from(adapter.battle().units[index].id)));
                if let Some(target) = target {
                    input.command = Some(tantalus_game::battle::BattleCommand {
                        actor: player_entities[unit_index],
                        action: attack.clone(),
                        targets: vec![target],
                    });
                }
            }
        }
        {
            let mut services = WorldGameServices::new(&mut world);
            if let Err(error) = adapter.advance(&input, &mut services) {
                report.error = Some(format!("tick {tick}: {error}"));
                report.ticks = tick + 1;
                return report;
            }
        }
        // Script unit writes flow back into the machine.
        let dirty = adapter.hooks_mut().take_dirty_views();
        apply_unit_views(adapter.battle_mut(), &dirty);
        if let Some(error) = &adapter.hooks().error {
            report.error = Some(format!("tick {tick}: AI: {error:?}"));
            report.ticks = tick + 1;
            return report;
        }
        // Deaths pay their bonus row (reference on-kill hook shape).
        for event in adapter.take_events() {
            let tantalus_game::battle::CombatEvent::UnitDefeated { unit } = event else {
                continue;
            };
            if !enemy_entities.contains(&unit) || report.enemy_deaths >= reward_rows.len() {
                continue;
            }
            if bonus.set_bonus(&mut reward_rows, report.enemy_deaths, &mut *draw_rng) {
                report.enemy_deaths += 1;
            }
        }
        // Flee-first stall detection (see the world path for the rationale).
        if flee_first && fled {
            let hp: u64 = adapter
                .battle()
                .units
                .iter()
                .map(|unit| u64::from(unit.cur_hp))
                .sum();
            if hp < party_hp {
                last_damage_tick = tick;
            }
            party_hp = hp;
            if tick > FLEE_STALL_WINDOW && tick - last_damage_tick >= FLEE_STALL_WINDOW {
                report.ticks = tick + 1;
                break;
            }
        }
        if let Some(outcome) = adapter.outcome() {
            report.outcome = Some(outcome);
            report.ticks = tick + 1;
            break;
        }
    }

    // Script-paid bonuses for enemies that never died (member 71 writes).
    for event in &adapter.hooks().driver.state.events {
        if let tantalus_legacy::battle::script_host::AiEvent::SetBonus { unit } = event
            && let Some(index) = enemy_entities.iter().position(|entity| entity == unit)
            && index < reward_rows.len()
        {
            bonus.set_bonus(&mut reward_rows, index, &mut *draw_rng);
        }
    }

    report.players_alive = adapter
        .battle()
        .units
        .iter()
        .filter(|unit| unit.is_player && unit.cur_hp > 0)
        .count();

    // Reward totals through the reference split (party-flee-key escapes pay
    // nothing; Flee-ability cost is separate UI-path work).
    if report.outcome == Some(BattleOutcome::Victory) {
        let member = || {
            if weak_party {
                weak_member(120)
            } else {
                demo_member(500)
            }
        };
        let members: [Option<PartyMember>; 4] = [
            Some(member()),
            Some(member()),
            Some(member()),
            Some(member()),
        ];
        let party = PartySnapshot {
            gil: 0,
            members,
            items: Vec::new(),
            cards: Vec::new(),
        };
        let reward = BattleReward::from_bonus(&bonus, &party);
        report.bonus_gil = reward.gil;
        report.bonus_exp = reward.shares.iter().filter_map(|share| share.exp).sum();
        report.bonus_ap = bonus.ap;
        report.bonus_items = reward.items;
        report.card = reward.card;
    }
    report
}

/// The definitive field hand-off: runs a battle for `scene`/`pattern`
/// against `world` with the party built from world state, script-driven
/// enemy AI, death-driven `set_bonus`, and the reference split committed
/// exactly once. (Drives the typed adapter directly; the coordinator wrapper
/// around the default-typed adapter is exercised in the tantalus-game
/// integration tests.)
///
/// `Err` reports an early failure (no party, unassemblable scene, undecodable
/// script, tick advance error). `Ok(None)` means the battle ran the full tick
/// budget without reaching an outcome — a legitimate shared shape with the
/// report path (the fog battles), not an error.
#[allow(clippy::too_many_arguments)] // battle runner: policy knobs mirror the report path.
pub fn run_world_battle(
    world: &mut tantalus_game::FieldWorld,
    entities: &[tantalus_contracts::EntityId],
    scene: &BattleScene,
    pattern: usize,
    script: &[u8],
    base: &dyn Fn(i32) -> tantalus_game::battle::legacy::PlayerBase,
    seed: u64,
    flee_first: bool,
) -> Result<(Option<BattleOutcome>, u64), String> {
    use tantalus_game::battle::GameServices;
    use tantalus_game::battle::legacy::{NoEnemyAi, party_machine_units};
    use tantalus_legacy::EventBinary;
    use tantalus_legacy::battle::host::{apply_unit_views, battle_views};
    use tantalus_legacy::battle::machine::{ABILITY_ATTACK, BattleConfig};
    use tantalus_legacy::battle::queue::cmd;
    use tantalus_legacy::battle::rewards::{BattleBonus, BattleReward, PartySnapshot};
    use tantalus_legacy::battle::script_host::{
        BattleScriptDriver, BattleScriptState, EnemyAiHooks, SceneOverlay, ScriptTable,
    };

    let players = party_machine_units(world, entities, base);
    if players.is_empty() {
        return Err("no party units assembled from world state".to_owned());
    }
    let setup = crate::battle_scene_rewards::assemble_legacy_battle(
        scene,
        pattern,
        &BattleConfig::default(),
        &tantalus_legacy::battle::StatusTable::new(vec![
            tantalus_legacy::battle::StatusDataRow::default();
            32
        ]),
        &players,
        std::collections::BTreeMap::from([(
            tantalus_contracts::ContentId::parse(ATTACK_ACTION).unwrap(),
            (cmd::ATTACK, ABILITY_ATTACK),
        )]),
    )
    .ok_or_else(|| "scene/pattern did not assemble".to_owned())?;
    let (data, units) = battle_views(setup.adapter.battle());
    let binary =
        EventBinary::parse(script).map_err(|error| format!("script decode failed: {error}"))?;
    let party_ids: Vec<u8> = players.iter().map(|(_, unit)| unit.character_id).collect();
    // `btl_sys.StartType` (btl_sys.cs:181): back-attack flag forces
    // BackAttack; non-special scenes roll random8 for back (24/256) then
    // preemptive (16/256), from the calculator stream, before the initial
    // ATB rolls. See the report path for the full note.
    let (start, start_code) = {
        let mut services = WorldGameServices::new(world);
        if scene.header.flags & 0x2 != 0 {
            (StartType::BackAttack, 0)
        } else if scene.header.flags & 0x1 != 0 {
            (StartType::Normal, 2)
        } else {
            let back = services.random_below(256) < 24;
            let first = !back && services.random_below(256) < 16;
            if back {
                (StartType::BackAttack, 0)
            } else if first {
                (StartType::FirstAttack, 1)
            } else {
                (StartType::Normal, 2)
            }
        }
    };
    let driver = BattleScriptDriver::new(
        ScriptTable::from_binary(&binary),
        BattleScriptState::new(data, units),
        Box::new(SplitMix64Random::new(seed)),
        &party_ids,
    );
    let hooks = EnemyAiHooks::new(
        NoEnemyAi,
        driver,
        SceneOverlay {
            start_type: start_code,
            ..SceneOverlay::default()
        },
    );
    let mut adapter = setup.adapter.with_hooks(hooks);
    {
        let mut services = WorldGameServices::new(world);
        if let Err(error) = adapter.battle_mut().roll_initial_atb(
            start,
            &mut ServicesRandom {
                services: &mut services,
            },
        ) {
            return Err(format!("initial ATB roll: {error:?}"));
        }
    }

    // `SB2_HEAD` 0x800 (`after_event`): story battles show no result
    // screen and pay nothing (`BattleResultUI.InitialEvent`).
    let event = scene.header.flags & 0x800 != 0;
    let member_flag = (1u8 << players.len().min(4)) - 1;
    let mut bonus = BattleBonus::new(setup.pattern_ap, member_flag, event);
    let mut rows = setup.reward_rows.clone();
    let mut deaths = 0usize;
    let mut draw_rng: Box<dyn BattleRandom> = Box::new(SplitMix64Random::new(seed ^ 0xD1AA));
    let attack = tantalus_contracts::ContentId::parse(ATTACK_ACTION).unwrap();
    let mut outcome = None;

    let mut fled = false;
    let mut ticks = 0u64;
    let mut party_hp = 0u64;
    let mut last_damage_tick = 0u64;
    for tick in 0..MAX_TICKS {
        let mut input = tantalus_game::battle::CombatInput::new(Tick::new(tick));
        // The machine gates the flee roll on the ATB running (`atb_runs`),
        // false at tick 0: wait for the ATB, then press ONCE and live with
        // the roll. Waiting consumes no draws, so both engine paths roll
        // from the same stream state; retrying until success would make
        // every runaway battle a guaranteed escape.
        if flee_first && !fled && adapter.battle().atb_runs() {
            fled = true;
            let mut services = WorldGameServices::new(world);
            let _ = adapter.flee_attempt(&mut services);
        }
        // Escape policy: once the flee key is tried the party never attacks.
        if !(flee_first && fled)
            && let Some(unit_index) = adapter.battle().ready_players.first().copied()
        {
            let target = adapter
                .battle()
                .units
                .iter()
                .enumerate()
                .find(|(_, unit)| !unit.is_player && unit.cur_hp > 0)
                .map(|(index, _)| {
                    tantalus_contracts::EntityId::new(u64::from(adapter.battle().units[index].id))
                });
            if let Some(target) = target {
                input.command = Some(tantalus_game::battle::BattleCommand {
                    actor: entities[unit_index],
                    action: attack.clone(),
                    targets: vec![target],
                });
            }
        }
        {
            let mut services = WorldGameServices::new(world);
            if let Err(error) = adapter.advance(&input, &mut services) {
                return Err(format!("tick {tick}: {error}"));
            }
        }
        let dirty = adapter.hooks_mut().take_dirty_views();
        apply_unit_views(adapter.battle_mut(), &dirty);
        for event in adapter.take_events() {
            if let tantalus_game::battle::CombatEvent::UnitDefeated { unit } = event
                && setup.enemy_entities.contains(&unit)
                && deaths < rows.len()
                && bonus.set_bonus(&mut rows, deaths, &mut *draw_rng)
            {
                deaths += 1;
            }
        }
        // Flee-first stall detection: after the press, if no player has lost
        // HP within the window, neither side can end the battle -- stop with
        // the honest no-outcome shape instead of burning the full budget.
        if flee_first && fled {
            let hp: u64 = adapter
                .battle()
                .units
                .iter()
                .map(|unit| u64::from(unit.cur_hp))
                .sum();
            if hp < party_hp {
                last_damage_tick = tick;
            }
            party_hp = hp;
            if tick > FLEE_STALL_WINDOW && tick - last_damage_tick >= FLEE_STALL_WINDOW {
                ticks = tick + 1;
                break;
            }
        }
        if let Some(done) = adapter.outcome() {
            outcome = Some(done);
            ticks = tick + 1;
            break;
        }
    }

    // Escape economics: the Flee ability costs a tenth of the would-be
    // spoils (the party flee key is free). `apply_flee_ability_cost` is a
    // no-op unless the escape came from the ability.
    if outcome == Some(BattleOutcome::Escape) {
        let enemy_bonus_gil: u32 = rows.iter().map(|row| row.gil).sum();
        tantalus_game::battle::legacy::apply_flee_ability_cost(
            world,
            adapter.fled_by_ability(),
            bonus.gil.max(0) as u32,
            enemy_bonus_gil,
        );
    }

    // Script-paid bonuses (`SetBattleCharData` member 71 -> `btl_sys.SetBonus`)
    // for enemies that end the battle alive; `set_bonus`'s `bonus_given`
    // guard skips rows already paid by death.
    for event in &adapter.hooks().driver.state.events {
        if let tantalus_legacy::battle::script_host::AiEvent::SetBonus { unit } = event
            && let Some(index) = setup
                .enemy_entities
                .iter()
                .position(|entity| entity == unit)
            && index < rows.len()
        {
            bonus.set_bonus(&mut rows, index, &mut *draw_rng);
        }
    }

    // Carry surviving HP/MP back to the field (the reference keeps current
    // HP/MP after battle; Opus's `field_battle::save_party` does the same for
    // the chain's report path). Party units occupy the machine list head in
    // party order.
    for (index, &character) in world.party.clone().iter().enumerate() {
        let Some(unit) = adapter.battle().units.get(index) else {
            break;
        };
        if let Some(stats) = world.character_stats.get_mut(&character) {
            stats.current_hp = i32::try_from(unit.cur_hp)
                .unwrap_or(i32::MAX)
                .clamp(0, stats.max_hp);
            stats.current_mp = i32::try_from(unit.cur_mp)
                .unwrap_or(i32::MAX)
                .clamp(0, stats.max_mp);
        }
    }

    if outcome == Some(BattleOutcome::Victory) {
        // Party members carry their learnable lists so AP shares form
        // (`from_bonus` only awards AP to lines with learnable rows).
        let mut members: [Option<PartyMember>; 4] = Default::default();
        for (slot, member) in members.iter_mut().enumerate() {
            let mut member_data = demo_member(500);
            if let Some(&character) = world.party.get(slot) {
                let base = base(character);
                member_data.learnable = base
                    .learnable
                    .iter()
                    .map(|&(id, max_ap)| tantalus_legacy::battle::rewards::PaEntry { id, max_ap })
                    .collect();
                member_data.pa = vec![0; member_data.learnable.len()];
                member_data.equipped_abilities = base.equipped_abilities.clone();
            }
            *member = Some(member_data);
        }
        let party = PartySnapshot {
            gil: world.gil,
            members,
            items: Vec::new(),
            cards: Vec::new(),
        };
        let reward = BattleReward::from_bonus(&bonus, &party);

        // AP: every party character with an AP share learns through their
        // equipped abilities (`AddAp` per ability, reference semantics).
        if !bonus.event {
            for (slot, share) in reward.shares.iter().enumerate() {
                let Some(ap) = share.ap else {
                    continue;
                };
                if players.get(slot).is_none() {
                    continue;
                }
                let base = base(world.party[slot]);
                for &ability in &base.equipped_abilities {
                    if let Some((_, max_ap)) = base.learnable.iter().find(|(id, _)| *id == ability)
                    {
                        world.apply_ap(world.party[slot], ability, *max_ap, i32::from(ap));
                    }
                }
            }
        }
        let lines: [Option<i32>; 4] = [
            world.party.first().copied(),
            world.party.get(1).copied(),
            world.party.get(2).copied(),
            world.party.get(3).copied(),
        ];
        let bundle = tantalus_game::battle::bridge::bundle_from_battle_reward(
            &reward,
            lines,
            (seed % 1000) + 1,
        );
        let mut services = WorldGameServices::new(world);
        let _ = services.commit_rewards(&bundle);
    }
    Ok((outcome, ticks))
}

/// World-hand-off twin of [`run_scene_battle`]: a fresh scratch world whose
/// party stats match `demo_players`, run through [`run_world_battle`] with
/// the same seed wiring. The report is filled from world deltas (gil, EXP,
/// survivor HP) so sweep outputs from both paths compare directly. Item and
/// card draws are NOT expected to match the report path: each path owns an
/// independent draw stream.
pub fn run_world_battle_report(
    scene_name: &str,
    scene: &BattleScene,
    pattern: usize,
    script: &[u8],
    seed: u64,
    weak_party: bool,
    flee_first: bool,
) -> BattleRunReport {
    let mut report = BattleRunReport {
        scene: scene_name.to_owned(),
        pattern,
        outcome: None,
        ticks: 0,
        error: None,
        enemy_deaths: 0,
        players_alive: 0,
        bonus_gil: 0,
        bonus_exp: 0,
        bonus_ap: 0,
        bonus_items: Vec::new(),
        card: None,
    };
    // All three random streams answer to the run seed: script AI
    // (SplitMix64(seed)), drop draws (SplitMix64(seed ^ 0xD1AA)), and the
    // calculator/hit-roll stream through GameServices (world RNG here).
    let mut world = FieldWorld::new();
    world.seed_rng(seed ^ 0xC0FF_EE11_DEAD_BEEF);
    world.party = vec![0, 1, 2, 3];
    world.gil = 0;
    let (hp, mp) = if weak_party { (120, 50) } else { (500, 100) };
    for &character in &world.party {
        world.character_stats.insert(
            character,
            tantalus_game::field::CharacterStats {
                current_hp: hp,
                max_hp: hp,
                current_mp: mp,
                max_mp: mp,
            },
        );
    }
    let entities: Vec<EntityId> = [101u64, 102, 104, 108]
        .iter()
        .map(|id| EntityId::new(*id))
        .collect();
    let base = |_: i32| {
        if weak_party {
            tantalus_game::battle::legacy::PlayerBase {
                level: 8,
                strength: 8,
                magic: 8,
                dexterity: 10,
                will: 8,
                physical_defence: 4,
                physical_evade: 2,
                magic_defence: 5,
                magic_evade: 2,
                weapon: None,
                learnable: Vec::new(),
                equipped_abilities: Vec::new(),
            }
        } else {
            tantalus_game::battle::legacy::PlayerBase {
                level: 20,
                strength: 30,
                magic: 30,
                dexterity: 30,
                will: 20,
                physical_defence: 10,
                physical_evade: 5,
                magic_defence: 12,
                magic_evade: 6,
                weapon: Some(WeaponStats {
                    power: 25,
                    rate: 0,
                    elements: tantalus_legacy::battle::Elements::NONE,
                    category: WeaponStats::SHORT_RANGE,
                    status: 0,
                }),
                learnable: Vec::new(),
                equipped_abilities: Vec::new(),
            }
        }
    };
    // `Ok(None)` mirrors the report path's "ran but reached no outcome"
    // rows (e.g. the fog battles, which assemble clean and simply never
    // end): not an error. `Err` is a genuine early failure and is reported.
    match run_world_battle(
        &mut world, &entities, scene, pattern, script, &base, seed, flee_first,
    ) {
        Ok((Some(outcome), ticks)) => {
            report.outcome = Some(outcome);
            report.ticks = ticks;
            report.bonus_gil = world.gil;
            report.bonus_exp = world.character_exp.values().copied().sum::<u64>() as u32;
        }
        Ok((None, ticks)) => {
            report.ticks = ticks;
        }
        Err(error) => report.error = Some(error),
    }
    report.players_alive = world
        .character_stats
        .values()
        .filter(|stats| stats.current_hp > 0)
        .count();
    report
}

/// Panic-containing wrapper for sweep drivers, mirroring
/// [`run_scene_battle_caught`].
pub fn run_world_battle_caught(
    scene_name: &str,
    scene: &BattleScene,
    pattern: usize,
    script: &[u8],
    seed: u64,
    weak_party: bool,
    flee_first: bool,
) -> BattleRunReport {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_world_battle_report(
            scene_name, scene, pattern, script, seed, weak_party, flee_first,
        )
    }))
    .unwrap_or_else(|_| BattleRunReport {
        scene: scene_name.to_owned(),
        pattern,
        outcome: None,
        ticks: 0,
        error: Some("panic during run".to_owned()),
        enemy_deaths: 0,
        players_alive: 0,
        bonus_gil: 0,
        bonus_exp: 0,
        bonus_ap: 0,
        bonus_items: Vec::new(),
        card: None,
    })
}
