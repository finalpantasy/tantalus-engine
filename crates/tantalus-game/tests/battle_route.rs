//! Full battle reward route: real draw RNG (`SetBonus`) -> reference split
//! (`from_bonus`) -> field bridge (`bundle_from_battle_reward`) -> exactly-once
//! world commit, with AP learning alongside. Field world stays the state
//! owner throughout.

use tantalus_contracts::ContentId;
use tantalus_game::FieldWorld;
use tantalus_game::battle::bridge::bundle_from_battle_reward;
use tantalus_game::battle::{GameServices, RewardBundle, WorldGameServices};
use tantalus_game::field::world::default_item_id_map;
use tantalus_legacy::battle::rewards::{
    BaseStats, BattleBonus, EnemyRewardData, LevelBonus, PartyMember, PartySnapshot,
};
use tantalus_legacy::battle::rng::{Draw, SequenceRandom};
use tantalus_legacy::shop::NO_ITEM as NO_SHOP_ITEM;

const NO_ITEM: u32 = 255;

fn content(name: &str) -> ContentId {
    ContentId::parse(&format!("fixture:{name}")).unwrap()
}

fn member(hp: u32) -> PartyMember {
    let base = BaseStats {
        dex: 23,
        str_: 21,
        mgc: 18,
        wpr: 23,
        gems: 18,
    };
    PartyMember {
        in_battle: true,
        cur_hp: hp,
        cur_mp: 36,
        rewards_disabled: false,
        level: 1,
        exp: 0,
        base,
        bonus: LevelBonus::default(),
        basis: [base.dex, base.str_, base.mgc, base.wpr],
        max_hp: 250,
        max_mp: 36,
        max_capa: u32::from(base.gems),
        cur_capa: u32::from(base.gems),
        elem: [base.dex, base.str_, base.mgc, base.wpr],
        equip_stat_bonus: [0; 4],
        equipped_abilities: vec![],
        learnable: vec![],
        pa: vec![],
    }
}

