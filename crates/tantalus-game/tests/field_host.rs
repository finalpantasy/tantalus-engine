//! Field host integration tests: decoded event programs drive game-side world
//! state through the shared `LegacyEventHost` bridge.

use tantalus_contracts::{EntityId, Tick};
use tantalus_game::{FieldHost, FieldScene, FieldWorld};
use tantalus_legacy::{LegacyEventHost, VmStatus, decode_program};

fn actor(id: u64) -> EntityId {
    EntityId::new(id)
}

#[test]
fn walk_command_moves_the_actor_and_the_wait_resolves_on_arrival() {
    // MOVE 0x23 to (64, 0): flags 0, x=64, z=0; then end.
    let program = decode_program(&[0x23, 0, 64, 0, 0, 0, 4]).unwrap();
    let mut scene = FieldScene::new();
    scene.add_entry(actor(1), program).unwrap();
    let mut world = FieldWorld::new();
    world.add_actor(actor(1), [0.0, 0.0, 0.0]);

    for tick in 0..8 {
        let mut host = FieldHost::new(&mut world);
        let steps = scene.step(Tick::new(tick), &mut host).unwrap();
        world.advance();
        if steps[0].report.status == VmStatus::Completed {
            assert_eq!(world.actors[&actor(1)].position, [64.0, 0.0, 0.0]);
            return;
        }
    }
    panic!("walk program did not complete");
}

#[test]
fn window_waits_until_the_director_closes_the_window() {
    // WindowSync 0x1F: window 3, flags 0, text 7; then end.
    let program = decode_program(&[0x1f, 0, 3, 0, 7, 0, 4]).unwrap();
    let mut scene = FieldScene::new();
    scene.add_entry(actor(1), program).unwrap();
    let mut world = FieldWorld::new();
    world.add_actor(actor(1), [0.0; 3]);

    {
        let mut host = FieldHost::new(&mut world);
        let steps = scene.step(Tick::new(0), &mut host).unwrap();
        assert_eq!(steps[0].report.status, VmStatus::Waiting);
    }
    assert!(world.windows[&3]);

    world.close_window(3);
    let mut host = FieldHost::new(&mut world);
    let steps = scene.step(Tick::new(1), &mut host).unwrap();
    assert_eq!(steps[0].report.status, VmStatus::Completed);
}

#[test]
fn gil_and_items_account_through_host_events() {
    // AddGil 0xCE with a 24-bit amount of 100; AddItem 0x48 item 12 count 3.
    let program = decode_program(&[0xce, 0, 100, 0, 0, 0x48, 0, 12, 0, 3, 4]).unwrap();
    let mut scene = FieldScene::new();
    scene.add_entry(actor(1), program).unwrap();
    let mut world = FieldWorld::new();
    world.add_actor(actor(1), [0.0; 3]);

    let mut host = FieldHost::new(&mut world);
    let steps = scene.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(steps[0].report.status, VmStatus::Completed);
    assert_eq!(world.gil, 100);
    assert_eq!(world.items[&12], 3);
}

#[test]
fn encount_records_an_encounter_request_for_the_coordinator() {
    // ENCOUNT 0x2A: rush 1, battle 0x1234.
    let program = decode_program(&[0x2a, 0, 1, 0x34, 0x12, 4]).unwrap();
    let mut scene = FieldScene::new();
    scene.add_entry(actor(1), program).unwrap();
    let mut world = FieldWorld::new();
    world.add_actor(actor(1), [0.0; 3]);

    let mut host = FieldHost::new(&mut world);
    scene.step(Tick::new(0), &mut host).unwrap();
    let encounters = world.take_encounters();
    assert_eq!(encounters.len(), 1);
    assert_eq!(encounters[0].rush, 1);
    assert_eq!(encounters[0].battle, 0x1234);
    assert!(!encounters[0].steiner_state);
    assert!(world.take_encounters().is_empty());
}

#[test]
fn jump_interpolates_positions_and_completes() {
    // SetupJump 0xE2 to (10, 20) height 5 in 2 steps; then ProgressJump 0xDC.
    let program = decode_program(&[0xe2, 0, 10, 0, 20, 0, 5, 0, 2, 0xdc, 4]).unwrap();
    let mut scene = FieldScene::new();
    scene.add_entry(actor(1), program).unwrap();
    let mut world = FieldWorld::new();
    world.add_actor(actor(1), [0.0, 0.0, 0.0]);

    for tick in 0..12 {
        let mut host = FieldHost::new(&mut world);
        let steps = scene.step(Tick::new(tick), &mut host).unwrap();
        if steps[0].report.status == VmStatus::Completed {
            assert_eq!(world.actors[&actor(1)].position[0], 10.0);
            assert_eq!(world.actors[&actor(1)].position[2], 20.0);
            return;
        }
    }
    panic!("jump program did not complete");
}

