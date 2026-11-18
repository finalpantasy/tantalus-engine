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
fn bgi_floor_is_the_floor_the_actor_stands_on() {
    // `B_BGIFLOOR` -> `BGI.BGI_charGetInfo` reads the actor's `activeFloor`, not the mesh's
    // start floor (Burmecia 755 tests for floor 3 before its floor gives way).
    use tantalus_contracts::{FieldNavigationMesh, NavigationTriangle, Point3};
    use tantalus_game::field::world::FieldWorld as World;

    let at = |x, z| Point3 { x, y: 0, z };
    let mesh = FieldNavigationMesh {
        vertices: vec![
            at(0, 0),
            at(100, 0),
            at(0, 100),
            at(200, 0),
            at(300, 0),
            at(200, 100),
        ],
        triangles: vec![
            NavigationTriangle {
                vertices: [0, 1, 2],
                floor: 0,
                offset: at(0, 0),
            },
            NavigationTriangle {
                vertices: [3, 4, 5],
                floor: 3,
                offset: at(0, 0),
            },
        ],
        active_floor: 0,
        active_triangle: 0,
        actor_anchor: at(10, 10),
    };
    let mut world = World::new();
    world.add_actor(actor(1), [210.0, 0.0, 10.0]);
    world.add_actor(actor(2), [10.0, 0.0, 10.0]);
    world.map_uid(7, actor(1));
    world.map_uid(8, actor(2));
    world.provide_navigation(mesh);
    world.control_actor = Some(actor(1));
    let mut host = FieldHost::new(&mut world);
    assert_eq!(host.read_bgi_character_floor(7).unwrap(), 3);
    assert_eq!(host.read_bgi_character_floor(8).unwrap(), 0);
    // `B_PTR 250` (`GetUIDFromLocalUID`) is the controlled object, so
    // `ObjectPointer(250), BgiCharacterFloor` reads the player's floor.
    let control = host.find_object_uid(250).unwrap().unwrap();
    assert_eq!(control, 1);
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
fn object_member_reads_answer_to_the_uid_an_object_was_created_with() {
    use tantalus_legacy::ObjectMemberRef;

    // `InitObject` source 9 with uid 133 (Madain Sari kitchen 1607): `GetObjByUID(133)` is entry 9's
    // object, and its `obj.uid` is 133.
    let mut world = FieldWorld::new();
    world.add_actor(actor(9), [40.0, 0.0, 12.0]);
    world.map_uid(9, actor(9));
    world.map_uid(133, actor(9));
    let mut host = FieldHost::new(&mut world);
    let read = |host: &mut FieldHost, object: u64, member: u8| {
        host.read_object_member(ObjectMemberRef {
            object: actor(object),
            member,
        })
        .unwrap()
    };
    assert_eq!(read(&mut host, 133, 5), 133);
    assert_eq!(read(&mut host, 133, 0), 40);
    assert_eq!(read(&mut host, 9, 5), 9);
}

#[test]
fn object_member_reads_of_a_party_slot_read_the_member_object() {
    use tantalus_legacy::ObjectMemberRef;

    // `GetObjByUID(251)` goes through `GetUIDFromLocalUID` to `partyUID[0]`. Map 2201
    // (20 entries, 11 to 18 linked to characters 0 to 7) with Eiko leading: slot 251 is
    // entry 17, and its init gives itself the player when its uid equals slot 251's.
    let mut world = FieldWorld::new();
    world.set_entry_links(tantalus_legacy::event::party::vanilla_links(
        std::iter::repeat_n(true, 20),
    ));
    world.party = vec![6, 4, 5, 7];
    world.setup_party_uid();
    for uid in 0..20 {
        world.map_uid(uid, actor(uid as u64));
    }
    world.add_actor(actor(17), [-171.0, 0.0, 1138.0]);
    let mut host = FieldHost::new(&mut world);
    let read = |host: &mut FieldHost, object: u64, member: u8| {
        host.read_object_member(ObjectMemberRef {
            object: actor(object),
            member,
        })
        .unwrap()
    };
    assert_eq!(read(&mut host, 251, 5), 17);
    assert_eq!(read(&mut host, 251, 0), -171);
    assert_eq!(read(&mut host, 17, 5), 17);
    assert_eq!(read(&mut host, 252, 5), 15, "Freya's entry");
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
    // `angleAsm`: +Z is facing 128, east +X is 192, -Z is 0, west -X is 64.
    world.add_actor(actor(1), [0.0, 0.0, 0.0]);
    for (destination, expected) in [
        ([0.0, 100.0], 128),
        ([100.0, 0.0], 192),
        ([0.0, -100.0], 0),
        ([-100.0, 0.0], 64),
    ] {
        world.actors.get_mut(&actor(1)).unwrap().position = [0.0, 0.0, 0.0];
        world.actors.get_mut(&actor(1)).unwrap().movement = Some(Movement {
            destination,
            keep_facing: false,
        });
        world.advance();
        assert_eq!(
            world.actors[&actor(1)].facing,
            expected,
            "destination {destination:?}"
        );
    }
}

#[test]
fn a_slide_moves_the_actor_without_turning_it() {
    // `MOVH` (0xA5) -> `MoveToward_mixed(x, 0, z, 1)`: the actor walks to (x, z) and keeps its
    // facing (flag 1). Field 1012 slides Zidane out of the region that started the slide.
    let mut world = FieldWorld::new();
    world.add_actor(actor(1), [0.0, 0.0, 0.0]);
    world.actors.get_mut(&actor(1)).unwrap().facing = 40;
    apply_events(
        &mut world,
        &[tantalus_legacy::HostEvent::WalkActorMixed {
            actor: actor(1),
            destination: [0, 0, 300],
            mode: 1,
        }],
    );
    for _ in 0..200 {
        world.advance();
    }
    let state = &world.actors[&actor(1)];
    assert_eq!(state.position[2], 300.0);
    assert_eq!(state.facing, 40);
    assert!(state.movement.is_none());
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

/// Runs `AMODE flags, count` then `ANIM 1` on actor 1 and returns the world.
fn animate_with_flags(flags: u8, count: u8) -> FieldWorld {
    // 0x3f AMODE: flags 0 (both operands inline bytes), then 0x40 ANIM: flags 0, id 1.
    let program = decode_program(&[0x3f, 0, flags, count, 0x40, 0, 1, 0, 4]).unwrap();
    let mut scene = FieldScene::new();
    scene.add_entry(actor(1), program).unwrap();
    let mut world = FieldWorld::new();
    world.add_actor(actor(1), [0.0; 3]);
    let mut host = FieldHost::new(&mut world);
    scene.step(Tick::new(0), &mut host).unwrap();
    world
}

#[test]
fn amode_hold_freezes_the_animation_on_its_last_frame() {
    // AMODE bit 0 is afHold.
    let mut world = animate_with_flags(1, 0);
    let state = &world.actors[&actor(1)];
    assert!(state.anim_hold && !state.anim_loop && !state.anim_frozen);
    for _ in 0..200 {
        world.advance_animations();
    }
    let state = &world.actors[&actor(1)];
    assert!(state.anim_frozen, "a held animation freezes when it ends");
    let frame = state.animation_frame;
    for _ in 0..50 {
        world.advance_animations();
    }
    assert_eq!(world.actors[&actor(1)].animation_frame, frame);
}

#[test]
fn amode_loop_restarts_the_animation_until_the_count_runs_out() {
    // AMODE bit 1 is afLoop, with a repeat count of 2: two passes, then the idle loop.
    let mut world = animate_with_flags(2, 2);
    let state = &world.actors[&actor(1)];
    assert!(state.anim_loop && state.anim_loops == 2);
    let span = state.animation_frames;
    assert!(span > 0);
    let mut restarts = 0;
    let mut previous = span;
    for _ in 0..(u32::from(span) * 4) {
        world.advance_animations();
        let frames = world.actors[&actor(1)].animation_frames;
        if frames > previous {
            restarts += 1;
        }
        previous = frames;
    }
    assert_eq!(restarts, 1, "one restart for a count of 2");
    assert_eq!(world.actors[&actor(1)].animation_frames, 0);
    assert!(!world.actors[&actor(1)].anim_frozen);
}

#[test]
fn plain_animation_ends_and_the_idle_loop_takes_over() {
    let mut world = animate_with_flags(0, 0);
    let span = world.actors[&actor(1)].animation_frames;
    for _ in 0..=span {
        world.advance_animations();
    }
    let state = &world.actors[&actor(1)];
    assert_eq!(state.animation_frames, 0);
    assert!(!state.anim_frozen);
}

#[test]
fn presentation_ops_record_scale_head_focus_icons_and_fades() {
    // 0x9f SCALE: flags 0, actor 255 (self), x 128, z 129, y 130.
    // 0x8b SETHFOCUS: flags 0, self 3, target 5.   0x47: flags 0, value 1.
    // 0x68 BUBBLE: flags 0, icon 2.   0xef HEREON: flags 0, mode 1.
    // 0x59 shade: flags 0, overlay 1, rgb 10 20 30.   0xec FADE: flags 0, six bytes.
    let program = decode_program(&[
        0x9f, 0, 255, 128, 129, 130, //
        0x8b, 0, 3, 5, //
        0x47, 0, 1, //
        0x68, 0, 2, //
        0xef, 0, 1, //
        0x59, 0, 1, 10, 20, 30, //
        0xec, 0, 7, 16, 0, 1, 2, 3, //
        4,
    ])
    .unwrap();
    let mut scene = FieldScene::new();
    scene.add_entry(actor(1), program).unwrap();
    let mut world = FieldWorld::new();
    world.add_actor(actor(1), [0.0; 3]);
    {
        let mut host = FieldHost::new(&mut world);
        scene.step(Tick::new(0), &mut host).unwrap();
    }
    let look = world.actor_presentation[&actor(1)];
    assert_eq!(look.scale, [-128 << 6, -127 << 6, -126 << 6]);
    assert_eq!((look.head_focus_self, look.head_focus_target), (3, 5));
    assert_eq!(look.head_focus_flags, 1);
    assert_eq!(world.bubble_icon, Some(2));
    assert_eq!(world.here_icon, 1);
    assert_eq!(world.tile_shade[&1], [10, 20, 30]);
    let fade = world.fade_filter.expect("a fade was requested");
    assert_eq!((fade.mode, fade.frames, fade.color), (7, 16, [1, 2, 3]));
    world.advance_animations();
    assert_eq!(world.fade_filter.unwrap().elapsed, 1);
}

fn apply_events(world: &mut FieldWorld, events: &[tantalus_legacy::HostEvent]) {
    let mut host = FieldHost::new(world);
    host.apply(events).unwrap();
}

#[test]
fn party_and_flow_ops_are_recorded() {
    use tantalus_legacy::HostEvent;
    let mut world = FieldWorld::new();
    apply_events(
        &mut world,
        &[
            // Script character 8 is Beatrix, character id 11.
            HostEvent::SetPlayerName {
                character: 8,
                text: 5,
            },
            HostEvent::ClearPartyStatus {
                character: 8,
                statuses: 0x7F,
            },
            HostEvent::SetPartyAvailability { mask: 0x0FFF },
            HostEvent::ChangeDisc {
                destination: (2 << 14) | 4000,
            },
            HostEvent::GameOver,
        ],
    );
    let beatrix = &world.character_info[&11];
    assert_eq!(beatrix.name_text, Some(5));
    assert_eq!(beatrix.cured_statuses, u32::MAX);
    assert_eq!(world.party_reserve, Some(0x0FFF));
    assert_eq!(world.disc_change, Some((4000, 2)));
    assert!(world.game_over);
}

#[test]
fn movement_speed_scales_the_walk_and_turns_use_the_turn_speed() {
    use tantalus_legacy::HostEvent;
    let mut world = FieldWorld::new();
    world.add_actor(actor(1), [0.0; 3]);
    world.add_actor(actor(2), [1000.0, 0.0, 0.0]);
    apply_events(
        &mut world,
        &[
            HostEvent::SetMovementSpeed {
                actor: actor(1),
                speed: 15,
            },
            HostEvent::SetTurnSpeed {
                actor: actor(1),
                speed: 0,
            },
            HostEvent::WalkActor {
                actor: actor(1),
                destination: [1000, 0],
            },
        ],
    );
    world.advance();
    // Half the default speed of 30 moves half the standard step.
    assert_eq!(world.actors[&actor(1)].position[0], world.walk_speed / 2.0);
    assert_eq!(world.actors[&actor(1)].turn_speed, 16);
    // The other actor lies due east, which the facing scale calls 192.
    apply_events(
        &mut world,
        &[HostEvent::TurnObjectToAngle {
            actor: actor(1),
            angle: 0,
        }],
    );
    assert_eq!(world.actors[&actor(1)].facing, 0);
    apply_events(
        &mut world,
        &[HostEvent::TurnTowardObject {
            actor: actor(1),
            target: 2,
            speed: 16,
        }],
    );
    let state = &world.actors[&actor(1)];
    assert_eq!(state.facing, 192);
    assert!(state.turn.is_some());
    // Facing an angle it already faces does not start a turn.
    apply_events(
        &mut world,
        &[HostEvent::TurnToAngle {
            actor: actor(1),
            angle: 192 << 4,
        }],
    );
    let before = world.actors[&actor(1)].turn;
    assert_eq!(world.actors[&actor(1)].facing, 192);
    assert_eq!(world.actors[&actor(1)].turn, before);
}

#[test]
fn attachments_follow_attach_offset_and_detach() {
    use tantalus_legacy::HostEvent;
    let mut world = FieldWorld::new();
    world.add_actor(actor(1), [0.0; 3]);
    world.add_actor(actor(2), [0.0; 3]);
    apply_events(
        &mut world,
        &[
            HostEvent::AttachObjectOffset {
                actor: actor(1),
                x: 1,
                y: 2,
                z: 3,
            },
            HostEvent::AttachObject {
                attached: actor(2),
                carrying: actor(1),
                bone: 13,
            },
        ],
    );
    let held = world.attachments[&actor(2)];
    assert_eq!(
        (held.carrier, held.bone, held.offset),
        (Some(actor(1)), 13, [1, 2, 3])
    );
    apply_events(
        &mut world,
        &[HostEvent::DetachObject { attached: actor(2) }],
    );
    assert_eq!(world.attachments[&actor(2)].carrier, None);
}

#[test]
fn quina_walks_slower_in_the_marsh() {
    use tantalus_legacy::HostEvent;
    let set = |map: i32, who: u64| {
        let mut world = FieldWorld::new();
        world.field_map_no = map;
        world.add_actor(actor(who), [0.0; 3]);
        apply_events(
            &mut world,
            &[HostEvent::SetMovementSpeed {
                actor: actor(who),
                speed: 30,
            }],
        );
        world.actors[&actor(who)].move_speed
    };
    assert_eq!(set(658, 18), 25);
    assert_eq!(set(658, 5), 30);
    assert_eq!(set(659, 18), 30);
}

/// Memoria's per-field script patches (`EventEngine.DoEventCode.cs`).
#[test]
fn memoria_field_script_patches() {
    use tantalus_legacy::HostEvent;
    let place = |map: i32, sc: u16, who: u64, event: HostEvent| {
        let mut world = FieldWorld::new();
        world.field_map_no = map;
        world
            .global_memory
            .set(0, tantalus_legacy::VariableType::UInt16, i32::from(sc));
        world.add_actor(actor(who), [0.0; 3]);
        apply_events(&mut world, &[event]);
        world.actors[&actor(who)].clone()
    };
    // V01: field 53 walk X 250 -> 330.
    let walk = |map| {
        place(
            map,
            0,
            3,
            HostEvent::WalkActor {
                actor: actor(3),
                destination: [250, 1200],
            },
        )
        .movement
        .map(|m| m.destination)
    };
    assert_eq!(walk(53), Some([330.0, 1200.0]));
    assert_eq!(walk(54), Some([250.0, 1200.0]));
    // G64: Kuja (2209, entry 4, scenario 9520) moves right; other scenarios do not.
    let kuja = |sc| {
        place(
            2209,
            sc,
            4,
            HostEvent::SetActorPosition {
                actor: actor(4),
                position: [1123, 98, 1067],
            },
        )
        .position[0]
    };
    assert_eq!(kuja(9520), 1323.0);
    assert_eq!(kuja(9500), 1123.0);
    // G64: Steiner in the Evil Forest nest (#892).
    let steiner = place(
        257,
        2300,
        8,
        HostEvent::SetActorPosition {
            actor: actor(8),
            position: [96, 32768, -736],
        },
    );
    assert_eq!((steiner.position[0], steiner.position[2]), (30.0, -716.0));
    // G62: Fossil Roo worker, Garnet's room Dagger height.
    let worker = place(
        1421,
        0,
        5,
        HostEvent::SetActorPosition {
            actor: actor(5),
            position: [1510, 32768, -2331],
        },
    );
    assert_eq!(worker.position[2], -2231.0);
    let dagger = place(
        1207,
        0,
        9,
        HostEvent::SetActorPosition {
            actor: actor(9),
            position: [10, 20, 30],
        },
    );
    assert_eq!(dagger.position[2], 1274.0);
}

/// The widescreen-era script patches of `EventEngine.DoEventCode.cs`: positions that put actors
/// in view too soon, and the field 103 `CFLAG` skip.
#[test]
fn memoria_widescreen_script_patches() {
    use tantalus_legacy::HostEvent;
    let run = |map: i32, sc: u16, who: u64, at: [f32; 3], event: HostEvent| {
        let mut world = FieldWorld::new();
        world.field_map_no = map;
        world
            .global_memory
            .set(0, tantalus_legacy::VariableType::UInt16, i32::from(sc));
        world.add_actor(actor(who), at);
        apply_events(&mut world, &[event]);
        world.actors[&actor(who)].clone()
    };
    let pos = |who: u64, x: i32, z: i32| HostEvent::SetActorPosition {
        actor: actor(who),
        position: [x, 32768, z],
    };
    let at =
        |state: &tantalus_game::field::world::ActorState| (state.position[0], state.position[2]);
    // 2600 Dagger, 2651 Mikoto, 1901 Gilmanesh.
    assert_eq!(
        at(&run(2600, 10700, 8, [0.0; 3], pos(8, 1173, -1685))),
        (1173.0, -1985.0)
    );
    assert_eq!(
        at(&run(2600, 10000, 8, [0.0; 3], pos(8, 1173, -1685))),
        (1173.0, -1685.0)
    );
    assert_eq!(
        at(&run(2651, 10890, 4, [0.0; 3], pos(4, 1449, -6844))),
        (1749.0, -7244.0)
    );
    assert_eq!(
        at(&run(1901, 7550, 2, [0.0; 3], pos(2, 142, 2172))),
        (400.0, 2172.0)
    );
    // 1815 Steiner on the dock; 1901 Quina walks further and is hidden past the edge.
    let walk = |map: i32, sc: u16, who: u64, from: f32, to: [i16; 2]| {
        run(
            map,
            sc,
            who,
            [from, 0.0, 0.0],
            HostEvent::WalkActor {
                actor: actor(who),
                destination: to,
            },
        )
    };
    let goal = |state: &tantalus_game::field::world::ActorState| {
        state.movement.as_ref().map(|m| m.destination)
    };
    assert_eq!(
        goal(&walk(1815, 7070, 3, 0.0, [-2312, -245])),
        Some([-3000.0, -245.0])
    );
    assert_eq!(
        goal(&walk(1815, 7000, 3, 0.0, [-2312, -245])),
        Some([-2312.0, -245.0])
    );
    let quina = walk(1901, 0, 2, 900.0, [672, 100]);
    assert_eq!(goal(&quina), Some([1000.0, 100.0]));
    assert_eq!(quina.flags & 63, 14);
    assert_ne!(walk(1901, 0, 2, 800.0, [672, 100]).flags & 63, 14);
    // 1550 Quina runs to the Mog (MOVE3 only).
    let mixed = |mode: u8, who: u64, to: [i32; 3]| {
        goal(&run(
            1550,
            0,
            who,
            [0.0; 3],
            HostEvent::WalkActorMixed {
                actor: actor(who),
                destination: to,
                mode,
            },
        ))
    };
    assert_eq!(mixed(2, 18, [1798, 0, 5]), Some([2800.0, 5.0]));
    assert_eq!(mixed(2, 2, [1797, 0, 5]), Some([2800.0, 5.0]));
    assert_eq!(mixed(1, 18, [1798, 0, 5]), Some([1798.0, 5.0]));
    assert_eq!(mixed(3, 15, [-1109, 0, -1014]), Some([-1155.0, -1070.0]));
    // 103: the crowd keeps its flags when the script asks for 14.
    let flags = |map: i32, who: u64, flags: u8| {
        run(
            map,
            0,
            who,
            [0.0; 3],
            HostEvent::SetObjectFlags {
                object: actor(who),
                flags,
            },
        )
        .flags
    };
    assert_ne!(flags(103, 9, 14), 14);
    assert_eq!(flags(103, 9, 15), 15);
    assert_eq!(flags(103, 18, 14), 14);
    assert_eq!(flags(104, 9, 14), 14);
}

#[test]
fn a_walk_loop_count_ends_the_walk_after_that_many_frames() {
    // `MoveToward_mixed_ex`: `MOVT` 1 makes the next walk a single frame; `CLRDIST`
    // restores the walk to arrival. Field 2204's frog Cid hops one step per press.
    use tantalus_legacy::HostEvent;
    let mut world = FieldWorld::new();
    world.add_actor(actor(1), [0.0; 3]);
    let walk = HostEvent::WalkActor {
        actor: actor(1),
        destination: [1000, 0],
    };
    apply_events(
        &mut world,
        &[
            HostEvent::ResetWalkState { actor: actor(1) },
            HostEvent::SetAnimationLoopCount {
                actor: actor(1),
                count: 1,
            },
            walk.clone(),
        ],
    );
    world.advance();
    let state = &world.actors[&actor(1)];
    assert_eq!(state.position[0], world.walk_speed);
    assert!(state.movement.is_none(), "one frame, then the walk ends");
    assert_eq!(state.walk_loops, 0);

    apply_events(
        &mut world,
        &[HostEvent::ResetWalkState { actor: actor(1) }, walk],
    );
    for _ in 0..3 {
        world.advance();
    }
    let state = &world.actors[&actor(1)];
    assert_eq!(state.position[0], world.walk_speed * 4.0);
    assert!(state.movement.is_some(), "255 walks until arrival");
}

#[test]
fn field_2204_walks_keep_their_overshoot_in_whole_units() {
    // `MoveToward_mixed_ex` snaps an arriving walk except in field 2204, where the step
    // is `GetMoveVector`'s whole-unit vector and turns by at most `omega` (11.25 degrees).
    use tantalus_legacy::HostEvent;
    let mut world = FieldWorld::new();
    world.field_map_no = 2204;
    world.add_actor(actor(1), [480.0, 0.0, 210.0]);
    // Facing east (192), the way the walk goes.
    world.actors.get_mut(&actor(1)).unwrap().facing = 192;
    apply_events(
        &mut world,
        &[
            HostEvent::SetMovementSpeed {
                actor: actor(1),
                speed: 4,
            },
            HostEvent::WalkActor {
                actor: actor(1),
                destination: [482, 210],
            },
        ],
    );
    world.advance();
    let state = &world.actors[&actor(1)];
    assert_eq!(state.position, [484.0, 0.0, 210.0], "no snap back to 482");
    assert!(state.movement.is_none());

    // Elsewhere the same walk lands on its target.
    world.field_map_no = 2203;
    world.actors.get_mut(&actor(1)).unwrap().position = [480.0, 0.0, 210.0];
    apply_events(
        &mut world,
        &[HostEvent::WalkActor {
            actor: actor(1),
            destination: [482, 210],
        }],
    );
    world.advance();
    assert_eq!(world.actors[&actor(1)].position, [482.0, 0.0, 210.0]);
}

/// The six walkmesh triangles of map 2204 (`EVT_SUNA_SP_RFG_0`) that frog Cid hops
/// across from his start (193, -171) to the key at (484, 210), as absolute x/z corners.
const FROG_CID_FLOOR: [[[i32; 2]; 3]; 6] = [
    [[72, -274], [355, -300], [278, -22]],
    [[565, -163], [278, -22], [355, -300]],
    [[565, -163], [445, 67], [278, -22]],
    [[278, -22], [445, 67], [154, 243]],
    [[154, 243], [445, 67], [419, 299]],
    [[419, 299], [445, 67], [552, 200]],
];

fn frog_cid_mesh() -> tantalus_contracts::FieldNavigationMesh {
    use tantalus_contracts::{NavigationTriangle, Point3};
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    for corners in FROG_CID_FLOOR {
        let base = vertices.len() as u32;
        vertices.extend(corners.map(|[x, z]| Point3 { x, y: 0, z }));
        triangles.push(NavigationTriangle {
            vertices: [base, base + 1, base + 2],
            floor: 0,
            offset: Point3 { x: 0, y: 0, z: 0 },
        });
    }
    tantalus_contracts::FieldNavigationMesh {
        vertices,
        triangles,
        active_floor: 0,
        active_triangle: 0,
        actor_anchor: tantalus_contracts::Point3 { x: 0, y: 0, z: 0 },
    }
}

#[test]
fn walkmesh_rebuild_keeps_the_reference_float_error() {
    // `UpdateActiveTri` rebuilds the position from `CalculateBarycentricRatioXZ` in
    // single precision: (197, -165), frog Cid's first hop, comes back one ulp off.
    let mesh = frog_cid_mesh();
    let navigation = tantalus_game::navigation::FieldNavigation::new(&mesh).unwrap();
    let rebuilt = navigation.rebuild_on_triangle(197.0, 0.0, -165.0).unwrap();
    assert_eq!(rebuilt, [197.0, -164.999_98]);
    assert_eq!(rebuilt[1].to_bits(), 0xc324_ffff, "one ulp above -165");
    assert!(
        navigation
            .rebuild_on_triangle(1000.0, 0.0, 1000.0)
            .is_none()
    );
}

#[test]
fn field_2204_pathing_actors_are_rebuilt_on_the_walkmesh() {
    let mut world = FieldWorld::new();
    world.provide_navigation(frog_cid_mesh());
    world.add_actor(actor(1), [197.0, 0.0, -165.0]);
    {
        let state = world.actors.get_mut(&actor(1)).unwrap();
        state.path_active = true;
        state.look.model = Some(425);
    }
    world.field_map_no = 2203;
    world.advance();
    assert_eq!(
        world.actors[&actor(1)].position[2],
        -165.0,
        "other fields keep it"
    );
    world.field_map_no = 2204;
    world.advance();
    assert_eq!(world.actors[&actor(1)].position[2], -164.999_98);
}

#[test]
fn frog_cid_lands_on_the_key_x_through_the_float_error() {
    // Map 2204 ends the chase when Cid's x reads 485 while every hop aims at (484, 210).
    // On whole units he circles the key forever; the walkmesh float error lets him land.
    // Needs the reference `ratan_tbl` (game data, local only); skipped without it.
    use tantalus_legacy::HostEvent;
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../local/probe/ratan_tbl.bin"
    );
    let Ok(table) = std::fs::read(path) else {
        eprintln!("skipped: no {path}");
        return;
    };
    let chase = |mesh: Option<tantalus_contracts::FieldNavigationMesh>| {
        let mut world = FieldWorld::new();
        world.field_map_no = 2204;
        world.provide_atan_table(table.clone());
        if let Some(mesh) = mesh {
            world.provide_navigation(mesh);
        }
        world.add_actor(actor(1), [193.0, 0.0, -171.0]);
        {
            let state = world.actors.get_mut(&actor(1)).unwrap();
            state.path_active = true;
            state.look.model = Some(425);
            state.facing = 151;
        }
        for hop in 0..400 {
            apply_events(
                &mut world,
                &[
                    HostEvent::SetMovementSpeed {
                        actor: actor(1),
                        speed: 4,
                    },
                    HostEvent::ResetWalkState { actor: actor(1) },
                    HostEvent::SetAnimationLoopCount {
                        actor: actor(1),
                        count: 1,
                    },
                    HostEvent::WalkActor {
                        actor: actor(1),
                        destination: [484, 210],
                    },
                ],
            );
            world.advance();
            if world.actors[&actor(1)].position[0].round() == 485.0 {
                return Some(hop);
            }
        }
        None
    };
    assert_eq!(chase(None), None, "whole units never read 485");
    assert_eq!(chase(Some(frog_cid_mesh())), Some(243));
}
