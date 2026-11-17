//! Encounter loop integration: a field ENCOUNT event drives the coordinator
//! into a battle, victory commits rewards into field world state exactly once.

use std::collections::BTreeMap;
use tantalus_contracts::{ContentId, EntityId, Tick};
use tantalus_game::battle::*;
use tantalus_game::field::world::EncounterRequest;
use tantalus_game::{FieldHost, FieldScene, FieldWorld};
use tantalus_legacy::{VmStatus, decode_program};

fn content(name: &str) -> ContentId {
    ContentId::parse(&format!("fixture:{name}")).unwrap()
}

fn attack() -> ActionDefinition {
    ActionDefinition {
        id: content("attack/raptor_claw"),
        power: 50,
        hit_rate: 100,
        mp_cost: 0,
        timeline: ActionTimeline {
            duration_ticks: 2,
            markers: vec![
                ActionMarker {
                    tick_offset: 1,
                    kind: tantalus_contracts::ActionMarkerKind::Hit,
                },
                ActionMarker {
                    tick_offset: 2,
                    kind: tantalus_contracts::ActionMarkerKind::Complete,
                },
            ],
        },
    }
}

fn ruleset() -> CombatRuleset {
    let caster = CombatUnit::new(EntityId::new(1), Team::Player, 500, 20);
    let mut enemy = CombatUnit::new(EntityId::new(2), Team::Enemy, 40, 0);
    enemy.reward = RewardBundle {
        resolution_id: 0,
        experience: 50,
        gil: 20,
        items: vec![],
        shares: None,
    };
    CombatRuleset::new(vec![attack()], vec![caster, enemy]).unwrap()
}

#[test]
fn field_encounter_runs_a_battle_and_commits_rewards_exactly_once() {
    // ENCOUNT 0x2A: rush 0, battle 0x55.
    let program = decode_program(&[0x2a, 0, 0, 0x55, 0x00, 4]).unwrap();
    let mut scene = FieldScene::new();
    scene.add_entry(EntityId::new(1), program).unwrap();
    let mut world = FieldWorld::new();
    world.add_actor(EntityId::new(1), [0.0; 3]);

    {
        let mut host = FieldHost::new(&mut world);
        let steps = scene.step(Tick::new(0), &mut host).unwrap();
        // The battle jump halts the script past the opcode; it resumes on the
        // next field frame after the battle.
        assert_eq!(steps[0].report.status, VmStatus::Waiting);
    }

    let encounters = world.take_encounters();
    assert_eq!(encounters.len(), 1);
    assert_eq!(encounters[0].battle, 0x55);

    let mut coordinator = EncounterCoordinator::new();
    coordinator.begin(encounters[0], ruleset()).unwrap();
    assert!(coordinator.is_active());
    // A second begin while active is a conflict.
    assert!(
        coordinator
            .begin(
                EncounterRequest {
                    rush: 0,
                    group: None,
                    battle: 0x56,
                    steiner_state: false,
                    timer: None,
                },
                ruleset(),
            )
            .is_err()
    );

    let mut outcome = None;
    for tick in 1..64 {
        let mut services = WorldGameServices::new(&mut world);
        let command = BattleCommand {
            actor: EntityId::new(1),
            action: content("attack/raptor_claw"),
            targets: vec![EntityId::new(2)],
        };
        let mut input = CombatInput::new(Tick::new(tick));
        input.command = Some(command);
        outcome = coordinator.advance(&input, &mut services).unwrap();
        if outcome.is_some() {
            break;
        }
    }
    assert_eq!(outcome, Some(BattleOutcome::Victory));

    // Victory committed the enemy reward once into world state.
    assert_eq!(world.gil, 20);

    // A duplicate commit of the same resolution is accepted but not applied.
    let duplicate = RewardBundle {
        resolution_id: 1,
        experience: 50,
        gil: 20,
        items: vec![],
        shares: None,
    };
    {
        let mut services = WorldGameServices::new(&mut world);
        services.commit_rewards(&duplicate).unwrap();
        services.commit_rewards(&duplicate).unwrap();
    }
    assert_eq!(world.gil, 20);
    assert_eq!(world.encounters.len(), 0, "no further encounters queued");

    let finished = coordinator.finish();
    assert_eq!(finished.map(|request| request.battle), Some(0x55));
    assert!(!coordinator.is_active());
}