#[test]
fn unbacked_reads_fail_explicitly_instead_of_inventing_values() {
    let mut world = FieldWorld::new();
    world.add_actor(actor(1), [0.0; 3]);
    let mut host = FieldHost::new(&mut world);
    assert!(host.read_atan_entry(0).is_err());
    assert!(host.read_custom_variable(3).is_err());
    assert!(host.read_bgi_character_floor(0).is_err());
    assert!(host.read_object_position(actor(99)).is_err());
}

#[test]
fn bgi_queries_resolve_through_the_navigation_mesh() {
    use tantalus_contracts::{FieldNavigationMesh, NavigationTriangle, Point3};
    use tantalus_game::field::world::FieldWorld as World;

    let mesh = FieldNavigationMesh {
        vertices: vec![
            Point3 { x: 0, y: 0, z: 0 },
            Point3 { x: 100, y: 0, z: 0 },
            Point3 { x: 0, y: 0, z: 100 },
        ],
        triangles: vec![NavigationTriangle {
            vertices: [0, 1, 2],
            floor: 0,
            offset: Point3 { x: 0, y: 0, z: 0 },
        }],
        active_floor: 0,
        active_triangle: 0,
        actor_anchor: Point3 { x: 10, y: 0, z: 10 },
    };
    let mut world = World::new();
    world.add_actor(actor(1), [10.0, 0.0, 10.0]);
    world.map_uid(7, actor(1));

    // Without a mesh the reads stay explicit.
    {
        let mut host = FieldHost::new(&mut world);
        assert!(host.read_bgi_character_triangle(7).is_err());
    }

    world.provide_navigation(mesh);
    let mut host = FieldHost::new(&mut world);
    assert_eq!(host.read_bgi_character_triangle(7).unwrap(), 0);
    assert_eq!(host.read_bgi_character_floor(7).unwrap(), 0);
    assert!(host.read_bgi_character_triangle(99).is_err());
}

#[test]
fn object_member_reads_use_reference_member_semantics() {
    use tantalus_legacy::ObjectMemberRef;

    let mut world = FieldWorld::new();
    world.add_actor(actor(1), [10.0, -4.0, 7.0]);
    world.map_uid(7, actor(1));
    let mut host = FieldHost::new(&mut world);

    // 0 pos.x, 1 -pos.y, 2 pos.z with the checking rule.
    assert_eq!(
        host.read_object_member(ObjectMemberRef {
            object: actor(1),
            member: 0,
        })
        .unwrap(),
        10
    );
    assert_eq!(
        host.read_object_member(ObjectMemberRef {
            object: actor(1),
            member: 1,
        })
        .unwrap(),
        4
    );
    assert_eq!(
        host.read_object_member(ObjectMemberRef {
            object: actor(1),
            member: 2,
        })
        .unwrap(),
        7
    );
    // 5 uid resolves through the reverse UID map.
    assert_eq!(
        host.read_object_member(ObjectMemberRef {
            object: actor(1),
            member: 5,
        })
        .unwrap(),
        7
    );
    // Members 3/4/6/7 read tracked actor state; 8+ follow the reference's
    // zero battle-data default; unknown uids implicitly register zero state.
    assert_eq!(
        host.read_object_member(ObjectMemberRef {
            object: actor(1),
            member: 4,
        })
        .unwrap(),
        0
    );
    assert_eq!(
        host.read_object_member(ObjectMemberRef {
            object: actor(1),
            member: 40,
        })
        .unwrap(),
        0
    );
    assert_eq!(
        host.read_object_member(ObjectMemberRef {
            object: actor(99),
            member: 0,
        })
        .unwrap(),
        0
    );
    // Halfway floats round away from zero in the reference, matching ceil.
    world.actors.get_mut(&actor(1)).unwrap().position = [0.5, 0.0, 0.0];
    let mut host = FieldHost::new(&mut world);
    assert_eq!(
        host.read_object_member(ObjectMemberRef {
            object: actor(1),
            member: 0,
        })
        .unwrap(),
        1
    );
}
