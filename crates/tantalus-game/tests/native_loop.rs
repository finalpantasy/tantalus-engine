//! The packaged independent loop: one `FieldDirector` runs field events into
//! world state, resolves an encounter into a battle, commits rewards exactly
//! once, and survives a save/restore with world continuity.

use tantalus_contracts::{ContentId, EntityId, Tick};
use tantalus_game::battle::*;
use tantalus_game::{FieldDirector, FieldSnapshot, FieldWorld};

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
fn packaged_loop_events_battle_rewards_save_and_restore() {
    let mut director = FieldDirector::new(FieldWorld::new());
    director.world_mut().add_actor(EntityId::new(1), [0.0; 3]);

    // Gil grant, an async window open, then ENCOUNT 0x2A (battle 0x55).
    // ENCOUNT completes the entry (the reference jumps to battle), so it must
    // come last.
    let program = [
        0xce, 0, 100, 0, 0, // AddGil 100
        0x20, 0, 3, 0, 7, 0, // WindowAsync window 3
        0x2a, 0, 0, 0x55, 0x00, // ENCOUNT battle 0x55
        4,
    ];
    director.add_entry(EntityId::new(1), &program).unwrap();

    let step = director.step().unwrap();
    assert_eq!(step.tick, Tick::new(0));
    assert!(step.completed_entries.contains(&EntityId::new(1)));
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

    // The restored field keeps running: entries restart from tick zero
    // (documented `resumed_from_start` semantics), so the gil grant re-applies
    // while the battle reward ledger does not.
    let step = restored.step().unwrap();
    assert!(step.completed_entries.contains(&EntityId::new(1)));
    assert_eq!(restored.world().gil, 220);
    assert_eq!(restored.world().encounters.len(), 0);
}
