use super::*;
use std::collections::{BTreeMap, VecDeque};
use tantalus_contracts::{EntityId, Tick};

const DEFAULT_INSTRUCTION_BUDGET: u32 = 1024;
const TRACE_CAPACITY: usize = 256;

#[derive(Debug, Clone)]
pub struct EventVm {
    program: EventProgram,
    entity: Option<EntityId>,
    pc: u32,
    variables: BTreeMap<(VariableScope, u16), i32>,
    wait: Option<WaitState>,
    status: VmStatus,
    instruction_budget: u32,
    traces: VecDeque<VmTraceEvent>,
}

impl EventVm {
    pub fn new(program: EventProgram) -> Self {
        Self {
            program,
            entity: None,
            pc: 0,
            variables: BTreeMap::new(),
            wait: None,
            status: VmStatus::Running,
            instruction_budget: DEFAULT_INSTRUCTION_BUDGET,
            traces: VecDeque::with_capacity(TRACE_CAPACITY),
        }
    }

    pub fn with_entity(mut self, entity: EntityId) -> Self {
        self.entity = Some(entity);
        self
    }

    pub fn with_instruction_budget(mut self, budget: u32) -> Self {
        self.instruction_budget = budget;
        self
    }

    pub fn pc(&self) -> u32 {
        self.pc
    }

    pub fn status(&self) -> VmStatus {
        self.status
    }

    pub fn wait_state(&self) -> Option<WaitState> {
        self.wait
    }

    pub fn variable(&self, scope: VariableScope, index: u16) -> i32 {
        self.variables.get(&(scope, index)).copied().unwrap_or(0)
    }

    pub fn traces(&self) -> impl Iterator<Item = &VmTraceEvent> {
        self.traces.iter()
    }

    pub fn step(
        &mut self,
        tick: Tick,
        host: &mut dyn LegacyEventHost,
    ) -> Result<VmStepReport, VmError> {
        let mut staged = self.clone();
        let report = staged.step_inner(tick, host)?;
        *self = staged;
        Ok(report)
    }

