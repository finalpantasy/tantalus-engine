use std::collections::BTreeMap;
use tantalus_contracts::{EngineError, EntityId, ErrorCategory, Tick};
use tantalus_legacy::*;

fn constant(value: i32) -> Vec<u8> {
    let mut bytes = vec![126];
    bytes.extend_from_slice(&value.to_le_bytes());
    bytes
}

fn append_expression(program: &mut Vec<u8>, expression: Vec<u8>) {
    program.extend(expression);
    program.push(127);
}

fn variable(scope: VariableScope, value_type: VariableType, index: u16) -> Vec<u8> {
    let source = match scope {
        VariableScope::Global => 0,
        VariableScope::Map => 1,
        VariableScope::Instance => 2,
    };
    let kind = match value_type {
        VariableType::SignedBit => 0,
        VariableType::Bit => 1,
        VariableType::Int24 => 2,
        VariableType::UInt24 => 3,
        VariableType::SignedByte => 4,
        VariableType::Byte => 5,
        VariableType::Int16 => 6,
        VariableType::UInt16 => 7,
    };
    let mut bytes = vec![0xc0 | source | (kind << 2) | if index > 255 { 0x20 } else { 0 }];
    bytes.push(index as u8);
    if index > 255 {
        bytes.push((index >> 8) as u8);
    }
    bytes
}

fn binary(left: Vec<u8>, right: Vec<u8>, opcode: u8) -> Vec<u8> {
    let mut bytes = left;
    bytes.extend(right);
    bytes.push(opcode);
    bytes
}

fn assign(target: Vec<u8>, value: Vec<u8>) -> Vec<u8> {
    binary(target, value, 44)
}

fn evaluate(expression: Vec<u8>) -> Vec<u8> {
    let mut bytes = vec![5];
    bytes.extend(expression);
    bytes.push(127);
    bytes
}

fn movement(actor: i32, x: i32, z: i32) -> Vec<u8> {
    let mut bytes = vec![0xbf, 0];
    for expression in [actor, x, z] {
        bytes.extend(constant(expression));
        bytes.push(127);
    }
    bytes
}

#[derive(Debug)]
struct Host {
    windows: BTreeMap<u8, bool>,
    movements: Vec<(u64, i32, i32)>,
    slots: BTreeMap<u8, EntityId>,
    member_target: Option<EntityId>,
    object_members: BTreeMap<ObjectMemberRef, i32>,
    member_writes: Vec<(ObjectMemberRef, i32)>,
    script_request_complete: bool,
    caller_request_acceptable: bool,
    script_request_acceptable: bool,
    object_uids: BTreeMap<i32, EntityId>,
    object_positions: BTreeMap<EntityId, [f32; 3]>,
    item_counts: BTreeMap<i32, i32>,
    animation_frames: BTreeMap<(EntityId, i32), i32>,
    bgi_animation_frames: BTreeMap<u32, i32>,
    party_members: Vec<i32>,
    party_menu_open: bool,
    actor_turn_active: bool,
    actor_animation_active: bool,
    actor_movement_active: bool,
    exit_movement_active: bool,
    random_u8: u8,
    key_on: i32,
    fail: bool,
}

impl Host {
    fn new() -> Self {
        Self {
            windows: BTreeMap::new(),
            movements: Vec::new(),
            slots: BTreeMap::new(),
            member_target: None,
            object_members: BTreeMap::new(),
            member_writes: Vec::new(),
            script_request_complete: false,
            caller_request_acceptable: false,
            script_request_acceptable: false,
            object_uids: BTreeMap::new(),
            object_positions: BTreeMap::new(),
            item_counts: BTreeMap::new(),
            animation_frames: BTreeMap::new(),
            bgi_animation_frames: BTreeMap::new(),
            party_members: Vec::new(),
            party_menu_open: false,
            actor_turn_active: false,
            actor_animation_active: false,
            actor_movement_active: false,
            exit_movement_active: false,
            random_u8: 0,
            key_on: 0,
            fail: false,
        }
    }

    fn target_error(message: impl Into<String>) -> EngineError {
        EngineError::new(ErrorCategory::UnknownContent, message, "fixture-host", true)
    }
}

impl LegacyEventHost for Host {
    fn apply(&mut self, events: &[HostEvent]) -> Result<(), EngineError> {
        if self.fail {
            return Err(EngineError::new(
                ErrorCategory::Internal,
                "synthetic host failure",
                "fixture-host",
                true,
            ));
        }
        for event in events {
            match event {
                HostEvent::ShowWindow { window, .. } => {
                    self.windows.insert(*window, true);
                }
                HostEvent::CloseWindow { window } => {
                    self.windows.insert(*window, false);
                }
                HostEvent::MoveActor { actor, x, z } => {
                    self.movements.push((actor.get(), *x, *z));
                }
                HostEvent::AddItem { .. } => {}
                HostEvent::RemoveItem { .. } => {}
                HostEvent::AddGil { .. } => {}
                HostEvent::RemoveGil { .. } => {}
                HostEvent::SetPartyHp { .. } => {}
                HostEvent::SetPartyMp { .. } => {}
                HostEvent::ClearAbilityAp { .. } => {}
                HostEvent::MasterAbility { .. } => {}
                HostEvent::WalkActor { .. } => {}
                HostEvent::MoveActorToExit { .. } => {}
                HostEvent::DeleteObject { .. } => {}
                HostEvent::SetActorPosition { .. } => {}
                HostEvent::SetIdleAnimation { .. } => {}
                HostEvent::SetWalkAnimation { .. } => {}
                HostEvent::SetRunAnimation { .. } => {}
                HostEvent::SetLeftTurnAnimation { .. } => {}
                HostEvent::SetRightTurnAnimation { .. } => {}
                HostEvent::SetJumpAnimation { .. } => {}
                HostEvent::SetActorRadii { .. } => {}
                HostEvent::SetIdleAnimationSpeeds { .. } => {}
                HostEvent::SetHeadFocusMasks { .. } => {}
                HostEvent::SetObjectFlags { .. } => {}
                HostEvent::SetActorFacing { .. } => {}
                HostEvent::StartActorTurn { .. } => {}
                HostEvent::AimAtCamera => {}
                HostEvent::DefinePlayerControl { .. } => {}
                HostEvent::SetControlDirection { .. } => {}
                HostEvent::InitializeObject { .. } => {}
                HostEvent::InitializeRegion { .. } => {}
                HostEvent::InitializeCode { .. } => {}
                HostEvent::SetTriangleActive { .. } => {}
                HostEvent::RunSound { .. } => {}
                HostEvent::AttachObjectOffset { .. } => {}
                HostEvent::ResetWalkState { .. } => {}
                HostEvent::SetMovementSpeed { .. } => {}
                HostEvent::SetActorModel { .. } => {}
                HostEvent::SetNavigationAttributeMask { .. } => {}
                HostEvent::SetShadowEnabled { .. } => {}
                HostEvent::SetTextVariable { .. } => {}
                HostEvent::ShowBattleMessage { .. } => {}
                HostEvent::SetObjectScale { .. } => {}
                HostEvent::SetVibrationFrameRate { .. } => {}
                HostEvent::RequestScriptAsync { .. } => {}
                HostEvent::DefineTileViewport { .. } => {}
                HostEvent::PreserveAndHideAll => {}
                HostEvent::RestoreAllVisibility => {}
                HostEvent::SetTimerRunning { .. } => {}
                HostEvent::RequestCallerAsync { .. } => {}
                HostEvent::CalculateExitPosition => {}
                HostEvent::CalculateScreenPosition { .. } => {}
                HostEvent::RequestCallerSync { .. } => {}
                HostEvent::SetSpsParameters { .. } => {}
                HostEvent::SetTileShadeColor { .. } => {}
                HostEvent::SetSimulationFloor { .. } => {}
                HostEvent::SetSimulationFrameRate { .. } => {}
                HostEvent::SetTileBlockActive { .. } => {}
                HostEvent::SetTileLoop { .. } => {}
                HostEvent::SetTileParallax { .. } => {}
                HostEvent::SetTileOrigin { .. } => {}
                HostEvent::RunTileAnimation { .. } => {}
                HostEvent::SetTileAnimationActive { .. } => {}
                HostEvent::ResetTileAnimation { .. } => {}
                HostEvent::SetTileAnimationRate { .. } => {}
                HostEvent::SetTileAnimationFrameWait { .. } => {}
                HostEvent::SetTileAnimationFlags { .. } => {}
                HostEvent::SetTileAnimationRange { .. } => {}
                HostEvent::IncrementFrogCount => {}
                HostEvent::SetTileAnimationFrame { .. } => {}
                HostEvent::OpenPartyMenu { .. } => self.party_menu_open = true,
                HostEvent::InitializeCharacter { .. } => {}
                HostEvent::StopTextureAnimation { .. } => {}
                HostEvent::RunBattleCode { .. } => {}
                HostEvent::SetBattleMusicContinuation { .. } => {}
                HostEvent::RunModelCode { .. } => {}
                HostEvent::SetRunEnabled { .. } => {}
                HostEvent::SetHereIcon { .. } => {}
                HostEvent::SetPartyAvailability { .. } => {}
                HostEvent::MoveTile { .. } => {}
                HostEvent::SetTileScrollWithOffset { .. } => {}
                HostEvent::SetTileViewportAlpha { .. } => {}
                HostEvent::SetCameraBounds { .. } => {}
                HostEvent::SetCharacterRow { .. } => {}
                HostEvent::SetActorPitchRoll { .. } => {}
                HostEvent::AttachTile { .. } => {}
                HostEvent::DispatchCinematic { .. } => {}
                HostEvent::DefineRegion { .. } => {}
                HostEvent::ClearPartyStatus { .. } => {}
                HostEvent::PlayAnimation { .. } => {}
                HostEvent::SetAnimationRange { .. } => {}
                HostEvent::SetHeadFocusFlags { .. } => {}
                HostEvent::SetWalkTurnSpeed { .. } => {}
                HostEvent::PreloadField { .. } => {}
                HostEvent::SetModelLight { .. } => {}
                HostEvent::AddBattleStatus { .. } => {}
                HostEvent::ChangeElementalAffinity { .. } => {}
                HostEvent::SetRain { .. } => {}
                HostEvent::ChangeField { .. } => {}
            }
        }
        Ok(())
    }

    fn window_is_open(&self, window: u8) -> bool {
        self.windows.get(&window).copied().unwrap_or(false)
    }

    fn read_object_member(&mut self, reference: ObjectMemberRef) -> Result<i32, EngineError> {
        Ok(self
            .object_members
            .get(&reference)
            .copied()
            .unwrap_or(((reference.object.get() as i32) << 8) | reference.member as i32))
    }

    fn select_object_member_target(&mut self, slot: Option<u8>) -> Result<EntityId, EngineError> {
        let object = match slot {
            Some(slot) => self
                .slots
                .get(&slot)
                .copied()
                .ok_or_else(|| Self::target_error(format!("object slot {slot} is empty")))?,
            None => self
                .member_target
                .ok_or_else(|| Self::target_error("no current object-member target"))?,
        };
        if slot.is_some() {
            self.member_target = Some(object);
        }
        Ok(object)
    }

    fn write_object_member(
        &mut self,
        reference: ObjectMemberRef,
        value: i32,
    ) -> Result<(), EngineError> {
        self.object_members.insert(reference, value);
        self.member_writes.push((reference, value));
        Ok(())
    }

    fn read_system_variable(&mut self, variable: u8) -> Result<i32, EngineError> {
        if variable == 5 {
            Ok(77)
        } else {
            Err(Self::target_error(format!(
                "unknown system variable {variable}"
            )))
        }
    }

    fn read_atan_entry(&mut self, offset: i32) -> Result<u16, EngineError> {
        match offset {
            0 => Ok(0),
            1024 => Ok(128),
            _ => Err(Self::target_error(format!(
                "unknown arctangent table offset {offset}"
            ))),
        }
    }

    fn resolve_object_uid(&mut self, uid: i32) -> Result<EntityId, EngineError> {
        self.object_uids
            .get(&uid)
            .copied()
            .ok_or_else(|| Self::target_error(format!("unknown object UID {uid}")))
    }

    fn find_object_uid(&mut self, uid: u8) -> Result<Option<i32>, EngineError> {
        let uid = i32::from(uid);
        Ok(self.object_uids.contains_key(&uid).then_some(uid))
    }

    fn read_object_position(&mut self, object: EntityId) -> Result<[f32; 3], EngineError> {
        self.object_positions
            .get(&object)
            .copied()
            .ok_or_else(|| Self::target_error(format!("unknown position for object {object}")))
    }

    fn read_item_count(&mut self, item: i32) -> Result<i32, EngineError> {
        Ok(self.item_counts.get(&item).copied().unwrap_or(0))
    }

    fn read_animation_frame(
        &mut self,
        actor: EntityId,
        animation: i32,
    ) -> Result<i32, EngineError> {
        self.animation_frames
            .get(&(actor, animation))
            .copied()
            .ok_or_else(|| Self::target_error(format!("unknown animation {animation} for {actor}")))
    }

    fn read_bgi_animation_frame(&mut self, animation: u32) -> Result<i32, EngineError> {
        self.bgi_animation_frames
            .get(&animation)
            .copied()
            .ok_or_else(|| Self::target_error(format!("unknown BGI animation {animation}")))
    }

    fn read_bgi_character_triangle(&mut self, character: i32) -> Result<i32, EngineError> {
        match character {
            7 => Ok(23),
            _ => Err(Self::target_error(format!(
                "unknown BGI character {character}"
            ))),
        }
    }

    fn read_character_max_mp(&mut self, character: i32) -> Result<i32, EngineError> {
        match character {
            7 => Ok(80),
            _ => Err(Self::target_error(format!(
                "unknown maximum MP for character {character}"
            ))),
        }
    }

    fn party_contains(&mut self, character: i32) -> Result<bool, EngineError> {
        Ok(self.party_members.contains(&character))
    }

    fn add_party_member(&mut self, character: i32) -> Result<bool, EngineError> {
        if character < 0 {
            return Ok(true);
        }
        if self.party_members.contains(&character) {
            return Ok(false);
        }
        if self.party_members.len() == 4 {
            return Ok(true);
        }
        self.party_members.push(character);
        Ok(false)
    }

    fn party_menu_is_open(&self) -> bool {
        self.party_menu_open
    }

    fn actor_turn_is_active(&mut self, _actor: EntityId) -> Result<bool, EngineError> {
        Ok(self.actor_turn_active)
    }

    fn actor_animation_is_active(&mut self, _actor: EntityId) -> Result<bool, EngineError> {
        Ok(self.actor_animation_active)
    }

    fn field_actor_movement_is_active(&mut self, _actor: EntityId) -> Result<bool, EngineError> {
        Ok(self.actor_movement_active)
    }

    fn field_exit_movement_is_active(&mut self, _actor: EntityId) -> Result<bool, EngineError> {
        Ok(self.exit_movement_active)
    }

    fn read_random_u8(&mut self) -> Result<u8, EngineError> {
        Ok(self.random_u8)
    }

    fn read_key_on(&mut self) -> Result<i32, EngineError> {
        Ok(self.key_on)
    }

    fn caller_request_acceptable(&mut self, _level: i32) -> Result<bool, EngineError> {
        Ok(self.caller_request_acceptable)
    }

    fn script_request_acceptable(
        &mut self,
        _target: EntityId,
        _level: i32,
    ) -> Result<bool, EngineError> {
        Ok(self.script_request_acceptable)
    }

    fn script_request_is_complete(&mut self, _entity: EntityId) -> Result<bool, EngineError> {
        Ok(self.script_request_complete)
    }

    fn read_custom_variable(&mut self, variable: u16) -> Result<i32, EngineError> {
        if variable == 0 {
            Ok(17)
        } else {
            Err(EngineError::new(
                ErrorCategory::UnknownContent,
                format!("unknown custom variable {variable}"),
                "fixture-host",
                true,
            ))
        }
    }
}

#[test]
fn expressions_assign_and_preserve_integer_widths() {
    let mut bytes = Vec::new();
    bytes.extend(evaluate(assign(
        variable(VariableScope::Global, VariableType::Int24, 0),
        binary(constant(2), binary(constant(3), constant(4), 17), 20),
    )));
    bytes.extend(evaluate(assign(
        variable(VariableScope::Map, VariableType::Byte, 1),
        constant(300),
    )));
    bytes.extend(evaluate(assign(
        variable(VariableScope::Instance, VariableType::Int24, 2),
        constant(0x00ff_ffff),
    )));
    bytes.extend(evaluate(assign(
        variable(VariableScope::Global, VariableType::Int24, 3),
        binary(constant(9), constant(0), 18),
    )));
    bytes.push(4);

    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(report.status, VmStatus::Completed);
    assert_eq!(vm.variable(VariableScope::Global, 0), 14);
    assert_eq!(vm.variable(VariableScope::Map, 1), 44);
    assert_eq!(vm.variable(VariableScope::Instance, 2), -1);
    assert_eq!(vm.variable(VariableScope::Global, 3), 9);
}

#[test]
fn conditional_jumps_select_the_correct_signed_target() {
    for (opcode, initial, expected_x) in [(3, 1, 50), (2, 0, 50), (3, 0, -50), (2, 1, -50)] {
        let mut bytes = evaluate(assign(
            variable(VariableScope::Global, VariableType::Int24, 0),
            constant(initial),
        ));
        bytes.push(opcode);
        append_expression(
            &mut bytes,
            variable(VariableScope::Global, VariableType::Int24, 0),
        );
        let mut wrong = movement(1, -50, 0);
        wrong.push(4);
        let relative = wrong.len() as i16;
        bytes.extend(relative.to_le_bytes());
        bytes.extend(wrong);
        bytes.extend(movement(1, expected_x, 0));
        bytes.push(4);

        let mut vm = EventVm::new(decode_program(&bytes).unwrap());
        let mut host = Host::new();
        vm.step(Tick::new(0), &mut host).unwrap();
        assert_eq!(
            host.movements,
            vec![(1, expected_x, 0)],
            "opcode {opcode}, value {initial}"
        );
    }
}

#[test]
fn timed_wait_resumes_on_the_declared_tick() {
    let mut bytes = vec![0x22, 0];
    append_expression(&mut bytes, constant(3));
    bytes.extend(movement(1, 50, 0));
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let mut host = Host::new();

    for (tick, expected) in [(0, 2), (1, 1), (2, 0)] {
        let report = vm.step(Tick::new(tick), &mut host).unwrap();
        assert_eq!(report.status, VmStatus::Waiting);
        assert_eq!(vm.wait_state(), Some(WaitState::Frames(expected)));
        assert!(host.movements.is_empty());
    }
    let report = vm.step(Tick::new(3), &mut host).unwrap();
    assert_eq!(report.status, VmStatus::Completed);
    assert_eq!(host.movements, vec![(1, 50, 0)]);
}

