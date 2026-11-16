//! Zero-state dry run of decoded event functions.
//!
//! The probe executes a decoded function against a permissive host: every read returns zero or
//! a plain default, every wait query reports "finished", and every host event is only counted.
//! Two reads carry state so retry loops can end: random (system variable 0 and
//! `read_random_u8`) walks 0, 1, 2 ... 255 and wraps, and an object member reads back the
//! last value written to it, and `SetActorFacing` sets member 3 (facing) as the field host does.
//! It proves that real bytecode decodes *and* runs through the VM without a runtime error. It
//! does not model gameplay state, so the events it records are not a behavior claim.

use super::{
    CharacterStat, EventProgram, EventVm, HostEvent, KeyState, LegacyEventHost, ObjectMemberRef,
    VmError, VmStatus, WaitState,
};
use std::collections::BTreeMap;
use tantalus_contracts::{EngineError, EntityId, Tick};

/// Outcome of probing one decoded function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionProbe {
    pub status: VmStatus,
    /// The wait the function stopped on, when `status` is `Waiting`.
    pub wait: Option<WaitState>,
    pub ticks: u64,
    pub events: BTreeMap<String, usize>,
    pub error: Option<VmError>,
}

/// Runs `program` as `entity` for at most `max_ticks` ticks against a zero-state host.
pub fn probe_program(program: &EventProgram, entity: EntityId, max_ticks: u64) -> FunctionProbe {
    let mut vm = EventVm::new(program.clone()).with_entity(entity);
    let mut host = ProbeHost::default();
    let mut events = BTreeMap::new();
    let mut ticks = 0;
    let mut error = None;
    while ticks < max_ticks && vm.status() != VmStatus::Completed {
        match vm.step(Tick::new(ticks), &mut host) {
            Ok(report) => {
                for event in &report.events {
                    *events.entry(event_name(event)).or_insert(0) += 1;
                }
            }
            Err(failure) => {
                error = Some(failure);
                break;
            }
        }
        ticks += 1;
    }
    FunctionProbe {
        status: vm.status(),
        wait: vm.wait_state(),
        ticks,
        events,
        error,
    }
}

fn event_name(event: &HostEvent) -> String {
    let text = format!("{event:?}");
    text.split(|character: char| !character.is_alphanumeric())
        .next()
        .unwrap_or_default()
        .to_owned()
}

#[derive(Debug, Default)]
pub(crate) struct ProbeHost {
    random: u8,
    members: BTreeMap<ObjectMemberRef, i32>,
}

impl ProbeHost {
    fn next_random(&mut self) -> u8 {
        let value = self.random;
        self.random = self.random.wrapping_add(1);
        value
    }
}

impl LegacyEventHost for ProbeHost {
    fn apply(&mut self, events: &[HostEvent]) -> Result<(), EngineError> {
        for event in events {
            if let HostEvent::SetActorFacing { actor, angle } = event {
                // Member 3 is the facing byte, as the field host stores it.
                let reference = ObjectMemberRef {
                    object: *actor,
                    member: 3,
                };
                self.members
                    .insert(reference, i32::from((*angle >> 4) & 0xFF));
            }
        }
        Ok(())
    }

    fn window_is_open(&self, _window: u8) -> bool {
        false
    }

    fn read_object_member(&mut self, reference: ObjectMemberRef) -> Result<i32, EngineError> {
        Ok(self.members.get(&reference).copied().unwrap_or(0))
    }

    fn select_object_member_target(&mut self, slot: Option<u8>) -> Result<EntityId, EngineError> {
        Ok(EntityId::new(u64::from(slot.unwrap_or(0))))
    }

    fn write_object_member(
        &mut self,
        reference: ObjectMemberRef,
        value: i32,
    ) -> Result<(), EngineError> {
        self.members.insert(reference, value);
        Ok(())
    }

    fn read_system_variable(&mut self, variable: u8) -> Result<i32, EngineError> {
        // System variable 0 is `Comn.random8()`.
        Ok(if variable == 0 {
            i32::from(self.next_random())
        } else {
            0
        })
    }

    fn read_system_list(&mut self, _index: u8) -> Result<i32, EngineError> {
        Ok(0)
    }

    fn write_system_list(&mut self, _index: u8, _value: i32) -> Result<(), EngineError> {
        Ok(())
    }

    fn read_atan_entry(&mut self, _offset: i32) -> Result<u16, EngineError> {
        Ok(0)
    }

    fn resolve_object_uid(&mut self, uid: i32) -> Result<EntityId, EngineError> {
        Ok(EntityId::new(u64::from(uid as u32)))
    }

    fn find_object_uid(&mut self, uid: u8) -> Result<Option<i32>, EngineError> {
        Ok(Some(i32::from(uid)))
    }

    fn read_object_position(&mut self, _object: EntityId) -> Result<[f32; 3], EngineError> {
        Ok([0.0; 3])
    }

    fn read_item_count(&mut self, _item: i32) -> Result<i32, EngineError> {
        Ok(0)
    }

    fn read_animation_frame(
        &mut self,
        _actor: EntityId,
        _animation: i32,
    ) -> Result<i32, EngineError> {
        Ok(0)
    }

    fn read_bgi_animation_frame(&mut self, _animation: u32) -> Result<i32, EngineError> {
        Ok(0)
    }

    fn read_bgi_character_triangle(&mut self, _character: i32) -> Result<i32, EngineError> {
        Ok(0)
    }

    fn read_bgi_character_floor(&mut self, _character: i32) -> Result<i32, EngineError> {
        Ok(0)
    }

    fn read_character_max_mp(&mut self, _character: i32) -> Result<i32, EngineError> {
        Ok(0)
    }

    fn party_contains(&mut self, _character: i32) -> Result<bool, EngineError> {
        Ok(false)
    }

    fn add_party_member(&mut self, _character: i32) -> Result<bool, EngineError> {
        Ok(false)
    }

    fn party_menu_is_open(&self) -> bool {
        false
    }

    fn actor_turn_is_active(&mut self, _actor: EntityId) -> Result<bool, EngineError> {
        Ok(false)
    }

    fn shared_script_is_active(&mut self, _actor: EntityId) -> Result<bool, EngineError> {
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

    fn actor_jump_is_active(&mut self, _actor: EntityId) -> Result<bool, EngineError> {
        Ok(false)
    }

    fn read_random_u8(&mut self) -> Result<u8, EngineError> {
        Ok(self.next_random())
    }

    fn read_key_on(&mut self) -> Result<i32, EngineError> {
        Ok(0)
    }

    fn read_key_state(&mut self, _state: KeyState) -> Result<i32, EngineError> {
        Ok(0)
    }

    fn read_character_stat(
        &mut self,
        _character: i32,
        _stat: CharacterStat,
    ) -> Result<i32, EngineError> {
        Ok(0)
    }

    fn caller_request_acceptable(&mut self, _level: i32) -> Result<bool, EngineError> {
        Ok(true)
    }

    fn script_request_acceptable(
        &mut self,
        _target: EntityId,
        _level: i32,
    ) -> Result<bool, EngineError> {
        Ok(true)
    }

    fn script_request_is_complete(&mut self, _entity: EntityId) -> Result<bool, EngineError> {
        Ok(true)
    }

    fn read_custom_variable(&mut self, _variable: u16) -> Result<i32, EngineError> {
        Ok(0)
    }
}
