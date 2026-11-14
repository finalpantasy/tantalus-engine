use std::collections::BTreeSet;
use std::path::PathBuf;
use tantalus_contracts::{ContentId, Point3, Tick};
use tantalus_core::Registry;
use tantalus_game::world::*;
use tantalus_mods::PackageLoader;

fn population() -> WorldPopulation {
    let package_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/synthetic/raptor_package");
    let staged = PackageLoader::stage(&package_path).unwrap();
    let mut registry = Registry::new();
    registry.apply(staged.patch).unwrap();
    WorldPopulation::from_registry(&registry, staged.manifest.world.spawn_regions).unwrap()
}

fn update(tick: u64, active: bool, player: Point3, visible: bool) -> WorldUpdate {
    WorldUpdate {
        tick: Tick::new(tick),
        active_cells: if active {
            BTreeSet::from([ContentId::parse("my_mod:cell/eastern_marsh").unwrap()])
        } else {
            BTreeSet::new()
        },
        player: PlayerObservation {
            position: player,
            visible,
        },
    }
}

#[test]
fn actors_spawn_patrol_pursue_and_request_one_encounter() {
    let mut population = population();
    let mut commands = population
        .update(&update(
            0,
            true,
            Point3 {
                x: 10_000,
                y: 0,
                z: 0,
            },
            false,
        ))
        .unwrap();
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, WorldCommand::SpawnActor { .. }))
    );
    let actor = population.actors().next().unwrap().clone();

    commands = population
        .update(&update(
            1,
            true,
            Point3 {
                x: 2_000,
                y: 0,
                z: 0,
            },
            true,
        ))
        .unwrap();
    assert!(
        commands.iter().any(|command| {
            matches!(
                command,
                WorldCommand::SetBehavior {
                    behavior: ActorBehaviorState::Pursue,
                    ..
                }
            )
        }),
        "commands were {commands:?}"
    );
    assert!(population.actor(actor.id).unwrap().position.x > 0);

    commands = population
        .update(&update(2, true, Point3::default(), true))
        .unwrap();
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, WorldCommand::EncounterRequested { .. }))
    );
    assert!(population.actor(actor.id).unwrap().encounter_pending);
    assert!(
        population
            .update(&update(3, true, Point3::default(), true))
            .unwrap()
            .iter()
            .all(|command| !matches!(command, WorldCommand::EncounterRequested { .. }))
    );
}

#[test]
fn unload_reload_and_respawn_preserve_actor_identity() {
    let mut population = population();
    population
        .update(&update(
            0,
            true,
            Point3 {
                x: 10_000,
                y: 0,
                z: 0,
            },
            false,
        ))
        .unwrap();
    let actor = population.actors().next().unwrap().id;

    population
        .update(&update(1, false, Point3::default(), false))
        .unwrap();
    assert!(!population.actor(actor).unwrap().active);
    let second_unload = population
        .update(&update(2, false, Point3::default(), false))
        .unwrap();
    assert!(
        !second_unload
            .iter()
            .any(|command| matches!(command, WorldCommand::DespawnActor { .. }))
    );

    population
        .update(&update(3, true, Point3::default(), false))
        .unwrap();
    assert!(population.actor(actor).unwrap().active);
    assert_eq!(population.actors().count(), 1);

    population
        .record_outcome(actor, EncounterOutcome::Defeated, Tick::new(4))
        .unwrap();
    assert_eq!(
        population.actor(actor).unwrap().respawn_at,
        Some(Tick::new(24))
    );
    population
        .update(&update(5, true, Point3::default(), false))
        .unwrap();
    assert_eq!(
        population.actor(actor).unwrap().lifecycle,
        ActorLifecycle::Defeated
    );

    population
        .update(&update(23, true, Point3::default(), false))
        .unwrap();
    assert_eq!(
        population.actor(actor).unwrap().lifecycle,
        ActorLifecycle::Defeated
    );
    let respawn = population
        .update(&update(24, true, Point3::default(), false))
        .unwrap();
    assert!(respawn.iter().any(
        |command| matches!(command, WorldCommand::SpawnActor { actor: id, .. } if *id == actor)
    ));
    assert_eq!(
        population.actor(actor).unwrap().lifecycle,
        ActorLifecycle::Alive
    );
    assert_eq!(population.actors().count(), 1);
}

#[test]
fn cancelled_and_fled_encounters_release_ownership_without_defeat() {
    let mut population = population();
    population
        .update(&update(0, true, Point3::default(), true))
        .unwrap();
    let actor = population.actors().next().unwrap().id;
    assert!(population.actor(actor).unwrap().encounter_pending);

    population
        .record_outcome(actor, EncounterOutcome::Cancelled, Tick::new(1))
        .unwrap();
    assert!(!population.actor(actor).unwrap().encounter_pending);
    assert_eq!(
        population.actor(actor).unwrap().lifecycle,
        ActorLifecycle::Alive
    );

    population
        .record_outcome(actor, EncounterOutcome::PlayerFled, Tick::new(2))
        .unwrap();
    assert_eq!(
        population.actor(actor).unwrap().lifecycle,
        ActorLifecycle::Alive
    );
}

#[test]
fn identical_world_inputs_produce_identical_commands() {
    let mut left = population();
    let mut right = population();
    for tick in 0..8 {
        let input = update(
            tick,
            tick != 5,
            Point3 {
                x: tick as i32 * 300,
                y: 0,
                z: 0,
            },
            true,
        );
        assert_eq!(left.update(&input).unwrap(), right.update(&input).unwrap());
    }
}

#[test]
fn perception_respects_visibility_and_movement_does_not_overshoot() {
    let behavior = BehaviorDefinition {
        id: ContentId::parse("fixture:behavior/test").unwrap(),
        disposition: Disposition::Aggressive,
        patrol_radius: 0,
        sight_range: 100,
        hearing_range: 50,
        pursuit_speed: 10,
        leash_range: 200,
        encounter_range: 20,
    };
    let actor = Point3::default();
    assert!(
        perceive(
            actor,
            &PlayerObservation {
                position: Point3 { x: 80, y: 0, z: 0 },
                visible: true
            },
            &behavior
        )
        .noticed_player
    );
    assert!(
        !perceive(
            actor,
            &PlayerObservation {
                position: Point3 { x: 80, y: 0, z: 0 },
                visible: false
            },
            &behavior
        )
        .noticed_player
    );
    assert!(
        perceive(
            actor,
            &PlayerObservation {
                position: Point3 { x: 40, y: 0, z: 0 },
                visible: false
            },
            &behavior
        )
        .noticed_player
    );

    let target = Point3 { x: 5, y: 0, z: 0 };
    assert_eq!(move_toward(actor, target, 10), target);
}