#[test]
fn wait_turn_yields_then_resumes_after_the_actor_turn_ends() {
    let program = decode_program(&[80, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 1);
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    host.actor_turn_active = true;

    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.status, VmStatus::Waiting);
    assert_eq!(vm.wait_state(), Some(WaitState::ActorTurn));
    assert_eq!(
        vm.step(Tick::new(1), &mut host).unwrap().status,
        VmStatus::Waiting
    );

    host.actor_turn_active = false;
    assert_eq!(
        vm.step(Tick::new(2), &mut host).unwrap().status,
        VmStatus::Completed
    );

    let mut vm = EventVm::new(decode_program(&[80, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("actor context"));
}

#[test]
fn wait_animation_continues_immediately_or_waits_for_completion() {
    let program = decode_program(&[65, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 1);
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    assert_eq!(
        vm.step(Tick::new(0), &mut host).unwrap().status,
        VmStatus::Completed
    );

    let mut vm = EventVm::new(decode_program(&[65, 4]).unwrap()).with_entity(EntityId::new(42));
    host.actor_animation_active = true;
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.status, VmStatus::Waiting);
    assert_eq!(vm.pc(), 0);
    assert_eq!(vm.wait_state(), Some(WaitState::ActorAnimation));
    assert_eq!(
        vm.step(Tick::new(1), &mut host).unwrap().status,
        VmStatus::Waiting
    );

    host.actor_animation_active = false;
    assert_eq!(
        vm.step(Tick::new(2), &mut host).unwrap().status,
        VmStatus::Completed
    );

    let mut vm = EventVm::new(decode_program(&[65, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("actor context"));
}

#[test]
fn synchronous_window_waits_for_close_while_async_continues() {
    let mut sync = vec![0x1f, 0];
    append_expression(&mut sync, constant(2));
    append_expression(&mut sync, constant(0));
    append_expression(&mut sync, constant(100));
    sync.push(4);
    let mut vm = EventVm::new(decode_program(&sync).unwrap());
    let mut host = Host::new();

    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.status, VmStatus::Waiting);
    assert_eq!(vm.wait_state(), Some(WaitState::Window(2)));
    assert!(vm.step(Tick::new(1), &mut host).unwrap().events.is_empty());
    host.windows.insert(2, false);
    assert_eq!(
        vm.step(Tick::new(2), &mut host).unwrap().status,
        VmStatus::Completed
    );

    let mut async_program = vec![0x20, 0];
    append_expression(&mut async_program, constant(3));
    append_expression(&mut async_program, constant(0));
    append_expression(&mut async_program, constant(101));
    async_program.push(4);
    let mut vm = EventVm::new(decode_program(&async_program).unwrap());
    let mut host = Host::new();
    assert_eq!(
        vm.step(Tick::new(0), &mut host).unwrap().status,
        VmStatus::Completed
    );
    assert!(host.window_is_open(3));
}

#[test]
fn close_window_and_actor_movement_reach_the_host_once() {
    let mut bytes = vec![0x20, 0];
    append_expression(&mut bytes, constant(3));
    append_expression(&mut bytes, constant(0));
    append_expression(&mut bytes, constant(101));
    bytes.push(0x21);
    bytes.push(0);
    append_expression(&mut bytes, constant(3));
    bytes.extend(movement(7, -100, 200));
    bytes.push(4);

    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.events.len(), 3);
    assert!(!host.window_is_open(3));
    assert_eq!(host.movements, vec![(7, -100, 200)]);
    assert!(vm.step(Tick::new(1), &mut host).unwrap().events.is_empty());
}

#[test]
fn unsupported_and_truncated_programs_report_the_byte_offset() {
    let unsupported = decode_program(&[0x0c]).unwrap_err();
    assert_eq!(unsupported.kind, VmErrorKind::UnsupportedOpcode);
    assert_eq!(unsupported.offset, 0);
    assert_eq!(unsupported.opcode, Some(0x0c));

    let truncated = decode_program(&[5, 126, 1]).unwrap_err();
    assert_eq!(truncated.kind, VmErrorKind::Decode);
    assert_eq!(truncated.offset, 3);
    assert!(truncated.message.contains("instruction at 0"));
    assert!(truncated.message.contains("opcode Some(5)"));

    let truncated_variable = decode_program(&[5, 0xaf, 0xff]).unwrap_err();
    assert_eq!(truncated_variable.kind, VmErrorKind::Decode);
    assert_eq!(truncated_variable.offset, 3);
    assert_eq!(truncated_variable.opcode, Some(0xaf));
    assert!(
        truncated_variable
            .message
            .contains("two-byte custom variable index")
    );
}

#[test]
fn jump_switch_selects_consecutive_cases_and_uses_default_outside_range() {
    fn switch_program(value: i32) -> Vec<u8> {
        let mut bytes = vec![5];
        append_expression(&mut bytes, constant(value));
        bytes.extend_from_slice(&[11, 2, 5, 0, 11, 0, 9, 0, 9, 0, 0, 0, 4]);
        bytes
    }

    let program = decode_program(&switch_program(5)).unwrap();
    let InstructionKind::JumpSwitch {
        base,
        default_relative,
        case_relative,
        ..
    } = &program.instructions[&7].kind
    else {
        panic!("expected switch");
    };
    assert_eq!(*base, 5);
    assert_eq!(*default_relative, 11);
    assert_eq!(case_relative, &[9, 9]);

    for value in [5, 6] {
        let mut vm = EventVm::new(decode_program(&switch_program(value)).unwrap());
        let mut host = Host::new();
        let report = vm.step(Tick::new(0), &mut host).unwrap();
        assert_eq!(report.status, VmStatus::Running);
        assert_eq!(vm.pc(), 19);
        assert_eq!(
            vm.step(Tick::new(1), &mut host).unwrap().status,
            VmStatus::Completed
        );
    }

    for value in [4, 7] {
        let mut vm = EventVm::new(decode_program(&switch_program(value)).unwrap());
        let mut host = Host::new();
        let report = vm.step(Tick::new(0), &mut host).unwrap();
        assert_eq!(report.status, VmStatus::Completed);
    }

    assert!(decode_program(&[11, 1, 0]).is_err());
}

#[test]
fn jump_switch_ex_matches_explicit_values_and_masks_the_expression() {
    fn switch_program(value: i32) -> Vec<u8> {
        const CASES: [[u16; 2]; 2] = [[100, 8], [u16::MAX, 10]];
        const DEFAULT: u16 = 12;

        let mut bytes = vec![5];
        append_expression(&mut bytes, constant(value));
        bytes.extend_from_slice(&[6, CASES.len() as u8]);
        bytes.extend_from_slice(&DEFAULT.to_le_bytes());
        for [case, relative] in CASES {
            bytes.extend_from_slice(&case.to_le_bytes());
            bytes.extend_from_slice(&relative.to_le_bytes());
        }
        bytes.extend_from_slice(&[0, 0, 0, 0, 4]);
        bytes
    }

    let program = decode_program(&switch_program(100)).unwrap();
    let instruction = &program.instructions[&7];
    assert_eq!(instruction.length, 12);
    let InstructionKind::JumpSwitchEx {
        default_relative,
        cases,
    } = &instruction.kind
    else {
        panic!("expected extended switch");
    };
    assert_eq!(*default_relative, 12);
    assert_eq!(cases, &[[100, 8], [u16::MAX, 10]]);

    for (value, expected_pc) in [(100, 21), (u16::MAX.into(), 23), (-1, 23)] {
        let mut vm = EventVm::new(decode_program(&switch_program(value)).unwrap());
        let mut host = Host::new();
        let report = vm.step(Tick::new(0), &mut host).unwrap();
        assert_eq!(report.status, VmStatus::Running);
        assert_eq!(vm.pc(), expected_pc);
        assert!(vm.traces().any(|trace| trace.opcode == Some(6)));
    }

    for value in [50, 0x10000] {
        let mut vm = EventVm::new(decode_program(&switch_program(value)).unwrap());
        let mut host = Host::new();
        assert_eq!(
            vm.step(Tick::new(0), &mut host).unwrap().status,
            VmStatus::Completed
        );
    }

    let truncated = decode_program(&[6, 1, 0]).unwrap_err();
    assert_eq!(truncated.kind, VmErrorKind::Decode);
    assert!(truncated.message.contains("opcode Some(6)"));
}

#[test]
fn wide_jump_switch_supports_more_than_255_cases_and_uses_default() {
    fn switch_program(value: i32) -> Vec<u8> {
        const CASES: usize = 300;
        const BASE: i16 = 5;
        const SWITCH_LENGTH: u16 = 7 + 2 * CASES as u16;
        const FIRST_CASE: u16 = SWITCH_LENGTH - 2;
        const LAST_CASE: u16 = FIRST_CASE + 2;
        const DEFAULT: u16 = LAST_CASE + 2;

        let mut bytes = vec![5];
        append_expression(&mut bytes, constant(value));
        bytes.push(13);
        bytes.extend_from_slice(&(CASES as u16).to_le_bytes());
        bytes.extend_from_slice(&BASE.to_le_bytes());
        bytes.extend_from_slice(&DEFAULT.to_le_bytes());
        for case in 0..CASES {
            let relative = match case {
                0 => FIRST_CASE,
                case if case == CASES - 1 => LAST_CASE,
                _ => DEFAULT,
            };
            bytes.extend_from_slice(&relative.to_le_bytes());
        }
        bytes.extend_from_slice(&[0, 0, 0, 0, 4]);
        bytes
    }

    let program = decode_program(&switch_program(5)).unwrap();
    let instruction = &program.instructions[&7];
    assert_eq!(instruction.length, 607);
    let InstructionKind::JumpSwitchWide {
        base,
        default_relative,
        case_relative,
    } = &instruction.kind
    else {
        panic!("expected wide switch");
    };
    assert_eq!(*base, 5);
    assert_eq!(*default_relative, 609);
    assert_eq!(case_relative.len(), 300);
    assert_eq!(case_relative[0], 605);
    assert_eq!(case_relative[299], 607);

    for (value, expected_pc) in [(5, 616), (304, 618)] {
        let mut vm = EventVm::new(decode_program(&switch_program(value)).unwrap());
        let mut host = Host::new();
        let report = vm.step(Tick::new(0), &mut host).unwrap();
        assert_eq!(report.status, VmStatus::Running);
        assert_eq!(vm.pc(), expected_pc);
        assert!(vm.traces().any(|trace| trace.opcode == Some(13)));
    }

    for value in [4, 305] {
        let mut vm = EventVm::new(decode_program(&switch_program(value)).unwrap());
        let mut host = Host::new();
        assert_eq!(
            vm.step(Tick::new(0), &mut host).unwrap().status,
            VmStatus::Completed
        );
    }

    let truncated = decode_program(&[13, 1, 0, 0]).unwrap_err();
    assert_eq!(truncated.kind, VmErrorKind::Decode);
    assert!(truncated.message.contains("opcode Some(13)"));
}

#[test]
fn cinematic_dispatches_operation_argument_and_repeated_first_operand() {
    let program = decode_program(&[40, 0, 24, 120, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 4);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::DispatchCinematic {
            operation: 24,
            argument: 120,
            auxiliary: 24
        }]
    );
    assert_eq!(report.status, VmStatus::Completed);

    let mut bytes = vec![40, 1];
    let mut operation = variable(VariableScope::Global, VariableType::UInt16, 7);
    operation.push(6);
    append_expression(&mut bytes, operation);
    bytes.extend_from_slice(&[120, 4]);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::DispatchCinematic {
            operation: 1,
            argument: 120,
            auxiliary: 2
        }]
    );
    assert_eq!(vm.variable(VariableScope::Global, 7), 2);

    let truncated = decode_program(&[40, 0, 1]).unwrap_err();
    assert_eq!(truncated.kind, VmErrorKind::Decode);
    assert!(truncated.message.contains("opcode Some(40)"));
}

#[test]
fn region_definition_decodes_bounded_signed_vertices_and_expression_operands() {
    let points: [[i16; 2]; 4] = [[-10, 20], [30, -40], [50, 60], [-70, -80]];
    let mut bytes = vec![41, 0, points.len() as u8];
    for [x, z] in points {
        bytes.extend_from_slice(&x.to_le_bytes());
        bytes.extend_from_slice(&z.to_le_bytes());
    }
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    assert_eq!(program.instructions[&0].length, 19);
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::DefineRegion {
            region: EntityId::new(42),
            vertices: points.to_vec()
        }]
    );
    assert_eq!(report.status, VmStatus::Completed);

    let mut bytes = vec![41, 0b0001_0010, 2];
    append_expression(&mut bytes, constant(-300));
    bytes.extend_from_slice(&100_i16.to_le_bytes());
    bytes.extend_from_slice(&200_i16.to_le_bytes());
    append_expression(&mut bytes, constant(300));
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap()).with_entity(EntityId::new(42));
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::DefineRegion {
            region: EntityId::new(42),
            vertices: vec![[-300, 100], [200, 300]]
        }]
    );

    let mut vm = EventVm::new(decode_program(&[41, 0, 0, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("object context"));

    let oversized = decode_program(&[41, 0, 9]).unwrap_err();
    assert_eq!(oversized.kind, VmErrorKind::Decode);
    assert!(oversized.message.contains("eight-vertex"));
    let dynamic = decode_program(&[41, 1]).unwrap_err();
    assert_eq!(dynamic.kind, VmErrorKind::UnsupportedOpcode);
    assert!(dynamic.message.contains("vertex counts"));
    assert!(decode_program(&[41, 0]).is_err());
}

#[test]
fn clear_party_status_preserves_character_and_status_mask() {
    let program = decode_program(&[217, 0, 2, 127, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 4);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::ClearPartyStatus {
            character: 2,
            statuses: 127
        }]
    );
    assert_eq!(report.status, VmStatus::Completed);

    let mut bytes = vec![217, 3];
    append_expression(&mut bytes, constant(120));
    append_expression(&mut bytes, constant(85));
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::ClearPartyStatus {
            character: 120,
            statuses: 85
        }]
    );

    let truncated = decode_program(&[217, 0, 2]).unwrap_err();
    assert_eq!(truncated.kind, VmErrorKind::Decode);
    assert!(truncated.message.contains("opcode Some(217)"));
}

#[test]
fn bitwise_and_evaluates_the_shared_binary_expression_path() {
    let mut bytes = evaluate(binary(constant(90), constant(60), 36));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    assert_eq!(
        program.instructions[&0].kind,
        InstructionKind::Evaluate(Expression(vec![
            ExpressionOp::Constant(90),
            ExpressionOp::Constant(60),
            ExpressionOp::BitwiseAnd,
            ExpressionOp::End,
        ]))
    );
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.status, VmStatus::Completed);

    let mut bytes = vec![217, 3];
    append_expression(&mut bytes, constant(120));
    append_expression(&mut bytes, binary(constant(127), constant(85), 36));
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::ClearPartyStatus {
            character: 120,
            statuses: 85
        }]
    );
}

#[test]
fn bitwise_xor_evaluates_the_shared_binary_expression_path() {
    let result = variable(VariableScope::Global, VariableType::Int16, 63);
    let expression = assign(result, binary(constant(90), constant(60), 37));
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 63), 102);

    let mut bytes = vec![61, 1];
    append_expression(
        &mut bytes,
        binary(constant(16), binary(constant(3), constant(5), 37), 38),
    );
    bytes.extend_from_slice(&[42, 4]);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap()).with_entity(EntityId::new(42));
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetAnimationRange {
            actor: EntityId::new(42),
            start: 22,
            end: 42
        }]
    );
}

#[test]
fn bitwise_or_evaluates_the_shared_binary_expression_path() {
    let result = variable(VariableScope::Global, VariableType::Int16, 62);
    let expression = assign(result, binary(constant(90), constant(60), 38));
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 62), 126);

    let mut bytes = vec![61, 1];
    append_expression(&mut bytes, binary(constant(16), constant(3), 38));
    bytes.extend_from_slice(&[42, 4]);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap()).with_entity(EntityId::new(42));
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetAnimationRange {
            actor: EntityId::new(42),
            start: 19,
            end: 42
        }]
    );
}

#[test]
fn play_animation_uses_the_current_actor_and_unsigned_animation_id() {
    let program = decode_program(&[64, 0, 0x34, 0x12, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 4);
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::PlayAnimation {
            actor: EntityId::new(42),
            animation: 0x1234
        }]
    );
    assert_eq!(report.status, VmStatus::Completed);

    let mut bytes = vec![64, 1];
    append_expression(&mut bytes, constant(300));
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap()).with_entity(EntityId::new(42));
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::PlayAnimation {
            actor: EntityId::new(42),
            animation: 300
        }]
    );

    let mut vm = EventVm::new(decode_program(&[64, 0, 1, 0, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("actor context"));
    assert!(decode_program(&[64, 0, 1]).is_err());
}

#[test]
fn animation_range_sets_start_and_end_for_the_current_actor() {
    let program = decode_program(&[61, 0, 18, 42, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 4);
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetAnimationRange {
            actor: EntityId::new(42),
            start: 18,
            end: 42
        }]
    );
    assert_eq!(report.status, VmStatus::Completed);

    let mut bytes = vec![61, 3];
    append_expression(&mut bytes, constant(300));
    append_expression(&mut bytes, constant(-1));
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap()).with_entity(EntityId::new(42));
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetAnimationRange {
            actor: EntityId::new(42),
            start: 44,
            end: 255
        }]
    );

    let mut vm = EventVm::new(decode_program(&[61, 0, 1, 2, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("actor context"));
    assert!(decode_program(&[61, 0, 1]).is_err());
}

#[test]
fn preload_field_preserves_hint_and_signed_field_identity() {
    let program = decode_program(&[253, 0, 5, 53, 0, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 5);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::PreloadField { hint: 5, field: 53 }]
    );
    assert_eq!(report.status, VmStatus::Completed);

    let mut bytes = vec![253, 3];
    append_expression(&mut bytes, constant(48));
    append_expression(&mut bytes, constant(-300));
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::PreloadField {
            hint: 48,
            field: -300
        }]
    );

    let truncated = decode_program(&[253, 0, 5, 1]).unwrap_err();
    assert_eq!(truncated.kind, VmErrorKind::Decode);
    assert!(truncated.message.contains("opcode Some(253)"));
}

#[test]
fn sps_consumes_two_operands_and_returns_zero_without_host_effects() {
    let result = variable(VariableScope::Global, VariableType::Int16, 74);
    let expression = assign(result.clone(), binary(constant(11), constant(22), 108));
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 74), 0);
    assert!(report.events.is_empty());
    assert!(host.member_writes.is_empty());

    let missing = binary(constant(11), vec![], 108);
    let mut vm = EventVm::new(decode_program(&evaluate(missing)).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert!(error.message.contains("missing first SPS operand"));
}

#[test]
fn bgvalpha_emits_viewport_and_signed_alpha_factors() {
    let program = decode_program(&[0xe6, 0, 3, 0xd0, 0x02, 0x30, 0xfd, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 7);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetTileViewportAlpha {
            viewport: 3,
            alpha: [720, -720],
        }]
    );
    assert!(vm.traces().any(|trace| trace.opcode == Some(0xe6)));
    assert_eq!(report.status, VmStatus::Completed);

    let mut bytes = vec![0xe6, 7];
    for value in [5, -300, 700] {
        append_expression(&mut bytes, constant(value));
    }
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetTileViewportAlpha {
            viewport: 5,
            alpha: [-300, 700],
        }]
    );

    let truncated = decode_program(&[0xe6, 0, 3, 1]).unwrap_err();
    assert_eq!(truncated.kind, VmErrorKind::Decode);
    assert!(truncated.message.contains("opcode Some(230)"));
}