    fn step_inner(
        &mut self,
        tick: Tick,
        host: &mut dyn LegacyEventHost,
    ) -> Result<VmStepReport, VmError> {
        if self.status == VmStatus::Completed {
            return Ok(self.report(tick, 0, Vec::new()));
        }

        if let Some(wait) = self.wait {
            match wait {
                WaitState::Frames(remaining) => {
                    self.status = VmStatus::Waiting;
                    if remaining > 0 {
                        self.wait = Some(WaitState::Frames(remaining - 1));
                        return Ok(self.report(tick, 0, Vec::new()));
                    }
                    self.wait = None;
                    self.status = VmStatus::Running;
                }
                WaitState::Window(window) => {
                    self.status = VmStatus::Waiting;
                    if host.window_is_open(window) {
                        return Ok(self.report(tick, 0, Vec::new()));
                    }
                    self.wait = None;
                    self.status = VmStatus::Running;
                }
            }
        }

        let mut operations = 0;
        let mut events = Vec::new();
        let mut last_opcode = None;
        while self.status == VmStatus::Running {
            if operations >= self.instruction_budget {
                return Err(VmError::new(
                    VmErrorKind::InstructionLimit,
                    self.pc,
                    last_opcode,
                    "instruction budget exhausted",
                ));
            }

            let instruction = self.program.instruction(self.pc).cloned().ok_or_else(|| {
                VmError::new(
                    VmErrorKind::InvalidJump,
                    self.pc,
                    last_opcode,
                    "no instruction exists at the current program counter",
                )
            })?;
            let opcode = opcode_for(&instruction.kind);
            last_opcode = Some(opcode);
            self.trace(tick, instruction.offset, opcode, &instruction.kind);
            operations += 1;
            let next = instruction.next_offset();

            match instruction.kind {
                InstructionKind::Nop => {
                    self.pc = next;
                    break;
                }
                InstructionKind::Jump(relative) => {
                    self.pc = jump_target(&instruction, relative)?;
                }
                InstructionKind::JumpIfNot {
                    ref condition,
                    relative,
                } => {
                    let target = jump_target(&instruction, relative)?;
                    let value = self.evaluate(host, condition, instruction.offset, opcode)?;
                    self.pc = if value == 0 { target } else { next };
                }
                InstructionKind::JumpIf {
                    ref condition,
                    relative,
                } => {
                    let target = jump_target(&instruction, relative)?;
                    let value = self.evaluate(host, condition, instruction.offset, opcode)?;
                    self.pc = if value != 0 { target } else { next };
                }
                InstructionKind::Evaluate(expression) => {
                    self.evaluate(host, &expression, instruction.offset, opcode)?;
                    self.pc = next;
                }
                InstructionKind::Return => {
                    self.status = VmStatus::Completed;
                }
                InstructionKind::Wait(expression) => {
                    let frames = self.evaluate(host, &expression, instruction.offset, opcode)?;
                    self.pc = next;
                    if frames <= 0 {
                        continue;
                    }
                    let remaining = if frames > 254 { 253 } else { frames - 1 };
                    self.wait = Some(WaitState::Frames(remaining as u8));
                    self.status = VmStatus::Waiting;
                }
                InstructionKind::WindowSync {
                    window,
                    flags,
                    text,
                }
                | InstructionKind::WindowAsync {
                    window,
                    flags,
                    text,
                } => {
                    let window = self.evaluate_u8(host, &window, instruction.offset, opcode)?;
                    let flags = self.evaluate_u8(host, &flags, instruction.offset, opcode)?;
                    let text = self.evaluate_u16(host, &text, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::ShowWindow {
                        window,
                        flags,
                        text,
                    });
                    if opcode == 0x1f {
                        self.wait = Some(WaitState::Window(window));
                        self.status = VmStatus::Waiting;
                    }
                }
                InstructionKind::WaitWindow(window) => {
                    let window = self.evaluate_u8(host, &window, instruction.offset, opcode)?;
                    self.pc = next;
                    self.wait = Some(WaitState::Window(window));
                    self.status = VmStatus::Waiting;
                }
                InstructionKind::CloseWindow(window) => {
                    let window = self.evaluate_u8(host, &window, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::CloseWindow { window });
                }
                InstructionKind::SetControlDirection { arrow, analog } => {
                    let arrow = self.evaluate(host, &arrow, instruction.offset, opcode)? as i16;
                    let analog = self.evaluate(host, &analog, instruction.offset, opcode)? as i16;
                    self.pc = next;
                    events.push(HostEvent::SetControlDirection { arrow, analog });
                }
                InstructionKind::DefinePlayerControl => {
                    let Some(entity) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "player control requires current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::DefinePlayerControl { actor: entity });
                }
                InstructionKind::AimAtCamera => {
                    self.pc = next;
                    events.push(HostEvent::AimAtCamera);
                }
                InstructionKind::MoveActor { actor, x, z } => {
                    let actor = self.evaluate(host, &actor, instruction.offset, opcode)?;
                    let x = self.evaluate(host, &x, instruction.offset, opcode)?;
                    let z = self.evaluate(host, &z, instruction.offset, opcode)?;
                    if actor < 0 {
                        return Err(VmError::new(
                            VmErrorKind::InvalidExpression,
                            instruction.offset,
                            Some(opcode),
                            "actor identity cannot be negative",
                        ));
                    }
                    self.pc = next;
                    events.push(HostEvent::MoveActor {
                        actor: EntityId::new(actor as u64),
                        x,
                        z,
                    });
                }
            }
        }

        if !events.is_empty() {
            host.apply(&events)
                .map_err(|error| VmError::host(error, self.pc, last_opcode.unwrap_or(0)))?;
        }
        Ok(self.report(tick, operations, events))
    }

