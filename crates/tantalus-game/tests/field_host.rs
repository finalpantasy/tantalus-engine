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
fn shown_text_raises_its_signals_one_per_tick_into_sysvar_8() {
    // WindowSync 0x1F: window 3, flags 0, text 7.
    let program = decode_program(&[0x1f, 0, 3, 0, 7, 0, 4]).unwrap();
    let mut scene = FieldScene::new();
    scene.add_entry(actor(1), program).unwrap();
    let mut world = FieldWorld::new();
    world.add_actor(actor(1), [0.0; 3]);
    world.dialog_signals.insert(7, vec![5, -1]);
    world.mes_signal = 1;

    scene
        .step(Tick::new(0), &mut FieldHost::new(&mut world))
        .unwrap();
    assert_eq!(FieldHost::new(&mut world).read_system_variable(8), Ok(1));
    world.advance();
    assert_eq!(FieldHost::new(&mut world).read_system_variable(8), Ok(5));
    // A negative signal increments (DialogBoxSymbols.OnSignal).
    world.advance();
    assert_eq!(world.mes_signal, 6);
    world.advance();
    assert_eq!(world.mes_signal, 6);
}

#[test]
fn set_signal_opcode_sets_dialog_progression() {
    // SETSIGNAL 0xE3 with value 2.
    let program = decode_program(&[0xe3, 0, 2, 4]).unwrap();
    let mut scene = FieldScene::new();
    scene.add_entry(actor(1), program).unwrap();
    let mut world = FieldWorld::new();
    world.add_actor(actor(1), [0.0; 3]);
    scene
        .step(Tick::new(0), &mut FieldHost::new(&mut world))
        .unwrap();
    assert_eq!(world.mes_signal, 2);
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
    // SetupJump 0xE2 words X 10, height 20 (stored negated, pos[1] = -20),
    // Z 5, in 2 steps; then ProgressJump 0xDC.
    let program = decode_program(&[0xe2, 0, 10, 0, 20, 0, 5, 0, 2, 0xdc, 4]).unwrap();
    let mut scene = FieldScene::new();
    scene.add_entry(actor(1), program).unwrap();
    let mut world = FieldWorld::new();
    world.add_actor(actor(1), [0.0, 0.0, 0.0]);

    for tick in 0..12 {
        let mut host = FieldHost::new(&mut world);
        let steps = scene.step(Tick::new(tick), &mut host).unwrap();
        if steps[0].report.status == VmStatus::Completed {
            assert_eq!(world.actors[&actor(1)].position, [10.0, -20.0, 5.0]);
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

#[test]
fn define_player_control_designates_the_interactive_player() {
    use tantalus_legacy::decode_program;

    // DefinePlayerControl 0x2C is current-object; entry entity 1 declares
    // itself the controlled actor.
    let program = decode_program(&[0x2c, 4]).unwrap();
    let mut scene = tantalus_game::FieldScene::new();
    scene.add_entry(actor(1), program).unwrap();
    let mut world = FieldWorld::new();
    world.add_actor(actor(1), [0.0; 3]);
    let mut host = FieldHost::new(&mut world);
    scene.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(world.player(), Some(actor(1)));
    assert_eq!(world.control_actor, Some(actor(1)));
}

#[test]
fn walking_faces_the_movement_direction_on_the_reference_scale() {
    use tantalus_game::field::world::Movement;

    let mut world = FieldWorld::new();
    // North is -Z (facing 128), east is +X (192), south +Z (0), west -X (64).
    world.add_actor(actor(1), [0.0, 0.0, 0.0]);
    for (destination, expected) in [
        ([0.0, -100.0], 128),
        ([100.0, 0.0], 192),
        ([0.0, 100.0], 0),
        ([-100.0, 0.0], 64),
    ] {
        world.actors.get_mut(&actor(1)).unwrap().position = [0.0, 0.0, 0.0];
        world.actors.get_mut(&actor(1)).unwrap().movement = Some(Movement { destination });
        world.advance();
        assert_eq!(
            world.actors[&actor(1)].facing,
            expected,
            "destination {destination:?}"
        );
    }
}

#[test]
fn turn_loops_read_their_own_facing_through_uid_255() {
    use tantalus_legacy::decode_program;

    // Map 100 entry 19 turns itself with `DIRE(self.facing + n)` and polls
    // `self.facing` (`getvobj(255, 3)`). Uid 255 is the running object and
    // member 3 is `fixed >> 4 & 255`, so the value wraps at 256.
    let program = decode_program(&[
        0x36, 0, 250, // DIRE 250
        0x22, 0, 1, // WAIT 1: host events apply at the end of the step
        0x36, 1, 120, 255, 3, 125, 10, 0, 20, 127, // DIRE self.facing + 10
        0x04,
    ])
    .unwrap();
    let mut scene = tantalus_game::FieldScene::new();
    scene.add_entry(actor(1), program).unwrap();
    let mut world = FieldWorld::new();
    world.add_actor(actor(1), [0.0; 3]);
    let mut host = FieldHost::new(&mut world);
    for tick in 0..3 {
        scene.step(Tick::new(tick), &mut host).unwrap();
    }
    assert_eq!(world.actors[&actor(1)].facing, 4);
    assert!(!world.actors.contains_key(&actor(255)));
}

#[test]
fn deleting_an_object_pauses_every_function_until_it_is_initialized_again() {
    use tantalus_legacy::decode_program;

    // EBin `DELETE` (0x1C) disposes the whole object (`DisposeObj`), so its
    // other functions stop too; `NEW` (InitObject) brings it back.
    let deleter = decode_program(&[
        0x1C, 0, 2, // DELETE 2
        0x04,
    ])
    .unwrap();
    let mover = decode_program(&[
        0x22, 0, 1, // WAIT 1
        0x04,
    ])
    .unwrap();
    let mut scene = tantalus_game::FieldScene::new();
    scene.add_entry(actor(1), deleter).unwrap();
    scene
        .add_object_function(actor(2 * 256 + 1), actor(2), mover)
        .unwrap();
    let mut world = FieldWorld::new();
    world.add_actor(actor(1), [0.0; 3]);
    world.add_actor(actor(2), [0.0; 3]);
    {
        let mut host = FieldHost::new(&mut world);
        scene.step(Tick::new(0), &mut host).unwrap();
        for tick in 1..4 {
            let steps = scene.step(Tick::new(tick), &mut host).unwrap();
            assert!(steps.iter().all(|step| step.entity != actor(2 * 256 + 1)));
        }
    }
    assert!(world.is_object_deleted(actor(2)));
    world.restore_object(actor(2));
    let mut host = FieldHost::new(&mut world);
    let steps = scene.step(Tick::new(4), &mut host).unwrap();
    assert!(steps.iter().any(|step| step.entity == actor(2 * 256 + 1)));
}
