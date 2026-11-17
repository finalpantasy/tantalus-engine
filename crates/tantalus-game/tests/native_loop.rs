//! The packaged independent loop: one `FieldDirector` runs field events into
//! world state, resolves an encounter into a battle, commits rewards exactly
//! once, and survives a save/restore with world continuity.

use tantalus_contracts::{ContentId, EntityId, Tick};
use tantalus_game::battle::*;
use tantalus_game::{FieldDirector, FieldHost, FieldSnapshot, FieldWorld};

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
fn packaged_loop_events_battle_rewards_save_and_restore() {
    let mut director = FieldDirector::new(FieldWorld::new());
    director.world_mut().add_actor(EntityId::new(1), [0.0; 3]);

    // Gil grant, an async window open, then ENCOUNT 0x2A (battle 0x55).
    // ENCOUNT halts the entry past the opcode (the reference jumps to battle
    // and resumes there afterwards), so the trailing RETURN runs after it.
    let program = [
        0xce, 0, 100, 0, 0, // AddGil 100
        0x20, 0, 3, 0, 7, 0, // WindowAsync window 3
        0x2a, 0, 0, 0x55, 0x00, // ENCOUNT battle 0x55
        4,
    ];
    director.add_entry(EntityId::new(1), &program).unwrap();

    let step = director.step().unwrap();
    assert_eq!(step.tick, Tick::new(0));
    assert!(!step.completed_entries.contains(&EntityId::new(1)));
    assert_eq!(step.encounters.len(), 1);
    assert_eq!(step.encounters[0].battle, 0x55);
    assert_eq!(director.world().gil, 100);
    assert!(director.world().windows[&3]);

    // Resolve the encounter into a battle and run it to victory.
    let request = step.encounters[0];
    director.begin_battle(request, ruleset()).unwrap();
    let mut outcome = None;
    for battle_tick in 1..64u64 {
        let mut input = CombatInput::new(Tick::new(battle_tick));
        input.command = Some(BattleCommand {
            actor: EntityId::new(1),
            action: content("attack/raptor_claw"),
            targets: vec![EntityId::new(2)],
        });
        outcome = director.advance_battle(&input).unwrap();
        if outcome.is_some() {
            break;
        }
    }
    assert_eq!(outcome, Some(BattleOutcome::Victory));
    assert_eq!(director.world().gil, 120);
    assert_eq!(director.finish_battle().map(|r| r.battle), Some(0x55));

    // Save, serialize, restore: the world continues exactly.
    let json = director.save().unwrap().to_json().unwrap();
    let snapshot = FieldSnapshot::from_json(&json).unwrap();
    let mut restored = FieldDirector::restore(&snapshot).unwrap();
    assert_eq!(restored.world(), director.world());
    assert!(!restored.coordinator().is_active());

    // The restored field resumes recorded positions: the entry continues past
    // ENCOUNT to its RETURN, so neither the gil grant nor a second encounter
    // re-appears.
    let step = restored.step().unwrap();
    assert!(step.completed_entries.contains(&EntityId::new(1)));
    assert_eq!(restored.world().gil, 120);
    assert_eq!(restored.world().encounters.len(), 0);
}

#[test]
fn director_step_surfaces_script_requests_for_orchestration() {
    // Entry 0 requests level 1, target entry 2, function tag 5.
    // REQ 0x10: flags 0, level byte, target byte, tag byte.
    let program = [0x10, 0, 1, 2, 5, 4];
    let mut director = FieldDirector::new(FieldWorld::new());
    director.world_mut().add_actor(EntityId::new(0), [0.0; 3]);
    director.add_entry(EntityId::new(0), &program).unwrap();

    let step = director.step().unwrap();
    assert_eq!(step.completed_entries, [EntityId::new(0)]);
    assert_eq!(
        step.requests,
        [tantalus_game::RequestedScript {
            level: 1,
            target: EntityId::new(2),
            tag: 5,
            synchronous: false,
        }]
    );
    // The host recorded the pending request under the target entity.
    assert!(director.world().request_pending(EntityId::new(2)));
    // Completing the request clears it for the waiting VM's next query.
    director.world_mut().complete_request(EntityId::new(2));
    assert!(!director.world().request_pending(EntityId::new(2)));
}