    fn evaluate(
        &mut self,
        host: &mut dyn LegacyEventHost,
        expression: &Expression,
        offset: u32,
        opcode: u8,
    ) -> Result<i32, VmError> {
        #[derive(Clone, Copy)]
        struct StackValue {
            value: i32,
            variable: Option<VariableRef>,
        }

        let mut stack: Vec<StackValue> = Vec::new();
        let logical_plans = logical_plans(&expression.0, offset, opcode)?;
        let mut index = 0;
        while index < expression.0.len() {
            if let Some(plan) = logical_plans.get(&index) {
                let left = stack.last().ok_or_else(|| {
                    expression_error(offset, opcode, "missing short-circuit left operand")
                })?;
                let skip = if plan.is_and {
                    left.value == 0
                } else {
                    left.value != 0
                };
                if skip {
                    stack.pop();
                    stack.push(StackValue {
                        value: i32::from(!plan.is_and),
                        variable: None,
                    });
                    index = plan.operator_index + 1;
                    continue;
                }
            }
            let operation = &expression.0[index];
            let binary = |stack: &mut Vec<StackValue>, _operation: &ExpressionOp| {
                let right = stack
                    .pop()
                    .ok_or_else(|| expression_error(offset, opcode, "missing right operand"))?;
                let left = stack
                    .pop()
                    .ok_or_else(|| expression_error(offset, opcode, "missing left operand"))?;
                Ok::<_, VmError>((left, right))
            };
            match operation {
                ExpressionOp::Constant(value) => stack.push(StackValue {
                    value: *value,
                    variable: None,
                }),
                ExpressionOp::Variable(reference) => stack.push(StackValue {
                    value: self.variable(reference.scope, reference.index),
                    variable: Some(*reference),
                }),
                ExpressionOp::Negate => {
                    let value = stack
                        .pop()
                        .ok_or_else(|| expression_error(offset, opcode, "missing operand"))?;
                    stack.push(StackValue {
                        value: value.value.wrapping_neg(),
                        variable: None,
                    });
                }
                ExpressionOp::Not => {
                    let value = stack
                        .pop()
                        .ok_or_else(|| expression_error(offset, opcode, "missing operand"))?;
                    stack.push(StackValue {
                        value: i32::from(value.value <= 0),
                        variable: None,
                    });
                }
                ExpressionOp::Add
                | ExpressionOp::Subtract
                | ExpressionOp::Multiply
                | ExpressionOp::Divide
                | ExpressionOp::Remainder
                | ExpressionOp::Less
                | ExpressionOp::Greater
                | ExpressionOp::GreaterOrEqual
                | ExpressionOp::Equal
                | ExpressionOp::NotEqual => {
                    let (left, right) = binary(&mut stack, operation)?;
                    let left = left.value;
                    let right = right.value;
                    let value = match operation {
                        ExpressionOp::Add => left.wrapping_add(right),
                        ExpressionOp::Subtract => left.wrapping_sub(right),
                        ExpressionOp::Multiply => left.wrapping_mul(right),
                        ExpressionOp::Divide if right != 0 => left.wrapping_div(right),
                        ExpressionOp::Remainder if right != 0 => left.wrapping_rem(right),
                        ExpressionOp::Divide | ExpressionOp::Remainder => left,
                        ExpressionOp::Less => i32::from(left < right),
                        ExpressionOp::Greater => i32::from(left > right),
                        ExpressionOp::GreaterOrEqual => i32::from(left >= right),
                        ExpressionOp::Equal => i32::from(left == right),
                        ExpressionOp::NotEqual => i32::from(left != right),
                        _ => unreachable!(),
                    };
                    stack.push(StackValue {
                        value,
                        variable: None,
                    });
                }
                ExpressionOp::LogicalAnd | ExpressionOp::LogicalOr => {
                    let (left, right) = binary(&mut stack, operation)?;
                    let value = match operation {
                        ExpressionOp::LogicalAnd => left.value != 0 && right.value != 0,
                        ExpressionOp::LogicalOr => left.value != 0 || right.value != 0,
                        _ => unreachable!(),
                    };
                    stack.push(StackValue {
                        value: i32::from(value),
                        variable: None,
                    });
                }
                ExpressionOp::Assign => {
                    let value = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing assignment value")
                    })?;
                    let target = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing assignment target")
                    })?;
                    let Some(variable) = target.variable else {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "assignment target is not a variable",
                        ));
                    };
                    let normalized = normalize(value.value, variable.value_type);
                    self.variables
                        .insert((variable.scope, variable.index), normalized);
                    stack.push(StackValue {
                        value: normalized,
                        variable: None,
                    });
                }
                ExpressionOp::PreDecrement => {
                    let target = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing pre-decrement target")
                    })?;
                    let Some(variable) = target.variable else {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "pre-decrement target is not a variable",
                        ));
                    };
                    let decremented = normalize(target.value.wrapping_sub(1), variable.value_type);
                    self.variables
                        .insert((variable.scope, variable.index), decremented);
                    stack.push(StackValue {
                        value: decremented,
                        variable: None,
                    });
                }
                ExpressionOp::PostDecrement => {
                    let target = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing post-decrement target")
                    })?;
                    let Some(variable) = target.variable else {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "post-decrement target is not a variable",
                        ));
                    };
                    let decremented = normalize(target.value.wrapping_sub(1), variable.value_type);
                    self.variables
                        .insert((variable.scope, variable.index), decremented);
                    stack.push(StackValue {
                        value: target.value,
                        variable: None,
                    });
                }
                ExpressionOp::CustomVariable(variable) => {
                    let value = host
                        .read_custom_variable(*variable)
                        .map_err(|error| VmError::host(error, offset, opcode))?;
                    stack.push(StackValue {
                        value,
                        variable: None,
                    });
                }
                ExpressionOp::ShiftRightAssignMembers => {
                    if stack.len() < 3 {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "member shift assignment requires mask and value inputs",
                        ));
                    }
                    let mask = stack[stack.len() - 3].value;
                    if mask != 0 {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "nonzero member selections require object-slot host state",
                        ));
                    }
                    stack.truncate(stack.len() - 2);
                    stack.push(StackValue {
                        value: 0,
                        variable: None,
                    });
                }
                ExpressionOp::ReadObjectMember(reference) => {
                    let value = host
                        .read_object_member(*reference)
                        .map_err(|error| VmError::host(error, offset, opcode))?;
                    stack.push(StackValue {
                        value,
                        variable: None,
                    });
                }
                ExpressionOp::Pad => stack.push(StackValue {
                    value: 0,
                    variable: None,
                }),
                ExpressionOp::PostIncrementMembers | ExpressionOp::PreDecrementMembers => {
                    if stack.len() < 2 {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "object-member operation requires selection inputs",
                        ));
                    }
                    if stack[stack.len() - 2].value != 0 {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "nonzero member selections require the object-member host",
                        ));
                    }
                    stack.truncate(stack.len() - 2);
                    stack.push(StackValue {
                        value: 0,
                        variable: None,
                    });
                }
                ExpressionOp::End => break,
            }
            index += 1;
        }
        Ok(stack.last().map(|value| value.value).unwrap_or(0))
    }

    fn evaluate_u8(
        &mut self,
        host: &mut dyn LegacyEventHost,
        expression: &Expression,
        offset: u32,
        opcode: u8,
    ) -> Result<u8, VmError> {
        let value = self.evaluate(host, expression, offset, opcode)?;
        if !(0..=u8::MAX.into()).contains(&value) {
            return Err(expression_error(offset, opcode, "value does not fit in u8"));
        }
        Ok(value as u8)
    }

    fn evaluate_u16(
        &mut self,
        host: &mut dyn LegacyEventHost,
        expression: &Expression,
        offset: u32,
        opcode: u8,
    ) -> Result<u16, VmError> {
        let value = self.evaluate(host, expression, offset, opcode)?;
        if !(0..=u16::MAX.into()).contains(&value) {
            return Err(expression_error(
                offset,
                opcode,
                "value does not fit in u16",
            ));
        }
        Ok(value as u16)
    }

    fn trace(&mut self, tick: Tick, offset: u32, opcode: u8, kind: &InstructionKind) {
        if self.traces.len() == TRACE_CAPACITY {
            self.traces.pop_front();
        }
        self.traces.push_back(VmTraceEvent {
            tick,
            offset,
            opcode: Some(opcode),
            description: format!("{kind:?}"),
        });
    }

    fn report(&self, tick: Tick, operations: u32, events: Vec<HostEvent>) -> VmStepReport {
        VmStepReport {
            tick,
            status: self.status,
            operations,
            events,
        }
    }
}

