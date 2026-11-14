use tantalus_contracts::{EngineError, EntityId, ErrorCategory, Tick};
use tantalus_game::field::FieldScene;
use tantalus_legacy::{HostEvent, LegacyEventHost, VmStatus, decode_program};

fn movement_program(actor: i32) -> tantalus_legacy::EventProgram {
    let mut bytes = vec![0xbf, 0];
    for value in [actor, 0, 0] {
        bytes.push(126);
        bytes.extend(value.to_le_bytes());
        bytes.push(127);
    }
    bytes.push(4);
    decode_program(&bytes).unwrap()
}

#[derive(Default)]
struct Host {
    actors: Vec<u64>,
}

impl LegacyEventHost for Host {
    fn apply(&mut self, events: &[HostEvent]) -> Result<(), EngineError> {
        for event in events {
            if let HostEvent::MoveActor { actor, .. } = event {
                self.actors.push(actor.get());
            }
        }
        Ok(())
    }

    fn window_is_open(&self, _window: u8) -> bool {
        false
    }
}

#[test]
fn entries_execute_in_stable_entity_order() {
    let mut field = FieldScene::new();
    field
        .add_entry(EntityId::new(2), movement_program(200))
        .unwrap();
    field
        .add_entry(EntityId::new(1), movement_program(100))
        .unwrap();

    let mut host = Host::default();
    let reports = field.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(host.actors, vec![100, 200]);
    assert_eq!(reports[0].entity, EntityId::new(1));
    assert_eq!(reports[1].entity, EntityId::new(2));
    assert!(
        reports
            .iter()
            .all(|report| report.report.status == VmStatus::Completed)
    );
    assert_eq!(field.len(), 2);
}

#[test]
fn duplicate_entries_are_rejected() {
    let mut field = FieldScene::new();
    field
        .add_entry(EntityId::new(1), movement_program(100))
        .unwrap();
    let error = field
        .add_entry(EntityId::new(1), movement_program(100))
        .unwrap_err();
    assert_eq!(error.category, ErrorCategory::Conflict);
}