#[test]
fn drawn_battle_rewards_pay_the_field_world_through_the_reference_split() {
    // Enemy A only has a common drop (slot 0, rate 200); enemy B has a rare
    // slot 1 (rate 96) and a common slot 0 with rate 0 (drawn, never hit).
    let mut enemies = vec![
        EnemyRewardData {
            type_id: 1,
            present: true,
            gil: 200,
            exp: 240,
            items: [12, NO_ITEM, NO_ITEM, NO_ITEM],
            item_rates: [200, 0, 0, 0],
            card: NO_ITEM,
            card_rate: 0,
            bonus_given: false,
        },
        EnemyRewardData {
            type_id: 2,
            present: true,
            gil: 100,
            exp: 60,
            items: [NO_SHOP_ITEM as u32, 13, NO_ITEM, NO_ITEM],
            item_rates: [0, 96, 0, 0],
            card: NO_ITEM,
            card_rate: 0,
            bonus_given: false,
        },
    ];
    let mut bonus = BattleBonus::new(3, 0b0011, false);
    // Draws: A slot0 (10 < 200 -> drop 12); B slot1 (50 < 96 -> drop 13),
    // B slot0 (99, rate 0 -> no drop).
    let draws = [
        (Draw::Random8, 10),
        (Draw::Random8, 50),
        (Draw::Random8, 99),
    ];
    let mut rng = SequenceRandom::with_kinds(&draws);
    assert!(bonus.set_bonus(&mut enemies, 0, &mut rng));
    assert!(bonus.set_bonus(&mut enemies, 1, &mut rng));
    assert_eq!(bonus.gil, 300);
    assert_eq!(bonus.exp, 300);
    assert_eq!(bonus.ap, 3);
    assert_eq!(bonus.items, vec![12, 13]);

    // Party state mirrors the field world: lines 0 and 1 alive and taking
    // part; both carry a learnable ability for the AP award.
    let mut party = PartySnapshot {
        gil: 100,
        members: [None, None, None, None],
        items: vec![],
        cards: vec![],
    };
    for line in 0..2usize {
        let mut member = member(200);
        member.learnable = vec![tantalus_legacy::battle::rewards::PaEntry {
            id: 200 + line as i32,
            max_ap: 2,
        }];
        member.pa = vec![0];
        party.members[line] = Some(member);
    }

    let reward = tantalus_legacy::battle::rewards::BattleReward::from_bonus(&bonus, &party);
    // Reference split: 300 EXP over two eligible lines, remainder lost.
    assert_eq!(reward.shares[0].exp, Some(150));
    assert_eq!(reward.shares[1].exp, Some(150));
    assert_eq!(reward.shares[0].ap, Some(3));
    assert_eq!(reward.gil, 300);
    assert_eq!(reward.items, vec![(12, 1), (13, 1)]);

    // Field world: the party, its inventory mapping, and the growth provider.
    let mut world = FieldWorld::new();
    world.party = vec![0, 1];
    for character in [0, 1] {
        world.character_stats.insert(
            character,
            tantalus_game::field::CharacterStats {
                current_hp: 200,
                max_hp: 250,
                current_mp: 36,
                max_mp: 36,
            },
        );
    }
    world.gil = 100;
    world.item_id_map = Some(default_item_id_map([12, 13]));
    world.level_growth = Some(std::sync::Arc::new(|_, total| {
        (total >= 150).then_some(tantalus_game::field::CharacterStats {
            current_hp: 200,
            max_hp: 300,
            current_mp: 36,
            max_mp: 40,
        })
    }));

    let bundle: RewardBundle =
        bundle_from_battle_reward(&reward, [Some(0), Some(1), None, None], 1);
    {
        let mut services = WorldGameServices::new(&mut world);
        services.commit_rewards(&bundle).unwrap();
        // AP applies through the per-ability primitive after the commit.
        assert!(services.world.apply_ap(0, 200, 2, 3));
        assert!(services.world.apply_ap(1, 201, 2, 3));
    }

    assert_eq!(world.gil, 400);
    assert_eq!(
        world.character_exp,
        std::collections::BTreeMap::from([(0, 150), (1, 150)])
    );
    // The growth provider leveled both characters to the next stats.
    assert_eq!(world.character_stats[&0].max_hp, 300);
    assert_eq!(world.items[&12], 1, "drawn common drop lands");
    assert_eq!(world.items[&13], 1, "drawn rare drop lands");
    assert!(world.learned_abilities[&0].contains(&200));
    assert!(world.learned_abilities[&1].contains(&201));

    // A repeated commit of the same resolution pays nothing.
    {
        let mut services = WorldGameServices::new(&mut world);
        services.commit_rewards(&bundle).unwrap();
    }
    assert_eq!(world.gil, 400);
    assert_eq!(world.character_exp[&0], 150);
}

#[test]
fn card_drops_are_recorded_not_dropped() {
    // The bridge documents cards as unmodeled; the reward keeps them for the
    // Tetra collection without touching field state.
    let mut bonus = BattleBonus::new(0, 0b0001, false);
    bonus.exp = 10;
    let mut party = PartySnapshot {
        gil: 0,
        members: [None, None, None, None],
        items: vec![],
        cards: vec![],
    };
    party.members[0] = Some(member(100));
    let reward = tantalus_legacy::battle::rewards::BattleReward::from_bonus(&bonus, &party);
    assert_eq!(reward.card, None, "no card in the bonus");

    let bundle = bundle_from_battle_reward(&reward, [Some(0), None, None, None], 2);
    assert!(
        bundle.items.is_empty(),
        "no items means nothing to map: {}",
        bundle
            .items
            .iter()
            .map(ContentId::as_str)
            .collect::<Vec<_>>()
            .join(",")
    );
}