#[test]
fn add_status_decodes_target_permanent_flag_and_script_arguments() {
    let mut bytes = vec![0xff, 0x1d, 0];
    for value in [0x1234_i32, 7, 1, -10, 20, -30] {
        bytes.extend_from_slice(&value.to_le_bytes()[..3]);
    }
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    assert_eq!(program.instructions[&0].length, 21);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::AddBattleStatus {
            target: 0x1234,
            status: 7,
            permanent: true,
            arguments: [-10, 20, -30],
        }]
    );
    assert!(vm.traces().any(|trace| trace.opcode == Some(0x11d)));
    assert_eq!(report.status, VmStatus::Completed);

    let mut bytes = vec![0xff, 0x1d, 63];
    for value in [8, 9, 0, 100, 200, 300] {
        append_expression(&mut bytes, constant(value));
    }
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::AddBattleStatus {
            target: 8,
            status: 9,
            permanent: false,
            arguments: [100, 200, 300],
        }]
    );

    let truncated = decode_program(&[0xff, 0x1d, 0, 1]).unwrap_err();
    assert_eq!(truncated.kind, VmErrorKind::Decode);
    assert!(truncated.message.contains("opcode Some(285)"));
}

#[test]
fn model_light_decodes_signed_operands_and_wraps_colors_and_shadows() {
    let mut bytes = vec![0xff, 0x21, 0];
    for value in [-1_i32, 300, -2, 128, -5, 130] {
        bytes.extend_from_slice(&value.to_le_bytes()[..3]);
    }
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    assert_eq!(program.instructions[&0].length, 21);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetModelLight {
            model: -1,
            color: [44, 254, 128],
            shadow_intensity: -5,
            shadow_scale: -126,
        }]
    );
    assert!(vm.traces().any(|trace| trace.opcode == Some(0x121)));
    assert_eq!(report.status, VmStatus::Completed);

    let mut bytes = vec![0xff, 0x21, 63];
    for value in [7, 10, 20, 30, -1, 2] {
        append_expression(&mut bytes, constant(value));
    }
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetModelLight {
            model: 7,
            color: [10, 20, 30],
            shadow_intensity: -1,
            shadow_scale: 2,
        }]
    );

    let truncated = decode_program(&[0xff, 0x21, 0, 1]).unwrap_err();
    assert_eq!(truncated.kind, VmErrorKind::Decode);
    assert!(truncated.message.contains("opcode Some(289)"));
}

#[test]
fn elemental_affinity_decodes_signed_24_bit_operands() {
    let mut bytes = vec![0xff, 0x22, 0];
    for value in [0x1234_i32, 4, -3, 1] {
        bytes.extend_from_slice(&value.to_le_bytes()[..3]);
    }
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    assert_eq!(program.instructions[&0].length, 15);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::ChangeElementalAffinity {
            target: 0x1234,
            affinity_kind: 4,
            element: -3,
            add: true
        }]
    );
    assert!(vm.traces().any(|trace| trace.opcode == Some(0x122)));
    assert_eq!(report.status, VmStatus::Completed);

    let mut bytes = vec![0xff, 0x22, 15];
    for value in [7, 2, 9, 0] {
        append_expression(&mut bytes, constant(value));
    }
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::ChangeElementalAffinity {
            target: 7,
            affinity_kind: 2,
            element: 9,
            add: false
        }]
    );

    let truncated = decode_program(&[0xff, 0x22, 0, 1]).unwrap_err();
    assert_eq!(truncated.kind, VmErrorKind::Decode);
    assert!(truncated.message.contains("opcode Some(290)"));
}

#[test]
fn rain_preserves_strength_and_speed_operands() {
    let program = decode_program(&[216, 0, 125, 20, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 4);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetRain {
            strength: 125,
            speed: 20
        }]
    );
    assert_eq!(report.status, VmStatus::Completed);

    let mut bytes = vec![216, 3];
    append_expression(&mut bytes, constant(80));
    append_expression(&mut bytes, constant(12));
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetRain {
            strength: 80,
            speed: 12
        }]
    );

    let truncated = decode_program(&[216, 0, 125]).unwrap_err();
    assert_eq!(truncated.kind, VmErrorKind::Decode);
    assert!(truncated.message.contains("opcode Some(216)"));
}

#[test]
fn field_change_emits_destination_and_stops_old_scene_execution() {
    let mut bytes = vec![43, 0, 53, 0];
    bytes.extend(evaluate(assign(
        variable(VariableScope::Global, VariableType::UInt16, 9),
        constant(1),
    )));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    assert_eq!(program.instructions[&0].length, 4);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.events, [HostEvent::ChangeField { field: 53 }]);
    assert_eq!(report.status, VmStatus::Completed);
    assert_eq!(vm.variable(VariableScope::Global, 9), 0);

    let mut bytes = vec![43, 1];
    append_expression(&mut bytes, constant(-300));
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.events, [HostEvent::ChangeField { field: -300 }]);

    let truncated = decode_program(&[43, 0, 1]).unwrap_err();
    assert_eq!(truncated.kind, VmErrorKind::Decode);
    assert!(truncated.message.contains("opcode Some(43)"));
}

#[test]
fn aidle_sets_actor_idle_animation_from_wide_operand() {
    let program = decode_program(&[51, 0, 0x34, 0x12, 4]).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetIdleAnimation {
            actor: EntityId::new(42),
            animation: 0x1234
        }]
    );

    let mut bytes = vec![51, 1];
    append_expression(&mut bytes, constant(300));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetIdleAnimation {
            actor: EntityId::new(42),
            animation: 300
        }]
    );

    let mut vm = EventVm::new(decode_program(&[51, 0, 1, 0, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("actor context"));
}

#[test]
fn awalk_sets_actor_walk_animation_from_wide_operand() {
    let program = decode_program(&[52, 0, 0x34, 0x12, 4]).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetWalkAnimation {
            actor: EntityId::new(42),
            animation: 0x1234
        }]
    );

    let mut bytes = vec![52, 1];
    append_expression(&mut bytes, constant(300));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetWalkAnimation {
            actor: EntityId::new(42),
            animation: 300
        }]
    );

    let mut vm = EventVm::new(decode_program(&[52, 0, 1, 0, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("actor context"));
}

#[test]
fn arun_sets_actor_run_animation_from_wide_operand() {
    let program = decode_program(&[53, 0, 0x34, 0x12, 4]).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetRunAnimation {
            actor: EntityId::new(42),
            animation: 0x1234
        }]
    );

    let mut bytes = vec![53, 1];
    append_expression(&mut bytes, constant(300));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetRunAnimation {
            actor: EntityId::new(42),
            animation: 300
        }]
    );

    let mut vm = EventVm::new(decode_program(&[53, 0, 1, 0, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("actor context"));
}

#[test]
fn aturnl_sets_actor_left_turn_animation_from_wide_operand() {
    let program = decode_program(&[122, 0, 0x34, 0x12, 4]).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetLeftTurnAnimation {
            actor: EntityId::new(42),
            animation: 0x1234
        }]
    );

    let mut bytes = vec![122, 1];
    append_expression(&mut bytes, constant(300));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetLeftTurnAnimation {
            actor: EntityId::new(42),
            animation: 300
        }]
    );

    let mut vm = EventVm::new(decode_program(&[122, 0, 1, 0, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("actor context"));
}

#[test]
fn aturnr_sets_actor_right_turn_animation_from_wide_operand() {
    let program = decode_program(&[123, 0, 0x34, 0x12, 4]).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetRightTurnAnimation {
            actor: EntityId::new(42),
            animation: 0x1234
        }]
    );

    let mut bytes = vec![123, 1];
    append_expression(&mut bytes, constant(300));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetRightTurnAnimation {
            actor: EntityId::new(42),
            animation: 300
        }]
    );

    let mut vm = EventVm::new(decode_program(&[123, 0, 1, 0, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("actor context"));
}

#[test]
fn ajump_sets_actor_jump_animation_and_parameters() {
    let program = decode_program(&[148, 0, 0xff, 0xff, 2, 3, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 6);
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetJumpAnimation {
            actor: EntityId::new(42),
            animation: u16::MAX,
            parameters: [2, 3]
        }]
    );

    let mut bytes = vec![148, 7];
    for value in [300, 4, 5] {
        append_expression(&mut bytes, constant(value));
    }
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetJumpAnimation {
            actor: EntityId::new(42),
            animation: 300,
            parameters: [4, 5]
        }]
    );

    let mut vm = EventVm::new(decode_program(&[148, 0, 1, 0, 2, 3, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("actor context"));
}

#[test]
fn radius_sets_actor_pathing_collision_and_talk_radii() {
    let program = decode_program(&[75, 0, 30, 8, 9, 4]).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetActorRadii {
            actor: EntityId::new(42),
            pathing_size: 30,
            collision_radius: 8,
            talk_radius: 9
        }]
    );

    let mut bytes = vec![75, 7];
    for value in [-1, 300, -2] {
        append_expression(&mut bytes, constant(value));
    }
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetActorRadii {
            actor: EntityId::new(42),
            pathing_size: -1,
            collision_radius: 44,
            talk_radius: 254
        }]
    );

    let mut vm = EventVm::new(decode_program(&[75, 0, 30, 8, 9, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("actor context"));
}

#[test]
fn idlespeed_sets_all_four_actor_animation_speeds() {
    let program = decode_program(&[134, 0, 16, 20, 24, 28, 4]).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetIdleAnimationSpeeds {
            actor: EntityId::new(42),
            speeds: [16, 20, 24, 28]
        }]
    );

    let mut bytes = vec![134, 15];
    for value in [-1, 256, 32, 64] {
        append_expression(&mut bytes, constant(value));
    }
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetIdleAnimationSpeeds {
            actor: EntityId::new(42),
            speeds: [255, 0, 32, 64]
        }]
    );

    let mut vm = EventVm::new(decode_program(&[134, 0, 16, 20, 24, 28, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("actor context"));
}

#[test]
fn neckid_sets_actor_head_focus_masks() {
    let program = decode_program(&[139, 0, 3, 12, 4]).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetHeadFocusMasks {
            actor: EntityId::new(42),
            self_mask: 3,
            target_mask: 12
        }]
    );

    let mut bytes = vec![139, 3];
    append_expression(&mut bytes, constant(-1));
    append_expression(&mut bytes, constant(256));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetHeadFocusMasks {
            actor: EntityId::new(42),
            self_mask: 255,
            target_mask: 0
        }]
    );

    let mut vm = EventVm::new(decode_program(&[139, 0, 3, 12, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("actor context"));
}

#[test]
fn neckflag_replaces_head_focus_flags_with_the_masked_operand() {
    let program = decode_program(&[71, 0, 3, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 3);
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetHeadFocusFlags {
            actor: EntityId::new(42),
            flags: 3
        }]
    );
    assert_eq!(report.status, VmStatus::Completed);

    let mut bytes = vec![71, 1];
    append_expression(&mut bytes, constant(-1));
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap()).with_entity(EntityId::new(42));
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetHeadFocusFlags {
            actor: EntityId::new(42),
            flags: 7
        }]
    );

    let mut vm = EventVm::new(decode_program(&[71, 0, 3, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("actor context"));
    assert!(decode_program(&[71, 0]).is_err());
}

#[test]
fn mrot_sets_walk_turn_speed_before_wrapping_to_a_byte() {
    let program = decode_program(&[85, 0, 0, 85, 0, 16, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 3);
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [
            HostEvent::SetWalkTurnSpeed {
                actor: EntityId::new(42),
                speed: 255
            },
            HostEvent::SetWalkTurnSpeed {
                actor: EntityId::new(42),
                speed: 16
            }
        ]
    );
    assert_eq!(report.status, VmStatus::Completed);

    let mut bytes = vec![85, 1];
    append_expression(&mut bytes, constant(256));
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap()).with_entity(EntityId::new(42));
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetWalkTurnSpeed {
            actor: EntityId::new(42),
            speed: 0
        }]
    );

    let mut vm = EventVm::new(decode_program(&[85, 0, 16, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("actor context"));
    assert!(decode_program(&[85, 0]).is_err());
}

#[test]
fn cflag_sets_current_object_mutable_flags() {
    let program = decode_program(&[147, 0, 14, 4]).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetObjectFlags {
            object: EntityId::new(42),
            flags: 14
        }]
    );

    let mut bytes = vec![147, 1];
    append_expression(&mut bytes, constant(-1));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetObjectFlags {
            object: EntityId::new(42),
            flags: 255
        }]
    );

    let mut vm = EventVm::new(decode_program(&[147, 0, 14, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("object context"));
}

#[test]
fn dire_sets_actor_facing_with_shifted_fixed_point_angle() {
    let program = decode_program(&[54, 0, 192, 4]).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetActorFacing {
            actor: EntityId::new(42),
            angle: 3072
        }]
    );

    let mut bytes = vec![54, 1];
    append_expression(&mut bytes, constant(-1));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetActorFacing {
            actor: EntityId::new(42),
            angle: -16
        }]
    );

    let mut vm = EventVm::new(decode_program(&[54, 0, 192, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("actor context"));
}

#[test]
fn turn_starts_an_animated_actor_turn_with_angle_and_speed() {
    let program = decode_program(&[86, 0, 192, 16, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 4);
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::StartActorTurn {
            actor: EntityId::new(42),
            angle: 3072,
            speed: 16
        }]
    );
    assert_eq!(report.status, VmStatus::Completed);

    let mut bytes = vec![86, 3];
    append_expression(&mut bytes, constant(-1));
    append_expression(&mut bytes, constant(-2));
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap()).with_entity(EntityId::new(42));
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::StartActorTurn {
            actor: EntityId::new(42),
            angle: -16,
            speed: -2
        }]
    );

    let mut vm = EventVm::new(decode_program(&[86, 0, 192, 16, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("actor context"));
    assert!(decode_program(&[86, 0, 192]).is_err());
}

#[test]
fn pos_sets_current_actor_xz_and_reference_default_height() {
    let program = decode_program(&[29, 0, 0xf6, 0xff, 20, 0, 4]).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetActorPosition {
            actor: EntityId::new(42),
            position: [-10, 32768, 20]
        }]
    );

    let mut bytes = vec![29, 3];
    append_expression(&mut bytes, constant(-1000));
    append_expression(&mut bytes, constant(2000));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetActorPosition {
            actor: EntityId::new(42),
            position: [-1000, 32768, 2000]
        }]
    );

    let mut vm = EventVm::new(decode_program(&[29, 0, 1, 0, 20, 0, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("actor context"));
}

#[test]
fn delete_terminates_self_and_emits_typed_target_for_other_objects() {
    let program = decode_program(&[28, 255, 4]).unwrap();
    assert_eq!(
        program.instructions[&0].kind,
        InstructionKind::DeleteObject(255)
    );
    assert_eq!(program.instructions[&0].length, 2);
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.operations, 1);
    assert_eq!(report.status, VmStatus::Completed);
    assert_eq!(
        report.events,
        [HostEvent::DeleteObject {
            target: EntityId::new(42)
        }]
    );

    let program = decode_program(&[28, 7, 0, 0, 4]).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.status, VmStatus::Running);
    assert_eq!(
        report.events,
        [HostEvent::DeleteObject {
            target: EntityId::new(7)
        }]
    );
    assert_eq!(
        vm.step(Tick::new(1), &mut host).unwrap().status,
        VmStatus::Completed
    );

    let mut vm = EventVm::new(decode_program(&[28, 255, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("object context"));
}

#[test]
fn subtract_assign_writes_variable_or_member_and_returns_the_result() {
    let target = variable(VariableScope::Global, VariableType::Int16, 37);
    let result = variable(VariableScope::Global, VariableType::Int16, 38);
    let bytes = [
        evaluate(assign(target.clone(), constant(20))),
        evaluate(assign(result, binary(target, constant(8), 51))),
        vec![4],
    ]
    .concat();
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 37), 12);
    assert_eq!(vm.variable(VariableScope::Global, 38), 12);

    let reference = ObjectMemberRef {
        object: EntityId::new(42),
        member: 4,
    };
    let mut host = Host::new();
    host.member_target = Some(reference.object);
    host.object_members.insert(reference, 20);
    let bytes = [vec![41, 4], constant(8), vec![51]].concat();
    let program = decode_program(&[evaluate(bytes), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(host.object_members[&reference], 12);
    assert_eq!(host.member_writes, [(reference, 12)]);
}

#[test]
fn add_assign_writes_variable_or_member_and_returns_the_result() {
    let target = variable(VariableScope::Global, VariableType::Int16, 44);
    let result = variable(VariableScope::Global, VariableType::Int16, 45);
    let bytes = [
        evaluate(assign(target.clone(), constant(20))),
        evaluate(assign(result, binary(target, constant(8), 50))),
        vec![4],
    ]
    .concat();
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 44), 28);
    assert_eq!(vm.variable(VariableScope::Global, 45), 28);

    let reference = ObjectMemberRef {
        object: EntityId::new(42),
        member: 4,
    };
    let mut host = Host::new();
    host.member_target = Some(reference.object);
    host.object_members.insert(reference, 20);
    let bytes = [vec![41, 4], constant(8), vec![50]].concat();
    let program = decode_program(&[evaluate(bytes), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(host.object_members[&reference], 28);
    assert_eq!(host.member_writes, [(reference, 28)]);
}

#[test]
fn multiply_assign_normalizes_storage_and_returns_the_raw_result() {
    let target = variable(VariableScope::Global, VariableType::Byte, 60);
    let result = variable(VariableScope::Global, VariableType::Int16, 61);
    let bytes = [
        evaluate(assign(target.clone(), constant(200))),
        evaluate(assign(result, binary(target, constant(2), 47))),
        vec![4],
    ]
    .concat();
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 60), 144);
    assert_eq!(vm.variable(VariableScope::Global, 61), 400);

    let reference = ObjectMemberRef {
        object: EntityId::new(42),
        member: 4,
    };
    let mut host = Host::new();
    host.member_target = Some(reference.object);
    host.object_members.insert(reference, 20);
    let bytes = [vec![41, 4], constant(8), vec![47]].concat();
    let program = decode_program(&[evaluate(bytes), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(host.object_members[&reference], 160);
    assert_eq!(host.member_writes, [(reference, 160)]);
}

#[test]
fn instruction_budget_stops_an_infinite_relative_jump() {
    let bytes = [1, 0xfd, 0xff];
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program).with_instruction_budget(16);
    let mut host = Host::new();
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::InstructionLimit);
    assert_eq!(error.offset, 0);
}

#[test]
fn host_failure_rolls_back_the_vm_step() {
    let mut bytes = movement(1, 50, 0);
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let mut host = Host::new();
    host.fail = true;

    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert_eq!(vm.pc(), 0);
    assert_eq!(vm.status(), VmStatus::Running);
}

#[test]
fn nop_yields_before_the_remaining_program() {
    let bytes = [0, 0, 4];
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let mut host = Host::new();
    assert_eq!(
        vm.step(Tick::new(0), &mut host).unwrap().status,
        VmStatus::Running
    );
    assert_eq!(vm.pc(), 2);
    assert_eq!(
        vm.step(Tick::new(1), &mut host).unwrap().status,
        VmStatus::Completed
    );
}

