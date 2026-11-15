//! Save/reload round trips: field snapshots restore world state exactly and
//! rebuild entry programs, marking mid-function entries as restarted.

use std::collections::BTreeMap;
use tantalus_contracts::{EntityId, Tick};
use tantalus_game::{
    FIELD_SNAPSHOT_FORMAT, FieldHost, FieldScene, FieldWorld, restore_scene, snapshot_scene,
};
use tantalus_legacy::{VmStatus, decode_program};

fn actor(id: u64) -> EntityId {
    EntityId::new(id)
}

fn run_until_completed(
    scene: &mut FieldScene,
    world: &mut FieldWorld,
    max_ticks: u64,
) -> (u64, VmStatus) {
    for tick in 0..max_ticks {
        let mut host = FieldHost::new(world);
        let steps = scene.step(Tick::new(tick), &mut host).unwrap();
        world.advance();
        if steps[0].report.status == VmStatus::Completed {
            return (tick, steps[0].report.status);
        }
    }
    panic!("program did not complete");
}

#[test]
fn snapshot_round_trips_world_and_rebuilds_entries() {
    // Gil + item program, then a WAIT-then-gil program whose pc stops past
    // the entry point, exercising the mid-function restart flag.
    let program_a = vec![0xce, 0, 100, 0, 0, 0x48, 0, 12, 0, 3, 4];
    let program_b = vec![0x22, 0, 5, 0xce, 0, 50, 0, 0, 4];
    let mut scene = FieldScene::new();
    scene
        .add_entry(actor(1), decode_program(&program_a).unwrap())
        .unwrap();
    scene
        .add_entry(actor(2), decode_program(&program_b).unwrap())
        .unwrap();
    let mut world = FieldWorld::new();
    world.add_actor(actor(1), [1.0, 2.0, 3.0]);
    world.add_actor(actor(2), [0.0; 3]);

    let mut sources = BTreeMap::new();
    sources.insert(actor(1), program_a.clone());
    sources.insert(actor(2), program_b.clone());

    // Run: entry 1 completes; entry 2 walks one tick then we snapshot mid-walk.
    {
        let mut host = FieldHost::new(&mut world);
        scene.step(Tick::new(0), &mut host).unwrap();
    }
    world.advance();
    let snapshot = snapshot_scene(&scene, &world, &sources).unwrap();
    assert_eq!(snapshot.format, FIELD_SNAPSHOT_FORMAT);
    let entry_a = snapshot
        .entries
        .iter()
        .find(|entry| entry.entity == actor(1))
        .unwrap();
    let entry_b = snapshot
        .entries
        .iter()
        .find(|entry| entry.entity == actor(2))
        .unwrap();
    assert!(!entry_a.resumed_from_start);
    assert!(!entry_b.resumed_from_start);
    assert!(entry_b.wait_state.is_some());

    let json = snapshot.to_json().unwrap();
    let decoded = tantalus_game::FieldSnapshot::from_json(&json).unwrap();
    assert_eq!(decoded, snapshot);

    let (mut restored_scene, mut restored_world) = restore_scene(&decoded).unwrap();
    assert_eq!(restored_world, world);
    assert_eq!(restored_scene.len(), 2);

    // Restored entries resume their recorded positions: entry 1 stays
    // completed (no gil re-application), and entry 2 continues inside its
    // recorded wait, then applies its gil grant exactly once.
    let (tick, status) = run_until_completed(&mut restored_scene, &mut restored_world, 16);
    assert_eq!(status, VmStatus::Completed);
    assert_eq!(tick, 0);
    assert_eq!(restored_world.gil, 100);
    assert_eq!(restored_world.items[&12], 3);

    for tick in 1..12u64 {
        let mut host = FieldHost::new(&mut restored_world);
        let steps = restored_scene.step(Tick::new(tick), &mut host).unwrap();
        restored_world.advance();
        if steps
            .iter()
            .all(|step| step.report.status == VmStatus::Completed)
        {
            break;
        }
    }
    assert_eq!(restored_world.gil, 150);
    assert_eq!(restored_world.items[&12], 3);
}

#[test]
fn snapshot_rejects_unknown_format_and_missing_sources() {
    let scene = FieldScene::new();
    let world = FieldWorld::new();
    let mut bad = snapshot_scene(&scene, &world, &BTreeMap::new()).unwrap();
    bad.format = FIELD_SNAPSHOT_FORMAT + 1;
    let error = bad
        .to_json()
        .ok()
        .and_then(|json| tantalus_game::FieldSnapshot::from_json(&json).err());
    assert!(error.is_some());

    let mut scene = FieldScene::new();
    scene
        .add_entry(actor(1), decode_program(&[4]).unwrap())
        .unwrap();
    let error = snapshot_scene(&scene, &world, &BTreeMap::new()).unwrap_err();
    assert!(format!("{error}").contains("no program source bytes"));
}