fn opcode_for(kind: &InstructionKind) -> u8 {
    match kind {
        InstructionKind::Nop => 0x00,
        InstructionKind::Jump(_) => 0x01,
        InstructionKind::JumpIfNot { .. } => 0x02,
        InstructionKind::JumpIf { .. } => 0x03,
        InstructionKind::Return => 0x04,
        InstructionKind::Evaluate(_) => 0x05,
        InstructionKind::WindowSync { .. } => 0x1f,
        InstructionKind::WindowAsync { .. } => 0x20,
        InstructionKind::CloseWindow(_) => 0x21,
        InstructionKind::Wait(_) => 0x22,
        InstructionKind::WaitWindow(_) => 0x54,
        InstructionKind::MoveActor { .. } => 0xbf,
        InstructionKind::AimAtCamera => 0xb8,
        InstructionKind::DefinePlayerControl => 0x2c,
        InstructionKind::SetControlDirection { .. } => 0x67,
    }
}

fn jump_target(instruction: &Instruction, relative: i16) -> Result<u32, VmError> {
    let target = i64::from(instruction.next_offset()) + i64::from(relative);
    if !(0..=u32::MAX.into()).contains(&target) {
        return Err(VmError::new(
            VmErrorKind::InvalidJump,
            instruction.offset,
            Some(opcode_for(&instruction.kind)),
            "relative jump leaves the event program",
        ));
    }
    Ok(target as u32)
}