#[test]
fn unused_pprint_consumes_only_its_opcode_and_continues() {
    let program = decode_program(&[108, 4]).unwrap();
    assert_eq!(program.instructions[&0].kind, InstructionKind::NoOp(108));
    assert_eq!(program.instructions[&0].length, 1);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.status, VmStatus::Completed);
    assert_eq!(report.operations, 2);
    assert!(report.events.is_empty());
}

#[test]
fn member_post_increment_rejects_missing_selection_inputs() {
    let program = decode_program(&evaluate(vec![8])).unwrap();
    let InstructionKind::Evaluate(expression) = &program.instructions[&0].kind else {
        panic!("expected expression");
    };
    assert!(matches!(
        expression.0.as_slice(),
        [ExpressionOp::PostIncrementMembers, ExpressionOp::End]
    ));

    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::InvalidExpression);
    assert!(error.message.contains("selection inputs"));
}

#[test]
fn expression_padding_produces_zero() {
    for opcode in [0, 1, 2, 3, 103, 104, 105] {
        let program = decode_program(&[evaluate(vec![opcode]), vec![4]].concat()).unwrap();
        let mut vm = EventVm::new(program);
        let mut host = Host::new();
        vm.step(Tick::new(0), &mut host).unwrap();
        assert_eq!(vm.status(), VmStatus::Completed);
    }
}

#[test]
fn direct_post_decrement_writes_back_the_normalized_value() {
    let target = variable(VariableScope::Map, VariableType::Int16, 7);
    let program =
        decode_program(&[evaluate([target, vec![5]].concat()), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.status(), VmStatus::Completed);
    assert_eq!(vm.variable(VariableScope::Map, 7), -1);
}

#[test]
fn aim_instruction_consumes_its_flag_and_defers_to_the_host() {
    let program = decode_program(&[0xb8, 0, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 2);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.events, [HostEvent::AimAtCamera]);
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn player_control_instruction_requires_actor_context() {
    let program = decode_program(&[0x2c, 0, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 2);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("actor context"));
}

#[test]
fn player_control_uses_the_current_entity_when_available() {
    let program = decode_program(&[0x2c, 0, 4]).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::DefinePlayerControl {
            actor: EntityId::new(42)
        }]
    );
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn objspec_matches_the_current_zero_result_without_member_access() {
    let result = variable(VariableScope::Global, VariableType::Int16, 74);
    let expression = assign(result, vec![78]);
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 74), 0);
    assert_eq!(host.member_target, None);
    assert!(host.member_writes.is_empty());
    assert!(decode_program(&[5, 78]).is_err());
}

#[test]
fn direct_object_member_reads_use_object_and_member_identity() {
    let target = variable(VariableScope::Global, VariableType::Int16, 9);
    let read = vec![120, 42, 7];
    let program = decode_program(&[evaluate(assign(target, read)), vec![4]].concat()).unwrap();
    let InstructionKind::Evaluate(expression) = &program.instructions[&0].kind else {
        panic!("expected expression");
    };
    assert!(expression.0.iter().any(|operation| {
        matches!(
            operation,
            ExpressionOp::ReadObjectMember(ObjectMemberRef { object, member: 7 })
                if object.get() == 42
        )
    }));

    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 9), (42 << 8) | 7);
}

#[test]
fn empty_member_selection_shift_assignment_preserves_the_reference_stack_behavior() {
    let program = decode_program(&[evaluate(vec![3, 0, 1, 59, 0]), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn nonzero_member_selection_shift_assignment_is_explicitly_rejected() {
    let expression = [constant(1), constant(0), constant(0), vec![59]].concat();
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert!(error.message.contains("object-slot host"));
}

#[test]
fn logical_operators_skip_or_execute_right_side_effects_correctly() {
    for (left, opcode, expected_result, expected_side_effect) in
        [(0, 39, 0, 0), (1, 39, 0, -1), (1, 40, 1, 0), (0, 40, 0, -1)]
    {
        let side_target = variable(VariableScope::Map, VariableType::Int16, 21);
        let result_target = variable(VariableScope::Global, VariableType::Int16, 10);
        let expression = assign(
            result_target,
            [constant(left), side_target, vec![5], vec![opcode]].concat(),
        );
        let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
        let mut vm = EventVm::new(program);
        let mut host = Host::new();
        vm.step(Tick::new(0), &mut host).unwrap();
        assert_eq!(
            vm.variable(VariableScope::Global, 10),
            expected_result,
            "result for left={left} opcode={opcode}"
        );
        assert_eq!(
            vm.variable(VariableScope::Map, 21),
            expected_side_effect,
            "side effect for left={left} opcode={opcode}"
        );
    }
}

#[test]
fn empty_member_increment_and_decrement_follow_the_reference_stack_path() {
    for opcode in [8, 11] {
        let program = decode_program(&[evaluate(vec![0, 0, opcode, 0]), vec![4]].concat()).unwrap();
        let mut vm = EventVm::new(program);
        let mut host = Host::new();
        vm.step(Tick::new(0), &mut host).unwrap();
        assert_eq!(vm.status(), VmStatus::Completed);

        let expression = [constant(1), constant(0), vec![opcode]].concat();
        let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
        let mut vm = EventVm::new(program);
        let error = vm.step(Tick::new(0), &mut host).unwrap_err();
        assert!(error.message.contains("object-member host"));
    }
}

#[test]
fn twist_sets_both_control_directions() {
    let mut bytes = vec![0x67, 0];
    append_expression(&mut bytes, constant(90));
    append_expression(&mut bytes, constant(-90));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetControlDirection {
            arrow: 90,
            analog: -90
        }]
    );
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn direct_pre_decrement_returns_the_new_value() {
    let target = variable(VariableScope::Map, VariableType::Int16, 8);
    let result = variable(VariableScope::Global, VariableType::Int16, 11);
    let expression = assign(result, [target, vec![7]].concat());
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Map, 8), -1);
    assert_eq!(vm.variable(VariableScope::Global, 11), -1);
}

#[test]
fn custom_variables_read_through_the_host_by_serialized_id() {
    let target = variable(VariableScope::Global, VariableType::Int16, 12);
    let program =
        decode_program(&[evaluate(assign(target, vec![0xe3, 0, 0])), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 12), 17);
}

#[test]
fn greater_or_equal_assigns_the_comparison_result() {
    let target = variable(VariableScope::Global, VariableType::Int16, 13);
    let comparison = binary(constant(2), constant(2), 27);
    let program =
        decode_program(&[evaluate(assign(target, comparison)), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 13), 1);
}

#[test]
fn empty_expressions_evaluate_to_zero() {
    let program = decode_program(&[evaluate(Vec::new()), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 14), 0);
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn bgsmove_consumes_three_operands_and_continues_without_invented_side_effects() {
    let mut bytes = vec![0x58, 0];
    for value in [10, -20, 30] {
        append_expression(&mut bytes, constant(value));
    }
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert!(report.events.is_empty());
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn expression_casts_apply_width_and_signedness() {
    for (opcode, input, expected) in [
        (70, 255, -1),
        (71, 255, 255),
        (72, 65_535, -1),
        (73, 65_535, 65_535),
    ] {
        let target_type = if expected < 0 {
            VariableType::Int16
        } else {
            VariableType::UInt16
        };
        let target = variable(VariableScope::Global, target_type, 15);
        let expression = assign(target, [constant(input), vec![opcode]].concat());
        let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
        let mut vm = EventVm::new(program);
        let mut host = Host::new();
        vm.step(Tick::new(0), &mut host).unwrap();
        assert_eq!(vm.variable(VariableScope::Global, 15), expected);
    }
}

#[test]
fn real_member_cast_sequence_decodes() {
    let bytes = [
        5, 104, 0, 8, 0, 3, 0, 70, 0, 5, 213, 24, 125, 1, 0, 44, 127, 4,
    ];
    let program = decode_program(&bytes).unwrap();
    assert!(program.instructions.contains_key(&0));
    assert!(program.instructions.contains_key(&17));
}

#[test]
fn member_xor_assignment_uses_the_last_selected_target() {
    let result = variable(VariableScope::Global, VariableType::Int16, 16);
    let expression = assign(
        result,
        [vec![41, 2], constant(0b101), constant(-1), vec![68]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    host.slots.insert(0, EntityId::new(10));
    host.slots.insert(2, EntityId::new(12));
    let first = ObjectMemberRef {
        object: EntityId::new(10),
        member: 2,
    };
    let last = ObjectMemberRef {
        object: EntityId::new(12),
        member: 2,
    };
    host.object_members.insert(first, 0);
    host.object_members.insert(last, 9);

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 16), -5);
    assert_eq!(host.object_members[&first], 0);
    assert_eq!(host.object_members[&last], -5);
    assert_eq!(host.member_target, Some(EntityId::new(12)));
    assert_eq!(host.member_writes, [(last, -5)]);
}

#[test]
fn member_and_assignment_uses_the_last_selected_target() {
    let result = variable(VariableScope::Global, VariableType::Int16, 51);
    let expression = assign(
        result,
        [vec![41, 2], constant(0b101), constant(5), vec![67]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    host.slots.insert(0, EntityId::new(10));
    host.slots.insert(2, EntityId::new(12));
    let last = ObjectMemberRef {
        object: EntityId::new(12),
        member: 2,
    };
    host.object_members.insert(last, 255);

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 51), 5);
    assert_eq!(host.object_members[&last], 5);
    assert_eq!(host.member_target, Some(last.object));
    assert_eq!(host.member_writes, [(last, 5)]);

    let result = variable(VariableScope::Global, VariableType::Int16, 51);
    let expression = assign(
        result,
        [vec![41, 4], constant(0), constant(1), vec![67]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let current = ObjectMemberRef {
        object: EntityId::new(42),
        member: 4,
    };
    let mut host = Host::new();
    host.member_target = Some(current.object);
    host.object_members.insert(current, 15);
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 51), 0);
    assert_eq!(host.member_target, Some(current.object));
    assert_eq!(host.member_writes, [(current, 0)]);
}

#[test]
fn member_or_assignment_uses_the_last_selected_target() {
    let result = variable(VariableScope::Global, VariableType::Int16, 52);
    let expression = assign(
        result,
        [vec![41, 2], constant(0b101), constant(2), vec![69]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    host.slots.insert(0, EntityId::new(10));
    host.slots.insert(2, EntityId::new(12));
    let first = ObjectMemberRef {
        object: EntityId::new(10),
        member: 2,
    };
    let last = ObjectMemberRef {
        object: EntityId::new(12),
        member: 2,
    };
    host.object_members.insert(first, 0);
    host.object_members.insert(last, 255);

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 52), 6);
    assert_eq!(host.object_members[&last], 6);
    assert_eq!(host.member_target, Some(last.object));
    assert_eq!(host.member_writes, [(last, 6)]);

    let result = variable(VariableScope::Global, VariableType::Int16, 52);
    let expression = assign(
        result,
        [vec![41, 4], constant(0), constant(3), vec![69]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let current = ObjectMemberRef {
        object: EntityId::new(42),
        member: 4,
    };
    let mut host = Host::new();
    host.member_target = Some(current.object);
    host.object_members.insert(current, 15);
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 52), 3);
    assert_eq!(host.member_target, Some(current.object));
    assert_eq!(host.member_writes, [(current, 3)]);
}

#[test]
fn empty_member_xor_mask_preserves_and_writes_the_current_target() {
    let result = variable(VariableScope::Global, VariableType::Int16, 17);
    let expression = assign(
        result,
        [vec![41, 4], constant(0), constant(1), vec![68]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let current = ObjectMemberRef {
        object: EntityId::new(42),
        member: 4,
    };
    host.member_target = Some(current.object);
    host.object_members.insert(current, 15);

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 17), 1);
    assert_eq!(host.member_target, Some(current.object));
    assert_eq!(host.member_writes, [(current, 1)]);
}

#[test]
fn all_member_assignment_writes_each_selection_without_reading_old_values() {
    let result = variable(VariableScope::Global, VariableType::Int16, 54);
    let expression = assign(
        result,
        [vec![41, 2], constant(0b101), constant(7), vec![45]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    host.slots.insert(0, EntityId::new(10));
    host.slots.insert(2, EntityId::new(12));
    let first = ObjectMemberRef {
        object: EntityId::new(10),
        member: 2,
    };
    let last = ObjectMemberRef {
        object: EntityId::new(12),
        member: 2,
    };

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 54), 7);
    assert_eq!(host.object_members[&first], 7);
    assert_eq!(host.object_members[&last], 7);
    assert_eq!(host.member_target, Some(last.object));
    assert_eq!(host.member_writes, [(first, 7), (last, 7)]);

    let result = variable(VariableScope::Global, VariableType::Int16, 54);
    let expression = assign(
        result,
        [vec![41, 4], constant(0), constant(1), vec![45]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 54), 0);
    assert_eq!(host.member_target, None);
    assert!(host.member_writes.is_empty());
}

#[test]
fn all_member_xor_assignment_writes_each_selection_and_returns_the_last_value() {
    let result = variable(VariableScope::Global, VariableType::Int16, 36);
    let expression = assign(
        result,
        [vec![41, 2], constant(0b101), constant(2), vec![65]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    host.slots.insert(0, EntityId::new(10));
    host.slots.insert(2, EntityId::new(12));
    let first = ObjectMemberRef {
        object: EntityId::new(10),
        member: 2,
    };
    let last = ObjectMemberRef {
        object: EntityId::new(12),
        member: 2,
    };
    host.object_members.insert(first, 3);
    host.object_members.insert(last, 9);

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 36), 11);
    assert_eq!(host.object_members[&first], 1);
    assert_eq!(host.object_members[&last], 11);
    assert_eq!(host.member_target, Some(last.object));
    assert_eq!(host.member_writes, [(first, 1), (last, 11)]);

    let result = variable(VariableScope::Global, VariableType::Int16, 36);
    let expression = assign(
        result,
        [vec![41, 4], constant(0), constant(1), vec![65]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 36), 0);
    assert_eq!(host.member_target, None);
    assert!(host.member_writes.is_empty());
}

#[test]
fn member_pre_increment_updates_every_selected_slot_and_returns_the_last_new_value() {
    let result = variable(VariableScope::Global, VariableType::Int16, 40);
    let expression = assign(result, [constant(0b101), vec![41, 2], vec![10]].concat());
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    host.slots.insert(0, EntityId::new(10));
    host.slots.insert(2, EntityId::new(12));
    let first = ObjectMemberRef {
        object: EntityId::new(10),
        member: 2,
    };
    let last = ObjectMemberRef {
        object: EntityId::new(12),
        member: 2,
    };
    host.object_members.insert(first, 3);
    host.object_members.insert(last, 9);

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 40), 10);
    assert_eq!(host.object_members[&first], 4);
    assert_eq!(host.object_members[&last], 10);
    assert_eq!(host.member_target, Some(last.object));
    assert_eq!(host.member_writes, [(first, 4), (last, 10)]);

    let result = variable(VariableScope::Global, VariableType::Int16, 40);
    let expression = assign(result, [constant(0), vec![41, 4], vec![10]].concat());
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 40), 0);
    assert_eq!(host.member_target, None);
    assert!(host.member_writes.is_empty());
}

#[test]
fn member_post_decrement_updates_every_selected_slot_and_returns_the_last_old_value() {
    let result = variable(VariableScope::Global, VariableType::Int16, 18);
    let expression = assign(result, [constant(0b101), vec![41, 2], vec![9]].concat());
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    host.slots.insert(0, EntityId::new(10));
    host.slots.insert(2, EntityId::new(12));
    let first = ObjectMemberRef {
        object: EntityId::new(10),
        member: 2,
    };
    let last = ObjectMemberRef {
        object: EntityId::new(12),
        member: 2,
    };
    host.object_members.insert(first, 3);
    host.object_members.insert(last, 9);

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 18), 9);
    assert_eq!(host.object_members[&first], 2);
    assert_eq!(host.object_members[&last], 8);
    assert_eq!(host.member_target, Some(last.object));
    assert_eq!(host.member_writes, [(first, 2), (last, 8)]);
}

#[test]
fn empty_member_post_decrement_returns_zero_without_selecting_or_writing() {
    let result = variable(VariableScope::Global, VariableType::Int16, 19);
    let expression = assign(result, [constant(0), vec![41, 4], vec![9]].concat());
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 19), 0);
    assert_eq!(host.member_target, None);
    assert!(host.member_writes.is_empty());
}

#[test]
fn member_logical_not_returns_zero_flags_for_selected_members_without_writes() {
    let result = variable(VariableScope::Global, VariableType::Int16, 20);
    let expression = assign(result, [constant(0b101), vec![41, 2], vec![15]].concat());
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    host.slots.insert(0, EntityId::new(10));
    host.slots.insert(2, EntityId::new(12));
    host.object_members.insert(
        ObjectMemberRef {
            object: EntityId::new(10),
            member: 2,
        },
        0,
    );
    host.object_members.insert(
        ObjectMemberRef {
            object: EntityId::new(12),
            member: 2,
        },
        9,
    );

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 20), 1);
    assert_eq!(host.member_target, Some(EntityId::new(12)));
    assert!(host.member_writes.is_empty());
}

#[test]
fn empty_member_logical_not_returns_zero_without_selecting_a_target() {
    let result = variable(VariableScope::Global, VariableType::Int16, 21);
    let expression = assign(result, [constant(0), vec![41, 4], vec![15]].concat());
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 21), 0);
    assert_eq!(host.member_target, None);
    assert!(host.member_writes.is_empty());
}

#[test]
fn direct_pre_increment_returns_and_stores_the_new_variable_value() {
    let target = variable(VariableScope::Map, VariableType::Int16, 8);
    let result = variable(VariableScope::Global, VariableType::Int16, 22);
    let bytes = [
        evaluate(assign(target.clone(), constant(4))),
        evaluate(assign(result, [target, vec![6]].concat())),
        vec![4],
    ]
    .concat();
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Map, 8), 5);
    assert_eq!(vm.variable(VariableScope::Global, 22), 5);
}

#[test]
fn direct_pre_increment_writes_the_current_member_target() {
    let result = variable(VariableScope::Global, VariableType::Int16, 23);
    let expression = assign(result, [vec![41, 4], vec![6]].concat());
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let current = ObjectMemberRef {
        object: EntityId::new(42),
        member: 4,
    };
    host.member_target = Some(current.object);
    host.object_members.insert(current, 15);

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 23), 16);
    assert_eq!(host.member_writes, [(current, 16)]);
}

#[test]
fn bitwise_complement_inverts_every_bit() {
    for (input, expected) in [(0, -1), (-1, 0), (0x55, !0x55)] {
        let result = variable(VariableScope::Global, VariableType::Int24, 24);
        let expression = assign(result, [constant(input), vec![16]].concat());
        let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
        let mut vm = EventVm::new(program);
        let mut host = Host::new();

        vm.step(Tick::new(0), &mut host).unwrap();

        assert_eq!(vm.variable(VariableScope::Global, 24), expected);
    }
}

