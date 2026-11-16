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

/// Four stand-in party members (ids 1, 2, 4, 8). `weak` swaps the armed
/// level-20 default for an unarmed level-8 party so sweeps can exercise
/// mass-defeat paths (decided-defeat gate, reward suppression) instead of
/// only the endgame superboss wipes.
fn demo_players(weak: bool) -> Vec<BattleUnit> {
    [1u16, 2, 4, 8]
        .iter()
        .map(|id| {
            let mut unit = BattleUnit::new(*id, true);
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
            } else {
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
) -> BattleRunReport {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_scene_battle(scene_name, scene, pattern, script, seed, weak_party)
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
    // `SB2_HEAD` 0x800 (`after_event`): story battles pay nothing.
    let event = scene.header.flags & 0x800 != 0;
    let mut bonus = BattleBonus::new(pattern_ap, 0x0F, event);
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
pub fn run_world_battle(
    world: &mut tantalus_game::FieldWorld,
    entities: &[tantalus_contracts::EntityId],
    scene: &BattleScene,
    pattern: usize,
    script: &[u8],
    base: &dyn Fn(i32) -> tantalus_game::battle::legacy::PlayerBase,
    seed: u64,
) -> Result<Option<BattleOutcome>, String> {
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
    let driver = BattleScriptDriver::new(
        ScriptTable::from_binary(&binary),
        BattleScriptState::new(data, units),
        Box::new(SplitMix64Random::new(seed)),
        &[],
    );
    let hooks = EnemyAiHooks::new(NoEnemyAi, driver, SceneOverlay::default());
    let mut adapter = setup.adapter.with_hooks(hooks);

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
        if let Some(done) = adapter.outcome() {
            outcome = Some(done);
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
    Ok(outcome)
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
    let mut world = FieldWorld::new();
    world.party = vec![1, 2, 4, 8];
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
    match run_world_battle(&mut world, &entities, scene, pattern, script, &base, seed) {
        Ok(Some(outcome)) => {
            report.outcome = Some(outcome);
            report.bonus_gil = world.gil;
            report.bonus_exp = world.character_exp.values().copied().sum::<u64>() as u32;
        }
        Ok(None) => {}
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
) -> BattleRunReport {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_world_battle_report(scene_name, scene, pattern, script, seed, weak_party)
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
