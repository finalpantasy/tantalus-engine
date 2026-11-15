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
    fail: bool,
}

impl Host {
    fn new() -> Self {
        Self {
            windows: BTreeMap::new(),
            movements: Vec::new(),
            fail: false,
        }
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
                HostEvent::AimAtCamera => {}
                HostEvent::DefinePlayerControl { .. } => {}
                HostEvent::SetControlDirection { .. } => {}
            }
        }
        Ok(())
    }

    fn window_is_open(&self, window: u8) -> bool {
        self.windows.get(&window).copied().unwrap_or(false)
    }

    fn read_object_member(&mut self, reference: ObjectMemberRef) -> Result<i32, EngineError> {
        Ok(((reference.object.get() as i32) << 8) | reference.member as i32)
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
    let unsupported = decode_program(&[0xff]).unwrap_err();
    assert_eq!(unsupported.kind, VmErrorKind::UnsupportedOpcode);
    assert_eq!(unsupported.offset, 0);
    assert_eq!(unsupported.opcode, Some(0xff));

    let truncated = decode_program(&[5, 126, 1]).unwrap_err();
    assert_eq!(truncated.kind, VmErrorKind::Decode);
    assert_eq!(truncated.offset, 3);
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
    let bytes = [0, 4];
    let mut vm = EventVm::new(decode_program(&bytes).unwrap());
    let mut host = Host::new();
    assert_eq!(
        vm.step(Tick::new(0), &mut host).unwrap().status,
        VmStatus::Running
    );
    assert_eq!(vm.pc(), 1);
    assert_eq!(
        vm.step(Tick::new(1), &mut host).unwrap().status,
        VmStatus::Completed
    );
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
    for opcode in 0..=3 {
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