#[test]
fn member_equality_returns_comparison_flags_without_writes() {
    let result = variable(VariableScope::Global, VariableType::Int16, 25);
    let expression = assign(
        result,
        [constant(0b101), vec![41, 2], constant(9), vec![34]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    host.slots.insert(0, EntityId::new(10));
    host.slots.insert(2, EntityId::new(12));
    host.object_members.insert(
        ObjectMemberRef {
            object: EntityId::new(10),
            member: 2,
        },
        9,
    );
    host.object_members.insert(
        ObjectMemberRef {
            object: EntityId::new(12),
            member: 2,
        },
        8,
    );

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 25), 1);
    assert_eq!(host.member_target, Some(EntityId::new(12)));
    assert!(host.member_writes.is_empty());
}

#[test]
fn member_inequality_returns_comparison_flags_without_writes() {
    let result = variable(VariableScope::Global, VariableType::Int16, 64);
    let expression = assign(
        result,
        [constant(0b101), vec![41, 2], constant(9), vec![35]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    host.slots.insert(0, EntityId::new(10));
    host.slots.insert(2, EntityId::new(12));
    host.object_members.insert(
        ObjectMemberRef {
            object: EntityId::new(10),
            member: 2,
        },
        9,
    );
    host.object_members.insert(
        ObjectMemberRef {
            object: EntityId::new(12),
            member: 2,
        },
        8,
    );

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 64), 4);
    assert_eq!(host.member_target, Some(EntityId::new(12)));
    assert!(host.member_writes.is_empty());

    let result = variable(VariableScope::Global, VariableType::Int16, 64);
    let expression = assign(
        result,
        [constant(0), vec![41, 4], constant(1), vec![35]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 64), 0);
    assert_eq!(host.member_target, None);
}

#[test]
fn empty_member_equality_returns_zero_without_selecting_a_target() {
    let result = variable(VariableScope::Global, VariableType::Int16, 26);
    let expression = assign(
        result,
        [constant(0), vec![41, 4], constant(1), vec![34]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 26), 0);
    assert_eq!(host.member_target, None);
    assert!(host.member_writes.is_empty());
}

#[test]
fn member_xor_returns_nonzero_flags_without_writes() {
    let result = variable(VariableScope::Global, VariableType::Int16, 67);
    let expression = assign(
        result,
        [constant(0b101), vec![41, 2], constant(12), vec![86]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    host.slots.insert(0, EntityId::new(10));
    host.slots.insert(2, EntityId::new(12));
    host.object_members.insert(
        ObjectMemberRef {
            object: EntityId::new(10),
            member: 2,
        },
        15,
    );
    host.object_members.insert(
        ObjectMemberRef {
            object: EntityId::new(12),
            member: 2,
        },
        12,
    );

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 67), 1);
    assert_eq!(host.member_target, Some(EntityId::new(12)));
    assert!(host.member_writes.is_empty());

    let result = variable(VariableScope::Global, VariableType::Int16, 67);
    let expression = assign(
        result,
        [constant(0), vec![41, 4], constant(1), vec![86]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 67), 0);
    assert_eq!(host.member_target, None);
}

#[test]
fn member_less_returns_comparison_flags_without_writes() {
    let result = variable(VariableScope::Global, VariableType::Int16, 52);
    let expression = assign(
        result,
        [constant(0b101), vec![41, 2], constant(9), vec![28]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    host.slots.insert(0, EntityId::new(10));
    host.slots.insert(2, EntityId::new(12));
    host.object_members.insert(
        ObjectMemberRef {
            object: EntityId::new(10),
            member: 2,
        },
        8,
    );
    host.object_members.insert(
        ObjectMemberRef {
            object: EntityId::new(12),
            member: 2,
        },
        9,
    );

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 52), 0b001);
    assert_eq!(host.member_target, Some(EntityId::new(12)));
    assert!(host.member_writes.is_empty());

    let result = variable(VariableScope::Global, VariableType::Int16, 52);
    let expression = assign(
        result,
        [constant(0), vec![41, 4], constant(1), vec![28]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 52), 0);
    assert_eq!(host.member_target, None);
    assert!(host.member_writes.is_empty());
}

#[test]
fn member_less_or_equal_returns_comparison_flags_without_writes() {
    let result = variable(VariableScope::Global, VariableType::Int16, 34);
    let expression = assign(
        result,
        [constant(0b101), vec![41, 2], constant(9), vec![30]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    host.slots.insert(0, EntityId::new(10));
    host.slots.insert(2, EntityId::new(12));
    host.object_members.insert(
        ObjectMemberRef {
            object: EntityId::new(10),
            member: 2,
        },
        9,
    );
    host.object_members.insert(
        ObjectMemberRef {
            object: EntityId::new(12),
            member: 2,
        },
        8,
    );

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 34), 0b101);
    assert_eq!(host.member_target, Some(EntityId::new(12)));
    assert!(host.member_writes.is_empty());

    let result = variable(VariableScope::Global, VariableType::Int16, 34);
    let expression = assign(
        result,
        [constant(0), vec![41, 4], constant(1), vec![30]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 34), 0);
    assert_eq!(host.member_target, None);
}

#[test]
fn member_greater_returns_comparison_flags_without_writes() {
    let result = variable(VariableScope::Global, VariableType::Int16, 35);
    let expression = assign(
        result,
        [constant(0b101), vec![41, 2], constant(9), vec![29]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    host.slots.insert(0, EntityId::new(10));
    host.slots.insert(2, EntityId::new(12));
    host.object_members.insert(
        ObjectMemberRef {
            object: EntityId::new(10),
            member: 2,
        },
        10,
    );
    host.object_members.insert(
        ObjectMemberRef {
            object: EntityId::new(12),
            member: 2,
        },
        9,
    );

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 35), 0b001);
    assert_eq!(host.member_target, Some(EntityId::new(12)));
    assert!(host.member_writes.is_empty());

    let result = variable(VariableScope::Global, VariableType::Int16, 35);
    let expression = assign(
        result,
        [constant(0), vec![41, 4], constant(1), vec![29]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 35), 0);
    assert_eq!(host.member_target, None);
}

#[test]
fn member_greater_or_equal_returns_comparison_flags_without_writes() {
    let result = variable(VariableScope::Global, VariableType::Int16, 37);
    let expression = assign(
        result,
        [constant(0b101), vec![41, 2], constant(9), vec![31]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    host.slots.insert(0, EntityId::new(10));
    host.slots.insert(2, EntityId::new(12));
    host.object_members.insert(
        ObjectMemberRef {
            object: EntityId::new(10),
            member: 2,
        },
        10,
    );
    host.object_members.insert(
        ObjectMemberRef {
            object: EntityId::new(12),
            member: 2,
        },
        9,
    );

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 37), 0b101);
    assert_eq!(host.member_target, Some(EntityId::new(12)));
    assert!(host.member_writes.is_empty());

    let result = variable(VariableScope::Global, VariableType::Int16, 37);
    let expression = assign(
        result,
        [constant(0), vec![41, 4], constant(1), vec![31]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 37), 0);
    assert_eq!(host.member_target, None);
    assert!(host.member_writes.is_empty());
}

#[test]
fn all_member_and_assignment_writes_each_selection_and_returns_the_last_value() {
    let result = variable(VariableScope::Global, VariableType::Int16, 38);
    let expression = assign(
        result,
        [vec![41, 2], constant(0b101), constant(6), vec![64]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    host.slots.insert(0, EntityId::new(10));
    host.slots.insert(2, EntityId::new(12));
    let first = ObjectMemberRef {
        object: EntityId::new(10),
        member: 2,
    };
    let last = ObjectMemberRef {
        object: EntityId::new(12),
        member: 2,
    };
    host.object_members.insert(first, 3);
    host.object_members.insert(last, 11);

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 38), 2);
    assert_eq!(host.object_members[&first], 2);
    assert_eq!(host.object_members[&last], 2);
    assert_eq!(host.member_target, Some(last.object));
    assert_eq!(host.member_writes, [(first, 2), (last, 2)]);

    let result = variable(VariableScope::Global, VariableType::Int16, 38);
    let expression = assign(
        result,
        [vec![41, 4], constant(0), constant(1), vec![64]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 38), 0);
    assert_eq!(host.member_target, None);
    assert!(host.member_writes.is_empty());
}

#[test]
fn all_member_right_shift_assignment_writes_each_selection_and_returns_the_last_value() {
    let result = variable(VariableScope::Global, VariableType::Int16, 68);
    let expression = assign(
        result,
        [vec![41, 2], constant(0b101), constant(2), vec![60]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    host.slots.insert(0, EntityId::new(10));
    host.slots.insert(2, EntityId::new(12));
    let first = ObjectMemberRef {
        object: EntityId::new(10),
        member: 2,
    };
    let last = ObjectMemberRef {
        object: EntityId::new(12),
        member: 2,
    };
    host.object_members.insert(first, -16);
    host.object_members.insert(last, 32);

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 68), 8);
    assert_eq!(host.object_members[&first], -4);
    assert_eq!(host.object_members[&last], 8);
    assert_eq!(host.member_target, Some(last.object));
    assert_eq!(host.member_writes, [(first, -4), (last, 8)]);

    let result = variable(VariableScope::Global, VariableType::Int16, 68);
    let expression = assign(
        result,
        [vec![41, 4], constant(0), constant(1), vec![60]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 68), 0);
    assert_eq!(host.member_target, None);
    assert!(host.member_writes.is_empty());
}

#[test]
fn all_member_subtract_assignment_writes_each_selection_and_returns_the_last_value() {
    let result = variable(VariableScope::Global, VariableType::Int16, 69);
    let expression = assign(
        result,
        [vec![41, 2], constant(0b101), constant(6), vec![58]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    host.slots.insert(0, EntityId::new(10));
    host.slots.insert(2, EntityId::new(12));
    let first = ObjectMemberRef {
        object: EntityId::new(10),
        member: 2,
    };
    let last = ObjectMemberRef {
        object: EntityId::new(12),
        member: 2,
    };
    host.object_members.insert(first, 3);
    host.object_members.insert(last, 11);

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 69), 5);
    assert_eq!(host.object_members[&first], -3);
    assert_eq!(host.object_members[&last], 5);
    assert_eq!(host.member_target, Some(last.object));
    assert_eq!(host.member_writes, [(first, -3), (last, 5)]);

    let result = variable(VariableScope::Global, VariableType::Int16, 69);
    let expression = assign(
        result,
        [vec![41, 4], constant(0), constant(1), vec![58]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 69), 0);
    assert_eq!(host.member_target, None);
    assert!(host.member_writes.is_empty());
}

#[test]
fn all_member_divide_assignment_preserves_zero_divisors_and_returns_the_last_value() {
    let result = variable(VariableScope::Global, VariableType::Int16, 70);
    let expression = assign(
        result,
        [vec![41, 2], constant(0b101), constant(2), vec![55]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    host.slots.insert(0, EntityId::new(10));
    host.slots.insert(2, EntityId::new(12));
    let first = ObjectMemberRef {
        object: EntityId::new(10),
        member: 2,
    };
    let last = ObjectMemberRef {
        object: EntityId::new(12),
        member: 2,
    };
    host.object_members.insert(first, -16);
    host.object_members.insert(last, 32);

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 70), 16);
    assert_eq!(host.object_members[&first], -8);
    assert_eq!(host.object_members[&last], 16);
    assert_eq!(host.member_target, Some(last.object));
    assert_eq!(host.member_writes, [(first, -8), (last, 16)]);

    let result = variable(VariableScope::Global, VariableType::Int16, 70);
    let expression = assign(
        result,
        [vec![41, 2], constant(0b101), constant(0), vec![55]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    host.object_members.insert(first, -16);
    host.object_members.insert(last, 32);
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 70), 32);
    assert_eq!(host.object_members[&first], -16);
    assert_eq!(host.object_members[&last], 32);

    let result = variable(VariableScope::Global, VariableType::Int16, 70);
    let expression = assign(
        result,
        [vec![41, 4], constant(0), constant(1), vec![55]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 70), 0);
    assert_eq!(host.member_target, None);
    assert!(host.member_writes.is_empty());
}

#[test]
fn all_member_multiply_assignment_writes_each_selection_and_returns_the_last_value() {
    let result = variable(VariableScope::Global, VariableType::Int16, 71);
    let expression = assign(
        result,
        [vec![41, 2], constant(0b101), constant(2), vec![54]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    host.slots.insert(0, EntityId::new(10));
    host.slots.insert(2, EntityId::new(12));
    let first = ObjectMemberRef {
        object: EntityId::new(10),
        member: 2,
    };
    let last = ObjectMemberRef {
        object: EntityId::new(12),
        member: 2,
    };
    host.object_members.insert(first, -16);
    host.object_members.insert(last, 32);

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 71), 64);
    assert_eq!(host.object_members[&first], -32);
    assert_eq!(host.object_members[&last], 64);
    assert_eq!(host.member_target, Some(last.object));
    assert_eq!(host.member_writes, [(first, -32), (last, 64)]);

    let result = variable(VariableScope::Global, VariableType::Int16, 71);
    let expression = assign(
        result,
        [vec![41, 4], constant(0), constant(1), vec![54]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 71), 0);
    assert_eq!(host.member_target, None);
    assert!(host.member_writes.is_empty());
}

#[test]
fn member_and_returns_bit_intersection_flags_without_writes() {
    let result = variable(VariableScope::Global, VariableType::Int16, 46);
    let expression = assign(
        result,
        [constant(0b101), vec![41, 2], constant(2), vec![84]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    host.slots.insert(0, EntityId::new(10));
    host.slots.insert(2, EntityId::new(12));
    host.object_members.insert(
        ObjectMemberRef {
            object: EntityId::new(10),
            member: 2,
        },
        3,
    );
    host.object_members.insert(
        ObjectMemberRef {
            object: EntityId::new(12),
            member: 2,
        },
        8,
    );

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 46), 0b001);
    assert_eq!(host.member_target, Some(EntityId::new(12)));
    assert!(host.member_writes.is_empty());

    let result = variable(VariableScope::Global, VariableType::Int16, 46);
    let expression = assign(
        result,
        [constant(0), vec![41, 4], constant(1), vec![84]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 46), 0);
    assert_eq!(host.member_target, None);
    assert!(host.member_writes.is_empty());
}

#[test]
fn member_nand_returns_negated_intersection_flags_without_writes() {
    let result = variable(VariableScope::Global, VariableType::Int16, 63);
    let expression = assign(
        result,
        [constant(0b101), vec![41, 2], constant(2), vec![85]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    host.slots.insert(0, EntityId::new(10));
    host.slots.insert(2, EntityId::new(12));
    host.object_members.insert(
        ObjectMemberRef {
            object: EntityId::new(10),
            member: 2,
        },
        3,
    );
    host.object_members.insert(
        ObjectMemberRef {
            object: EntityId::new(12),
            member: 2,
        },
        8,
    );

    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 63), 0b100);
    assert_eq!(host.member_target, Some(EntityId::new(12)));
    assert!(host.member_writes.is_empty());

    let result = variable(VariableScope::Global, VariableType::Int16, 63);
    let expression = assign(
        result,
        [constant(0), vec![41, 4], constant(1), vec![85]].concat(),
    );
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 63), 0);
    assert_eq!(host.member_target, None);
}

#[test]
fn system_variables_read_by_serialized_id_and_reject_unknown_ids() {
    let result = variable(VariableScope::Global, VariableType::Int16, 27);
    let program =
        decode_program(&[evaluate(assign(result, vec![122, 5])), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 27), 77);

    let program = decode_program(&evaluate(vec![122, 6])).unwrap();
    let mut vm = EventVm::new(program);
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("unknown system variable 6"));
}

#[test]
fn new3_emits_object_initialization_and_preserves_the_default_id_for_the_host() {
    let program = decode_program(&[9, 7, 0, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 3);
    assert_eq!(
        program.instructions[&0].kind,
        InstructionKind::InitializeObject {
            source: 7,
            unique_id: 0
        }
    );
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    let report = vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(
        report.events,
        [HostEvent::InitializeObject {
            source: 7,
            unique_id: 0
        }]
    );
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn new2_emits_region_initialization_with_a_distinct_host_command() {
    let program = decode_program(&[8, 11, 130, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 3);
    assert_eq!(
        program.instructions[&0].kind,
        InstructionKind::InitializeRegion {
            source: 11,
            unique_id: 130
        }
    );
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    let report = vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(
        report.events,
        [HostEvent::InitializeRegion {
            source: 11,
            unique_id: 130
        }]
    );
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn new_emits_code_initialization_with_a_distinct_host_command() {
    let program = decode_program(&[7, 3, 0, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 3);
    assert_eq!(
        program.instructions[&0].kind,
        InstructionKind::InitializeCode {
            source: 3,
            unique_id: 0
        }
    );
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    let report = vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(
        report.events,
        [HostEvent::InitializeCode {
            source: 3,
            unique_id: 0
        }]
    );
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn bgiactivet_supports_immediate_and_expression_arguments() {
    let program = decode_program(&[154, 0, 0x34, 0x12, 1, 4]).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetTriangleActive {
            triangle: 0x1234,
            active: true
        }]
    );

    let mut bytes = vec![154, 3];
    append_expression(&mut bytes, constant(-1));
    append_expression(&mut bytes, constant(0));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program);
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetTriangleActive {
            triangle: u32::MAX,
            active: false
        }]
    );
}

#[test]
fn fldsnd0_preserves_signed_ids_and_supplies_zero_audio_arguments() {
    let program = decode_program(&[197, 0, 0x00, 0xd0, 0xe3, 0x0a, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 6);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    let report = vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(
        report.events,
        [HostEvent::RunSound {
            code: -12288,
            sound: 2787,
            arguments: [0, 0, 0]
        }]
    );
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn field_sound_variants_supply_only_their_declared_arguments() {
    for (opcode, length, arguments) in [
        (197, 6, [0, 0, 0]),
        (198, 9, [-1000, 0, 0]),
        (199, 10, [-1000, 250, 0]),
        (200, 11, [-1000, 250, 130]),
    ] {
        let mut bytes = vec![opcode, 0, 0x00, 0xd0, 0xe3, 0x0a];
        if opcode != 197 {
            bytes.extend_from_slice(&(-1000_i32).to_le_bytes()[..3]);
        }
        if opcode == 199 || opcode == 200 {
            bytes.push(250);
        }
        if opcode == 200 {
            bytes.push(130);
        }
        bytes.push(4);
        let program = decode_program(&bytes).unwrap();
        assert_eq!(program.instructions[&0].length, length);
        let mut vm = EventVm::new(program);
        let mut host = Host::new();
        let report = vm.step(Tick::new(0), &mut host).unwrap();
        assert_eq!(
            report.events,
            [HostEvent::RunSound {
                code: -12288,
                sound: 2787,
                arguments,
            }]
        );
        assert!(
            vm.traces()
                .any(|trace| trace.opcode == Some(u32::from(opcode)))
        );
    }

    let mut bytes = vec![199, 15];
    for value in [-2, 300, -1000, 128] {
        append_expression(&mut bytes, constant(value));
    }
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::RunSound {
            code: -2,
            sound: 300,
            arguments: [-1000, 128, 0],
        }]
    );

    let truncated = decode_program(&[199, 0, 1, 0, 2]).unwrap_err();
    assert_eq!(truncated.kind, VmErrorKind::Decode);
    assert!(truncated.message.contains("opcode Some(199)"));
}

#[test]
fn attachoffset_emits_signed_coordinates_for_the_current_actor() {
    let bytes = [212, 0, 10, 0, 0xec, 0xff, 30, 0, 4];
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();

    let report = vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(
        report.events,
        [HostEvent::AttachObjectOffset {
            actor: EntityId::new(42),
            x: 10,
            y: -20,
            z: 30
        }]
    );

    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("actor context"));
}