#[test]
fn reward_items_apply_through_the_legacy_id_mapping() {
    let mut world = tantalus_game::FieldWorld::new();
    world.item_id_map = Some(std::sync::Arc::new(|content: &ContentId| {
        content
            .to_string()
            .strip_prefix("fixture:item/")
            .and_then(|tail| tail.parse::<i32>().ok())
    }));
    let bundle = RewardBundle {
        resolution_id: 7,
        experience: 50,
        gil: 20,
        items: vec![
            ContentId::parse("fixture:item/12").unwrap(),
            ContentId::parse("fixture:unknown/99").unwrap(),
        ],
        shares: None,
    };
    let mut services = WorldGameServices::new(&mut world);
    services.commit_rewards(&bundle).unwrap();
    assert_eq!(services.world.items[&12], 1);
    assert_eq!(services.unmapped_items.len(), 1);
    assert_eq!(services.world.gil, 20);

    // Without a mapping, items are recorded but not applied.
    let mut bare = tantalus_game::FieldWorld::new();
    let mut services = WorldGameServices::new(&mut bare);
    services.commit_rewards(&bundle).unwrap();
    assert!(services.world.items.is_empty());
    assert_eq!(services.unmapped_items.len(), 2);
}

#[test]
fn reward_experience_levels_characters_through_the_growth_provider() {
    let mut world = tantalus_game::FieldWorld::new();
    world.party = vec![0];
    world.character_stats.insert(
        0,
        tantalus_game::field::CharacterStats {
            current_hp: 50,
            max_hp: 100,
            current_mp: 20,
            max_mp: 36,
        },
    );
    world.level_growth = Some(std::sync::Arc::new(|_, total_exp| {
        (total_exp >= 100).then_some(tantalus_game::field::CharacterStats {
            current_hp: 160,
            max_hp: 160,
            current_mp: 50,
            max_mp: 50,
        })
    }));
    let bundle = RewardBundle {
        resolution_id: 8,
        experience: 120,
        gil: 0,
        items: vec![],
        shares: None,
    };
    let mut services = WorldGameServices::new(&mut world);
    services.commit_rewards(&bundle).unwrap();
    drop(services);

    assert_eq!(world.character_exp[&0], 120);
    // Level-up: the max HP/MP delta is added to current values
    // (50 + 60 = 110, 20 + 14 = 34) and then the new stats are stored.
    let stats = world.character_stats[&0];
    assert_eq!(stats.max_hp, 160);
    assert_eq!(stats.current_hp, 110);
    assert_eq!(stats.max_mp, 50);
    assert_eq!(stats.current_mp, 34);

    // Without a provider the experience still accumulates.
    let mut bare = tantalus_game::FieldWorld::new();
    bare.party = vec![0];
    let mut services = WorldGameServices::new(&mut bare);
    services.commit_rewards(&bundle).unwrap();
    drop(services);
    assert_eq!(bare.character_exp[&0], 120);
    assert!(bare.character_stats.is_empty());
}