#[test]
fn request_orchestration_cycle_spawns_completes_and_clears() {
    use tantalus_legacy::decode_program;

    let mut director = FieldDirector::new(FieldWorld::new());
    director.world_mut().add_actor(EntityId::new(0), [0.0; 3]);
    director.world_mut().add_actor(EntityId::new(1), [0.0; 3]);
    // Entry 0 requests level 1 of entry 1, function tag 7, then ends.
    director
        .add_entry(EntityId::new(0), &[0x10, 0, 1, 1, 7, 4])
        .unwrap();

    let step = director.step().unwrap();
    assert_eq!(step.completed_entries, [EntityId::new(0)]);
    assert_eq!(
        step.requests,
        [tantalus_game::RequestedScript {
            level: 1,
            target: EntityId::new(1),
            tag: 7,
            synchronous: false,
        }]
    );
    assert!(director.world().request_pending(EntityId::new(1)));

    // The orchestrator spawns the requested function as its own VM.
    let spawned = EntityId::new(256 + 7);
    director
        .add_entry_program(spawned, decode_program(&[0xce, 0, 50, 0, 0, 4]).unwrap())
        .unwrap();

    let mut completed_spawn = false;
    for _ in 0..8 {
        let step = director.step().unwrap();
        if step.completed_entries.contains(&spawned) {
            completed_spawn = true;
            break;
        }
    }
    assert!(completed_spawn);
    assert_eq!(director.world().gil, 50);

    // Completing the request clears the host pend so a waiting REQSW resumes.
    director.world_mut().complete_request(EntityId::new(1));
    assert!(!director.world().request_pending(EntityId::new(1)));
}

#[test]
fn player_input_injection_reaches_host_key_queries() {
    use tantalus_legacy::LegacyEventHost;

    let mut director = FieldDirector::new(FieldWorld::new());
    director.world_mut().add_actor(EntityId::new(1), [0.0; 3]);
    director.set_player(EntityId::new(1));
    assert_eq!(director.world().player(), Some(EntityId::new(1)));

    // No input: key reads report zero.
    {
        let mut host = FieldHost::new(director.world_mut());
        assert_eq!(host.read_key_on().unwrap(), 0);
    }

    // Injected input is visible to the host key queries until changed.
    director.set_input_mask(0x10);
    {
        let mut host = FieldHost::new(director.world_mut());
        assert_eq!(host.read_key_on().unwrap(), 0x10);
        assert_eq!(
            host.read_key_state(tantalus_legacy::KeyState::Held)
                .unwrap(),
            0x10
        );
        assert_eq!(
            host.read_key_state(tantalus_legacy::KeyState::Off).unwrap(),
            0
        );
    }

    director.set_input_mask(0);
    {
        let mut host = FieldHost::new(director.world_mut());
        assert_eq!(host.read_key_on().unwrap(), 0);
    }
}

#[test]
fn talk_triggers_fire_when_player_confirms_within_talk_radius() {
    let mut director = FieldDirector::new(FieldWorld::new());
    director.world_mut().add_actor(EntityId::new(1), [0.0; 3]);
    // Default radii are 16 each (reference PosObj), so the default reach
    // is 4 * (16 + 16) = 128; the object at (30, 40) is 50 away (inside),
    // and 500 away is outside.
    director
        .world_mut()
        .add_actor(EntityId::new(2), [30.0, 0.0, 40.0]);
    director.set_player(EntityId::new(1));

    // No confirm mask configured: detection stays disabled.
    director.set_input_mask(0x20);
    let step = director.step().unwrap();
    assert!(step.talk_triggers.is_empty());

    // Confirm armed but not pressed.
    director.set_confirm_mask(Some(0x20));
    director.set_input_mask(0);
    let step = director.step().unwrap();
    assert!(step.talk_triggers.is_empty());

    // Confirm pressed within the default reach: the object reports a trigger.
    director.set_input_mask(0x20);
    let step = director.step().unwrap();
    assert_eq!(step.talk_triggers, [EntityId::new(2)]);

    // Out of range: distance beyond the summed reach excludes the object.
    director
        .world_mut()
        .actors
        .get_mut(&EntityId::new(2))
        .unwrap()
        .position = [500.0, 0.0, 0.0];
    let step = director.step().unwrap();
    assert!(step.talk_triggers.is_empty());
}

#[test]
fn an_object_no_init_object_created_cannot_be_talked_to() {
    // `EventCollision.Collision` walks live objects only: an entry's actor stub that
    // `InitObject` has not created yet (Madain Sari 1601 entry 9 at scenario 6610) is not talkable.
    let mut director = FieldDirector::new(FieldWorld::new());
    director.world_mut().add_actor(EntityId::new(1), [0.0; 3]);
    director
        .world_mut()
        .add_actor(EntityId::new(2), [30.0, 0.0, 40.0]);
    director.world_mut().mark_uncreated(EntityId::new(2));
    director.set_player(EntityId::new(1));
    director.set_confirm_mask(Some(0x20));
    director.set_input_mask(0x20);
    let step = director.step().unwrap();
    assert!(step.talk_triggers.is_empty());

    director.world_mut().init_object(2, 0);
    let step = director.step().unwrap();
    assert_eq!(step.talk_triggers, [EntityId::new(2)]);
}