#[test]
fn bgimask_emits_the_wrapped_attribute_byte() {
    let program = decode_program(&[39, 0, 0x80, 4]).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetNavigationAttributeMask { mask: 0x80 }]
    );

    let mut bytes = vec![39, 1];
    append_expression(&mut bytes, constant(-1));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program);
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetNavigationAttributeMask { mask: 0xff }]
    );
}

#[test]
fn model_sets_actor_model_and_scaled_eye_height() {
    let program = decode_program(&[47, 0, 0x34, 0x12, 20, 4]).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetActorModel {
            actor: EntityId::new(42),
            model: 0x1234,
            eye_height: -80
        }]
    );

    let mut bytes = vec![47, 3];
    append_expression(&mut bytes, constant(300));
    append_expression(&mut bytes, constant(-1));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetActorModel {
            actor: EntityId::new(42),
            model: 300,
            eye_height: 4
        }]
    );

    let mut vm = EventVm::new(decode_program(&[47, 0, 1, 0, 20, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("actor context"));
}

#[test]
fn clrdist_resets_the_current_actor_walk_state() {
    let program = decode_program(&[37, 0, 4]).unwrap();
    assert_eq!(
        program.instructions[&0].kind,
        InstructionKind::ResetWalkState
    );
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::ResetWalkState {
            actor: EntityId::new(42)
        }]
    );
    assert_eq!(vm.status(), VmStatus::Completed);

    let mut vm = EventVm::new(decode_program(&[37, 0, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("actor context"));
}

#[test]
fn mspeed_sets_the_current_actor_speed_and_requires_actor_context() {
    let program = decode_program(&[38, 0, 30, 4]).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetMovementSpeed {
            actor: EntityId::new(42),
            speed: 30
        }]
    );

    let mut bytes = vec![38, 1];
    append_expression(&mut bytes, constant(-1));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetMovementSpeed {
            actor: EntityId::new(42),
            speed: 255
        }]
    );

    let mut vm = EventVm::new(decode_program(&[38, 0, 30, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("actor context"));
}

#[test]
fn cast_list_matches_the_reference_empty_implementation() {
    let result = variable(VariableScope::Global, VariableType::Int16, 28);
    let program = decode_program(&[evaluate(assign(result, vec![74])), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 28), 0);
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn right_shift_is_arithmetic_and_masks_the_shift_count() {
    for (left, right, expected) in [(-8, 2, -2), (16, 2, 4), (1, 33, 0)] {
        let result = variable(VariableScope::Global, VariableType::Int24, 29);
        let expression = assign(result, binary(constant(left), constant(right), 23));
        let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
        let mut vm = EventVm::new(program);
        let mut host = Host::new();

        vm.step(Tick::new(0), &mut host).unwrap();

        assert_eq!(vm.variable(VariableScope::Global, 29), expected);
    }
}

#[test]
fn left_shift_wraps_and_masks_the_shift_count() {
    for (left, right, expected) in [(3, 2, 12), (-2, 1, -4), (1, 33, 2)] {
        let result = variable(VariableScope::Global, VariableType::Int24, 31);
        let expression = assign(result, binary(constant(left), constant(right), 22));
        let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
        let mut vm = EventVm::new(program);
        let mut host = Host::new();

        vm.step(Tick::new(0), &mut host).unwrap();

        assert_eq!(vm.variable(VariableScope::Global, 31), expected);
    }
}

#[test]
fn sine_uses_fixed_point_degrees_and_truncates_the_result() {
    for (angle, expected) in [(0, 0), (32, 2896), (64, 4096), (-64, -4096)] {
        let result = variable(VariableScope::Global, VariableType::Int24, 32);
        let expression = assign(result, binary(constant(angle), vec![], 98));
        let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
        let mut vm = EventVm::new(program);
        let mut host = Host::new();

        vm.step(Tick::new(0), &mut host).unwrap();

        assert_eq!(vm.variable(VariableScope::Global, 32), expected);
    }
}

#[test]
fn sine2_uses_the_unshifted_fixed_point_angle() {
    for (angle, expected) in [(0, 0), (1024, 4096), (2048, 0), (3072, -4096)] {
        let result = variable(VariableScope::Global, VariableType::Int24, 66);
        let expression = assign(result, binary(constant(angle), vec![], 80));
        let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
        let mut vm = EventVm::new(program);
        let mut host = Host::new();

        vm.step(Tick::new(0), &mut host).unwrap();

        assert_eq!(vm.variable(VariableScope::Global, 66), expected);
    }

    let program = decode_program(&[5, 80, 127, 4]).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::InvalidExpression);
    assert!(error.message.contains("unshifted sine operand"));
}

#[test]
fn cosine_uses_a_shifted_fixed_point_angle() {
    for (angle, expected) in [(0, 4096), (64, 0), (128, -4096), (192, 0)] {
        let result = variable(VariableScope::Global, VariableType::Int24, 73);
        let expression = assign(result, binary(constant(angle), vec![], 99));
        let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
        let mut vm = EventVm::new(program);
        let mut host = Host::new();

        vm.step(Tick::new(0), &mut host).unwrap();

        assert_eq!(vm.variable(VariableScope::Global, 73), expected);
    }

    let program = decode_program(&[5, 99, 127, 4]).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::InvalidExpression);
    assert!(error.message.contains("shifted cosine operand"));
}

#[test]
fn cosine2_uses_the_unshifted_fixed_point_angle() {
    for (angle, expected) in [(0, 4096), (1024, 0), (2048, -4096)] {
        let result = variable(VariableScope::Global, VariableType::Int24, 65);
        let expression = assign(result, binary(constant(angle), vec![], 81));
        let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
        let mut vm = EventVm::new(program);
        let mut host = Host::new();

        vm.step(Tick::new(0), &mut host).unwrap();

        assert_eq!(vm.variable(VariableScope::Global, 65), expected);
    }

    let program = decode_program(&[5, 81, 127, 4]).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::InvalidExpression);
    assert!(error.message.contains("cosine operand"));
}

#[test]
fn unnamed_expression_114_has_no_stack_effect() {
    let result = variable(VariableScope::Global, VariableType::Int24, 33);
    let expression = assign(result, binary(constant(41), vec![], 114));
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 33), 41);
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn select_chooses_an_ordered_active_bit_with_one_random_byte() {
    for (random_u8, expected) in [(0, 1), (255, 4)] {
        let result = variable(VariableScope::Global, VariableType::Int24, 72);
        let expression = assign(result, binary(constant(0b101), vec![], 77));
        let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
        let mut vm = EventVm::new(program);
        let mut host = Host::new();
        host.random_u8 = random_u8;

        vm.step(Tick::new(0), &mut host).unwrap();

        assert_eq!(vm.variable(VariableScope::Global, 72), expected);
    }

    let result = variable(VariableScope::Global, VariableType::Int24, 72);
    let expression = assign(result, binary(constant(0), vec![], 77));
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 72), 0);

    let program = decode_program(&[5, 77, 127, 4]).unwrap();
    let mut vm = EventVm::new(program);
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::InvalidExpression);
    assert!(error.message.contains("random-selection operand"));
}

#[test]
fn keyon2_consumes_its_operand_and_returns_zero() {
    let result = variable(VariableScope::Global, VariableType::Int24, 39);
    let expression = assign(result, binary(constant(99), vec![], 90));
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 39), 0);
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn keyon_returns_mask_intersection_with_host_input() {
    let mut host = Host::new();
    for (mask, pressed, expected) in [(2, 2, 1), (1, 2, 0), (i32::MIN, i32::MIN, 1)] {
        host.key_on = pressed;
        let result = variable(VariableScope::Global, VariableType::Int16, 53);
        let expression = assign(result, binary(constant(mask), vec![], 79));
        let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
        let mut vm = EventVm::new(program);

        vm.step(Tick::new(0), &mut host).unwrap();

        assert_eq!(vm.variable(VariableScope::Global, 53), expected);
    }
}

#[test]
fn key2_consumes_its_operand_and_returns_zero() {
    let result = variable(VariableScope::Global, VariableType::Int24, 42);
    let expression = assign(result, binary(constant(99), vec![], 92));
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 42), 0);
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn have_item_returns_the_actual_count() {
    let result = variable(VariableScope::Global, VariableType::Int16, 47);
    let expression = assign(result, binary(constant(12), vec![], 100));
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    host.item_counts.insert(12, 3);

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 47), 3);

    let result = variable(VariableScope::Global, VariableType::Int16, 47);
    let expression = assign(result, binary(constant(13), vec![], 100));
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 47), 0);
}

#[test]
fn frame_reads_the_current_object_animation_frame() {
    let result = variable(VariableScope::Global, VariableType::Int16, 48);
    let expression = assign(result, binary(constant(7), vec![], 106));
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let actor = EntityId::new(42);
    let mut vm = EventVm::new(program).with_entity(actor);
    let mut host = Host::new();
    host.animation_frames.insert((actor, 7), 19);

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 48), 19);

    let result = variable(VariableScope::Global, VariableType::Int16, 48);
    let expression = assign(result, binary(constant(7), vec![], 106));
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::InvalidExpression);
    assert!(error.message.contains("object context"));
}

#[test]
fn baframe_reads_unsigned_bgi_animation_frame() {
    let mut host = Host::new();
    host.bgi_animation_frames.insert(7, 23);
    host.bgi_animation_frames.insert(u32::MAX, 11);
    for (animation, expected) in [(7, 23), (-1, 11)] {
        let result = variable(VariableScope::Global, VariableType::Int16, 49);
        let expression = assign(result, binary(constant(animation), vec![], 101));
        let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
        let mut vm = EventVm::new(program);

        vm.step(Tick::new(0), &mut host).unwrap();

        assert_eq!(vm.variable(VariableScope::Global, 49), expected);
    }
}

#[test]
fn party_check_returns_zero_or_one_from_host_membership() {
    let mut host = Host::new();
    host.party_members.push(2);
    for (character, expected) in [(2, 1), (3, 0)] {
        let result = variable(VariableScope::Global, VariableType::Int16, 50);
        let expression = assign(result, binary(constant(character), vec![], 107));
        let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
        let mut vm = EventVm::new(program);

        vm.step(Tick::new(0), &mut host).unwrap();

        assert_eq!(vm.variable(VariableScope::Global, 50), expected);
    }
}

#[test]
fn party_add_preserves_the_reference_failure_result_and_membership() {
    for (initial, character, expected, expected_members) in [
        (vec![1, 3], 2, 0, vec![1, 3, 2]),
        (vec![1, 2], 2, 0, vec![1, 2]),
        (vec![0, 1, 2, 3], 4, 1, vec![0, 1, 2, 3]),
        (vec![], -1, 1, vec![]),
    ] {
        let result = variable(VariableScope::Global, VariableType::Int16, 51);
        let expression = assign(result, binary(constant(character), vec![], 109));
        let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
        let mut vm = EventVm::new(program);
        let mut host = Host::new();
        host.party_members = initial;

        vm.step(Tick::new(0), &mut host).unwrap();

        assert_eq!(vm.variable(VariableScope::Global, 51), expected);
        assert_eq!(host.party_members, expected_members);
    }

    let program = decode_program(&[5, 109, 127, 4]).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert!(error.message.contains("missing party-add operand"));
}

#[test]
fn angle_to_object_resolves_uid_and_truncates_position_deltas() {
    let result = variable(VariableScope::Global, VariableType::Int16, 43);
    let expression = assign(result, binary(constant(7), vec![], 96));
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let source = EntityId::new(42);
    let target = EntityId::new(77);
    let mut vm = EventVm::new(program).with_entity(source);
    let mut host = Host::new();
    host.object_uids.insert(7, target);
    host.object_positions.insert(source, [10.25, 0.0, 20.75]);
    host.object_positions.insert(target, [11.75, 0.0, 20.25]);

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 43), -64);

    let result = variable(VariableScope::Global, VariableType::Int16, 43);
    let expression = assign(result, binary(constant(7), vec![], 96));
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::InvalidExpression);
    assert!(error.message.contains("object context"));
}

#[test]
fn distance_to_object_uses_fractional_positions_before_truncating() {
    let result = variable(VariableScope::Global, VariableType::Int16, 47);
    let expression = assign(result, binary(constant(7), vec![], 97));
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let source = EntityId::new(42);
    let target = EntityId::new(77);
    let mut vm = EventVm::new(program).with_entity(source);
    let mut host = Host::new();
    host.object_uids.insert(7, target);
    host.object_positions.insert(source, [10.25, 0.0, 20.75]);
    host.object_positions.insert(target, [11.0, 0.0, 21.5]);

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 47), 1);

    let result = variable(VariableScope::Global, VariableType::Int16, 47);
    let expression = assign(result, binary(constant(7), vec![], 97));
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::InvalidExpression);
    assert!(error.message.contains("object context"));
}

#[test]
fn object_pointer_returns_the_legacy_uid_or_zero_for_an_absent_object() {
    let result = variable(VariableScope::Global, VariableType::Int16, 44);
    let expression = assign(result, vec![95, 7]);
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    host.object_uids.insert(7, EntityId::new(77));

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 44), 7);

    let result = variable(VariableScope::Global, VariableType::Int16, 44);
    let expression = assign(result, vec![95, 8]);
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 44), 0);
    assert!(decode_program(&[5, 95]).is_err());
}

#[test]
fn bgiid_reads_the_characters_navigation_triangle() {
    let result = variable(VariableScope::Global, VariableType::Int16, 45);
    let expression = assign(result, binary(constant(7), vec![], 112));
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 45), 23);

    let program = decode_program(&[5, 112, 127, 4]).unwrap();
    let mut vm = EventVm::new(program);
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::InvalidExpression);
    assert!(error.message.contains("BGI character operand"));
    assert!(decode_program(&[5, 112]).is_err());
}

#[test]
fn maxmp_reads_the_characters_host_owned_maximum() {
    let result = variable(VariableScope::Global, VariableType::Int16, 46);
    let expression = assign(result, binary(constant(7), vec![], 111));
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 46), 80);

    let program = decode_program(&[5, 111, 127, 4]).unwrap();
    let mut vm = EventVm::new(program);
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::InvalidExpression);
    assert!(error.message.contains("maximum-MP character operand"));
    assert!(decode_program(&[5, 111]).is_err());
}

#[test]
fn angle2_uses_the_reference_quadrants_and_atan_table() {
    for (delta_x, delta_z, expected) in [
        (0, 0, 0),
        (1, 0, -64),
        (0, 1, -128),
        (-1, 0, 64),
        (0, -1, 0),
        (2, 1, -72),
    ] {
        let result = variable(VariableScope::Global, VariableType::Int16, 30);
        let expression = assign(result, binary(constant(delta_x), constant(delta_z), 102));
        let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
        let mut vm = EventVm::new(program);
        let mut host = Host::new();

        vm.step(Tick::new(0), &mut host).unwrap();

        assert_eq!(
            vm.variable(VariableScope::Global, 30),
            expected,
            "delta_x={delta_x}, delta_z={delta_z}"
        );
    }
}

#[test]
fn angle_subtracts_fractional_actor_positions_before_fixed_point_conversion() {
    let result = variable(VariableScope::Global, VariableType::Int16, 31);
    let expression = assign(result, binary(constant(12), constant(20), 93));
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let actor = EntityId::new(42);
    let mut vm = EventVm::new(program).with_entity(actor);
    let mut host = Host::new();
    host.object_positions.insert(actor, [10.25, 0.0, 20.75]);

    vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(vm.variable(VariableScope::Global, 31), -64);

    let result = variable(VariableScope::Global, VariableType::Int16, 31);
    let expression = assign(result, binary(constant(12), constant(20), 93));
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::InvalidExpression);
    assert!(error.message.contains("object context"));
}

#[test]
fn pad0e_consumes_its_flag_and_yields_before_continuing() {
    let program = decode_program(&[14, 0, 4]).unwrap();
    assert_eq!(program.instructions[&0].kind, InstructionKind::Yield(14));
    assert_eq!(program.instructions[&0].length, 2);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.status, VmStatus::Running);
    assert_eq!(vm.pc(), 2);
    assert!(report.events.is_empty());
    assert_eq!(
        vm.step(Tick::new(1), &mut host).unwrap().status,
        VmStatus::Completed
    );
}

#[test]
fn pad10_consumes_its_flag_and_yields_through_the_default_path() {
    let program = decode_program(&[10, 0, 4]).unwrap();
    assert_eq!(program.instructions[&0].kind, InstructionKind::Yield(10));
    assert_eq!(program.instructions[&0].length, 2);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.status, VmStatus::Running);
    assert_eq!(vm.pc(), 2);
    assert!(report.events.is_empty());
    assert_eq!(
        vm.step(Tick::new(1), &mut host).unwrap().status,
        VmStatus::Completed
    );
    assert!(decode_program(&[10]).is_err());
}

#[test]
fn pad15_consumes_its_flag_and_yields_through_the_default_path() {
    let program = decode_program(&[15, 0, 4]).unwrap();
    assert_eq!(program.instructions[&0].kind, InstructionKind::Yield(15));
    assert_eq!(program.instructions[&0].length, 2);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.status, VmStatus::Running);
    assert_eq!(vm.pc(), 2);
    assert!(report.events.is_empty());
    assert_eq!(
        vm.step(Tick::new(1), &mut host).unwrap().status,
        VmStatus::Completed
    );
    assert!(decode_program(&[15]).is_err());
}