#[test]
fn default_item_id_map_resolves_vanilla_table_identities() {
    use tantalus_game::field::world::default_item_id_map;

    let map = default_item_id_map([0, 12, 255]);
    assert_eq!(map(&ContentId::parse("ffix:item/12").unwrap()), Some(12));
    assert_eq!(map(&ContentId::parse("ffix:item/0").unwrap()), Some(0));
    // Identities outside the table are unmapped, never guessed.
    assert_eq!(map(&ContentId::parse("ffix:item/77").unwrap()), None);
    assert_eq!(map(&content("item/12")), None);

    // The default map drives real reward application end to end.
    let mut world = tantalus_game::FieldWorld::new();
    world.item_id_map = Some(default_item_id_map([12]));
    let bundle = RewardBundle {
        resolution_id: 9,
        experience: 0,
        gil: 0,
        items: vec![ContentId::parse("ffix:item/12").unwrap()],
        shares: None,
    };
    let mut services = WorldGameServices::new(&mut world);
    services.commit_rewards(&bundle).unwrap();
    assert_eq!(services.world.items[&12], 1);
    assert!(services.unmapped_items.is_empty());
}

#[test]
fn reference_exp_split_divides_across_eligible_party_members() {
    // BattleResultUI.AnalyzeArgument: total / eligible count, remainder lost.
    let all_alive = split_experience(100, &[0, 1, 2], &|_| true);
    assert_eq!(all_alive, BTreeMap::from([(0, 33), (1, 33), (2, 33)]));

    // A dead line drops out and the others split the full total.
    let one_dead = split_experience(100, &[0, 1, 2], &|member| member != 1);
    assert_eq!(one_dead, BTreeMap::from([(0, 50), (2, 50)]));

    // No eligible line: no shares, not a zero-per-line grant.
    assert!(split_experience(100, &[0], &|_| false).is_empty());

    // Shares on the bundle override the full-grant fallback: with shares
    // present only the listed characters receive experience.
    let mut world = tantalus_game::FieldWorld::new();
    world.party = vec![0, 1];
    world.character_stats.insert(
        0,
        tantalus_game::field::CharacterStats {
            current_hp: 10,
            max_hp: 10,
            current_mp: 0,
            max_mp: 0,
        },
    );
    world.character_stats.insert(
        1,
        tantalus_game::field::CharacterStats {
            current_hp: 10,
            max_hp: 10,
            current_mp: 0,
            max_mp: 0,
        },
    );
    let bundle = RewardBundle {
        resolution_id: 10,
        experience: 100,
        gil: 0,
        items: vec![],
        shares: Some(split_experience(100, &world.party.clone(), &|member| {
            world.exp_eligible(member)
        })),
    };
    let mut services = WorldGameServices::new(&mut world);
    services.commit_rewards(&bundle).unwrap();
    drop(services);
    assert_eq!(world.character_exp[&0], 50);
    assert_eq!(world.character_exp[&1], 50);

    // Eligibility through the world: a dead character is excluded, and an
    // empty share set grants nothing even with a party present.
    let mut world = tantalus_game::FieldWorld::new();
    world.party = vec![0, 1];
    world.character_stats.insert(
        0,
        tantalus_game::field::CharacterStats {
            current_hp: 0,
            max_hp: 10,
            current_mp: 0,
            max_mp: 0,
        },
    );
    world.character_stats.insert(
        1,
        tantalus_game::field::CharacterStats {
            current_hp: 10,
            max_hp: 10,
            current_mp: 0,
            max_mp: 0,
        },
    );
    assert!(!world.exp_eligible(0));
    assert!(world.exp_eligible(1));
    let shares = Some(split_experience(100, &world.party.clone(), &|member| {
        world.exp_eligible(member)
    }));
    let bundle = RewardBundle {
        resolution_id: 11,
        experience: 100,
        gil: 0,
        items: vec![],
        shares,
    };
    let mut services = WorldGameServices::new(&mut world);
    services.commit_rewards(&bundle).unwrap();
    drop(services);
    assert_eq!(world.character_exp.get(&0), None);
    assert_eq!(world.character_exp[&1], 100);
}

