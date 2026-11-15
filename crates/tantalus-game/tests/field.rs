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
    controls: Vec<u64>,
}

impl LegacyEventHost for Host {
    fn apply(&mut self, events: &[HostEvent]) -> Result<(), EngineError> {
        for event in events {
            match event {
                HostEvent::MoveActor { actor, .. } => self.actors.push(actor.get()),
                HostEvent::DefinePlayerControl { actor } => self.controls.push(actor.get()),
                _ => {}
            }
        }
        Ok(())
    }

    fn window_is_open(&self, _window: u8) -> bool {
        false
    }

    fn read_object_member(
        &mut self,
        _reference: tantalus_legacy::ObjectMemberRef,
    ) -> Result<i32, EngineError> {
        Err(EngineError::new(
            ErrorCategory::UnknownContent,
            "field fixture has no object members",
            "fixture-host",
            true,
        ))
    }

    fn select_object_member_target(&mut self, _slot: Option<u8>) -> Result<EntityId, EngineError> {
        Err(EngineError::new(
            ErrorCategory::UnknownContent,
            "field fixture has no object-member targets",
            "fixture-host",
            true,
        ))
    }

    fn write_object_member(
        &mut self,
        _reference: tantalus_legacy::ObjectMemberRef,
        _value: i32,
    ) -> Result<(), EngineError> {
        Err(EngineError::new(
            ErrorCategory::UnknownContent,
            "field fixture has no object members",
            "fixture-host",
            true,
        ))
    }

    fn read_system_variable(&mut self, variable: u8) -> Result<i32, EngineError> {
        Err(EngineError::new(
            ErrorCategory::UnknownContent,
            format!("field fixture has no system variable {variable}"),
            "fixture-host",
            true,
        ))
    }

    fn read_atan_entry(&mut self, offset: i32) -> Result<u16, EngineError> {
        Err(EngineError::new(
            ErrorCategory::UnknownContent,
            format!("field fixture has no arctangent table entry at {offset}"),
            "fixture-host",
            true,
        ))
    }

    fn resolve_object_uid(&mut self, uid: i32) -> Result<EntityId, EngineError> {
        Err(EngineError::new(
            ErrorCategory::UnknownContent,
            format!("field fixture has no object UID {uid}"),
            "fixture-host",
            true,
        ))
    }

    fn find_object_uid(&mut self, _uid: u8) -> Result<Option<i32>, EngineError> {
        Ok(None)
    }

    fn read_object_position(&mut self, entity: EntityId) -> Result<[f32; 3], EngineError> {
        Err(EngineError::new(
            ErrorCategory::UnknownContent,
            format!("field fixture has no position for {entity}"),
            "fixture-host",
            true,
        ))
    }

    fn read_item_count(&mut self, item: i32) -> Result<i32, EngineError> {
        Err(EngineError::new(
            ErrorCategory::UnknownContent,
            format!("field fixture has no item count for {item}"),
            "fixture-host",
            true,
        ))
    }

    fn read_animation_frame(
        &mut self,
        entity: EntityId,
        animation: i32,
    ) -> Result<i32, EngineError> {
        Err(EngineError::new(
            ErrorCategory::UnknownContent,
            format!("field fixture has no animation {animation} frame for {entity}"),
            "fixture-host",
            true,
        ))
    }

    fn read_bgi_animation_frame(&mut self, animation: u32) -> Result<i32, EngineError> {
        Err(EngineError::new(
            ErrorCategory::UnknownContent,
            format!("field fixture has no BGI animation {animation} frame"),
            "fixture-host",
            true,
        ))
    }

    fn read_bgi_character_triangle(&mut self, character: i32) -> Result<i32, EngineError> {
        Err(EngineError::new(
            ErrorCategory::UnknownContent,
            format!("field fixture has no BGI character {character}"),
            "fixture-host",
            true,
        ))
    }

    fn read_character_max_mp(&mut self, character: i32) -> Result<i32, EngineError> {
        Err(EngineError::new(
            ErrorCategory::UnknownContent,
            format!("field fixture has no maximum MP for character {character}"),
            "fixture-host",
            true,
        ))
    }

    fn party_contains(&mut self, character: i32) -> Result<bool, EngineError> {
        Err(EngineError::new(
            ErrorCategory::UnknownContent,
            format!("field fixture cannot check party character {character}"),
            "fixture-host",
            true,
        ))
    }

    fn add_party_member(&mut self, character: i32) -> Result<bool, EngineError> {
        Err(EngineError::new(
            ErrorCategory::UnknownContent,
            format!("field fixture cannot add party character {character}"),
            "fixture-host",
            true,
        ))
    }

    fn party_menu_is_open(&self) -> bool {
        false
    }

    fn actor_turn_is_active(&mut self, _actor: EntityId) -> Result<bool, EngineError> {
        Ok(false)
    }

    fn actor_animation_is_active(&mut self, _actor: EntityId) -> Result<bool, EngineError> {
        Ok(false)
    }

    fn field_actor_movement_is_active(&mut self, _actor: EntityId) -> Result<bool, EngineError> {
        Ok(false)
    }

    fn field_exit_movement_is_active(&mut self, _actor: EntityId) -> Result<bool, EngineError> {
        Ok(false)
    }

    fn read_random_u8(&mut self) -> Result<u8, EngineError> {
        Err(EngineError::new(
            ErrorCategory::UnknownContent,
            "field fixture has no random-byte source",
            "fixture-host",
            true,
        ))
    }

    fn read_key_on(&mut self) -> Result<i32, EngineError> {
        Err(EngineError::new(
            ErrorCategory::UnknownContent,
            "field fixture has no key-on state",
            "fixture-host",
            true,
        ))
    }

    fn caller_request_acceptable(&mut self, _level: i32) -> Result<bool, EngineError> {
        Ok(false)
    }

    fn script_request_acceptable(
        &mut self,
        _target: EntityId,
        _level: i32,
    ) -> Result<bool, EngineError> {
        Ok(false)
    }

    fn script_request_is_complete(&mut self, entity: EntityId) -> Result<bool, EngineError> {
        Err(EngineError::new(
            ErrorCategory::UnknownContent,
            format!("field fixture cannot track script completion for {entity}"),
            "fixture-host",
            true,
        ))
    }

    fn read_custom_variable(&mut self, variable: u16) -> Result<i32, EngineError> {
        Err(EngineError::new(
            ErrorCategory::UnknownContent,
            format!("field fixture has no custom variable {variable}"),
            "fixture-host",
            true,
        ))
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

#[test]
fn field_entries_supply_actor_identity_for_player_control() {
    let mut field = FieldScene::new();
    field
        .add_entry(EntityId::new(42), decode_program(&[0x2c, 0, 4]).unwrap())
        .unwrap();
    let mut host = Host::default();
    field.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(host.controls, [42]);
}
