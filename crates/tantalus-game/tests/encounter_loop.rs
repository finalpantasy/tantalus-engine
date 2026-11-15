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