#[test]
fn shadowon_enables_the_current_actor_shadow() {
    let program = decode_program(&[127, 0, 4]).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();

    let report = vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(
        report.events,
        [HostEvent::SetShadowEnabled {
            actor: EntityId::new(42),
            enabled: true
        }]
    );

    let mut vm = EventVm::new(decode_program(&[127, 0, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("actor context"));
}

#[test]
fn shadowoff_disables_the_current_actor_shadow_and_preserves_its_opcode() {
    let program = decode_program(&[128, 0, 4]).unwrap();
    assert_eq!(
        program.instructions[&0].kind,
        InstructionKind::SetShadowEnabled {
            enabled: false,
            opcode: 128
        }
    );
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();

    let report = vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(
        report.events,
        [HostEvent::SetShadowEnabled {
            actor: EntityId::new(42),
            enabled: false
        }]
    );

    let mut vm = EventVm::new(decode_program(&[128, 0, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert_eq!(error.opcode, Some(128));
    assert!(error.message.contains("actor context"));
}

#[test]
fn pad17_yields_while_retaining_its_opcode() {
    let program = decode_program(&[23, 0, 4]).unwrap();
    assert_eq!(program.instructions[&0].kind, InstructionKind::Yield(23));
    assert_eq!(program.instructions[&0].length, 2);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    let report = vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(report.status, VmStatus::Running);
    assert_eq!(vm.pc(), 2);
    assert_eq!(
        vm.step(Tick::new(1), &mut host).unwrap().status,
        VmStatus::Completed
    );
}

#[test]
fn mesvalue_sets_the_script_keyed_text_variable() {
    let program = decode_program(&[102, 0, 3, 0xfb, 0xff, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 5);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    let report = vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(
        report.events,
        [HostEvent::SetTextVariable {
            script: 3,
            value: -5
        }]
    );
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn member_assignment_writes_the_lower_target_without_changing_members() {
    let result = variable(VariableScope::Global, VariableType::Int16, 31);
    let expression = [result, vec![41, 2], constant(0b101), vec![46]].concat();
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    host.slots.insert(0, EntityId::new(10));
    host.slots.insert(2, EntityId::new(12));
    host.object_members.insert(
        ObjectMemberRef {
            object: EntityId::new(10),
            member: 2,
        },
        0,
    );
    host.object_members.insert(
        ObjectMemberRef {
            object: EntityId::new(12),
            member: 2,
        },
        9,
    );

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 31), 4);
    assert_eq!(host.member_target, Some(EntityId::new(12)));
    assert!(host.member_writes.is_empty());
}

#[test]
fn empty_member_assignment_uses_the_reference_global_bit_fallback() {
    let expression = [vec![41, 4], constant(0), vec![46]].concat();
    let program = decode_program(&[evaluate(expression), vec![4]].concat()).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Global, 0), 0);
    assert_eq!(host.member_target, None);
    assert!(host.member_writes.is_empty());
}

#[test]
fn chrscale_preserves_renderer_scale_and_vertical_ratio() {
    let program = decode_program(&[159, 0, 7, 64, 32, 16, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 6);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    let report = vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(
        report.events,
        [HostEvent::SetObjectScale {
            actor: EntityId::new(7),
            scale: [4096, 2048, 1024],
            vertical_scale: 32
        }]
    );
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn vibrate_sets_the_signed_frame_rate() {
    let program = decode_program(&[250, 0, 0x00, 0xff, 4]).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    let report = vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(
        report.events,
        [HostEvent::SetVibrationFrameRate { rate: -256 }]
    );
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn req_emits_an_async_script_request_with_level_target_and_tag() {
    let program = decode_program(&[16, 0, 2, 7, 11, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 5);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    let report = vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(
        report.events,
        [HostEvent::RequestScriptAsync {
            level: 2,
            target: EntityId::new(7),
            tag: 11
        }]
    );
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn reqsw_waits_for_target_acceptance_and_requests_once() {
    let program = decode_program(&[18, 0, 2, 7, 11, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 5);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.status, VmStatus::Waiting);
    assert!(report.events.is_empty());
    assert_eq!(vm.pc(), 0);
    assert_eq!(
        vm.wait_state(),
        Some(WaitState::ScriptReadiness {
            level: 2,
            target: EntityId::new(7),
            tag: 11
        })
    );
    assert!(vm.step(Tick::new(1), &mut host).unwrap().events.is_empty());

    host.script_request_acceptable = true;
    let report = vm.step(Tick::new(2), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::RequestScriptAsync {
            level: 2,
            target: EntityId::new(7),
            tag: 11
        }]
    );
    assert_eq!(report.status, VmStatus::Completed);
    assert!(vm.step(Tick::new(3), &mut host).unwrap().events.is_empty());

    let mut bytes = vec![18, 3];
    append_expression(&mut bytes, constant(3));
    append_expression(&mut bytes, constant(8));
    bytes.extend_from_slice(&[12, 4]);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::RequestScriptAsync {
            level: 3,
            target: EntityId::new(8),
            tag: 12
        }]
    );

    assert!(decode_program(&[18, 0, 2, 7]).is_err());
}

#[test]
fn bgvdefine_emits_the_complete_viewport_rectangle() {
    let bytes = [201, 0, 2, 10, 0, 0xec, 0xff, 30, 0, 40, 0, 4];
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    let report = vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(
        report.events,
        [HostEvent::DefineTileViewport {
            viewport: 2,
            x: 10,
            y: -20,
            width: 30,
            height: 40
        }]
    );
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn direct_post_increment_stores_the_new_value_and_returns_the_old_value() {
    let target = variable(VariableScope::Map, VariableType::Int16, 9);
    let result = variable(VariableScope::Global, VariableType::Int16, 32);
    let bytes = [
        evaluate(assign(target.clone(), constant(4))),
        evaluate(assign(result, [target, vec![4]].concat())),
        vec![4],
    ]
    .concat();
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(vm.variable(VariableScope::Map, 9), 5);
    assert_eq!(vm.variable(VariableScope::Global, 32), 4);
}

#[test]
fn popshow_emits_scene_visibility_restoration() {
    let program = decode_program(&[214, 0, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 2);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    let report = vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(report.events, [HostEvent::RestoreAllVisibility]);
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn timercontrol_emits_running_and_paused_states() {
    for (operand, running) in [(0, false), (1, true)] {
        let program = decode_program(&[125, 0, operand, 4]).unwrap();
        let mut vm = EventVm::new(program);
        let mut host = Host::new();

        let report = vm.step(Tick::new(0), &mut host).unwrap();

        assert_eq!(report.events, [HostEvent::SetTimerRunning { running }]);
        assert_eq!(vm.status(), VmStatus::Completed);
    }
}

#[test]
fn reply_emits_a_caller_targeted_async_request() {
    let program = decode_program(&[22, 0, 2, 11, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 4);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    let report = vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(
        report.events,
        [HostEvent::RequestCallerAsync { level: 2, tag: 11 }]
    );
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn replysw_waits_for_caller_acceptance_and_requests_once() {
    let program = decode_program(&[24, 0, 2, 11, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 4);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.status, VmStatus::Waiting);
    assert!(report.events.is_empty());
    assert_eq!(vm.pc(), 0);
    assert_eq!(
        vm.wait_state(),
        Some(WaitState::CallerRequest { level: 2, tag: 11 })
    );
    assert!(vm.step(Tick::new(1), &mut host).unwrap().events.is_empty());

    host.caller_request_acceptable = true;
    let report = vm.step(Tick::new(2), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::RequestCallerAsync { level: 2, tag: 11 }]
    );
    assert_eq!(report.status, VmStatus::Completed);
    assert!(vm.step(Tick::new(3), &mut host).unwrap().events.is_empty());

    let mut bytes = vec![24, 1];
    append_expression(&mut bytes, constant(3));
    bytes.extend_from_slice(&[12, 4]);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::RequestCallerAsync { level: 3, tag: 12 }]
    );

    assert!(decode_program(&[24, 0, 2]).is_err());
}

#[test]
fn mjpos_requests_exit_position_calculation() {
    let program = decode_program(&[164, 0, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 2);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    let report = vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(report.events, [HostEvent::CalculateExitPosition]);
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn itemadd_decodes_item_and_count_operands() {
    let program = decode_program(&[72, 0, 0x34, 0x12, 5, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 5);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::AddItem {
            item: 0x1234,
            count: 5,
        }]
    );
    assert!(vm.traces().any(|trace| trace.opcode == Some(72)));
    assert_eq!(report.status, VmStatus::Completed);

    let mut bytes = vec![72, 3];
    append_expression(&mut bytes, constant(-2));
    append_expression(&mut bytes, constant(300));
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::AddItem {
            item: -2,
            count: 300,
        }]
    );

    let truncated = decode_program(&[72, 0, 1, 0]).unwrap_err();
    assert_eq!(truncated.kind, VmErrorKind::Decode);
    assert!(truncated.message.contains("opcode Some(72)"));
}

#[test]
fn itemdelete_decodes_item_and_count_operands() {
    let program = decode_program(&[73, 0, 0xfe, 0xff, 2, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 5);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::RemoveItem { item: -2, count: 2 }]
    );
    assert!(vm.traces().any(|trace| trace.opcode == Some(73)));
    assert_eq!(report.status, VmStatus::Completed);

    let mut bytes = vec![73, 3];
    append_expression(&mut bytes, constant(0x1234));
    append_expression(&mut bytes, constant(300));
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::RemoveItem {
            item: 0x1234,
            count: 300,
        }]
    );

    let truncated = decode_program(&[73, 0, 1, 0]).unwrap_err();
    assert_eq!(truncated.kind, VmErrorKind::Decode);
    assert!(truncated.message.contains("opcode Some(73)"));
}

#[test]
fn gil_commands_decode_unsigned_amounts_and_leave_clamping_to_the_host() {
    // `getv3u`: bit 1 clear reads an unsigned 24-bit immediate (no sign extension).
    let mut host = Host::new();
    for (opcode, expected) in [
        (0xce_u8, HostEvent::AddGil { amount: 1_000_000 }),
        (0xcf_u8, HostEvent::RemoveGil { amount: 1_000_000 }),
    ] {
        let program = decode_program(&[opcode, 0, 0x40, 0x42, 0x0f, 4]).unwrap();
        assert_eq!(program.instructions[&0].length, 5);
        let mut vm = EventVm::new(program);
        let report = vm.step(Tick::new(0), &mut host).unwrap();
        assert_eq!(report.events, [expected]);
        assert!(
            vm.traces()
                .any(|trace| trace.opcode == Some(u32::from(opcode)))
        );
        assert_eq!(report.status, VmStatus::Completed);
    }
    let mut vm = EventVm::new(decode_program(&[0xce, 0, 0xff, 0xff, 0xff, 4]).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.events, [HostEvent::AddGil { amount: 0xff_ffff }]);

    // Expression operand: the value is reinterpreted as unsigned like `(UInt32)getv()`.
    let mut bytes = vec![0xcf, 1];
    append_expression(&mut bytes, constant(-1));
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.events, [HostEvent::RemoveGil { amount: u32::MAX }]);

    for truncated in [&[0xce_u8, 0, 1, 2][..], &[0xcf, 1, 0xff][..]] {
        let error = decode_program(truncated).unwrap_err();
        assert_eq!(error.kind, VmErrorKind::Decode);
        assert!(error.message.contains("opcode Some(20"));
    }
}

#[test]
fn party_member_commands_decode_character_and_value_operands() {
    let mut host = Host::new();
    // Immediate operands: HP/MP use a signed 16-bit value, AP commands a byte ability index.
    let cases: [(u8, &[u8], HostEvent); 4] = [
        (
            0xf1,
            &[3, 0x0f, 0x27],
            HostEvent::SetPartyHp {
                character: 3,
                hp: 9999,
            },
        ),
        (
            0xf2,
            &[1, 0xe7, 0x03],
            HostEvent::SetPartyMp {
                character: 1,
                mp: 999,
            },
        ),
        (
            0xf3,
            &[2, 17],
            HostEvent::ClearAbilityAp {
                character: 2,
                ability: 17,
            },
        ),
        (
            0xf4,
            &[8, 0xff],
            HostEvent::MasterAbility {
                character: 8,
                ability: 255,
            },
        ),
    ];
    for (opcode, operands, expected) in cases {
        let mut bytes = vec![opcode, 0];
        bytes.extend_from_slice(operands);
        bytes.push(4);
        let program = decode_program(&bytes).unwrap();
        assert_eq!(program.instructions[&0].length, 2 + operands.len() as u32);
        let mut vm = EventVm::new(program);
        let report = vm.step(Tick::new(0), &mut host).unwrap();
        assert_eq!(report.events, [expected]);
        assert!(
            vm.traces()
                .any(|trace| trace.opcode == Some(u32::from(opcode)))
        );
        assert_eq!(report.status, VmStatus::Completed);
    }

    // Both operands as expressions; negative HP stays raw for the host.
    let mut bytes = vec![0xf1, 3];
    append_expression(&mut bytes, constant(5));
    append_expression(&mut bytes, constant(-7));
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetPartyHp {
            character: 5,
            hp: -7,
        }]
    );

    // Mixed: expression character, immediate ability.
    let mut bytes = vec![0xf4, 1];
    append_expression(&mut bytes, constant(4));
    bytes.push(9);
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::MasterAbility {
            character: 4,
            ability: 9,
        }]
    );

    // Truncated operands report the opcode.
    for truncated in [
        &[0xf1_u8, 0, 3, 0x0f][..],
        &[0xf3, 0, 2][..],
        &[0xf2, 1][..],
    ] {
        let error = decode_program(truncated).unwrap_err();
        assert_eq!(error.kind, VmErrorKind::Decode);
        assert!(
            error.message.contains("opcode Some(24"),
            "{}",
            error.message
        );
    }
}

#[test]
fn move_walks_the_current_actor_and_repeats_until_movement_finishes() {
    let program = decode_program(&[35, 0, 0xd0, 0x02, 0x30, 0xfd, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 6);
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();

    for tick in 0..2 {
        host.actor_movement_active = tick == 1;
        let report = vm.step(Tick::new(tick), &mut host).unwrap();
        assert_eq!(report.status, VmStatus::Waiting);
        assert_eq!(vm.pc(), 0);
        assert_eq!(vm.wait_state(), Some(WaitState::ActorMovement { next: 6 }));
        assert_eq!(
            report.events,
            [HostEvent::WalkActor {
                actor: EntityId::new(42),
                destination: [720, -720],
            }]
        );
    }

    host.actor_movement_active = false;
    let report = vm.step(Tick::new(2), &mut host).unwrap();
    assert!(report.events.is_empty());
    assert_eq!(report.status, VmStatus::Completed);
    assert_eq!(vm.pc(), 6);

    let mut bytes = vec![35, 3];
    append_expression(&mut bytes, constant(-300));
    append_expression(&mut bytes, constant(700));
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap()).with_entity(EntityId::new(42));
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::WalkActor {
            actor: EntityId::new(42),
            destination: [-300, 700],
        }]
    );

    let mut vm = EventVm::new(decode_program(&[35, 0, 1, 0, 2, 0, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("actor context"));

    let truncated = decode_program(&[35, 0, 1]).unwrap_err();
    assert_eq!(truncated.kind, VmErrorKind::Decode);
    assert!(truncated.message.contains("opcode Some(35)"));
}

#[test]
fn movj_yields_and_repeats_until_exit_movement_finishes() {
    let program = decode_program(&[160, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 1);
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    host.exit_movement_active = true;

    for tick in 0..2 {
        let report = vm.step(Tick::new(tick), &mut host).unwrap();
        assert_eq!(report.status, VmStatus::Running);
        assert_eq!(vm.pc(), 0);
        assert_eq!(
            report.events,
            [HostEvent::MoveActorToExit {
                actor: EntityId::new(42)
            }]
        );
    }

    host.exit_movement_active = false;
    let report = vm.step(Tick::new(2), &mut host).unwrap();
    assert_eq!(report.status, VmStatus::Running);
    assert_eq!(vm.pc(), 1);
    assert_eq!(
        report.events,
        [HostEvent::MoveActorToExit {
            actor: EntityId::new(42)
        }]
    );
    assert!(vm.step(Tick::new(3), &mut host).unwrap().events.is_empty());
    assert_eq!(vm.status(), VmStatus::Completed);

    let mut vm = EventVm::new(decode_program(&[160, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("actor context"));
}

#[test]
fn getscreen_requests_screen_position_for_the_selected_object() {
    let program = decode_program(&[169, 0, 7, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 3);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::CalculateScreenPosition { object: 7 }]
    );
    assert_eq!(report.status, VmStatus::Completed);

    let mut bytes = vec![169, 1];
    append_expression(&mut bytes, constant(-1));
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::CalculateScreenPosition { object: -1 }]
    );
    assert!(decode_program(&[169, 0]).is_err());
}

#[test]
fn replyew_requests_once_and_waits_for_script_completion() {
    let program = decode_program(&[20, 0, 2, 11, 4]).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();

    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::RequestCallerSync { level: 2, tag: 11 }]
    );
    assert_eq!(report.status, VmStatus::Waiting);
    assert_eq!(vm.wait_state(), Some(WaitState::ScriptRequest));

    assert!(vm.step(Tick::new(1), &mut host).unwrap().events.is_empty());
    assert_eq!(vm.status(), VmStatus::Waiting);
    host.script_request_complete = true;
    assert_eq!(
        vm.step(Tick::new(2), &mut host).unwrap().status,
        VmStatus::Completed
    );
}

#[test]
fn bglattach_preserves_offsets_filter_mode_and_color() {
    let bytes = [146, 0, 2, 0xf6, 0xff, 20, 255, 1, 2, 3, 4];
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    let report = vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(
        report.events,
        [HostEvent::AttachTile {
            overlay: 2,
            x: -10,
            y: 20,
            filter_mode: -1,
            color: [1, 2, 3]
        }]
    );
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn bglcolor_decodes_immediate_and_expression_components() {
    let program = decode_program(&[89, 0, 2, 255, 128, 1, 4]).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetTileShadeColor {
            overlay: 2,
            color: [255, 128, 1]
        }]
    );

    let mut bytes = vec![89, 10, 3];
    append_expression(&mut bytes, constant(128));
    bytes.push(255);
    append_expression(&mut bytes, constant(-1));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program);
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetTileShadeColor {
            overlay: 3,
            color: [128, 255, 255]
        }]
    );
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn sps_decodes_byte_ids_and_signed_wide_arguments() {
    let bytes = [179, 0, 2, 3, 0xf6, 0xff, 20, 0, 0xd2, 0xff, 4];
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetSpsParameters {
            object: 2,
            operation: 3,
            arguments: [-10, 20, -46]
        }]
    );

    let mut bytes = vec![179, 31];
    for value in [1, 4, -100, 200, -300] {
        append_expression(&mut bytes, constant(value));
    }
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program);
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetSpsParameters {
            object: 1,
            operation: 4,
            arguments: [-100, 200, -300]
        }]
    );
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn pushhide_emits_visibility_preservation_command() {
    let program = decode_program(&[213, 0, 4]).unwrap();
    assert_eq!(
        program.instructions[&0].kind,
        InstructionKind::PreserveAndHideAll
    );
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.events, [HostEvent::PreserveAndHideAll]);
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn printquad_consumes_its_flag_and_continues_without_side_effects() {
    let program = decode_program(&[120, 0, 4]).unwrap();
    assert_eq!(program.instructions[&0].kind, InstructionKind::NoOp(120));
    assert_eq!(program.instructions[&0].length, 2);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.status, VmStatus::Completed);
    assert_eq!(report.operations, 2);
    assert!(report.events.is_empty());
}

#[test]
fn globalclear_consumes_its_flag_and_continues_without_side_effects() {
    let program = decode_program(&[209, 0, 4]).unwrap();
    assert_eq!(program.instructions[&0].kind, InstructionKind::NoOp(209));
    assert_eq!(program.instructions[&0].length, 2);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.status, VmStatus::Completed);
    assert_eq!(report.operations, 2);
    assert!(report.events.is_empty());
}

#[test]
fn mesb_emits_battle_message_text() {
    let program = decode_program(&[208, 0, 2, 0, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 4);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.events, [HostEvent::ShowBattleMessage { text: 2 }]);
    assert_eq!(vm.status(), VmStatus::Completed);

    let mut bytes = vec![208, 1];
    append_expression(&mut bytes, constant(300));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program);
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.events, [HostEvent::ShowBattleMessage { text: 300 }]);
}