#[test]
fn field_encounter_drives_the_legacy_machine_to_victory_and_pays_rewards() {
    use std::collections::BTreeMap;
    use tantalus_contracts::EntityId;
    use tantalus_game::battle::legacy::{LegacyBattleAdapter, unit_mappings};
    use tantalus_game::battle::{BattleCommand, EncounterCoordinator, GameServices};
    use tantalus_game::field::world::{EncounterRequest, default_item_id_map};
    use tantalus_legacy::battle::machine::{ABILITY_ATTACK, Battle as LegacyBattle, BattleConfig};
    use tantalus_legacy::battle::queue::cmd;
    use tantalus_legacy::battle::rewards::{BattleBonus, BattleReward, PartyMember, PartySnapshot};
    use tantalus_legacy::battle::rng::{Draw, SequenceRandom};
    use tantalus_legacy::battle::{BattleUnit, StatusDataRow, StatusTable, WeaponStats};

    // Machine battle: one armed player, one fragile enemy.
    let mut player = BattleUnit::new(1, true);
    player.level = 20;
    player.cur_hp = 500;
    player.max_hp = 500;
    player.strength = 30;
    player.dexterity = 30;
    player.weapon = Some(WeaponStats {
        power: 25,
        rate: 0,
        elements: tantalus_legacy::battle::Elements::NONE,
        category: WeaponStats::SHORT_RANGE,
        status: 0,
    });
    let mut enemy = BattleUnit::new(16, false);
    enemy.level = 1;
    enemy.cur_hp = 60;
    enemy.max_hp = 60;
    enemy.strength = 5;
    enemy.magic = 5;
    enemy.dexterity = 5;
    enemy.will = 10;
    let battle = LegacyBattle::new(
        vec![player, enemy],
        BattleConfig::default(),
        StatusTable::new(vec![StatusDataRow::default(); 32]),
    );
    let player_entity = EntityId::new(101);
    let enemy_entity = EntityId::new(102);
    let (unit_map, target_bits) = unit_mappings(&[(player_entity, 1), (enemy_entity, 16)]);
    let action_map = BTreeMap::from([(content("action/attack"), (cmd::ATTACK, ABILITY_ATTACK))]);
    let adapter = LegacyBattleAdapter::new(battle, action_map, unit_map, target_bits);

    // Field: an ENCOUNT request begins the battle through the coordinator.
    let mut coordinator = EncounterCoordinator::new();
    coordinator
        .begin(
            EncounterRequest {
                rush: 0,
                group: None,
                battle: 0x55,
                steiner_state: false,
            },
            adapter,
        )
        .unwrap();

    let mut world = FieldWorld::new();
    world.party = vec![0];
    world.gil = 50;
    world.item_id_map = Some(default_item_id_map([12]));
    world.character_stats.insert(
        0,
        tantalus_game::field::CharacterStats {
            current_hp: 500,
            max_hp: 500,
            current_mp: 100,
            max_mp: 100,
        },
    );

    // Death-driven rewards: each machine death notification pays that
    // enemy's bonus row into the shared bonus, like the reference's
    // `SetBonus` on kill.
    let mut bonus = BattleBonus::new(2, 0b0001, false);
    let mut enemies = vec![tantalus_legacy::battle::rewards::EnemyRewardData {
        type_id: 0,
        present: true,
        gil: 100,
        exp: 80,
        items: [12, 255, 255, 255],
        item_rates: [256, 0, 0, 0],
        card: 255,
        card_rate: 0,
        bonus_given: false,
    }];
    let mut rng = SequenceRandom::with_kinds(&[(Draw::Random8, 0)]);
    let mut deaths = 0usize;

    let mut outcome = None;
    for tick in 0..4000u64 {
        let mut services = WorldGameServices::new(&mut world);
        let mut input =
            tantalus_game::battle::CombatInput::new(tantalus_contracts::Tick::new(tick));
        input.command = Some(BattleCommand {
            actor: player_entity,
            action: content("action/attack"),
            targets: vec![enemy_entity],
        });
        if let Some(done) = coordinator.advance(&input, &mut services).unwrap() {
            outcome = Some(done);
        }
        drop(services);
        if let Some(tantalus_game::battle::AnyRuleset::Legacy(adapter)) =
            coordinator.active_ruleset_mut()
        {
            for event in adapter.take_events() {
                if let tantalus_game::battle::CombatEvent::UnitDefeated { .. } = event {
                    assert!(bonus.set_bonus(&mut enemies, deaths, &mut rng));
                    deaths += 1;
                }
            }
        }
        if outcome.is_some() {
            break;
        }
    }
    assert_eq!(outcome, Some(tantalus_game::battle::BattleOutcome::Victory));
    assert_eq!(deaths, 1, "the one enemy died exactly once");

    let mut member: PartyMember = {
        let base = tantalus_legacy::battle::rewards::BaseStats {
            dex: 23,
            str_: 21,
            mgc: 18,
            wpr: 23,
            gems: 18,
        };
        PartyMember {
            in_battle: true,
            cur_hp: 500,
            cur_mp: 100,
            rewards_disabled: false,
            level: 1,
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
    };
    member.learnable = vec![tantalus_legacy::battle::rewards::PaEntry { id: 7, max_ap: 2 }];
    member.pa = vec![0];
    let party = PartySnapshot {
        gil: 50,
        members: [Some(member), None, None, None],
        items: vec![],
        cards: vec![],
    };
    let reward = BattleReward::from_bonus(&bonus, &party);
    let bundle = tantalus_game::battle::bridge::bundle_from_battle_reward(
        &reward,
        [Some(0), None, None, None],
        1,
    );
    {
        let mut services = WorldGameServices::new(&mut world);
        services.commit_rewards(&bundle).unwrap();
    }
    assert_eq!(world.gil, 150);
    assert_eq!(world.character_exp[&0], 80);
    assert_eq!(world.items[&12], 1, "drawn drop lands in field state");

    coordinator.finish().unwrap();
}

#[test]
fn legacy_battle_replays_identically_from_identical_world_state() {
    use std::collections::BTreeMap;
    use tantalus_contracts::EntityId;
    use tantalus_game::battle::legacy::{LegacyBattleAdapter, unit_mappings};
    use tantalus_game::battle::{BattleCommand, EncounterCoordinator};
    use tantalus_legacy::battle::machine::{ABILITY_ATTACK, Battle as LegacyBattle, BattleConfig};
    use tantalus_legacy::battle::queue::cmd;
    use tantalus_legacy::battle::{BattleUnit, StatusDataRow, StatusTable, WeaponStats};

    fn run() -> (u64, u32) {
        let mut player = BattleUnit::new(1, true);
        player.level = 20;
        player.cur_hp = 500;
        player.max_hp = 500;
        player.strength = 30;
        player.dexterity = 30;
        player.will = 20;
        player.weapon = Some(WeaponStats {
            power: 25,
            rate: 0,
            elements: tantalus_legacy::battle::Elements::NONE,
            category: WeaponStats::SHORT_RANGE,
            status: 0,
        });
        let mut enemy = BattleUnit::new(16, false);
        enemy.cur_hp = 60;
        enemy.max_hp = 60;
        enemy.will = 10;
        let adapter = LegacyBattleAdapter::new(
            LegacyBattle::new(
                vec![player, enemy],
                BattleConfig::default(),
                StatusTable::new(vec![StatusDataRow::default(); 32]),
            ),
            BTreeMap::from([(content("action/attack"), (cmd::ATTACK, ABILITY_ATTACK))]),
            unit_mappings(&[(EntityId::new(101), 1), (EntityId::new(102), 16)]).0,
            unit_mappings(&[(EntityId::new(101), 1), (EntityId::new(102), 16)]).1,
        );
        let mut coordinator = EncounterCoordinator::new();
        coordinator
            .begin(
                tantalus_game::field::world::EncounterRequest {
                    rush: 0,
                    group: None,
                    battle: 0x55,
                    steiner_state: false,
                },
                adapter,
            )
            .unwrap();
        // The world RNG backs the machine's draws; gil marks world state.
        let mut world = FieldWorld::new();
        world.gil = 7;
        for tick in 0..4000u64 {
            let mut services = WorldGameServices::new(&mut world);
            let mut input =
                tantalus_game::battle::CombatInput::new(tantalus_contracts::Tick::new(tick));
            input.command = Some(BattleCommand {
                actor: EntityId::new(101),
                action: content("action/attack"),
                targets: vec![EntityId::new(102)],
            });
            if coordinator
                .advance(&input, &mut services)
                .unwrap()
                .is_some()
            {
                return (tick, world.gil);
            }
        }
        panic!("battle did not end");
    }

    assert_eq!(run(), run(), "identical world state replays identically");
}

#[test]
fn adapter_runs_with_script_driven_enemy_ai_hooks() {
    use std::collections::BTreeMap;
    use tantalus_contracts::EntityId;
    use tantalus_game::battle::legacy::{LegacyBattleAdapter, NoEnemyAi, unit_mappings};
    use tantalus_game::battle::{BattleCommand, EncounterCoordinator};
    use tantalus_game::field::world::EncounterRequest;
    use tantalus_legacy::battle::host::battle_views;
    use tantalus_legacy::battle::machine::{ABILITY_ATTACK, Battle as LegacyBattle, BattleConfig};
    use tantalus_legacy::battle::queue::cmd;
    use tantalus_legacy::battle::rng::SplitMix64Random;
    use tantalus_legacy::battle::script_host::{
        BattleScriptDriver, BattleScriptState, EnemyAiHooks, SceneOverlay, ScriptTable,
    };
    use tantalus_legacy::battle::{BattleUnit, StatusDataRow, StatusTable, WeaponStats};

    let mut player = BattleUnit::new(1, true);
    player.character_id = 0;
    player.level = 20;
    player.cur_hp = 500;
    player.max_hp = 500;
    player.strength = 30;
    player.dexterity = 30;
    player.will = 20;
    player.weapon = Some(WeaponStats {
        power: 25,
        rate: 0,
        elements: tantalus_legacy::battle::Elements::NONE,
        category: WeaponStats::SHORT_RANGE,
        status: 0,
    });
    let mut enemy = BattleUnit::new(16, false);
    enemy.cur_hp = 60;
    enemy.max_hp = 60;
    enemy.will = 10;
    let battle = LegacyBattle::new(
        vec![player, enemy],
        BattleConfig::default(),
        StatusTable::new(vec![StatusDataRow::default(); 32]),
    );
    let (data, units) = battle_views(&battle);
    let driver = BattleScriptDriver::new(
        ScriptTable { entries: vec![] },
        BattleScriptState::new(data, units),
        Box::new(SplitMix64Random::new(7)),
        &[0],
    );
    let ai_hooks = EnemyAiHooks::new(NoEnemyAi, driver, SceneOverlay::default());
    let adapter = LegacyBattleAdapter::new(
        battle,
        BTreeMap::from([(content("action/attack"), (cmd::ATTACK, ABILITY_ATTACK))]),
        unit_mappings(&[(EntityId::new(101), 1), (EntityId::new(102), 16)]).0,
        unit_mappings(&[(EntityId::new(101), 1), (EntityId::new(102), 16)]).1,
    )
    .with_hooks(ai_hooks);

    let mut coordinator = EncounterCoordinator::new();
    coordinator
        .begin(
            EncounterRequest {
                rush: 0,
                group: None,
                battle: 0x55,
                steiner_state: false,
            },
            adapter,
        )
        .unwrap();
    let mut world = FieldWorld::new();
    let mut outcome = None;
    for tick in 0..4000u64 {
        let mut services = WorldGameServices::new(&mut world);
        let mut input =
            tantalus_game::battle::CombatInput::new(tantalus_contracts::Tick::new(tick));
        input.command = Some(BattleCommand {
            actor: EntityId::new(101),
            action: content("action/attack"),
            targets: vec![EntityId::new(102)],
        });
        if let Some(done) = coordinator.advance(&input, &mut services).unwrap() {
            outcome = Some(done);
            break;
        }
    }
    assert_eq!(
        outcome,
        Some(tantalus_game::battle::BattleOutcome::Victory),
        "script-driven AI hooks run the battle to victory"
    );
}

#[test]
fn escape_family_results_map_to_distinct_outcomes() {
    use std::collections::BTreeMap;
    use tantalus_contracts::EntityId;
    use tantalus_game::battle::BattleOutcome;
    use tantalus_game::battle::legacy::{LegacyBattleAdapter, unit_mappings};
    use tantalus_legacy::battle::machine::{Battle as LegacyBattle, BattleConfig};
    use tantalus_legacy::battle::{BattleUnit, StatusDataRow, StatusTable};

    fn forced_outcome(result: u8) -> BattleOutcome {
        let mut player = BattleUnit::new(1, true);
        player.will = 10;
        let mut enemy = BattleUnit::new(16, false);
        enemy.will = 10;
        let mut battle = LegacyBattle::new(
            vec![player, enemy],
            BattleConfig::default(),
            StatusTable::new(vec![StatusDataRow::default(); 32]),
        );
        battle.result = Some(result);
        battle.over = true;
        let mut adapter = LegacyBattleAdapter::new(
            battle,
            BTreeMap::new(),
            unit_mappings(&[(EntityId::new(101), 1)]).0,
            unit_mappings(&[(EntityId::new(101), 1)]).1,
        );
        let mut world = FieldWorld::new();
        let mut services = WorldGameServices::new(&mut world);
        let input = tantalus_game::battle::CombatInput::new(tantalus_contracts::Tick::new(0));
        adapter
            .advance(&input, &mut services)
            .unwrap()
            .outcome
            .unwrap()
    }

    assert_eq!(forced_outcome(1), BattleOutcome::Victory);
    assert_eq!(forced_outcome(3), BattleOutcome::Defeat);
    assert_eq!(forced_outcome(4), BattleOutcome::Escape);
    assert_eq!(forced_outcome(7), BattleOutcome::EnemyFlee);
}

#[test]
fn flee_ability_runs_the_escape_sequence_to_the_escape_outcome() {
    use std::collections::BTreeMap;
    use tantalus_contracts::EntityId;
    use tantalus_game::battle::legacy::{LegacyBattleAdapter, unit_mappings};
    use tantalus_game::battle::BattleOutcome;
    use tantalus_legacy::battle::machine::{Battle as LegacyBattle, BattleConfig};
    use tantalus_legacy::battle::{BattleUnit, StatusDataRow, StatusTable};

    let mut player = BattleUnit::new(1, true);
    player.will = 20;
    player.dexterity = 30;
    player.cur_hp = 500;
    player.max_hp = 500;
    let mut enemy = BattleUnit::new(16, false);
    enemy.will = 10;
    enemy.cur_hp = 60;
    enemy.max_hp = 60;
    let config = BattleConfig {
        runaway: true,
        ..BattleConfig::default()
    };
    let mut adapter = LegacyBattleAdapter::new(
        LegacyBattle::new(
            vec![player, enemy],
            config,
            StatusTable::new(vec![StatusDataRow::default(); 32]),
        ),
        BTreeMap::new(),
        unit_mappings(&[(EntityId::new(101), 1)]).0,
        unit_mappings(&[(EntityId::new(101), 1)]).1,
    );
    let mut world = FieldWorld::new();
    assert!(
        adapter.flee_ability().unwrap(),
        "the flee ability queues SysEscape"
    );

    let mut outcome = None;
    for tick in 0..600u64 {
        let mut services = WorldGameServices::new(&mut world);
        let input = tantalus_game::battle::CombatInput::new(tantalus_contracts::Tick::new(tick));
        if let Some(done) = adapter.advance(&input, &mut services).unwrap().outcome {
            outcome = Some(done);
            break;
        }
    }
    assert_eq!(
        outcome,
        Some(BattleOutcome::Escape),
        "SysEscape ends the battle with the escape result"
    );
    // Nothing is paid on escape: gil is untouched and no items land.
    assert_eq!(world.gil, 0);
    assert!(world.items.is_empty());
}