#[test]
fn legacy_battle_reward_converts_and_commits_through_the_field_path() {
    use tantalus_game::battle::bridge::bundle_from_battle_reward;
    use tantalus_game::field::world::default_item_id_map;
    use tantalus_legacy::battle::rewards::{
        BaseStats, BattleBonus, BattleReward, LevelBonus, PartyMember, PartySnapshot,
    };

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

    // Lines 0 and 2 took part; line 2 is dead, so only line 0 is eligible
    // for the EXP split. Two potions and one ether drop.
    let mut bonus = BattleBonus::new(0, 0b0101, false);
    bonus.gil = 500;
    bonus.exp = 101;
    bonus.items = vec![12, 12, 30];
    let mut party = PartySnapshot {
        gil: 0,
        members: [None, None, None, None],
        items: vec![],
        cards: vec![],
    };
    party.members[0] = Some(member(50));
    party.members[2] = Some(member(0));
    let reward = BattleReward::from_bonus(&bonus, &party);
    assert_eq!(reward.shares[0].exp, Some(101));
    assert_eq!(reward.shares[2].exp, None, "dead line is excluded");

    let bundle = bundle_from_battle_reward(&reward, [Some(0), None, Some(1), None], 42);

    let mut world = tantalus_game::FieldWorld::new();
    world.party = vec![0, 1];
    for character in [0, 1] {
        world.character_stats.insert(
            character,
            tantalus_game::field::CharacterStats {
                current_hp: 50,
                max_hp: 100,
                current_mp: 20,
                max_mp: 36,
            },
        );
    }
    world.item_id_map = Some(default_item_id_map([12, 30]));
    {
        let mut services = WorldGameServices::new(&mut world);
        services.commit_rewards(&bundle).unwrap();
    }

    assert_eq!(world.gil, 500);
    assert_eq!(
        world.character_exp[&0], 101,
        "living character gets the share"
    );
    assert_eq!(
        world.character_exp.get(&1),
        None,
        "mapped-out line gets nothing"
    );
    assert_eq!(
        world.items[&12], 2,
        "stack expanded to one identity per unit"
    );
    assert_eq!(world.items[&30], 1);

    // Converting again with the same resolution id cannot double-pay.
    {
        let mut services = WorldGameServices::new(&mut world);
        services.commit_rewards(&bundle).unwrap();
    }
    assert_eq!(world.gil, 500);
    assert_eq!(world.character_exp[&0], 101);
}

#[test]
fn legacy_battle_reward_with_no_eligible_line_pays_nobody() {
    use tantalus_game::battle::bridge::bundle_from_battle_reward;
    use tantalus_legacy::battle::rewards::{BattleBonus, BattleReward, PartySnapshot};

    let mut bonus = BattleBonus::new(0, 0b0001, false);
    bonus.exp = 200;
    let mut party = PartySnapshot {
        gil: 0,
        members: [None, None, None, None],
        items: vec![],
        cards: vec![],
    };
    party.members[0] = Some(party_member_dead());
    let reward = BattleReward::from_bonus(&bonus, &party);
    let bundle = bundle_from_battle_reward(&reward, [Some(0), None, None, None], 7);

    let mut world = tantalus_game::FieldWorld::new();
    world.party = vec![0];
    world.character_stats.insert(
        0,
        tantalus_game::field::CharacterStats {
            current_hp: 0,
            max_hp: 100,
            current_mp: 0,
            max_mp: 36,
        },
    );
    {
        let mut services = WorldGameServices::new(&mut world);
        services.commit_rewards(&bundle).unwrap();
    }
    assert!(
        world.character_exp.is_empty(),
        "an all-empty share set suppresses the full-grant fallback"
    );
}

fn party_member_dead() -> tantalus_legacy::battle::rewards::PartyMember {
    let base = tantalus_legacy::battle::rewards::BaseStats {
        dex: 23,
        str_: 21,
        mgc: 18,
        wpr: 23,
        gems: 18,
    };
    tantalus_legacy::battle::rewards::PartyMember {
        in_battle: true,
        cur_hp: 0,
        cur_mp: 0,
        rewards_disabled: false,
        level: 1,
        exp: 0,
        base,
        bonus: tantalus_legacy::battle::rewards::LevelBonus::default(),
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