#[test]
fn region_occupancy_reports_containing_regions_for_the_player() {
    use tantalus_game::field::region::Region;

    let mut director = FieldDirector::new(FieldWorld::new());
    director
        .world_mut()
        .add_actor(EntityId::new(1), [50.0, 0.0, 50.0]);
    director.set_player(EntityId::new(1));
    director.world_mut().regions.insert(
        EntityId::new(2),
        Region {
            vertices: vec![[0, 0], [100, 0], [100, 100], [0, 100]],
        },
    );
    director.world_mut().regions.insert(
        EntityId::new(3),
        Region {
            vertices: vec![[200, 0], [300, 0], [300, 100], [200, 100]],
        },
    );

    let step = director.step().unwrap();
    assert_eq!(step.region_occupancy, [EntityId::new(2)]);

    director
        .world_mut()
        .actors
        .get_mut(&EntityId::new(1))
        .unwrap()
        .position = [250.0, 0.0, 50.0];
    let step = director.step().unwrap();
    assert_eq!(step.region_occupancy, [EntityId::new(3)]);
}

#[test]
fn walk_input_moves_the_player_by_the_configured_direction_bits() {
    let mut director = FieldDirector::new(FieldWorld::new());
    director.world_mut().add_actor(EntityId::new(1), [0.0; 3]);
    director.set_player(EntityId::new(1));
    // No mapping: input never moves the player.
    director.set_input_mask(0x08);
    director.step().unwrap();
    assert_eq!(
        director.world().actors[&EntityId::new(1)].position,
        [0.0; 3]
    );

    director.set_walk_bits(Some((0x01, 0x02, 0x04, 0x08)));
    // Input without user control never moves the character.
    director.set_input_mask(0x08);
    director.step().unwrap();
    assert_eq!(
        director.world().actors[&EntityId::new(1)].position,
        [0.0; 3]
    );
    director.world_mut().user_control = true;
    // Right at run speed: two walk steps of 30 per tick.
    director.set_input_mask(0x08);
    director.step().unwrap();
    assert_eq!(
        director.world().actors[&EntityId::new(1)].position,
        [60.0, 0.0, 0.0]
    );
    // Up moves positive Z (FieldMapActorController.UpdateMovement);
    // diagonals normalize.
    director.set_input_mask(0x01);
    director.step().unwrap();
    assert_eq!(
        director.world().actors[&EntityId::new(1)].position,
        [60.0, 0.0, 60.0]
    );
    director.set_input_mask(0x08 | 0x02);
    let before = director.world().actors[&EntityId::new(1)].position;
    director.step().unwrap();
    let after = director.world().actors[&EntityId::new(1)].position;
    let expected = 60.0f32 * std::f32::consts::FRAC_1_SQRT_2;
    assert!((after[0] - before[0] - expected).abs() < 1e-4);
    assert!((after[2] - before[2] + expected).abs() < 1e-4);
    // A control twist of 90 degrees turns up (+z) into +x.
    director.world_mut().control_twist = 90.0;
    director.set_input_mask(0x01);
    let before = director.world().actors[&EntityId::new(1)].position;
    director.step().unwrap();
    let after = director.world().actors[&EntityId::new(1)].position;
    assert!((after[0] - before[0] - 60.0).abs() < 1e-3);
    assert!((after[2] - before[2]).abs() < 1e-3);
    // Holding the walk button, or `DASHOFF`, walks at 30.
    director.world_mut().control_twist = 0.0;
    director.set_walk_hold_bit(Some(0x10));
    director.set_input_mask(0x08 | 0x10);
    let before = director.world().actors[&EntityId::new(1)].position;
    director.step().unwrap();
    let after = director.world().actors[&EntityId::new(1)].position;
    assert!((after[0] - before[0] - 30.0).abs() < 1e-3);
    director.world_mut().run_disabled = true;
    director.set_input_mask(0x08);
    let before = director.world().actors[&EntityId::new(1)].position;
    director.step().unwrap();
    let after = director.world().actors[&EntityId::new(1)].position;
    assert!((after[0] - before[0] - 30.0).abs() < 1e-3);
}

#[test]
fn walk_input_eases_the_turn_the_short_way() {
    let mut director = FieldDirector::new(FieldWorld::new());
    director.world_mut().add_actor(EntityId::new(1), [0.0; 3]);
    director.set_player(EntityId::new(1));
    director.set_walk_bits(Some((0x01, 0x02, 0x04, 0x08)));
    director.world_mut().user_control = true;
    // Facing 0 (south) to east (192) is -64 the short way; 40% of it.
    director.set_input_mask(0x08);
    director.step().unwrap();
    assert_eq!(director.world().actors[&EntityId::new(1)].facing, 230);
    for _ in 0..20 {
        director.step().unwrap();
    }
    assert_eq!(director.world().actors[&EntityId::new(1)].facing, 192);
}
