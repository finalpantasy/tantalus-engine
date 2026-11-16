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
use tantalus_game::battle::legacy::NoEnemyAi;
use tantalus_game::battle::{BattleOutcome, WorldGameServices};
use tantalus_legacy::EventBinary;
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
const ATTACK_ACTION: &str = "ffix:action/attack";

/// Four armed level-20 stand-in party members (ids 1, 2, 4, 8), matching the
/// AI demo's party shape until the chain feeds real party state.
fn demo_players() -> Vec<BattleUnit> {
    [1u16, 2, 4, 8]
        .iter()
        .map(|id| {
            let mut unit = BattleUnit::new(*id, true);
            unit.level = 20;
            unit.cur_hp = 500;
            unit.max_hp = 500;
            unit.cur_mp = 100;
            unit.max_mp = 100;
            unit.strength = 30;
            unit.magic = 30;
            unit.dexterity = 30;
            unit.will = 20;
            unit.weapon = Some(WeaponStats {
                power: 25,
                rate: 0,
                elements: tantalus_legacy::battle::Elements::NONE,
                category: WeaponStats::SHORT_RANGE,
                status: 0,
            });
            unit
        })
        .collect()
}

fn demo_member(hp: u32) -> PartyMember {
    let base = tantalus_legacy::battle::rewards::BaseStats {
        dex: 23,
        str_: 21,
        mgc: 18,
        wpr: 23,
        gems: 18,
    };
    PartyMember {
        in_battle: true,
        cur_hp: hp,
        cur_mp: 100,
        rewards_disabled: false,
        level: 20,
        exp: 0,
        base,
        bonus: Default::default(),
        basis: [base.dex, base.str_, base.mgc, base.wpr],
        max_hp: 500,
        max_mp: 100,
        max_capa: 18,
        cur_capa: 18,
        elem: [base.dex, base.str_, base.mgc, base.wpr],
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
) -> BattleRunReport {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_scene_battle(scene_name, scene, pattern, script, seed)
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

    let players = demo_players();
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
        &[0],
    );
    let hooks = EnemyAiHooks::new(NoEnemyAi, driver, SceneOverlay::default());
    let mut adapter = setup.adapter.with_hooks(hooks);
    let mut reward_rows: Vec<EnemyRewardData> = setup.reward_rows;
    let pattern_ap = setup.pattern_ap;
    let enemy_entities = setup.enemy_entities;

    let mut world = FieldWorld::new();
    let mut bonus = BattleBonus::new(pattern_ap, 0x0F, false);
    let mut draw_rng: Box<dyn BattleRandom> = Box::new(SplitMix64Random::new(seed ^ 0xD1AA));
    let attack = ContentId::parse(ATTACK_ACTION).unwrap();

    for tick in 0..MAX_TICKS {
        let mut input = tantalus_game::battle::CombatInput::new(Tick::new(tick));
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
        {
            let mut services = WorldGameServices::new(&mut world);
            if let Err(error) = adapter.advance(&input, &mut services) {
                report.error = Some(format!("tick {tick}: {error}"));
                report.ticks = tick;
                return report;
            }
        }
        // Script unit writes flow back into the machine.
        let dirty = adapter.hooks_mut().take_dirty_views();
        apply_unit_views(adapter.battle_mut(), &dirty);
        if let Some(error) = &adapter.hooks().error {
            report.error = Some(format!("tick {tick}: AI: {error:?}"));
            report.ticks = tick;
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
        if let Some(outcome) = adapter.outcome() {
            report.outcome = Some(outcome);
            report.ticks = tick;
            break;
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
        let members: [Option<PartyMember>; 4] = [
            Some(demo_member(500)),
            Some(demo_member(500)),
            Some(demo_member(500)),
            Some(demo_member(500)),
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
/// exactly once. Returns the outcome. (Drives the typed adapter directly;
/// the coordinator wrapper around the default-typed adapter is exercised in
/// the tantalus-game integration tests.)
pub fn run_world_battle(
    world: &mut tantalus_game::FieldWorld,
    entities: &[tantalus_contracts::EntityId],
    scene: &BattleScene,
    pattern: usize,
    script: &[u8],
    base: &dyn Fn(i32) -> tantalus_game::battle::legacy::PlayerBase,
    seed: u64,
) -> Option<BattleOutcome> {
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
        return None;
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
    )?;
    let (data, units) = battle_views(setup.adapter.battle());
    let binary = EventBinary::parse(script).ok()?;
    let driver = BattleScriptDriver::new(
        ScriptTable::from_binary(&binary),
        BattleScriptState::new(data, units),
        Box::new(SplitMix64Random::new(seed)),
        &[],
    );
    let hooks = EnemyAiHooks::new(NoEnemyAi, driver, SceneOverlay::default());
    let mut adapter = setup.adapter.with_hooks(hooks);

    let mut bonus = BattleBonus::new(setup.pattern_ap, 0x0F, false);
    let mut rows = setup.reward_rows.clone();
    let mut deaths = 0usize;
    let mut draw_rng: Box<dyn BattleRandom> = Box::new(SplitMix64Random::new(seed ^ 0xD1AA));
    let attack = tantalus_contracts::ContentId::parse(ATTACK_ACTION).unwrap();
    let mut outcome = None;

    for tick in 0..MAX_TICKS {
        let mut input = tantalus_game::battle::CombatInput::new(Tick::new(tick));
        if let Some(unit_index) = adapter.battle().ready_players.first().copied() {
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
                eprintln!("run_world_battle tick {tick}: {error}");
                return None;
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
        if let Some(done) = adapter.outcome() {
            outcome = Some(done);
            break;
        }
    }

    if outcome == Some(BattleOutcome::Victory) {
        let members: [Option<PartyMember>; 4] = [
            Some(demo_member(500)),
            Some(demo_member(500)),
            Some(demo_member(500)),
            Some(demo_member(500)),
        ];
        let party = PartySnapshot {
            gil: world.gil,
            members,
            items: Vec::new(),
            cards: Vec::new(),
        };
        let reward = BattleReward::from_bonus(&bonus, &party);
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
    outcome
}