fn normalize(value: i32, value_type: VariableType) -> i32 {
    match value_type {
        VariableType::SignedBit | VariableType::Bit => i32::from(value != 0),
        VariableType::Int24 => {
            let value = value & 0x00ff_ffff;
            if value & 0x0080_0000 != 0 {
                value | !0x00ff_ffff
            } else {
                value
            }
        }
        VariableType::UInt24 => value & 0x00ff_ffff,
        VariableType::SignedByte => i32::from(value as i8),
        VariableType::Byte => i32::from(value as u8),
        VariableType::Int16 => i32::from(value as i16),
        VariableType::UInt16 => i32::from(value as u16),
    }
}

fn expression_error(offset: u32, opcode: u8, message: impl Into<String>) -> VmError {
    VmError::new(
        VmErrorKind::InvalidExpression,
        offset,
        Some(opcode),
        message,
    )
}

struct LogicalPlan {
    operator_index: usize,
    is_and: bool,
}

fn logical_plans(
    operations: &[ExpressionOp],
    offset: u32,
    opcode: u8,
) -> Result<BTreeMap<usize, LogicalPlan>, VmError> {
    let mut plans = BTreeMap::new();
    for (index, operation) in operations.iter().enumerate() {
        let is_and = match operation {
            ExpressionOp::LogicalAnd => true,
            ExpressionOp::LogicalOr => false,
            _ => continue,
        };
        let right_start = operand_start(operations, index, offset, opcode)?;
        let _left_start = operand_start(operations, right_start, offset, opcode)?;
        plans.insert(
            right_start,
            LogicalPlan {
                operator_index: index,
                is_and,
            },
        );
    }
    Ok(plans)
}

fn operand_start(
    operations: &[ExpressionOp],
    end: usize,
    offset: u32,
    opcode: u8,
) -> Result<usize, VmError> {
    if end == 0 {
        return Err(expression_error(offset, opcode, "missing logical operand"));
    }
    let index = end - 1;
    match operations[index] {
        ExpressionOp::Constant(_)
        | ExpressionOp::Variable(_)
        | ExpressionOp::ReadObjectMember(_)
        | ExpressionOp::CustomVariable(_)
        | ExpressionOp::Pad => Ok(index),
        ExpressionOp::Negate
        | ExpressionOp::Not
        | ExpressionOp::PreDecrement
        | ExpressionOp::PostDecrement
        | ExpressionOp::PostIncrementMembers
        | ExpressionOp::PreDecrementMembers => operand_start(operations, index, offset, opcode),
        ExpressionOp::Add
        | ExpressionOp::Subtract
        | ExpressionOp::Multiply
        | ExpressionOp::Divide
        | ExpressionOp::Remainder
        | ExpressionOp::Less
        | ExpressionOp::Greater
        | ExpressionOp::GreaterOrEqual
        | ExpressionOp::Equal
        | ExpressionOp::NotEqual
        | ExpressionOp::Assign
        | ExpressionOp::LogicalAnd
        | ExpressionOp::LogicalOr => {
            let right = operand_start(operations, index, offset, opcode)?;
            operand_start(operations, right, offset, opcode)
        }
        ExpressionOp::ShiftRightAssignMembers | ExpressionOp::End => Err(expression_error(
            offset,
            opcode,
            "unsupported stack shape for a short-circuit operand",
        )),
    }
}
