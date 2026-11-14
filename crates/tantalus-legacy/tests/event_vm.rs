use std::collections::BTreeMap;
use tantalus_contracts::{EngineError, ErrorCategory, Tick};
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
            }
        }
        Ok(())
    }

    fn window_is_open(&self, window: u8) -> bool {
        self.windows.get(&window).copied().unwrap_or(false)
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
