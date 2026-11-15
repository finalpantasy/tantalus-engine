//! Encounter loop integration: a field ENCOUNT event drives the coordinator
//! into a battle, victory commits rewards into field world state exactly once.

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
    };
    let mut services = WorldGameServices::new(&mut world);
    services.commit_rewards(&bundle).unwrap();
    assert_eq!(services.world.items[&12], 1);
    assert!(services.unmapped_items.is_empty());
}