#[test]
fn debug_save_and_load_follow_the_reference_noop_path() {
    for opcode in [210, 211] {
        let program = decode_program(&[opcode, 0, 4]).unwrap();
        assert_eq!(
            program.instructions[&0].kind,
            InstructionKind::NoOp(u32::from(opcode))
        );
        assert_eq!(program.instructions[&0].length, 2);
        let mut vm = EventVm::new(program);
        let mut host = Host::new();
        let report = vm.step(Tick::new(0), &mut host).unwrap();
        assert_eq!(report.status, VmStatus::Completed);
        assert!(report.events.is_empty());
    }
}

#[test]
fn bsstart_consumes_two_operands_without_side_effects() {
    let program = decode_program(&[255, 0, 0, 16, 32, 4]).unwrap();
    assert_eq!(program.instructions[&0].kind, InstructionKind::NoOp(256));
    assert_eq!(program.instructions[&0].length, 5);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.status, VmStatus::Completed);
    assert!(report.events.is_empty());

    let mut bytes = vec![255, 0, 3];
    append_expression(&mut bytes, constant(7));
    append_expression(&mut bytes, constant(9));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program);
    assert_eq!(
        vm.step(Tick::new(0), &mut host).unwrap().status,
        VmStatus::Completed
    );
    assert!(decode_program(&[255, 0, 1]).is_err());
}

#[test]
fn bsfloor_sets_simulation_floor_with_extended_opcode() {
    let program = decode_program(&[255, 4, 0, 2, 3, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 5);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetSimulationFloor {
            simulation: 2,
            floor: 3
        }]
    );

    let mut bytes = vec![255, 4, 3];
    append_expression(&mut bytes, constant(1));
    append_expression(&mut bytes, constant(4));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program);
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetSimulationFloor {
            simulation: 1,
            floor: 4
        }]
    );
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn bsrate_sets_signed_simulation_frame_rate_with_extended_opcode() {
    let program = decode_program(&[255, 5, 0, 2, 0xfe, 0xff, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 6);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetSimulationFrameRate {
            simulation: 2,
            frame_rate: -2
        }]
    );

    let mut bytes = vec![255, 5, 3];
    append_expression(&mut bytes, constant(1));
    append_expression(&mut bytes, constant(30));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program);
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetSimulationFrameRate {
            simulation: 1,
            frame_rate: 30
        }]
    );
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn tile_animation_flags_decode_and_execute() {
    let program = decode_program(&[100, 0, 2, 0x60, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 4);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetTileAnimationFlags {
            animation: 2,
            flags: 96,
        }]
    );
    assert!(vm.traces().any(|trace| trace.opcode == Some(100)));
    assert_eq!(report.status, VmStatus::Completed);

    let mut bytes = vec![100, 3];
    append_expression(&mut bytes, constant(1));
    append_expression(&mut bytes, constant(0x20));
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetTileAnimationFlags {
            animation: 1,
            flags: 32,
        }]
    );

    let truncated = decode_program(&[100, 0, 2]).unwrap_err();
    assert_eq!(truncated.kind, VmErrorKind::Decode);
    assert!(truncated.message.contains("opcode Some(100)"));
}

#[test]
fn tile_animation_range_decodes_playback_bounds() {
    let program = decode_program(&[101, 0, 2, 3, 9, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 5);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetTileAnimationRange {
            animation: 2,
            first_frame: 3,
            last_frame: 9,
        }]
    );
    assert!(vm.traces().any(|trace| trace.opcode == Some(101)));
    assert_eq!(report.status, VmStatus::Completed);

    let mut bytes = vec![101, 7];
    for value in [1, 255, 0] {
        append_expression(&mut bytes, constant(value));
    }
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetTileAnimationRange {
            animation: 1,
            first_frame: 255,
            last_frame: 0,
        }]
    );

    let truncated = decode_program(&[101, 0, 2, 3]).unwrap_err();
    assert_eq!(truncated.kind, VmErrorKind::Decode);
    assert!(truncated.message.contains("opcode Some(101)"));
}

#[test]
fn incfrog_emits_a_no_operand_host_command() {
    let program = decode_program(&[224, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 1);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.events, [HostEvent::IncrementFrogCount]);
    assert!(vm.traces().any(|trace| trace.opcode == Some(224)));
    assert_eq!(report.status, VmStatus::Completed);
}

#[test]
fn tile_block_commands_decode_flag_and_signed_word_operands() {
    let mut host = Host::new();

    let program = decode_program(&[91, 0, 3, 2, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 4);
    let mut vm = EventVm::new(program);
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetTileBlockActive {
            block: 3,
            active: true,
        }]
    );
    assert!(vm.traces().any(|trace| trace.opcode == Some(91)));
    assert_eq!(report.status, VmStatus::Completed);

    let mut bytes = vec![92, 0b1000, 2, 1];
    bytes.extend_from_slice(&(-3i16).to_le_bytes());
    append_expression(&mut bytes, constant(-40));
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetTileLoop {
            block: 2,
            enabled: 1,
            x_speed: -3,
            y_speed: -40,
        }]
    );

    let mut bytes = vec![93, 0, 1, 0];
    bytes.extend_from_slice(&256i16.to_le_bytes());
    bytes.extend_from_slice(&(-256i16).to_le_bytes());
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetTileParallax {
            block: 1,
            enabled: 0,
            x_speed: 256,
            y_speed: -256,
        }]
    );

    let mut bytes = vec![94, 0, 4];
    bytes.extend_from_slice(&(-20i16).to_le_bytes());
    bytes.extend_from_slice(&30i16.to_le_bytes());
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetTileOrigin {
            block: 4,
            x: -20,
            y: 30,
        }]
    );

    assert_eq!(
        decode_program(&[92, 0, 2, 1, 0]).unwrap_err().kind,
        VmErrorKind::Decode
    );
    assert_eq!(
        decode_program(&[94, 0, 4, 1]).unwrap_err().kind,
        VmErrorKind::Decode
    );
}

#[test]
fn tile_animation_run_active_and_reset_commands_emit_raw_operands() {
    let mut host = Host::new();

    let program = decode_program(&[95, 0, 1, 7, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 4);
    let mut vm = EventVm::new(program);
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::RunTileAnimation {
            animation: 1,
            start_frame: 7,
        }]
    );
    assert!(vm.traces().any(|trace| trace.opcode == Some(95)));

    let mut bytes = vec![96, 3];
    append_expression(&mut bytes, constant(2));
    append_expression(&mut bytes, constant(0));
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetTileAnimationActive {
            animation: 2,
            active: false,
        }]
    );

    let program = decode_program(&[202, 0, 5, 9, 4]).unwrap();
    let mut vm = EventVm::new(program);
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::ResetTileAnimation {
            animation: 5,
            show_first_frame: 9,
        }]
    );
    assert_eq!(report.status, VmStatus::Completed);

    let truncated = decode_program(&[95, 0, 1]).unwrap_err();
    assert_eq!(truncated.kind, VmErrorKind::Decode);
    assert!(truncated.message.contains("opcode Some(95)"));
}

#[test]
fn bgarate_sets_tile_animation_speed_from_signed_word() {
    let program = decode_program(&[97, 0, 2, 0x00, 0x01, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 5);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetTileAnimationRate {
            animation: 2,
            frame_rate: 256,
        }]
    );
    assert!(vm.traces().any(|trace| trace.opcode == Some(97)));
    assert_eq!(report.status, VmStatus::Completed);

    let mut bytes = vec![97, 3];
    append_expression(&mut bytes, constant(1));
    append_expression(&mut bytes, constant(-128));
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetTileAnimationRate {
            animation: 1,
            frame_rate: -128,
        }]
    );

    let truncated = decode_program(&[97, 0, 2, 0x00]).unwrap_err();
    assert_eq!(truncated.kind, VmErrorKind::Decode);
    assert!(truncated.message.contains("opcode Some(97)"));
}

#[test]
fn bgawait_sets_tile_animation_frame_wait() {
    let program = decode_program(&[99, 0, 2, 255, 30, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 5);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetTileAnimationFrameWait {
            animation: 2,
            frame: 255,
            wait: 30,
        }]
    );
    assert!(vm.traces().any(|trace| trace.opcode == Some(99)));
    assert_eq!(report.status, VmStatus::Completed);

    let mut bytes = vec![99, 7];
    for value in [3, 12, 40] {
        append_expression(&mut bytes, constant(value));
    }
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetTileAnimationFrameWait {
            animation: 3,
            frame: 12,
            wait: 40,
        }]
    );

    let truncated = decode_program(&[99, 0, 2, 1]).unwrap_err();
    assert_eq!(truncated.kind, VmErrorKind::Decode);
    assert!(truncated.message.contains("opcode Some(99)"));
}

#[test]
fn bgaframe_sets_tile_animation_frame_without_clamping() {
    let program = decode_program(&[231, 0, 2, 255, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 4);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetTileAnimationFrame {
            animation: 2,
            frame: 255
        }]
    );

    let mut bytes = vec![231, 3];
    append_expression(&mut bytes, constant(1));
    append_expression(&mut bytes, constant(-1));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program);
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetTileAnimationFrame {
            animation: 1,
            frame: -1
        }]
    );
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn party_menu_emits_once_and_waits_for_host_closure() {
    let program = decode_program(&[178, 0, 2, 5, 0, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 5);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();

    let report = vm.step(Tick::new(0), &mut host).unwrap();

    assert_eq!(
        report.events,
        [HostEvent::OpenPartyMenu {
            minimum_party_size: 2,
            fixed_members: 5
        }]
    );
    assert_eq!(report.status, VmStatus::Waiting);
    assert_eq!(vm.wait_state(), Some(WaitState::PartyMenu));
    assert!(host.party_menu_open);
    assert!(vm.step(Tick::new(1), &mut host).unwrap().events.is_empty());
    assert_eq!(vm.status(), VmStatus::Waiting);

    host.party_menu_open = false;
    assert_eq!(
        vm.step(Tick::new(2), &mut host).unwrap().status,
        VmStatus::Completed
    );

    let mut bytes = vec![178, 3];
    append_expression(&mut bytes, constant(1));
    append_expression(&mut bytes, constant(-2));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program);
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::OpenPartyMenu {
            minimum_party_size: 1,
            fixed_members: -2
        }]
    );
}

#[test]
fn join_emits_complete_character_initialization() {
    let program = decode_program(&[254, 0, 2, 1, 3, 255, 4, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 7);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::InitializeCharacter {
            character: 2,
            enable_leveling: true,
            equipment_set: 3,
            category: 255,
            menu_preset: 4
        }]
    );
    assert_eq!(vm.status(), VmStatus::Completed);

    let mut bytes = vec![254, 31];
    for value in [2, 0, 3, 4, 5] {
        append_expression(&mut bytes, constant(value));
    }
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program);
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::InitializeCharacter {
            character: 2,
            enable_leveling: false,
            equipment_set: 3,
            category: 4,
            menu_preset: 5
        }]
    );
}

#[test]
fn texstop_emits_object_and_animation_identities() {
    let program = decode_program(&[194, 0, 2, 3, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 4);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::StopTextureAnimation {
            object: 2,
            animation: 3
        }]
    );
    assert_eq!(vm.status(), VmStatus::Completed);

    let mut bytes = vec![194, 3];
    append_expression(&mut bytes, constant(4));
    append_expression(&mut bytes, constant(5));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program);
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::StopTextureAnimation {
            object: 4,
            animation: 5
        }]
    );
}

#[test]
fn btlset_emits_unsigned_code_and_signed_value() {
    let program = decode_program(&[74, 0, 2, 0xfe, 0xff, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 5);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::RunBattleCode { code: 2, value: -2 }]
    );
    assert_eq!(vm.status(), VmStatus::Completed);

    let mut bytes = vec![74, 3];
    append_expression(&mut bytes, constant(-1));
    append_expression(&mut bytes, constant(300));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program);
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::RunBattleCode {
            code: u32::MAX,
            value: 300
        }]
    );
}

#[test]
fn songflag_sets_battle_music_continuation_from_a_boolean_operand() {
    let program = decode_program(&[27, 0, 0, 27, 0, 1, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 3);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [
            HostEvent::SetBattleMusicContinuation { enabled: false },
            HostEvent::SetBattleMusicContinuation { enabled: true }
        ]
    );
    assert_eq!(report.status, VmStatus::Completed);

    let mut bytes = vec![27, 1];
    append_expression(&mut bytes, constant(-1));
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetBattleMusicContinuation { enabled: true }]
    );
    assert!(decode_program(&[27, 0]).is_err());
}

#[test]
fn chrfx_dispatches_model_code_and_signed_arguments_for_the_current_object() {
    let program = decode_program(&[136, 0, 16, 0xcc, 0x07, 4, 0, 0x9d, 0x03, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 9);
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::RunModelCode {
            object: EntityId::new(42),
            code: 16,
            arguments: [1996, 4, 925]
        }]
    );
    assert_eq!(report.status, VmStatus::Completed);

    let mut bytes = vec![136, 15];
    append_expression(&mut bytes, constant(-1));
    append_expression(&mut bytes, constant(-2));
    append_expression(&mut bytes, constant(300));
    append_expression(&mut bytes, constant(-32769));
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap()).with_entity(EntityId::new(42));
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::RunModelCode {
            object: EntityId::new(42),
            code: -1,
            arguments: [-2, 300, 32767]
        }]
    );

    let mut vm =
        EventVm::new(decode_program(&[136, 0, 16, 0xcc, 0x07, 4, 0, 0x9d, 0x03, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("object context"));
    assert!(decode_program(&[136, 0, 16, 0]).is_err());
}

#[test]
fn dashon_enables_running() {
    let program = decode_program(&[240, 0, 4]).unwrap();
    assert_eq!(
        program.instructions[&0].kind,
        InstructionKind::SetRunEnabled {
            enabled: true,
            opcode: 240
        }
    );
    assert_eq!(program.instructions[&0].length, 2);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.events, [HostEvent::SetRunEnabled { enabled: true }]);
    assert_eq!(vm.status(), VmStatus::Completed);
}

#[test]
fn hereon_sets_the_raw_icon_display_mode() {
    let program = decode_program(&[239, 0, 3, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 3);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.events, [HostEvent::SetHereIcon { mode: 3 }]);
    assert!(vm.traces().any(|trace| trace.opcode == Some(239)));
    assert_eq!(report.status, VmStatus::Completed);

    let mut bytes = vec![239, 1];
    append_expression(&mut bytes, constant(0));
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.events, [HostEvent::SetHereIcon { mode: 0 }]);

    let truncated = decode_program(&[239]).unwrap_err();
    assert_eq!(truncated.kind, VmErrorKind::Decode);
    assert!(truncated.message.contains("opcode Some(239)"));
}

#[test]
fn fullmember_remaps_legacy_party_availability_bits() {
    let program = decode_program(&[180, 0, 0xff, 1, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 4);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetPartyAvailability { mask: 0x8ff }]
    );
    assert_eq!(vm.status(), VmStatus::Completed);

    let mut bytes = vec![180, 1];
    append_expression(&mut bytes, constant(2));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program);
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(report.events, [HostEvent::SetPartyAvailability { mask: 2 }]);
}

#[test]
fn bglmove_emits_signed_tile_offsets() {
    let program = decode_program(&[90, 0, 2, 0xf6, 0xff, 20, 0, 0xe2, 0xff, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 9);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::MoveTile {
            overlay: 2,
            offset: [-10, 20, -30]
        }]
    );
    assert_eq!(vm.status(), VmStatus::Completed);

    let mut bytes = vec![90, 15];
    for value in [1, 2, 3, 4] {
        append_expression(&mut bytes, constant(value));
    }
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program);
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::MoveTile {
            overlay: 1,
            offset: [2, 3, 4]
        }]
    );
}

#[test]
fn bglscrolloffset_emits_tile_scroll_speed_offset_and_axis() {
    let program = decode_program(&[228, 0, 2, 1, 0xf6, 0xff, 20, 0, 1, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 9);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetTileScrollWithOffset {
            overlay: 2,
            enabled: 1,
            speed: -10,
            offset: 20,
            is_x_offset: 1
        }]
    );
    assert_eq!(report.status, VmStatus::Completed);

    let mut bytes = vec![228, 31];
    for value in [3, 2, 300, -20, 4] {
        append_expression(&mut bytes, constant(value));
    }
    bytes.push(4);
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetTileScrollWithOffset {
            overlay: 3,
            enabled: 2,
            speed: 300,
            offset: -20,
            is_x_offset: 4
        }]
    );
    assert!(decode_program(&[228, 0, 2, 1, 0xf6]).is_err());
}

#[test]
fn bgvport_emits_camera_and_signed_bounds() {
    let program = decode_program(&[30, 0, 2, 0xf6, 0xff, 20, 0, 0xe2, 0xff, 40, 0, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 11);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetCameraBounds {
            camera: 2,
            bounds: [-10, 20, -30, 40]
        }]
    );
    assert_eq!(vm.status(), VmStatus::Completed);

    let mut bytes = vec![30, 31];
    for value in [1, 2, 3, 4, 5] {
        append_expression(&mut bytes, constant(value));
    }
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program);
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetCameraBounds {
            camera: 1,
            bounds: [2, 3, 4, 5]
        }]
    );
}

#[test]
fn setrow_emits_character_and_wrapped_row() {
    let program = decode_program(&[98, 0, 2, 255, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 4);
    let mut vm = EventVm::new(program);
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetCharacterRow {
            character: 2,
            row: 255
        }]
    );
    assert_eq!(vm.status(), VmStatus::Completed);

    let mut bytes = vec![98, 3];
    append_expression(&mut bytes, constant(3));
    append_expression(&mut bytes, constant(-1));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program);
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetCharacterRow {
            character: 3,
            row: 255
        }]
    );
}

#[test]
fn rotxz_sets_wrapped_actor_pitch_and_roll() {
    let program = decode_program(&[55, 0, 1, 2, 4]).unwrap();
    assert_eq!(program.instructions[&0].length, 4);
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let mut host = Host::new();
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetActorPitchRoll {
            actor: EntityId::new(42),
            pitch: 16,
            roll: 32
        }]
    );

    let mut bytes = vec![55, 3];
    append_expression(&mut bytes, constant(-1));
    append_expression(&mut bytes, constant(2));
    bytes.push(4);
    let program = decode_program(&bytes).unwrap();
    let mut vm = EventVm::new(program).with_entity(EntityId::new(42));
    let report = vm.step(Tick::new(0), &mut host).unwrap();
    assert_eq!(
        report.events,
        [HostEvent::SetActorPitchRoll {
            actor: EntityId::new(42),
            pitch: -16,
            roll: 32
        }]
    );

    let mut vm = EventVm::new(decode_program(&[55, 0, 1, 2, 4]).unwrap());
    let error = vm.step(Tick::new(0), &mut host).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Host);
    assert!(error.message.contains("actor context"));
}

#[test]
fn extended_prefixes_preserve_the_accumulated_opcode_in_errors() {
    for (bytes, opcode) in [(&[255, 1, 0][..], 257), (&[255, 255, 1, 0][..], 513)] {
        let error = decode_program(bytes).unwrap_err();
        assert_eq!(error.kind, VmErrorKind::UnsupportedOpcode);
        assert_eq!(error.offset, 0);
        assert_eq!(error.opcode, Some(opcode));
    }

    let truncated = decode_program(&[255]).unwrap_err();
    assert_eq!(truncated.kind, VmErrorKind::Decode);
    assert_eq!(truncated.offset, 1);
}
