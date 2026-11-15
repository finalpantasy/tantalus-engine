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
    expression_value: i32,
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
            expression_value: 0,
            wait: None,
            status: VmStatus::Running,
            instruction_budget: DEFAULT_INSTRUCTION_BUDGET,
            traces: VecDeque::with_capacity(TRACE_CAPACITY),
        }
    }

    /// Rebuilds a VM at a recorded position. `pc` must land on an instruction
    /// boundary, `status` and `wait` must agree (only `Waiting` carries a wait
    /// state), and a movement wait's `next` must also be a boundary. Variables
    /// and the instruction budget start fresh, as in [`EventVm::new`].
    pub fn resume(
        program: EventProgram,
        entity: EntityId,
        pc: u32,
        status: VmStatus,
        wait: Option<WaitState>,
    ) -> Result<Self, VmError> {
        let invalid = |offset: u32, message: &str| {
            Err(VmError::new(
                VmErrorKind::InvalidJump,
                offset,
                None,
                message,
            ))
        };
        match (status, wait) {
            (VmStatus::Waiting, None) => {
                return invalid(pc, "a waiting VM needs a wait state to resume");
            }
            (VmStatus::Running | VmStatus::Completed, Some(_)) => {
                return invalid(pc, "only a waiting VM can carry a wait state");
            }
            _ => {}
        }
        if status != VmStatus::Completed && !program.instructions.contains_key(&pc) {
            return invalid(pc, "resume position is not an instruction boundary");
        }
        if let Some(WaitState::ActorMovement { next }) = wait
            && !program.instructions.contains_key(&next)
        {
            return invalid(next, "movement wait target is not an instruction boundary");
        }
        let mut vm = Self::new(program).with_entity(entity);
        vm.pc = pc;
        vm.status = status;
        vm.wait = wait;
        Ok(vm)
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
                WaitState::PartyMenu => {
                    self.status = VmStatus::Waiting;
                    if host.party_menu_is_open() {
                        return Ok(self.report(tick, 0, Vec::new()));
                    }
                    self.wait = None;
                    self.status = VmStatus::Running;
                }
                WaitState::ActorTurn => {
                    self.status = VmStatus::Waiting;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            self.pc,
                            Some(0x50),
                            "turn wait requires current actor context",
                        ));
                    };
                    if host
                        .actor_turn_is_active(actor)
                        .map_err(|error| VmError::host(error, self.pc, 0x50))?
                    {
                        return Ok(self.report(tick, 0, Vec::new()));
                    }
                    self.wait = None;
                    self.status = VmStatus::Running;
                }
                WaitState::SharedScript => {
                    self.status = VmStatus::Waiting;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            self.pc,
                            Some(0x44),
                            "shared script wait requires current actor context",
                        ));
                    };
                    if host
                        .shared_script_is_active(actor)
                        .map_err(|error| VmError::host(error, self.pc, 0x44))?
                    {
                        return Ok(self.report(tick, 0, Vec::new()));
                    }
                    self.wait = None;
                    self.status = VmStatus::Running;
                }
                WaitState::ActorMovement { next } => {
                    self.status = VmStatus::Waiting;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            self.pc,
                            Some(0x23),
                            "movement wait requires current actor context",
                        ));
                    };
                    let active = host
                        .field_actor_movement_is_active(actor)
                        .map_err(|error| VmError::host(error, self.pc, 0x23))?;
                    self.wait = None;
                    self.status = VmStatus::Running;
                    if !active {
                        self.pc = next;
                    }
                }
                WaitState::ActorAnimation => {
                    self.status = VmStatus::Waiting;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            self.pc,
                            Some(0x41),
                            "animation wait requires current actor context",
                        ));
                    };
                    if host
                        .actor_animation_is_active(actor)
                        .map_err(|error| VmError::host(error, self.pc, 0x41))?
                    {
                        return Ok(self.report(tick, 0, Vec::new()));
                    }
                    self.wait = None;
                    self.status = VmStatus::Running;
                }
                WaitState::ObjectTurn { actor } => {
                    self.status = VmStatus::Waiting;
                    if host
                        .actor_turn_is_active(actor)
                        .map_err(|error| VmError::host(error, self.pc, 0xbc))?
                    {
                        return Ok(self.report(tick, 0, Vec::new()));
                    }
                    self.wait = None;
                    self.status = VmStatus::Running;
                }
                WaitState::ObjectAnimation { actor } => {
                    self.status = VmStatus::Waiting;
                    if host
                        .actor_animation_is_active(actor)
                        .map_err(|error| VmError::host(error, self.pc, 0xbe))?
                    {
                        return Ok(self.report(tick, 0, Vec::new()));
                    }
                    self.wait = None;
                    self.status = VmStatus::Running;
                }
                WaitState::CallerRequest { level, .. } => {
                    self.status = VmStatus::Waiting;
                    if !host
                        .caller_request_acceptable(level)
                        .map_err(|error| VmError::host(error, self.pc, 0x18))?
                    {
                        return Ok(self.report(tick, 0, Vec::new()));
                    }
                    self.wait = None;
                    self.status = VmStatus::Running;
                }
                WaitState::ScriptReadiness { level, target, .. } => {
                    self.status = VmStatus::Waiting;
                    if !host
                        .script_request_acceptable(target, level)
                        .map_err(|error| VmError::host(error, self.pc, 0x12))?
                    {
                        return Ok(self.report(tick, 0, Vec::new()));
                    }
                    self.wait = None;
                    self.status = VmStatus::Running;
                }
                WaitState::ScriptRequest => {
                    self.status = VmStatus::Waiting;
                    let Some(entity) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            self.pc,
                            None,
                            "script wait requires current actor context",
                        ));
                    };
                    if !host
                        .script_request_is_complete(entity)
                        .map_err(|error| VmError::host(error, self.pc, 0))?
                    {
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
                InstructionKind::Nop | InstructionKind::Yield(_) => {
                    self.pc = next;
                    break;
                }
                InstructionKind::NoOp(_) => {
                    self.pc = next;
                }
                InstructionKind::Jump(relative) => {
                    self.pc = jump_target(&instruction, relative)?;
                }
                InstructionKind::JumpIfNot { relative } => {
                    self.pc = if self.expression_value == 0 {
                        switch_target(&instruction, relative, instruction.length)?
                    } else {
                        next
                    };
                }
                InstructionKind::JumpIf { relative } => {
                    let target = jump_target(&instruction, relative)?;
                    self.pc = if self.expression_value != 0 {
                        target
                    } else {
                        next
                    };
                }
                InstructionKind::JumpSwitch {
                    base,
                    default_relative,
                    ref case_relative,
                }
                | InstructionKind::JumpSwitchWide {
                    base,
                    default_relative,
                    ref case_relative,
                } => {
                    let selected = self.expression_value.wrapping_sub(i32::from(base));
                    let relative = if selected >= 0 && selected < case_relative.len() as i32 {
                        case_relative[selected as usize]
                    } else {
                        default_relative
                    };
                    let anchor = if opcode == 0x0d { 2 } else { 1 };
                    self.pc = switch_target(&instruction, relative, anchor)?;
                }
                InstructionKind::JumpSwitchEx {
                    default_relative,
                    ref cases,
                } => {
                    let selected = (self.expression_value & 0xffff) as u16;
                    let relative = cases
                        .iter()
                        .find(|case| case[0] == selected)
                        .map(|case| case[1])
                        .unwrap_or(default_relative);
                    self.pc = switch_target(&instruction, relative, 4)?;
                }
                InstructionKind::Evaluate(expression) => {
                    self.expression_value =
                        self.evaluate(host, &expression, instruction.offset, opcode)?;
                    self.pc = next;
                }
                InstructionKind::SetObjectFlags(flags) => {
                    let flags = self.evaluate(host, &flags, instruction.offset, opcode)? as u8;
                    let Some(object) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "object flags require current object context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetObjectFlags { object, flags });
                }
                InstructionKind::SetHeadFocusMasks {
                    self_mask,
                    target_mask,
                } => {
                    let self_mask =
                        self.evaluate(host, &self_mask, instruction.offset, opcode)? as u8;
                    let target_mask =
                        self.evaluate(host, &target_mask, instruction.offset, opcode)? as u8;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "head-focus masks require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetHeadFocusMasks {
                        actor,
                        self_mask,
                        target_mask,
                    });
                }
                InstructionKind::SetHeadFocusFlags(flags) => {
                    let flags = self.evaluate(host, &flags, instruction.offset, opcode)? as u8 & 7;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "head-focus flags require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetHeadFocusFlags { actor, flags });
                }
                InstructionKind::SetWalkTurnSpeed(speed) => {
                    let speed = self.evaluate(host, &speed, instruction.offset, opcode)?;
                    let speed = if speed == 0 { u8::MAX } else { speed as u8 };
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "walk turn speed requires current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetWalkTurnSpeed { actor, speed });
                }
                InstructionKind::SetIdleAnimationSpeeds(expressions) => {
                    let [first, second, third, fourth] = expressions;
                    let speeds = [
                        self.evaluate(host, &first, instruction.offset, opcode)? as u8,
                        self.evaluate(host, &second, instruction.offset, opcode)? as u8,
                        self.evaluate(host, &third, instruction.offset, opcode)? as u8,
                        self.evaluate(host, &fourth, instruction.offset, opcode)? as u8,
                    ];
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "idle animation speeds require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetIdleAnimationSpeeds { actor, speeds });
                }
                InstructionKind::SetActorRadii {
                    pathing_size,
                    collision_radius,
                    talk_radius,
                } => {
                    let pathing_size =
                        self.evaluate(host, &pathing_size, instruction.offset, opcode)?;
                    let collision_radius =
                        self.evaluate(host, &collision_radius, instruction.offset, opcode)? as u8;
                    let talk_radius =
                        self.evaluate(host, &talk_radius, instruction.offset, opcode)? as u8;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "actor radii require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetActorRadii {
                        actor,
                        pathing_size,
                        collision_radius,
                        talk_radius,
                    });
                }
                InstructionKind::SetRightTurnAnimation(animation) => {
                    let animation =
                        self.evaluate(host, &animation, instruction.offset, opcode)? as u16;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "right-turn animation requires current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetRightTurnAnimation { actor, animation });
                }
                InstructionKind::SetJumpAnimation {
                    animation,
                    parameters,
                } => {
                    let animation =
                        self.evaluate(host, &animation, instruction.offset, opcode)? as u16;
                    let [first, second] = parameters;
                    let parameters = [
                        self.evaluate(host, &first, instruction.offset, opcode)? as u8,
                        self.evaluate(host, &second, instruction.offset, opcode)? as u8,
                    ];
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "jump animation requires current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetJumpAnimation {
                        actor,
                        animation,
                        parameters,
                    });
                }
                InstructionKind::SetLeftTurnAnimation(animation) => {
                    let animation =
                        self.evaluate(host, &animation, instruction.offset, opcode)? as u16;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "left-turn animation requires current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetLeftTurnAnimation { actor, animation });
                }
                InstructionKind::SetRunAnimation(animation) => {
                    let animation =
                        self.evaluate(host, &animation, instruction.offset, opcode)? as u16;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "run animation requires current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetRunAnimation { actor, animation });
                }
                InstructionKind::PlayAnimation(animation) => {
                    let animation =
                        self.evaluate(host, &animation, instruction.offset, opcode)? as u16;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "animation playback requires current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::PlayAnimation { actor, animation });
                }
                InstructionKind::SetAnimationRange { start, end } => {
                    let start = self.evaluate(host, &start, instruction.offset, opcode)? as u8;
                    let end = self.evaluate(host, &end, instruction.offset, opcode)? as u8;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "animation range requires current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetAnimationRange { actor, start, end });
                }
                InstructionKind::SetWalkAnimation(animation) => {
                    let animation =
                        self.evaluate(host, &animation, instruction.offset, opcode)? as u16;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "walk animation requires current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetWalkAnimation { actor, animation });
                }
                InstructionKind::SetIdleAnimation(animation) => {
                    let animation =
                        self.evaluate(host, &animation, instruction.offset, opcode)? as u16;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "idle animation requires current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetIdleAnimation { actor, animation });
                }
                InstructionKind::SetActorFacing(angle) => {
                    let angle = self
                        .evaluate(host, &angle, instruction.offset, opcode)?
                        .wrapping_shl(4) as i16;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "facing requires current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetActorFacing { actor, angle });
                }
                InstructionKind::StartActorTurn { angle, speed } => {
                    let angle = self
                        .evaluate(host, &angle, instruction.offset, opcode)?
                        .wrapping_shl(4) as i16;
                    let speed = self.evaluate(host, &speed, instruction.offset, opcode)?;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "actor turn requires current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::StartActorTurn {
                        actor,
                        angle,
                        speed,
                    });
                }
                InstructionKind::SetActorPosition { x, z } => {
                    let x = self.evaluate(host, &x, instruction.offset, opcode)?;
                    let z = self.evaluate(host, &z, instruction.offset, opcode)?;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "positioning requires current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetActorPosition {
                        actor,
                        position: [x, 32768, z],
                    });
                }
                InstructionKind::DeleteObject(target) => {
                    let (target, terminates) = if target == u8::MAX {
                        let Some(actor) = self.entity else {
                            return Err(VmError::new(
                                VmErrorKind::Host,
                                instruction.offset,
                                Some(opcode),
                                "self-deletion requires current object context",
                            ));
                        };
                        (actor, true)
                    } else {
                        (
                            EntityId::new(u64::from(target)),
                            self.entity == Some(EntityId::new(u64::from(target))),
                        )
                    };
                    self.pc = next;
                    events.push(HostEvent::DeleteObject { target });
                    if terminates {
                        self.status = VmStatus::Completed;
                        break;
                    }
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
                InstructionKind::SetActorPositionXzy { x, z, y } => {
                    let x = self.evaluate(host, &x, instruction.offset, opcode)?;
                    let z = self.evaluate(host, &z, instruction.offset, opcode)?;
                    let y = self.evaluate(host, &y, instruction.offset, opcode)?;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "positioning requires current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetActorPosition {
                        actor,
                        position: [x, y, z.wrapping_neg()],
                    });
                }
                InstructionKind::MoveActorXzy { actor, x, z, y } => {
                    let actor = self.evaluate(host, &actor, instruction.offset, opcode)?;
                    let x = self.evaluate(host, &x, instruction.offset, opcode)?;
                    let z = self.evaluate(host, &z, instruction.offset, opcode)?;
                    let y = self.evaluate(host, &y, instruction.offset, opcode)?;
                    if actor < 0 {
                        return Err(VmError::new(
                            VmErrorKind::InvalidExpression,
                            instruction.offset,
                            Some(opcode),
                            "actor identity cannot be negative",
                        ));
                    }
                    self.pc = next;
                    events.push(HostEvent::MoveActorXzy {
                        actor: EntityId::new(actor as u64),
                        position: [x, y, z.wrapping_neg()],
                    });
                }
                InstructionKind::WalkActorMixed {
                    opcode: walk_opcode,
                    x,
                    y,
                    z,
                } => {
                    let x = self.evaluate(host, &x, instruction.offset, opcode)?;
                    let y = self.evaluate(host, &y, instruction.offset, opcode)?;
                    let z = self.evaluate(host, &z, instruction.offset, opcode)?;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "walk requires current actor context",
                        ));
                    };
                    let mode = match walk_opcode {
                        0xa5 => 1,
                        0xa2 => 2,
                        _ => 3,
                    };
                    self.pc = instruction.offset;
                    self.wait = Some(WaitState::ActorMovement { next });
                    self.status = VmStatus::Waiting;
                    events.push(HostEvent::WalkActorMixed {
                        actor,
                        destination: [x, y.wrapping_neg(), z],
                        mode,
                    });
                }
                InstructionKind::TurnActorToPosition { x, z } => {
                    let x = self.evaluate(host, &x, instruction.offset, opcode)?;
                    let z = self.evaluate(host, &z, instruction.offset, opcode)?;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "current-object commands require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::TurnActorToPosition { actor, x, z });
                }
                InstructionKind::WaitSharedScript => {
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "shared script wait requires current actor context",
                        ));
                    };
                    if host
                        .shared_script_is_active(actor)
                        .map_err(|error| VmError::host(error, instruction.offset, opcode))?
                    {
                        self.wait = Some(WaitState::SharedScript);
                        self.status = VmStatus::Waiting;
                    } else {
                        self.pc = next;
                    }
                }
                InstructionKind::SetPathing { enabled } => {
                    let enabled = self.evaluate(host, &enabled, instruction.offset, opcode)?;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "current-object commands require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetPathing { actor, enabled });
                }
                InstructionKind::WaitTurn => {
                    if self.entity.is_none() {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "turn wait requires current actor context",
                        ));
                    }
                    self.pc = next;
                    self.wait = Some(WaitState::ActorTurn);
                    self.status = VmStatus::Waiting;
                }
                InstructionKind::WaitAnimation => {
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "animation wait requires current actor context",
                        ));
                    };
                    if host
                        .actor_animation_is_active(actor)
                        .map_err(|error| VmError::host(error, instruction.offset, opcode))?
                    {
                        self.wait = Some(WaitState::ActorAnimation);
                        self.status = VmStatus::Waiting;
                    } else {
                        self.pc = next;
                    }
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
                InstructionKind::BackgroundMove { x, y, z } => {
                    self.evaluate(host, &x, instruction.offset, opcode)? as i16;
                    self.evaluate(host, &y, instruction.offset, opcode)? as i16;
                    self.evaluate(host, &z, instruction.offset, opcode)? as i16;
                    self.pc = next;
                }
                InstructionKind::SetControlDirection { arrow, analog } => {
                    let arrow = self.evaluate(host, &arrow, instruction.offset, opcode)? as i16;
                    let analog = self.evaluate(host, &analog, instruction.offset, opcode)? as i16;
                    self.pc = next;
                    events.push(HostEvent::SetControlDirection { arrow, analog });
                }
                InstructionKind::Cinematic {
                    operation,
                    argument,
                    auxiliary,
                } => {
                    let operation = self.evaluate(host, &operation, instruction.offset, opcode)?;
                    let argument = self.evaluate(host, &argument, instruction.offset, opcode)?;
                    let auxiliary = self.evaluate(host, &auxiliary, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::DispatchCinematic {
                        operation,
                        argument,
                        auxiliary,
                    });
                }
                InstructionKind::DefineRegion { vertices } => {
                    let Some(region) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "region definition requires current object context",
                        ));
                    };
                    let mut evaluated = Vec::with_capacity(vertices.len());
                    for [x, z] in vertices {
                        let x = self.evaluate(host, &x, instruction.offset, opcode)? as i16;
                        let z = self.evaluate(host, &z, instruction.offset, opcode)? as i16;
                        evaluated.push([x, z]);
                    }
                    self.pc = next;
                    events.push(HostEvent::DefineRegion {
                        region,
                        vertices: evaluated,
                    });
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
                InstructionKind::ExitField => {
                    self.pc = next;
                    events.push(HostEvent::ExitField);
                }
                InstructionKind::DisableIdleAnimation => {
                    self.pc = next;
                    events.push(HostEvent::DisableIdleAnimation);
                }
                InstructionKind::PlayJumpAnimation { landing } => {
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "current-object commands require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::PlayJumpAnimation { actor, landing });
                }
                InstructionKind::StopAnimation => {
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "current-object commands require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::StopAnimation { actor });
                }
                InstructionKind::SetPathFloorActive { floor, active } => {
                    let floor = self.evaluate(host, &floor, instruction.offset, opcode)?;
                    let active = self.evaluate(host, &active, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::SetPathFloorActive { floor, active });
                }
                InstructionKind::PretendToBe { target } => {
                    let target = self.evaluate(host, &target, instruction.offset, opcode)?;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "current-object commands require current actor context",
                        ));
                    };
                    if target < 0 {
                        return Err(expression_error(
                            instruction.offset,
                            opcode,
                            "object identity cannot be negative",
                        ));
                    }
                    self.pc = next;
                    events.push(HostEvent::PretendToBe {
                        actor,
                        target: EntityId::new(target as u64),
                    });
                }
                InstructionKind::StartBattle {
                    rush,
                    group,
                    battle,
                } => {
                    let rush = self.evaluate(host, &rush, instruction.offset, opcode)?;
                    let group = match group {
                        Some(group) => {
                            Some(self.evaluate(host, &group, instruction.offset, opcode)?)
                        }
                        None => None,
                    };
                    let battle = self.evaluate(host, &battle, instruction.offset, opcode)?;
                    self.pc = next;
                    self.status = VmStatus::Completed;
                    events.push(HostEvent::StartBattle {
                        rush,
                        group,
                        battle: battle & 0x7fff,
                        steiner_state: battle >> 15 & 1 != 0,
                    });
                }
                InstructionKind::ChangeWorldMap { destination } => {
                    let destination =
                        self.evaluate(host, &destination, instruction.offset, opcode)? as i16;
                    self.pc = next;
                    self.status = VmStatus::Completed;
                    events.push(HostEvent::ChangeWorldMap { destination });
                }
                InstructionKind::SyncPartyUid => {
                    self.pc = next;
                    events.push(HostEvent::SyncPartyUid);
                }
                InstructionKind::StartObjectTurn {
                    object,
                    angle,
                    speed,
                } => {
                    let object = self.evaluate(host, &object, instruction.offset, opcode)?;
                    let angle = self
                        .evaluate(host, &angle, instruction.offset, opcode)?
                        .wrapping_shl(4) as i16;
                    let speed = self.evaluate(host, &speed, instruction.offset, opcode)?;
                    if object < 0 {
                        return Err(expression_error(
                            instruction.offset,
                            opcode,
                            "object identity cannot be negative",
                        ));
                    }
                    self.pc = next;
                    events.push(HostEvent::StartActorTurn {
                        actor: EntityId::new(object as u64),
                        angle,
                        speed,
                    });
                }
                InstructionKind::PlayObjectAnimation { object, animation } => {
                    let object = self.evaluate(host, &object, instruction.offset, opcode)?;
                    let animation =
                        self.evaluate(host, &animation, instruction.offset, opcode)? as u16;
                    if object < 0 {
                        return Err(expression_error(
                            instruction.offset,
                            opcode,
                            "object identity cannot be negative",
                        ));
                    }
                    self.pc = next;
                    events.push(HostEvent::PlayAnimation {
                        actor: EntityId::new(object as u64),
                        animation,
                    });
                }
                InstructionKind::PlayTextureAnimation {
                    object,
                    animation,
                    once,
                } => {
                    let object = self.evaluate(host, &object, instruction.offset, opcode)?;
                    let animation = self.evaluate(host, &animation, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::PlayTextureAnimation {
                        object,
                        animation,
                        once,
                    });
                }
                InstructionKind::DisableRun => {
                    self.pc = next;
                    events.push(HostEvent::DisableRun);
                }
                InstructionKind::SetTileCamera { overlay, camera } => {
                    let overlay = self.evaluate(host, &overlay, instruction.offset, opcode)?;
                    let camera = self.evaluate(host, &camera, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::SetTileCamera { overlay, camera });
                }
                InstructionKind::GameOver => {
                    self.pc = next;
                    self.status = VmStatus::Completed;
                    events.push(HostEvent::GameOver);
                }
                InstructionKind::StartCardGame { opponent } => {
                    let opponent =
                        self.evaluate(host, &opponent, instruction.offset, opcode)? as u16;
                    self.pc = next;
                    events.push(HostEvent::StartCardGame { opponent });
                }
                InstructionKind::SetModelColor {
                    object,
                    red,
                    green,
                    blue,
                } => {
                    let object = self.evaluate(host, &object, instruction.offset, opcode)?;
                    let red = self.evaluate(host, &red, instruction.offset, opcode)? as u8;
                    let green = self.evaluate(host, &green, instruction.offset, opcode)? as u8;
                    let blue = self.evaluate(host, &blue, instruction.offset, opcode)? as u8;
                    self.pc = next;
                    events.push(HostEvent::SetModelColor {
                        object,
                        red,
                        green,
                        blue,
                    });
                    // Memoria's first execution sets the color and returns FLOW_STATE_WAIT;
                    // the second execution only clears its marker and continues.
                    break;
                }
                InstructionKind::AimAtCamera => {
                    self.pc = next;
                    events.push(HostEvent::AimAtCamera);
                }
                InstructionKind::InitializeObject { source, unique_id } => {
                    self.pc = next;
                    events.push(HostEvent::InitializeObject { source, unique_id });
                }
                InstructionKind::InitializeRegion { source, unique_id } => {
                    self.pc = next;
                    events.push(HostEvent::InitializeRegion { source, unique_id });
                }
                InstructionKind::InitializeCode { source, unique_id } => {
                    self.pc = next;
                    events.push(HostEvent::InitializeCode { source, unique_id });
                }
                InstructionKind::SetTriangleActive { triangle, active } => {
                    let triangle =
                        self.evaluate(host, &triangle, instruction.offset, opcode)? as u32;
                    let active = self.evaluate(host, &active, instruction.offset, opcode)? != 0;
                    self.pc = next;
                    events.push(HostEvent::SetTriangleActive { triangle, active });
                }
                InstructionKind::RunSound {
                    code,
                    sound,
                    arguments,
                    ..
                } => {
                    let code = self.evaluate(host, &code, instruction.offset, opcode)?;
                    let sound = self.evaluate(host, &sound, instruction.offset, opcode)?;
                    let [first, second, third] = arguments;
                    let arguments = [
                        self.evaluate(host, &first, instruction.offset, opcode)?,
                        self.evaluate(host, &second, instruction.offset, opcode)?,
                        self.evaluate(host, &third, instruction.offset, opcode)?,
                    ];
                    self.pc = next;
                    events.push(HostEvent::RunSound {
                        code,
                        sound,
                        arguments,
                    });
                }
                InstructionKind::AttachObjectOffset { x, y, z } => {
                    let x = self.evaluate(host, &x, instruction.offset, opcode)?;
                    let y = self.evaluate(host, &y, instruction.offset, opcode)?;
                    let z = self.evaluate(host, &z, instruction.offset, opcode)?;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "attachment offset requires current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::AttachObjectOffset { actor, x, y, z });
                }
                InstructionKind::SetActorModel { model, eye_height } => {
                    let model = self.evaluate(host, &model, instruction.offset, opcode)? as u16;
                    let eye_height = self
                        .evaluate(host, &eye_height, instruction.offset, opcode)?
                        .wrapping_mul(-4) as i16;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "model selection requires current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetActorModel {
                        actor,
                        model,
                        eye_height,
                    });
                }
                InstructionKind::ResetWalkState => {
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "walk-state reset requires current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::ResetWalkState { actor });
                }
                InstructionKind::SetMovementSpeed(speed) => {
                    let speed = self.evaluate(host, &speed, instruction.offset, opcode)? as u8;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "movement speed requires current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetMovementSpeed { actor, speed });
                }
                InstructionKind::SetNavigationAttributeMask(mask) => {
                    let mask = self.evaluate(host, &mask, instruction.offset, opcode)? as u8;
                    self.pc = next;
                    events.push(HostEvent::SetNavigationAttributeMask { mask });
                }
                InstructionKind::SetShadowEnabled { enabled, .. } => {
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "shadow visibility requires current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetShadowEnabled { actor, enabled });
                }
                InstructionKind::SetTextVariable { script, value } => {
                    let script = self.evaluate(host, &script, instruction.offset, opcode)?;
                    let value = self.evaluate(host, &value, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::SetTextVariable { script, value });
                }
                InstructionKind::ShowBattleMessage(text) => {
                    let text = self.evaluate(host, &text, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::ShowBattleMessage { text });
                }
                InstructionKind::SetObjectScale { actor, x, z, y } => {
                    let actor = self.evaluate(host, &actor, instruction.offset, opcode)?;
                    if actor < 0 {
                        return Err(expression_error(
                            instruction.offset,
                            opcode,
                            "object identity cannot be negative",
                        ));
                    }
                    let x = self.evaluate(host, &x, instruction.offset, opcode)?;
                    let z = self.evaluate(host, &z, instruction.offset, opcode)?;
                    let y = self.evaluate(host, &y, instruction.offset, opcode)?;
                    let scale = [x, z, y].map(|value| i32::from(value as i8).wrapping_shl(6));
                    self.pc = next;
                    events.push(HostEvent::SetObjectScale {
                        actor: EntityId::new(actor as u64),
                        scale,
                        vertical_scale: z as u8,
                    });
                }
                InstructionKind::SetVibrationFrameRate(rate) => {
                    let rate = self.evaluate(host, &rate, instruction.offset, opcode)? as i16;
                    self.pc = next;
                    events.push(HostEvent::SetVibrationFrameRate { rate });
                }
                InstructionKind::PreloadField { hint, field } => {
                    let hint = self.evaluate(host, &hint, instruction.offset, opcode)?;
                    let field = self.evaluate(host, &field, instruction.offset, opcode)? as i16;
                    self.pc = next;
                    events.push(HostEvent::PreloadField { hint, field });
                }
                InstructionKind::SetRain { strength, speed } => {
                    let strength = self.evaluate(host, &strength, instruction.offset, opcode)?;
                    let speed = self.evaluate(host, &speed, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::SetRain { strength, speed });
                }
                InstructionKind::RequestScriptAsync { level, target, tag } => {
                    let level = self.evaluate(host, &level, instruction.offset, opcode)?;
                    let target = self.evaluate(host, &target, instruction.offset, opcode)?;
                    if target < 0 {
                        return Err(expression_error(
                            instruction.offset,
                            opcode,
                            "script target cannot be negative",
                        ));
                    }
                    self.pc = next;
                    events.push(HostEvent::RequestScriptAsync {
                        level,
                        target: EntityId::new(target as u64),
                        tag,
                    });
                }
                InstructionKind::RequestScriptSync { level, target, tag } => {
                    let level = self.evaluate(host, &level, instruction.offset, opcode)?;
                    let target = self.evaluate(host, &target, instruction.offset, opcode)?;
                    if target < 0 {
                        return Err(expression_error(
                            instruction.offset,
                            opcode,
                            "script target cannot be negative",
                        ));
                    }
                    let target = EntityId::new(target as u64);
                    if host
                        .script_request_acceptable(target, level)
                        .map_err(|error| VmError::host(error, instruction.offset, opcode))?
                    {
                        self.pc = next;
                        self.wait = Some(WaitState::ScriptRequest);
                        self.status = VmStatus::Waiting;
                        events.push(HostEvent::RequestScriptSync { level, target, tag });
                    } else {
                        // Memoria stay(): retry this instruction on the next step.
                        self.pc = instruction.offset;
                        break;
                    }
                }
                InstructionKind::RequestScriptWhenReady { level, target, tag } => {
                    let level = self.evaluate(host, &level, instruction.offset, opcode)?;
                    let target = self.evaluate(host, &target, instruction.offset, opcode)?;
                    if target < 0 {
                        return Err(expression_error(
                            instruction.offset,
                            opcode,
                            "script target cannot be negative",
                        ));
                    }
                    let target = EntityId::new(target as u64);
                    if host
                        .script_request_acceptable(target, level)
                        .map_err(|error| VmError::host(error, instruction.offset, opcode))?
                    {
                        self.pc = next;
                        events.push(HostEvent::RequestScriptAsync { level, target, tag });
                    } else {
                        self.wait = Some(WaitState::ScriptReadiness { level, target, tag });
                        self.status = VmStatus::Waiting;
                    }
                }
                InstructionKind::DefineTileViewport {
                    viewport,
                    x,
                    y,
                    width,
                    height,
                } => {
                    let viewport = self.evaluate(host, &viewport, instruction.offset, opcode)?;
                    let x = self.evaluate(host, &x, instruction.offset, opcode)? as i16;
                    let y = self.evaluate(host, &y, instruction.offset, opcode)? as i16;
                    let width = self.evaluate(host, &width, instruction.offset, opcode)? as i16;
                    let height = self.evaluate(host, &height, instruction.offset, opcode)? as i16;
                    self.pc = next;
                    events.push(HostEvent::DefineTileViewport {
                        viewport,
                        x,
                        y,
                        width,
                        height,
                    });
                }
                InstructionKind::PreserveAndHideAll => {
                    self.pc = next;
                    events.push(HostEvent::PreserveAndHideAll);
                }
                InstructionKind::RestoreAllVisibility => {
                    self.pc = next;
                    events.push(HostEvent::RestoreAllVisibility);
                }
                InstructionKind::SetTimerRunning(running) => {
                    let running = self.evaluate(host, &running, instruction.offset, opcode)? != 0;
                    self.pc = next;
                    events.push(HostEvent::SetTimerRunning { running });
                }
                InstructionKind::RequestCallerAsync { level, tag } => {
                    let level = self.evaluate(host, &level, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::RequestCallerAsync { level, tag });
                }
                InstructionKind::RequestCallerWhenReady { level, tag } => {
                    let level = self.evaluate(host, &level, instruction.offset, opcode)?;
                    if host
                        .caller_request_acceptable(level)
                        .map_err(|error| VmError::host(error, instruction.offset, opcode))?
                    {
                        self.pc = next;
                        events.push(HostEvent::RequestCallerAsync { level, tag });
                    } else {
                        self.wait = Some(WaitState::CallerRequest { level, tag });
                        self.status = VmStatus::Waiting;
                    }
                }
                InstructionKind::CalculateExitPosition => {
                    self.pc = next;
                    events.push(HostEvent::CalculateExitPosition);
                }
                InstructionKind::WalkToExit => {
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "exit walk requires current actor context",
                        ));
                    };
                    let moving = host
                        .field_exit_movement_is_active(actor)
                        .map_err(|error| VmError::host(error, instruction.offset, opcode))?;
                    self.pc = if moving { instruction.offset } else { next };
                    events.push(HostEvent::MoveActorToExit { actor });
                    self.status = VmStatus::Running;
                    break;
                }
                InstructionKind::CalculateScreenPosition(object) => {
                    let object = self.evaluate(host, &object, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::CalculateScreenPosition { object });
                }
                InstructionKind::ChangeField(field) => {
                    let field = self.evaluate(host, &field, instruction.offset, opcode)? as i16;
                    self.pc = next;
                    self.status = VmStatus::Completed;
                    events.push(HostEvent::ChangeField { field });
                }
                InstructionKind::RequestCallerSync { level, tag } => {
                    let level = self.evaluate(host, &level, instruction.offset, opcode)?;
                    if self.entity.is_none() {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "synchronous caller request requires current actor context",
                        ));
                    }
                    self.pc = next;
                    self.wait = Some(WaitState::ScriptRequest);
                    self.status = VmStatus::Waiting;
                    events.push(HostEvent::RequestCallerSync { level, tag });
                }
                InstructionKind::SetSpsParameters {
                    object,
                    operation,
                    arguments,
                } => {
                    let object = self.evaluate(host, &object, instruction.offset, opcode)?;
                    let operation = self.evaluate(host, &operation, instruction.offset, opcode)?;
                    let [first, second, third] = arguments;
                    let arguments = [
                        self.evaluate(host, &first, instruction.offset, opcode)? as i16,
                        self.evaluate(host, &second, instruction.offset, opcode)? as i16,
                        self.evaluate(host, &third, instruction.offset, opcode)? as i16,
                    ];
                    self.pc = next;
                    events.push(HostEvent::SetSpsParameters {
                        object,
                        operation,
                        arguments,
                    });
                }
                InstructionKind::SetTileShadeColor {
                    overlay,
                    red,
                    green,
                    blue,
                } => {
                    let overlay = self.evaluate(host, &overlay, instruction.offset, opcode)?;
                    let red = self.evaluate(host, &red, instruction.offset, opcode)? as u8;
                    let green = self.evaluate(host, &green, instruction.offset, opcode)? as u8;
                    let blue = self.evaluate(host, &blue, instruction.offset, opcode)? as u8;
                    self.pc = next;
                    events.push(HostEvent::SetTileShadeColor {
                        overlay,
                        color: [red, green, blue],
                    });
                }
                InstructionKind::SetSimulationFloor { simulation, floor } => {
                    let simulation =
                        self.evaluate(host, &simulation, instruction.offset, opcode)? as u32;
                    let floor = self.evaluate(host, &floor, instruction.offset, opcode)? as u32;
                    self.pc = next;
                    events.push(HostEvent::SetSimulationFloor { simulation, floor });
                }
                InstructionKind::SetSimulationFrameRate {
                    simulation,
                    frame_rate,
                } => {
                    let simulation =
                        self.evaluate(host, &simulation, instruction.offset, opcode)? as u32;
                    let frame_rate =
                        self.evaluate(host, &frame_rate, instruction.offset, opcode)? as i16;
                    self.pc = next;
                    events.push(HostEvent::SetSimulationFrameRate {
                        simulation,
                        frame_rate,
                    });
                }
                InstructionKind::SetTileBlockActive { block, active } => {
                    let block = self.evaluate(host, &block, instruction.offset, opcode)?;
                    let active = self.evaluate(host, &active, instruction.offset, opcode)? != 0;
                    self.pc = next;
                    events.push(HostEvent::SetTileBlockActive { block, active });
                }
                InstructionKind::SetTileLoop {
                    block,
                    enabled,
                    x_speed,
                    y_speed,
                } => {
                    let block = self.evaluate(host, &block, instruction.offset, opcode)?;
                    let enabled = self.evaluate(host, &enabled, instruction.offset, opcode)?;
                    let x_speed = self.evaluate(host, &x_speed, instruction.offset, opcode)? as i16;
                    let y_speed = self.evaluate(host, &y_speed, instruction.offset, opcode)? as i16;
                    self.pc = next;
                    events.push(HostEvent::SetTileLoop {
                        block,
                        enabled,
                        x_speed,
                        y_speed,
                    });
                }
                InstructionKind::SetTileParallax {
                    block,
                    enabled,
                    x_speed,
                    y_speed,
                } => {
                    let block = self.evaluate(host, &block, instruction.offset, opcode)?;
                    let enabled = self.evaluate(host, &enabled, instruction.offset, opcode)?;
                    let x_speed = self.evaluate(host, &x_speed, instruction.offset, opcode)? as i16;
                    let y_speed = self.evaluate(host, &y_speed, instruction.offset, opcode)? as i16;
                    self.pc = next;
                    events.push(HostEvent::SetTileParallax {
                        block,
                        enabled,
                        x_speed,
                        y_speed,
                    });
                }
                InstructionKind::SetTileOrigin { block, x, y } => {
                    let block = self.evaluate(host, &block, instruction.offset, opcode)?;
                    let x = self.evaluate(host, &x, instruction.offset, opcode)? as i16;
                    let y = self.evaluate(host, &y, instruction.offset, opcode)? as i16;
                    self.pc = next;
                    events.push(HostEvent::SetTileOrigin { block, x, y });
                }
                InstructionKind::RunTileAnimation {
                    animation,
                    start_frame,
                } => {
                    let animation = self.evaluate(host, &animation, instruction.offset, opcode)?;
                    let start_frame =
                        self.evaluate(host, &start_frame, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::RunTileAnimation {
                        animation,
                        start_frame,
                    });
                }
                InstructionKind::SetTileAnimationActive { animation, active } => {
                    let animation = self.evaluate(host, &animation, instruction.offset, opcode)?;
                    let active = self.evaluate(host, &active, instruction.offset, opcode)? != 0;
                    self.pc = next;
                    events.push(HostEvent::SetTileAnimationActive { animation, active });
                }
                InstructionKind::ResetTileAnimation {
                    animation,
                    show_first_frame,
                } => {
                    let animation = self.evaluate(host, &animation, instruction.offset, opcode)?;
                    let show_first_frame =
                        self.evaluate(host, &show_first_frame, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::ResetTileAnimation {
                        animation,
                        show_first_frame,
                    });
                }
                InstructionKind::StartVibration { frame } => {
                    let frame = self.evaluate(host, &frame, instruction.offset, opcode)? as i16;
                    self.pc = next;
                    events.push(HostEvent::StartVibration { frame });
                }
                InstructionKind::SetVibrationActive { active } => {
                    let active = self.evaluate(host, &active, instruction.offset, opcode)? != 0;
                    self.pc = next;
                    events.push(HostEvent::SetVibrationActive { active });
                }
                InstructionKind::RunVibrationTrack {
                    track,
                    sample,
                    active,
                } => {
                    let track = self.evaluate(host, &track, instruction.offset, opcode)?;
                    let sample = self.evaluate(host, &sample, instruction.offset, opcode)?;
                    let active = self.evaluate(host, &active, instruction.offset, opcode)? != 0;
                    self.pc = next;
                    events.push(HostEvent::RunVibrationTrack {
                        track,
                        sample,
                        active,
                    });
                }
                InstructionKind::ActivateVibrationTrack {
                    track,
                    sample,
                    active,
                } => {
                    let track = self.evaluate(host, &track, instruction.offset, opcode)? as u32;
                    let sample = self.evaluate(host, &sample, instruction.offset, opcode)? as u32;
                    let active = self.evaluate(host, &active, instruction.offset, opcode)? != 0;
                    self.pc = next;
                    events.push(HostEvent::ActivateVibrationTrack {
                        track,
                        sample,
                        active,
                    });
                }
                InstructionKind::SetVibrationFlags { flags } => {
                    let flags = self.evaluate(host, &flags, instruction.offset, opcode)? as i16;
                    self.pc = next;
                    events.push(HostEvent::SetVibrationFlags { flags });
                }
                InstructionKind::SetVibrationRange { start, end } => {
                    let start = self.evaluate(host, &start, instruction.offset, opcode)? as i16;
                    let end = self.evaluate(host, &end, instruction.offset, opcode)? as i16;
                    self.pc = next;
                    events.push(HostEvent::SetVibrationRange { start, end });
                }
                InstructionKind::SetShadowSize { x, z } => {
                    let x = self.evaluate(host, &x, instruction.offset, opcode)?;
                    let z = self.evaluate(host, &z, instruction.offset, opcode)?;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "shadow commands require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetShadowSize { actor, x, z });
                }
                InstructionKind::SetShadowOffset { x, z } => {
                    let x = self.evaluate(host, &x, instruction.offset, opcode)? as i16;
                    let z = self.evaluate(host, &z, instruction.offset, opcode)? as i16;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "shadow commands require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetShadowOffset { actor, x, z });
                }
                InstructionKind::LockShadowRotation { rotation } => {
                    let rotation = self
                        .evaluate(host, &rotation, instruction.offset, opcode)?
                        .wrapping_shl(4);
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "shadow commands require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::LockShadowRotation { actor, rotation });
                }
                InstructionKind::UnlockShadowRotation => {
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "shadow commands require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::UnlockShadowRotation { actor });
                }
                InstructionKind::SetShadowAmplifier { factor } => {
                    let factor = self.evaluate(host, &factor, instruction.offset, opcode)?;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "shadow commands require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetShadowAmplifier { actor, factor });
                }
                InstructionKind::MoveCamera {
                    x,
                    y,
                    duration,
                    mode,
                } => {
                    let x = self.evaluate(host, &x, instruction.offset, opcode)? as i16;
                    let y = self.evaluate(host, &y, instruction.offset, opcode)? as i16;
                    let duration =
                        self.evaluate(host, &duration, instruction.offset, opcode)? as u16;
                    let mode = self.evaluate(host, &mode, instruction.offset, opcode)? as u32;
                    self.pc = next;
                    events.push(HostEvent::MoveCamera {
                        x,
                        y,
                        duration,
                        mode,
                    });
                }
                InstructionKind::ReleaseCamera { duration, mode } => {
                    let duration = self.evaluate(host, &duration, instruction.offset, opcode)?;
                    let mode = self.evaluate(host, &mode, instruction.offset, opcode)? as u32;
                    self.pc = next;
                    events.push(HostEvent::ReleaseCamera { duration, mode });
                }
                InstructionKind::EnableCameraServices {
                    active,
                    frame_count,
                    mode,
                } => {
                    let active = self.evaluate(host, &active, instruction.offset, opcode)? as u32;
                    let frame_count =
                        self.evaluate(host, &frame_count, instruction.offset, opcode)?;
                    let mode = self.evaluate(host, &mode, instruction.offset, opcode)? as u32;
                    self.pc = next;
                    events.push(HostEvent::EnableCameraServices {
                        active,
                        frame_count,
                        mode,
                    });
                }
                InstructionKind::SetCameraFollowHeight { height } => {
                    let height = self.evaluate(host, &height, instruction.offset, opcode)? as i16;
                    self.pc = next;
                    events.push(HostEvent::SetCameraFollowHeight { height });
                }
                InstructionKind::SetRandomBattles {
                    pattern,
                    scene_0,
                    scene_1,
                    scene_2,
                    scene_3,
                } => {
                    let pattern = self.evaluate(host, &pattern, instruction.offset, opcode)? as u8;
                    let scene_0 = self.evaluate(host, &scene_0, instruction.offset, opcode)? as u16;
                    let scene_1 = self.evaluate(host, &scene_1, instruction.offset, opcode)? as u16;
                    let scene_2 = self.evaluate(host, &scene_2, instruction.offset, opcode)? as u16;
                    let scene_3 = self.evaluate(host, &scene_3, instruction.offset, opcode)? as u16;
                    let scenes = [scene_0, scene_1, scene_2, scene_3];
                    self.pc = next;
                    events.push(HostEvent::SetRandomBattles { pattern, scenes });
                }
                InstructionKind::SetRandomBattleFrequency { frequency } => {
                    let frequency =
                        self.evaluate(host, &frequency, instruction.offset, opcode)? as u8;
                    self.pc = next;
                    events.push(HostEvent::SetRandomBattleFrequency { frequency });
                }
                InstructionKind::SetBackgroundColor { red, green, blue } => {
                    let red = self.evaluate(host, &red, instruction.offset, opcode)?;
                    let green = self.evaluate(host, &green, instruction.offset, opcode)?;
                    let blue = self.evaluate(host, &blue, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::SetBackgroundColor { red, green, blue });
                }
                InstructionKind::AddControllerMask { pad, buttons } => {
                    let pad = self.evaluate(host, &pad, instruction.offset, opcode)?;
                    let buttons = self.evaluate(host, &buttons, instruction.offset, opcode)?;
                    if buttons < 0 {
                        return Err(expression_error(
                            instruction.offset,
                            opcode,
                            "controller button list cannot be negative",
                        ));
                    }
                    let buttons = buttons as u32;
                    self.pc = next;
                    events.push(HostEvent::AddControllerMask { pad, buttons });
                }
                InstructionKind::RemoveControllerMask { pad, buttons } => {
                    let pad = self.evaluate(host, &pad, instruction.offset, opcode)?;
                    let buttons = self.evaluate(host, &buttons, instruction.offset, opcode)?;
                    if buttons < 0 {
                        return Err(expression_error(
                            instruction.offset,
                            opcode,
                            "controller button list cannot be negative",
                        ));
                    }
                    let buttons = buttons as u32;
                    self.pc = next;
                    events.push(HostEvent::RemoveControllerMask { pad, buttons });
                }
                InstructionKind::ShowObjectMeshes { object, mesh } => {
                    let object = self.evaluate(host, &object, instruction.offset, opcode)?;
                    if object < 0 {
                        return Err(expression_error(
                            instruction.offset,
                            opcode,
                            "object identity cannot be negative",
                        ));
                    }
                    let object = EntityId::new(object as u64);
                    let mesh = self.evaluate(host, &mesh, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::ShowObjectMeshes { object, mesh });
                }
                InstructionKind::HideObjectMeshes { object, mesh } => {
                    let object = self.evaluate(host, &object, instruction.offset, opcode)?;
                    if object < 0 {
                        return Err(expression_error(
                            instruction.offset,
                            opcode,
                            "object identity cannot be negative",
                        ));
                    }
                    let object = EntityId::new(object as u64);
                    let mesh = self.evaluate(host, &mesh, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::HideObjectMeshes { object, mesh });
                }
                InstructionKind::SetObjectIndex { index } => {
                    let index = self.evaluate(host, &index, instruction.offset, opcode)? as u8;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "current-object commands require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetObjectIndex { actor, index });
                }
                InstructionKind::SetAnimationFlags { flags, loop_count } => {
                    let flags = ((self.evaluate(host, &flags, instruction.offset, opcode)?)
                        .wrapping_shl(3)
                        & 56) as u8;
                    let loop_count =
                        self.evaluate(host, &loop_count, instruction.offset, opcode)? as u8;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "current-object commands require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetAnimationFlags {
                        actor,
                        flags,
                        loop_count,
                    });
                }
                InstructionKind::SetInactiveAnimation { animation } => {
                    let animation =
                        self.evaluate(host, &animation, instruction.offset, opcode)? as u16;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "current-object commands require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetInactiveAnimation { actor, animation });
                }
                InstructionKind::SetFollowFocusTurn { enabled } => {
                    let enabled = self.evaluate(host, &enabled, instruction.offset, opcode)? != 0;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "current-object commands require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetFollowFocusTurn { actor, enabled });
                }
                InstructionKind::SetAnimationLoopCount { count } => {
                    let count = self.evaluate(host, &count, instruction.offset, opcode)? as u8;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "current-object commands require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetAnimationLoopCount { actor, count });
                }
                InstructionKind::SetAnimationSpeed { speed } => {
                    let speed = self.evaluate(host, &speed, instruction.offset, opcode)? as u8;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "current-object commands require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetAnimationSpeed { actor, speed });
                }
                InstructionKind::SetCharacterAttribute { set, flags } => {
                    let flags = self.evaluate(host, &flags, instruction.offset, opcode)?;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "current-object commands require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetCharacterAttribute { actor, set, flags });
                }
                InstructionKind::SetTurnSpeed { speed } => {
                    let speed = self.evaluate(host, &speed, instruction.offset, opcode)? as u8;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "current-object commands require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetTurnSpeed { actor, speed });
                }
                InstructionKind::SetRunSpeedLimit { limit } => {
                    let limit = self.evaluate(host, &limit, instruction.offset, opcode)? as u8;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "current-object commands require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetRunSpeedLimit { actor, limit });
                }
                InstructionKind::TurnToAngle { angle } => {
                    let angle = (self.evaluate(host, &angle, instruction.offset, opcode)?)
                        .wrapping_shl(4) as i16;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "current-object commands require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::TurnToAngle { actor, angle });
                }
                InstructionKind::SetTimerTime { seconds } => {
                    let seconds = self.evaluate(host, &seconds, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::SetTimerTime { seconds });
                }
                InstructionKind::EnableDialogChoices {
                    choices,
                    default_choice,
                } => {
                    let choices = self.evaluate(host, &choices, instruction.offset, opcode)?;
                    let default_choice =
                        self.evaluate(host, &default_choice, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::EnableDialogChoices {
                        choices,
                        default_choice,
                    });
                }
                InstructionKind::SetFieldName { text } => {
                    let text = self.evaluate(host, &text, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::SetFieldName { text });
                }
                InstructionKind::AttachObject {
                    attached,
                    carrying,
                    bone,
                } => {
                    let attached = self.evaluate(host, &attached, instruction.offset, opcode)?;
                    if attached < 0 {
                        return Err(expression_error(
                            instruction.offset,
                            opcode,
                            "object identity cannot be negative",
                        ));
                    }
                    let carrying = self.evaluate(host, &carrying, instruction.offset, opcode)?;
                    if carrying < 0 {
                        return Err(expression_error(
                            instruction.offset,
                            opcode,
                            "object identity cannot be negative",
                        ));
                    }
                    let bone = self.evaluate(host, &bone, instruction.offset, opcode)?;
                    let attached = EntityId::new(attached as u64);
                    let carrying = EntityId::new(carrying as u64);
                    self.pc = next;
                    events.push(HostEvent::AttachObject {
                        attached,
                        carrying,
                        bone,
                    });
                }
                InstructionKind::DetachObject { attached } => {
                    let attached = self.evaluate(host, &attached, instruction.offset, opcode)?;
                    if attached < 0 {
                        return Err(expression_error(
                            instruction.offset,
                            opcode,
                            "object identity cannot be negative",
                        ));
                    }
                    let attached = EntityId::new(attached as u64);
                    self.pc = next;
                    events.push(HostEvent::DetachObject { attached });
                }
                InstructionKind::TrackStart { x, z } => {
                    let x = self.evaluate(host, &x, instruction.offset, opcode)? as i16;
                    let z = self.evaluate(host, &z, instruction.offset, opcode)? as i16;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "current-object commands require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::TrackStart { actor, x, z });
                }
                InstructionKind::TrackSetLast { x, z } => {
                    let x = self.evaluate(host, &x, instruction.offset, opcode)? as i16;
                    let z = self.evaluate(host, &z, instruction.offset, opcode)? as i16;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "current-object commands require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::TrackSetLast { actor, x, z });
                }
                InstructionKind::TrackAdd => {
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "current-object commands require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::TrackAdd { actor });
                }
                InstructionKind::SetSoundPosition { x, z, y, volume } => {
                    let x = self.evaluate(host, &x, instruction.offset, opcode)?;
                    let z = self.evaluate(host, &z, instruction.offset, opcode)?;
                    let y = self.evaluate(host, &y, instruction.offset, opcode)?;
                    let volume = self.evaluate(host, &volume, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::SetSoundPosition {
                        position: [x, z, y],
                        volume,
                    });
                }
                InstructionKind::SetSoundObjectPosition { object, volume } => {
                    let object = self.evaluate(host, &object, instruction.offset, opcode)?;
                    if object < 0 {
                        return Err(expression_error(
                            instruction.offset,
                            opcode,
                            "object identity cannot be negative",
                        ));
                    }
                    let object = EntityId::new(object as u64);
                    let volume = self.evaluate(host, &volume, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::SetSoundObjectPosition { object, volume });
                }
                InstructionKind::ShowTimer { show } => {
                    let show = self.evaluate(host, &show, instruction.offset, opcode)? != 0;
                    self.pc = next;
                    events.push(HostEvent::ShowTimer { show });
                }
                InstructionKind::RunWorldCode { code, value } => {
                    let code = self.evaluate(host, &code, instruction.offset, opcode)?;
                    let value = self.evaluate(host, &value, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::RunWorldCode { code, value });
                }
                InstructionKind::EnableVictoryPose { character, enabled } => {
                    let character = self.evaluate(host, &character, instruction.offset, opcode)?;
                    let enabled = self.evaluate(host, &enabled, instruction.offset, opcode)? != 0;
                    self.pc = next;
                    events.push(HostEvent::EnableVictoryPose { character, enabled });
                }
                InstructionKind::RemoveParty { character } => {
                    let character = self.evaluate(host, &character, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::RemoveParty { character });
                }
                InstructionKind::SetPlayerName { character, text } => {
                    let character = self.evaluate(host, &character, instruction.offset, opcode)?;
                    let text = self.evaluate(host, &text, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::SetPlayerName { character, text });
                }
                InstructionKind::SetupJump { x, z, y, steps } => {
                    let x = self.evaluate(host, &x, instruction.offset, opcode)? as i16;
                    let z = self.evaluate(host, &z, instruction.offset, opcode)? as i16;
                    // The reference stores the second operand negated
                    // (`actor.jumpy = (short)-this.getv2()`); keep that sign
                    // convention here so the host receives actor-state values.
                    let y = (self.evaluate(host, &y, instruction.offset, opcode)?).wrapping_neg()
                        as i16;
                    let mut steps = self.evaluate(host, &steps, instruction.offset, opcode)? as u8;
                    if steps == 0 {
                        steps = 8;
                    }
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "current-object commands require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetupJump {
                        actor,
                        x,
                        z,
                        y,
                        steps,
                    });
                    // End the step so a following JUMP3 observes the applied
                    // jump state; the reference's immediate state updates are
                    // modeled here by the apply boundary.
                    break;
                }
                InstructionKind::SetDialogSignal { signal } => {
                    let signal = self.evaluate(host, &signal, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::SetDialogSignal { signal });
                }
                InstructionKind::SetTileLoopType {
                    overlay,
                    screen_anchored,
                } => {
                    let overlay = self.evaluate(host, &overlay, instruction.offset, opcode)?;
                    let screen_anchored =
                        self.evaluate(host, &screen_anchored, instruction.offset, opcode)? != 0;
                    self.pc = next;
                    events.push(HostEvent::SetTileLoopType {
                        overlay,
                        screen_anchored,
                    });
                }
                InstructionKind::ApplyFadeFilter {
                    mode,
                    frame,
                    unknown: _,
                    cyan,
                    magenta,
                    yellow,
                } => {
                    // The reference discards the third operand; fade-mode
                    // mapping (`mode & 2`), timing, and color presentation
                    // stay host-owned, as do map-specific color patches.
                    let mode = self.evaluate(host, &mode, instruction.offset, opcode)?;
                    let frame = self.evaluate(host, &frame, instruction.offset, opcode)?;
                    let cyan = self.evaluate(host, &cyan, instruction.offset, opcode)?;
                    let magenta = self.evaluate(host, &magenta, instruction.offset, opcode)?;
                    let yellow = self.evaluate(host, &yellow, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::ApplyFadeFilter {
                        mode,
                        frame,
                        cyan,
                        magenta,
                        yellow,
                    });
                }
                InstructionKind::OpenMenu { menu, subject } => {
                    let menu = self.evaluate(host, &menu, instruction.offset, opcode)?;
                    let subject = self.evaluate(host, &subject, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::OpenMenu { menu, subject });
                }
                InstructionKind::SetActiveTimeEventIcon { mode } => {
                    let mode = self.evaluate(host, &mode, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::SetActiveTimeEventIcon { mode });
                }
                InstructionKind::SetObjectOvalRatio { ratio } => {
                    let ratio = self.evaluate(host, &ratio, instruction.offset, opcode)?;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "current-object commands require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetObjectOvalRatio { actor, ratio });
                }
                InstructionKind::TerminateBattle => {
                    self.pc = next;
                    events.push(HostEvent::TerminateBattle);
                }
                InstructionKind::ResetIdleTimer => {
                    self.pc = next;
                    events.push(HostEvent::ResetIdleTimer);
                }
                InstructionKind::CloseAllWindows => {
                    self.pc = next;
                    events.push(HostEvent::CloseAllWindows);
                }
                InstructionKind::AlignActorToCameraEye => {
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "current-object commands require current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::AlignActorToCameraEye { actor });
                }
                InstructionKind::ProgressJump => {
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "current-object commands require current actor context",
                        ));
                    };
                    let active = host
                        .actor_jump_is_active(actor)
                        .map_err(|error| VmError::host(error, instruction.offset, opcode))?;
                    self.pc = if active { instruction.offset } else { next };
                    events.push(HostEvent::ProgressJump { actor });
                    self.status = VmStatus::Running;
                    break;
                }
                InstructionKind::QueueEnemyAttack { attack } => {
                    // SV_Target reads and ATB/counter queueing are host-owned.
                    let attack = self.evaluate(host, &attack, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::QueueEnemyAttack { attack });
                }
                InstructionKind::ShowObjectWindow {
                    object,
                    window,
                    flags,
                    text,
                    opcode: window_opcode,
                } => {
                    let object = self.evaluate(host, &object, instruction.offset, opcode)?;
                    if object < 0 {
                        return Err(expression_error(
                            instruction.offset,
                            opcode,
                            "object identity cannot be negative",
                        ));
                    }
                    let object = EntityId::new(object as u64);
                    let window = self.evaluate_u8(host, &window, instruction.offset, opcode)?;
                    let flags = self.evaluate_u8(host, &flags, instruction.offset, opcode)?;
                    let text = self.evaluate_u16(host, &text, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::ShowObjectWindow {
                        object,
                        window,
                        flags,
                        text,
                    });
                    if window_opcode == 0x95 {
                        self.wait = Some(WaitState::Window(window));
                        self.status = VmStatus::Waiting;
                    }
                }
                InstructionKind::RunSpsCode {
                    object,
                    code,
                    argument,
                    value,
                } => {
                    let object = self.evaluate(host, &object, instruction.offset, opcode)?;
                    let code = self.evaluate(host, &code, instruction.offset, opcode)?;
                    let argument = self.evaluate(host, &argument, instruction.offset, opcode)?;
                    let value = [
                        self.evaluate(host, &value[0], instruction.offset, opcode)? as i16,
                        self.evaluate(host, &value[1], instruction.offset, opcode)? as i16,
                    ];
                    self.pc = next;
                    events.push(HostEvent::RunSpsCode {
                        object,
                        code,
                        argument,
                        value,
                    });
                }
                InstructionKind::WalkTowardObject { target } => {
                    let target = self.evaluate(host, &target, instruction.offset, opcode)?;
                    if target < 0 {
                        return Err(expression_error(
                            instruction.offset,
                            opcode,
                            "object identity cannot be negative",
                        ));
                    }
                    let target = EntityId::new(target as u64);
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "walk requires current actor context",
                        ));
                    };
                    self.pc = instruction.offset;
                    self.wait = Some(WaitState::ActorMovement { next });
                    self.status = VmStatus::Waiting;
                    events.push(HostEvent::WalkTowardObject { actor, target });
                }
                InstructionKind::PreventWindowInit => {
                    self.pc = next;
                    events.push(HostEvent::PreventWindowInit);
                }
                InstructionKind::LockCameraFollow => {
                    self.pc = next;
                    events.push(HostEvent::LockCameraFollow);
                }
                InstructionKind::UnlockCameraFollow => {
                    self.pc = next;
                    events.push(HostEvent::UnlockCameraFollow);
                }
                InstructionKind::TurnObjectToAngle { target, angle } => {
                    let target = self.evaluate(host, &target, instruction.offset, opcode)?;
                    if target < 0 {
                        return Err(expression_error(
                            instruction.offset,
                            opcode,
                            "object identity cannot be negative",
                        ));
                    }
                    let actor = EntityId::new(target as u64);
                    let angle = self.evaluate(host, &angle, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::TurnObjectToAngle { actor, angle });
                }
                InstructionKind::ReturnEntryFunctions { object } => {
                    let object = self.evaluate(host, &object, instruction.offset, opcode)?;
                    if object < 0 {
                        return Err(expression_error(
                            instruction.offset,
                            opcode,
                            "object identity cannot be negative",
                        ));
                    }
                    let object = EntityId::new(object as u64);
                    self.pc = next;
                    events.push(HostEvent::ReturnEntryFunctions { object });
                }
                InstructionKind::ResetFieldName => {
                    self.pc = next;
                    events.push(HostEvent::ResetFieldName);
                }
                InstructionKind::DeleteAllCards => {
                    self.pc = next;
                    events.push(HostEvent::DeleteAllCards);
                }
                InstructionKind::WaitObjectTurn { target } => {
                    let target = self.evaluate(host, &target, instruction.offset, opcode)?;
                    if target < 0 {
                        return Err(expression_error(
                            instruction.offset,
                            opcode,
                            "object identity cannot be negative",
                        ));
                    }
                    let actor = EntityId::new(target as u64);
                    self.pc = next;
                    self.wait = Some(WaitState::ObjectTurn { actor });
                    self.status = VmStatus::Waiting;
                }
                InstructionKind::WaitObjectAnimation { target } => {
                    let target = self.evaluate(host, &target, instruction.offset, opcode)?;
                    if target < 0 {
                        return Err(expression_error(
                            instruction.offset,
                            opcode,
                            "object identity cannot be negative",
                        ));
                    }
                    let actor = EntityId::new(target as u64);
                    self.pc = next;
                    self.wait = Some(WaitState::ObjectAnimation { actor });
                    self.status = VmStatus::Waiting;
                }
                InstructionKind::QueueSpecialAttack { attack } => {
                    // SV_Target reads and battle-state checks are host-owned.
                    let attack = self.evaluate(host, &attack, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::QueueSpecialAttack { attack });
                }
                InstructionKind::ChangeDisc { destination } => {
                    let destination =
                        self.evaluate(host, &destination, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::ChangeDisc { destination });
                }
                InstructionKind::SetSimulationActive { simulation, active } => {
                    let simulation =
                        self.evaluate(host, &simulation, instruction.offset, opcode)?;
                    let active = self.evaluate(host, &active, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::SetSimulationActive { simulation, active });
                }
                InstructionKind::Watch { .. } => {
                    return Err(VmError::new(
                        VmErrorKind::UnsupportedOpcode,
                        instruction.offset,
                        Some(opcode),
                        "WATCH behavior is not proven by the pinned source",
                    ));
                }
                InstructionKind::SetSimulationFlags { simulation, flags } => {
                    let simulation =
                        self.evaluate(host, &simulation, instruction.offset, opcode)?;
                    let flags = self.evaluate(host, &flags, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::SetSimulationFlags { simulation, flags });
                }
                InstructionKind::SetSimulationAlgorithm {
                    simulation,
                    algorithm,
                } => {
                    let simulation =
                        self.evaluate(host, &simulation, instruction.offset, opcode)?;
                    let algorithm = self.evaluate(host, &algorithm, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::SetSimulationAlgorithm {
                        simulation,
                        algorithm,
                    });
                }
                InstructionKind::SetSimulationDelta { simulation, delta } => {
                    let simulation =
                        self.evaluate(host, &simulation, instruction.offset, opcode)?;
                    let delta = [
                        self.evaluate(host, &delta[0], instruction.offset, opcode)? as i16,
                        self.evaluate(host, &delta[1], instruction.offset, opcode)? as i16,
                    ];
                    self.pc = next;
                    events.push(HostEvent::SetSimulationDelta { simulation, delta });
                }
                InstructionKind::SetSimulationAxis { simulation, axis } => {
                    let simulation =
                        self.evaluate(host, &simulation, instruction.offset, opcode)?;
                    let axis = self.evaluate(host, &axis, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::SetSimulationAxis { simulation, axis });
                }
                InstructionKind::ShowNavigationAnimationFrame { animation, frame } => {
                    let animation = self.evaluate(host, &animation, instruction.offset, opcode)?;
                    let frame = self.evaluate(host, &frame, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::ShowNavigationAnimationFrame { animation, frame });
                }
                InstructionKind::SetUserControl { enabled, .. } => {
                    self.pc = next;
                    events.push(HostEvent::SetUserControl { enabled });
                }
                InstructionKind::RaiseWindows => {
                    self.pc = next;
                    events.push(HostEvent::RaiseWindows);
                }
                InstructionKind::CalculateScreenOrigin => {
                    self.pc = next;
                    events.push(HostEvent::CalculateScreenOrigin);
                }
                InstructionKind::RunSharedScript { entry } => {
                    let entry = self.evaluate(host, &entry, instruction.offset, opcode)?;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "shared script requires current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::RunSharedScript { actor, entry });
                }
                InstructionKind::TurnTowardObject { target, speed } => {
                    let target = self.evaluate(host, &target, instruction.offset, opcode)?;
                    let speed = self.evaluate(host, &speed, instruction.offset, opcode)?;
                    if target < 0 {
                        return Err(expression_error(
                            instruction.offset,
                            opcode,
                            "object identity cannot be negative",
                        ));
                    }
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "turn requires current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::TurnTowardObject {
                        actor,
                        target,
                        speed,
                    });
                }
                InstructionKind::ShowBubbleIcon { icon } => {
                    let icon = self.evaluate(host, &icon, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::ShowBubbleIcon { icon });
                }
                InstructionKind::SetFieldCamera { camera } => {
                    let camera = self.evaluate(host, &camera, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::SetFieldCamera { camera });
                }
                InstructionKind::EndSharedScript => {
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "shared script termination requires current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::EndSharedScript { actor });
                }
                InstructionKind::SetMenuControl { enabled, .. } => {
                    self.pc = next;
                    events.push(HostEvent::SetMenuControl { enabled });
                    if !enabled {
                        // Memoria MENUOFF returns FLOW_STATE_WAIT: yield after the command.
                        break;
                    }
                }
                InstructionKind::SetTileAnimationRate {
                    animation,
                    frame_rate,
                } => {
                    let animation = self.evaluate(host, &animation, instruction.offset, opcode)?;
                    let frame_rate =
                        self.evaluate(host, &frame_rate, instruction.offset, opcode)? as i16;
                    self.pc = next;
                    events.push(HostEvent::SetTileAnimationRate {
                        animation,
                        frame_rate,
                    });
                }
                InstructionKind::SetTileAnimationFrameWait {
                    animation,
                    frame,
                    wait,
                } => {
                    let animation = self.evaluate(host, &animation, instruction.offset, opcode)?;
                    let frame = self.evaluate(host, &frame, instruction.offset, opcode)?;
                    let wait = self.evaluate(host, &wait, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::SetTileAnimationFrameWait {
                        animation,
                        frame,
                        wait,
                    });
                }
                InstructionKind::SetTileAnimationFlags { animation, flags } => {
                    let animation = self.evaluate(host, &animation, instruction.offset, opcode)?;
                    let flags = self.evaluate(host, &flags, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::SetTileAnimationFlags { animation, flags });
                }
                InstructionKind::SetTileAnimationRange {
                    animation,
                    first_frame,
                    last_frame,
                } => {
                    let animation = self.evaluate(host, &animation, instruction.offset, opcode)?;
                    let first_frame =
                        self.evaluate(host, &first_frame, instruction.offset, opcode)?;
                    let last_frame =
                        self.evaluate(host, &last_frame, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::SetTileAnimationRange {
                        animation,
                        first_frame,
                        last_frame,
                    });
                }
                InstructionKind::IncrementFrogCount => {
                    self.pc = next;
                    events.push(HostEvent::IncrementFrogCount);
                }
                InstructionKind::SetTileAnimationFrame { animation, frame } => {
                    let animation = self.evaluate(host, &animation, instruction.offset, opcode)?;
                    let frame = self.evaluate(host, &frame, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::SetTileAnimationFrame { animation, frame });
                }
                InstructionKind::OpenPartyMenu {
                    minimum_party_size,
                    fixed_members,
                } => {
                    let minimum_party_size =
                        self.evaluate(host, &minimum_party_size, instruction.offset, opcode)?;
                    let fixed_members =
                        self.evaluate(host, &fixed_members, instruction.offset, opcode)?;
                    self.pc = next;
                    self.wait = Some(WaitState::PartyMenu);
                    self.status = VmStatus::Waiting;
                    events.push(HostEvent::OpenPartyMenu {
                        minimum_party_size,
                        fixed_members,
                    });
                }
                InstructionKind::InitializeCharacter {
                    character,
                    enable_leveling,
                    equipment_set,
                    category,
                    menu_preset,
                } => {
                    let character = self.evaluate(host, &character, instruction.offset, opcode)?;
                    let enable_leveling =
                        self.evaluate(host, &enable_leveling, instruction.offset, opcode)? != 0;
                    let equipment_set =
                        self.evaluate(host, &equipment_set, instruction.offset, opcode)?;
                    let category = self.evaluate(host, &category, instruction.offset, opcode)?;
                    let menu_preset =
                        self.evaluate(host, &menu_preset, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::InitializeCharacter {
                        character,
                        enable_leveling,
                        equipment_set,
                        category,
                        menu_preset,
                    });
                }
                InstructionKind::StopTextureAnimation { object, animation } => {
                    let object = self.evaluate(host, &object, instruction.offset, opcode)?;
                    let animation = self.evaluate(host, &animation, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::StopTextureAnimation { object, animation });
                }
                InstructionKind::RunBattleCode { code, value } => {
                    let code = self.evaluate(host, &code, instruction.offset, opcode)? as u32;
                    let value = self.evaluate(host, &value, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::RunBattleCode { code, value });
                }
                InstructionKind::SetBattleMusicContinuation(enabled) => {
                    let enabled = self.evaluate(host, &enabled, instruction.offset, opcode)? != 0;
                    self.pc = next;
                    events.push(HostEvent::SetBattleMusicContinuation { enabled });
                }
                InstructionKind::RunModelCode { code, arguments } => {
                    let code = self.evaluate(host, &code, instruction.offset, opcode)?;
                    let [first, second, third] = arguments;
                    let arguments = [
                        self.evaluate(host, &first, instruction.offset, opcode)? as i16,
                        self.evaluate(host, &second, instruction.offset, opcode)? as i16,
                        self.evaluate(host, &third, instruction.offset, opcode)? as i16,
                    ];
                    let Some(object) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "model code requires current object context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::RunModelCode {
                        object,
                        code,
                        arguments,
                    });
                }
                InstructionKind::SetModelLight {
                    model,
                    color,
                    shadow_intensity,
                    shadow_scale,
                } => {
                    let model = self.evaluate(host, &model, instruction.offset, opcode)?;
                    let red = self.evaluate(host, &color[0], instruction.offset, opcode)? as u8;
                    let green = self.evaluate(host, &color[1], instruction.offset, opcode)? as u8;
                    let blue = self.evaluate(host, &color[2], instruction.offset, opcode)? as u8;
                    let shadow_intensity =
                        self.evaluate(host, &shadow_intensity, instruction.offset, opcode)? as i8;
                    let shadow_scale =
                        self.evaluate(host, &shadow_scale, instruction.offset, opcode)? as i8;
                    self.pc = next;
                    events.push(HostEvent::SetModelLight {
                        model,
                        color: [red, green, blue],
                        shadow_intensity,
                        shadow_scale,
                    });
                }
                InstructionKind::AddBattleStatus {
                    target,
                    status,
                    permanent,
                    arguments,
                } => {
                    let target = self.evaluate(host, &target, instruction.offset, opcode)? as u16;
                    let status = self.evaluate(host, &status, instruction.offset, opcode)?;
                    let permanent =
                        self.evaluate(host, &permanent, instruction.offset, opcode)? != 0;
                    let [first, second, third] = arguments;
                    let arguments = [
                        self.evaluate(host, &first, instruction.offset, opcode)?,
                        self.evaluate(host, &second, instruction.offset, opcode)?,
                        self.evaluate(host, &third, instruction.offset, opcode)?,
                    ];
                    self.pc = next;
                    events.push(HostEvent::AddBattleStatus {
                        target,
                        status,
                        permanent,
                        arguments,
                    });
                }
                InstructionKind::ChangeElementalAffinity {
                    target,
                    kind,
                    element,
                    add,
                } => {
                    let target = self.evaluate(host, &target, instruction.offset, opcode)? as u16;
                    let affinity_kind = self.evaluate(host, &kind, instruction.offset, opcode)?;
                    let element = self.evaluate(host, &element, instruction.offset, opcode)?;
                    let add = self.evaluate(host, &add, instruction.offset, opcode)? != 0;
                    self.pc = next;
                    events.push(HostEvent::ChangeElementalAffinity {
                        target,
                        affinity_kind,
                        element,
                        add,
                    });
                }
                InstructionKind::SetRunEnabled { enabled, .. } => {
                    self.pc = next;
                    events.push(HostEvent::SetRunEnabled { enabled });
                }
                InstructionKind::SetHereIcon(mode) => {
                    let mode = self.evaluate(host, &mode, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::SetHereIcon { mode });
                }
                InstructionKind::SetPartyAvailability(mask) => {
                    let mask = self.evaluate(host, &mask, instruction.offset, opcode)?;
                    let mut available = mask & !0xf00;
                    available |= (mask >> 1) & 0x700;
                    if mask & 0x100 != 0 {
                        available |= 0x800;
                    }
                    self.pc = next;
                    events.push(HostEvent::SetPartyAvailability { mask: available });
                }
                InstructionKind::MoveTile {
                    overlay,
                    x,
                    y,
                    depth,
                } => {
                    let overlay = self.evaluate(host, &overlay, instruction.offset, opcode)?;
                    let x = self.evaluate(host, &x, instruction.offset, opcode)? as i16;
                    let y = self.evaluate(host, &y, instruction.offset, opcode)? as i16;
                    let depth = self.evaluate(host, &depth, instruction.offset, opcode)? as i16;
                    self.pc = next;
                    events.push(HostEvent::MoveTile {
                        overlay,
                        offset: [x, y, depth],
                    });
                }
                InstructionKind::SetTileScrollWithOffset {
                    overlay,
                    enabled,
                    speed,
                    offset,
                    is_x_offset,
                } => {
                    let overlay = self.evaluate(host, &overlay, instruction.offset, opcode)?;
                    let enabled = self.evaluate(host, &enabled, instruction.offset, opcode)?;
                    let speed = self.evaluate(host, &speed, instruction.offset, opcode)? as i16;
                    let offset = self.evaluate(host, &offset, instruction.offset, opcode)? as i16;
                    let is_x_offset =
                        self.evaluate(host, &is_x_offset, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::SetTileScrollWithOffset {
                        overlay,
                        enabled,
                        speed,
                        offset,
                        is_x_offset,
                    });
                }
                InstructionKind::SetTileViewportAlpha { viewport, alpha } => {
                    let viewport = self.evaluate(host, &viewport, instruction.offset, opcode)?;
                    let alpha = [
                        self.evaluate(host, &alpha[0], instruction.offset, opcode)? as i16,
                        self.evaluate(host, &alpha[1], instruction.offset, opcode)? as i16,
                    ];
                    self.pc = next;
                    events.push(HostEvent::SetTileViewportAlpha { viewport, alpha });
                }
                InstructionKind::SetCameraBounds { camera, bounds } => {
                    let camera = self.evaluate(host, &camera, instruction.offset, opcode)?;
                    let [min_x, max_x, min_y, max_y] = bounds;
                    let bounds = [
                        self.evaluate(host, &min_x, instruction.offset, opcode)? as i16,
                        self.evaluate(host, &max_x, instruction.offset, opcode)? as i16,
                        self.evaluate(host, &min_y, instruction.offset, opcode)? as i16,
                        self.evaluate(host, &max_y, instruction.offset, opcode)? as i16,
                    ];
                    self.pc = next;
                    events.push(HostEvent::SetCameraBounds { camera, bounds });
                }
                InstructionKind::SetCharacterRow { character, row } => {
                    let character = self.evaluate(host, &character, instruction.offset, opcode)?;
                    let row = self.evaluate(host, &row, instruction.offset, opcode)? as u8;
                    self.pc = next;
                    events.push(HostEvent::SetCharacterRow { character, row });
                }
                InstructionKind::ClearPartyStatus {
                    character,
                    statuses,
                } => {
                    let character = self.evaluate(host, &character, instruction.offset, opcode)?;
                    let statuses = self.evaluate(host, &statuses, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::ClearPartyStatus {
                        character,
                        statuses,
                    });
                }
                InstructionKind::SetActorPitchRoll { pitch, roll } => {
                    let pitch = self
                        .evaluate(host, &pitch, instruction.offset, opcode)?
                        .wrapping_shl(4) as i16;
                    let roll = self
                        .evaluate(host, &roll, instruction.offset, opcode)?
                        .wrapping_shl(4) as i16;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "pitch/roll requires current actor context",
                        ));
                    };
                    self.pc = next;
                    events.push(HostEvent::SetActorPitchRoll { actor, pitch, roll });
                }
                InstructionKind::AttachTile {
                    overlay,
                    x,
                    y,
                    filter_mode,
                    red,
                    green,
                    blue,
                } => {
                    let overlay = self.evaluate(host, &overlay, instruction.offset, opcode)?;
                    let x = self.evaluate(host, &x, instruction.offset, opcode)? as i16;
                    let y = self.evaluate(host, &y, instruction.offset, opcode)? as i16;
                    let filter_mode =
                        self.evaluate(host, &filter_mode, instruction.offset, opcode)? as i8;
                    let red = self.evaluate(host, &red, instruction.offset, opcode)? as u8;
                    let green = self.evaluate(host, &green, instruction.offset, opcode)? as u8;
                    let blue = self.evaluate(host, &blue, instruction.offset, opcode)? as u8;
                    self.pc = next;
                    events.push(HostEvent::AttachTile {
                        overlay,
                        x,
                        y,
                        filter_mode,
                        color: [red, green, blue],
                    });
                }
                InstructionKind::AddItem { item, count } => {
                    let item = self.evaluate(host, &item, instruction.offset, opcode)?;
                    let count = self.evaluate(host, &count, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::AddItem { item, count });
                }
                InstructionKind::RemoveItem { item, count } => {
                    let item = self.evaluate(host, &item, instruction.offset, opcode)?;
                    let count = self.evaluate(host, &count, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::RemoveItem { item, count });
                }
                InstructionKind::AddGil(amount) => {
                    let amount = self.evaluate(host, &amount, instruction.offset, opcode)? as u32;
                    self.pc = next;
                    events.push(HostEvent::AddGil { amount });
                }
                InstructionKind::RemoveGil(amount) => {
                    let amount = self.evaluate(host, &amount, instruction.offset, opcode)? as u32;
                    self.pc = next;
                    events.push(HostEvent::RemoveGil { amount });
                }
                InstructionKind::SetPartyHp { character, value } => {
                    let character = self.evaluate(host, &character, instruction.offset, opcode)?;
                    let hp = self.evaluate(host, &value, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::SetPartyHp { character, hp });
                }
                InstructionKind::SetPartyMp { character, value } => {
                    let character = self.evaluate(host, &character, instruction.offset, opcode)?;
                    let mp = self.evaluate(host, &value, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::SetPartyMp { character, mp });
                }
                InstructionKind::ClearAbilityAp { character, ability } => {
                    let character = self.evaluate(host, &character, instruction.offset, opcode)?;
                    let ability = self.evaluate(host, &ability, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::ClearAbilityAp { character, ability });
                }
                InstructionKind::MasterAbility { character, ability } => {
                    let character = self.evaluate(host, &character, instruction.offset, opcode)?;
                    let ability = self.evaluate(host, &ability, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::MasterAbility { character, ability });
                }
                InstructionKind::WalkActor { x, z } => {
                    let x = self.evaluate(host, &x, instruction.offset, opcode)? as i16;
                    let z = self.evaluate(host, &z, instruction.offset, opcode)? as i16;
                    let Some(actor) = self.entity else {
                        return Err(VmError::new(
                            VmErrorKind::Host,
                            instruction.offset,
                            Some(opcode),
                            "walk requires current actor context",
                        ));
                    };
                    self.pc = instruction.offset;
                    self.wait = Some(WaitState::ActorMovement { next });
                    self.status = VmStatus::Waiting;
                    events.push(HostEvent::WalkActor {
                        actor,
                        destination: [x, z],
                    });
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
        opcode: u32,
    ) -> Result<i32, VmError> {
        #[derive(Clone, Copy)]
        enum StackTarget {
            Variable(VariableRef),
            Member(u8),
            SystemList(u8),
        }

        #[derive(Clone, Copy)]
        struct StackValue {
            value: i32,
            target: Option<StackTarget>,
        }

        fn read_value(
            host: &mut dyn LegacyEventHost,
            value: StackValue,
            offset: u32,
            opcode: u32,
        ) -> Result<i32, VmError> {
            if let Some(StackTarget::Member(member)) = value.target {
                let object = host
                    .select_object_member_target(None)
                    .and_then(|object| host.read_object_member(ObjectMemberRef { object, member }))
                    .map_err(|error| VmError::host(error, offset, opcode))?;
                Ok(object)
            } else if let Some(StackTarget::SystemList(index)) = value.target {
                host.read_system_list(index)
                    .map_err(|error| VmError::host(error, offset, opcode))
            } else {
                Ok(value.value)
            }
        }

        let mut stack: Vec<StackValue> = Vec::new();
        let logical_plans = logical_plans(&expression.0, offset, opcode)?;
        let mut index = 0;
        while index < expression.0.len() {
            if let Some(plan) = logical_plans.get(&index) {
                let left = *stack.last().ok_or_else(|| {
                    expression_error(offset, opcode, "missing short-circuit left operand")
                })?;
                let left = read_value(host, left, offset, opcode)?;
                let skip = if plan.is_and { left == 0 } else { left != 0 };
                if skip {
                    stack.pop();
                    stack.push(StackValue {
                        value: i32::from(!plan.is_and),
                        target: None,
                    });
                    index = plan.operator_index + 1;
                    continue;
                }
            }
            let operation = &expression.0[index];
            let binary = |stack: &mut Vec<StackValue>,
                          _operation: &ExpressionOp,
                          host: &mut dyn LegacyEventHost| {
                let right = stack
                    .pop()
                    .ok_or_else(|| expression_error(offset, opcode, "missing right operand"))?;
                let right = read_value(host, right, offset, opcode)?;
                let left = stack
                    .pop()
                    .ok_or_else(|| expression_error(offset, opcode, "missing left operand"))?;
                let left = read_value(host, left, offset, opcode)?;
                Ok::<_, VmError>((left, right))
            };
            match operation {
                ExpressionOp::Constant(value) => stack.push(StackValue {
                    value: *value,
                    target: None,
                }),
                ExpressionOp::Variable(reference) => stack.push(StackValue {
                    value: self.variable(reference.scope, reference.index),
                    target: Some(StackTarget::Variable(*reference)),
                }),
                ExpressionOp::Member(member) => stack.push(StackValue {
                    value: 0,
                    target: Some(StackTarget::Member(*member)),
                }),
                ExpressionOp::Negate => {
                    let value = stack
                        .pop()
                        .ok_or_else(|| expression_error(offset, opcode, "missing operand"))?;
                    let value = read_value(host, value, offset, opcode)?;
                    stack.push(StackValue {
                        value: value.wrapping_neg(),
                        target: None,
                    });
                }
                ExpressionOp::Not => {
                    let value = stack
                        .pop()
                        .ok_or_else(|| expression_error(offset, opcode, "missing operand"))?;
                    let value = read_value(host, value, offset, opcode)?;
                    stack.push(StackValue {
                        value: i32::from(value <= 0),
                        target: None,
                    });
                }
                ExpressionOp::Complement => {
                    let value = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing complement operand")
                    })?;
                    let value = read_value(host, value, offset, opcode)?;
                    stack.push(StackValue {
                        value: !value,
                        target: None,
                    });
                }
                ExpressionOp::NoOp => {}
                ExpressionOp::ConsumeAndZero => {
                    let value = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing consumed operand")
                    })?;
                    read_value(host, value, offset, opcode)?;
                    stack.push(StackValue {
                        value: 0,
                        target: None,
                    });
                }
                ExpressionOp::Sine => {
                    let value = stack
                        .pop()
                        .ok_or_else(|| expression_error(offset, opcode, "missing sine operand"))?;
                    let angle = read_value(host, value, offset, opcode)?.wrapping_shl(4);
                    let radians = angle as f32 / 4096.0 * 360.0 * 0.017_453_292;
                    stack.push(StackValue {
                        value: (radians.sin() * 4096.0) as i32,
                        target: None,
                    });
                }
                ExpressionOp::SineUnshifted => {
                    let angle = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing unshifted sine operand")
                    })?;
                    let angle = read_value(host, angle, offset, opcode)?;
                    let radians = angle as f32 / 4096.0 * 360.0 * 0.017_453_292;
                    stack.push(StackValue {
                        value: (radians.sin() * 4096.0) as i32,
                        target: None,
                    });
                }
                ExpressionOp::Cosine => {
                    let angle = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing cosine operand")
                    })?;
                    let angle = read_value(host, angle, offset, opcode)?;
                    let radians = angle as f32 / 4096.0 * 360.0 * 0.017_453_292;
                    stack.push(StackValue {
                        value: (radians.cos() * 4096.0) as i32,
                        target: None,
                    });
                }
                ExpressionOp::CosineShifted => {
                    let angle = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing shifted cosine operand")
                    })?;
                    let angle = read_value(host, angle, offset, opcode)?.wrapping_shl(4);
                    let radians = angle as f32 / 4096.0 * 360.0 * 0.017_453_292;
                    stack.push(StackValue {
                        value: (radians.cos() * 4096.0) as i32,
                        target: None,
                    });
                }
                ExpressionOp::AngleToObject => {
                    let value = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing angle-target operand")
                    })?;
                    let uid = read_value(host, value, offset, opcode)?;
                    let source = self.entity.ok_or_else(|| {
                        expression_error(offset, opcode, "angle target requires object context")
                    })?;
                    let target = host
                        .resolve_object_uid(uid)
                        .map_err(|error| VmError::host(error, offset, opcode))?;
                    let target_position = host
                        .read_object_position(target)
                        .map_err(|error| VmError::host(error, offset, opcode))?;
                    let source_position = host
                        .read_object_position(source)
                        .map_err(|error| VmError::host(error, offset, opcode))?;
                    let delta_x = (target_position[0] - source_position[0]) as i32;
                    let delta_z = (target_position[2] - source_position[2]) as i32;
                    stack.push(StackValue {
                        value: angle2(host, delta_x, delta_z, offset, opcode)?,
                        target: None,
                    });
                }
                ExpressionOp::DistanceToObject => {
                    let value = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing distance-target operand")
                    })?;
                    let uid = read_value(host, value, offset, opcode)?;
                    let source = self.entity.ok_or_else(|| {
                        expression_error(offset, opcode, "distance target requires object context")
                    })?;
                    let target = host
                        .resolve_object_uid(uid)
                        .map_err(|error| VmError::host(error, offset, opcode))?;
                    let target_position = host
                        .read_object_position(target)
                        .map_err(|error| VmError::host(error, offset, opcode))?;
                    let source_position = host
                        .read_object_position(source)
                        .map_err(|error| VmError::host(error, offset, opcode))?;
                    let delta_x = target_position[0] - source_position[0];
                    let delta_z = target_position[2] - source_position[2];
                    let squared = f64::from(delta_x) * f64::from(delta_x)
                        + f64::from(delta_z) * f64::from(delta_z);
                    stack.push(StackValue {
                        value: (squared as f32).sqrt() as i32,
                        target: None,
                    });
                }
                ExpressionOp::ObjectPointer(uid) => {
                    let value = host
                        .find_object_uid(*uid)
                        .map_err(|error| VmError::host(error, offset, opcode))?
                        .unwrap_or(0);
                    stack.push(StackValue {
                        value,
                        target: None,
                    });
                }
                ExpressionOp::ItemCount => {
                    let value = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing item-count operand")
                    })?;
                    let item = read_value(host, value, offset, opcode)?;
                    let count = host
                        .read_item_count(item)
                        .map_err(|error| VmError::host(error, offset, opcode))?;
                    stack.push(StackValue {
                        value: count,
                        target: None,
                    });
                }
                ExpressionOp::AnimationFrame => {
                    let value = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing animation-frame operand")
                    })?;
                    let animation = read_value(host, value, offset, opcode)?;
                    let actor = self.entity.ok_or_else(|| {
                        expression_error(offset, opcode, "animation frame requires object context")
                    })?;
                    let frame = host
                        .read_animation_frame(actor, animation)
                        .map_err(|error| VmError::host(error, offset, opcode))?;
                    stack.push(StackValue {
                        value: frame,
                        target: None,
                    });
                }
                ExpressionOp::BgiAnimationFrame => {
                    let value = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing BGI animation-frame operand")
                    })?;
                    let animation = read_value(host, value, offset, opcode)? as u32;
                    let frame = host
                        .read_bgi_animation_frame(animation)
                        .map_err(|error| VmError::host(error, offset, opcode))?;
                    stack.push(StackValue {
                        value: frame,
                        target: None,
                    });
                }
                ExpressionOp::BgiCharacterTriangle => {
                    let value = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing BGI character operand")
                    })?;
                    let character = read_value(host, value, offset, opcode)?;
                    let triangle = host
                        .read_bgi_character_triangle(character)
                        .map_err(|error| VmError::host(error, offset, opcode))?;
                    stack.push(StackValue {
                        value: triangle,
                        target: None,
                    });
                }
                ExpressionOp::BgiCharacterFloor => {
                    let value = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing BGI character operand")
                    })?;
                    let character = read_value(host, value, offset, opcode)?;
                    let floor = host
                        .read_bgi_character_floor(character)
                        .map_err(|error| VmError::host(error, offset, opcode))?;
                    stack.push(StackValue {
                        value: floor,
                        target: None,
                    });
                }
                ExpressionOp::ActiveBitCount => {
                    let value = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing active-bit operand")
                    })?;
                    let active = read_value(host, value, offset, opcode)?;
                    // Memoria walks an Int16 mask: bit 15 is tested as the sign-extended 0xFFFF8000.
                    let mut count = 0;
                    let mut check: i16 = 1;
                    while check != 0 {
                        if active & i32::from(check) != 0 {
                            count += 1;
                        }
                        check = check.wrapping_shl(1);
                    }
                    stack.push(StackValue {
                        value: count,
                        target: None,
                    });
                }
                ExpressionOp::CharacterMaxMp => {
                    let value = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing maximum-MP character operand")
                    })?;
                    let character = read_value(host, value, offset, opcode)?;
                    let maximum = host
                        .read_character_max_mp(character)
                        .map_err(|error| VmError::host(error, offset, opcode))?;
                    stack.push(StackValue {
                        value: maximum,
                        target: None,
                    });
                }
                ExpressionOp::PartyContains => {
                    let value = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing party-check operand")
                    })?;
                    let character = read_value(host, value, offset, opcode)?;
                    let contains = host
                        .party_contains(character)
                        .map_err(|error| VmError::host(error, offset, opcode))?;
                    stack.push(StackValue {
                        value: i32::from(contains),
                        target: None,
                    });
                }
                ExpressionOp::PartyAdd => {
                    let value = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing party-add operand")
                    })?;
                    let character = read_value(host, value, offset, opcode)?;
                    let failed = host
                        .add_party_member(character)
                        .map_err(|error| VmError::host(error, offset, opcode))?;
                    stack.push(StackValue {
                        value: i32::from(failed),
                        target: None,
                    });
                }
                ExpressionOp::Sps => {
                    let right = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing second SPS operand")
                    })?;
                    read_value(host, right, offset, opcode)?;
                    let left = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing first SPS operand")
                    })?;
                    read_value(host, left, offset, opcode)?;
                    stack.push(StackValue {
                        value: 0,
                        target: None,
                    });
                }
                ExpressionOp::SelectRandomMember => {
                    let value = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing random-selection operand")
                    })?;
                    let mask = read_value(host, value, offset, opcode)?;
                    let candidates: Vec<u8> =
                        (0..8).filter(|slot| mask & (1 << slot) != 0).collect();
                    let selected = if candidates.is_empty() {
                        0
                    } else {
                        let random = u32::from(
                            host.read_random_u8()
                                .map_err(|error| VmError::host(error, offset, opcode))?,
                        );
                        let index = (candidates.len() as u32 * random) >> 8;
                        1 << candidates[index as usize]
                    };
                    stack.push(StackValue {
                        value: selected,
                        target: None,
                    });
                }
                ExpressionOp::PickFirstMember => {
                    if stack.len() < 2 {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "first-member pick needs a mask and a member operand",
                        ));
                    }
                    let body = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing first-member operand")
                    })?;
                    let mask = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing first-member mask")
                    })?;
                    let mask = read_value(host, mask, offset, opcode)?;
                    // Memoria scans only the first eight list slots.
                    let result = match (0..8u8).find(|slot| mask & (1 << slot) != 0) {
                        Some(slot) => {
                            host.select_object_member_target(Some(slot))
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                            read_value(host, body, offset, opcode)?
                        }
                        None => 0,
                    };
                    stack.push(StackValue {
                        value: result,
                        target: None,
                    });
                }
                ExpressionOp::CharacterStat(stat) => {
                    let value = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing character-stat operand")
                    })?;
                    let character = read_value(host, value, offset, opcode)?;
                    let result = host
                        .read_character_stat(character, *stat)
                        .map_err(|error| VmError::host(error, offset, opcode))?;
                    stack.push(StackValue {
                        value: result,
                        target: None,
                    });
                }
                ExpressionOp::KeyState(state) => {
                    let value = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing key-state operand")
                    })?;
                    let mask = read_value(host, value, offset, opcode)?;
                    let inputs = host
                        .read_key_state(*state)
                        .map_err(|error| VmError::host(error, offset, opcode))?;
                    stack.push(StackValue {
                        value: i32::from(mask & inputs != 0),
                        target: None,
                    });
                }
                ExpressionOp::KeyOn => {
                    let value = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing key-on operand")
                    })?;
                    let mask = read_value(host, value, offset, opcode)?;
                    let pressed = host
                        .read_key_on()
                        .map_err(|error| VmError::host(error, offset, opcode))?;
                    stack.push(StackValue {
                        value: i32::from(mask & pressed != 0),
                        target: None,
                    });
                }
                ExpressionOp::Add
                | ExpressionOp::Subtract
                | ExpressionOp::Multiply
                | ExpressionOp::Divide
                | ExpressionOp::Remainder
                | ExpressionOp::ShiftLeft
                | ExpressionOp::ShiftRight
                | ExpressionOp::Angle
                | ExpressionOp::Angle2
                | ExpressionOp::DistanceToPoint
                | ExpressionOp::LessOrEqual
                | ExpressionOp::Less
                | ExpressionOp::Greater
                | ExpressionOp::GreaterOrEqual
                | ExpressionOp::Equal
                | ExpressionOp::NotEqual
                | ExpressionOp::BitwiseAnd
                | ExpressionOp::BitwiseXor
                | ExpressionOp::BitwiseOr => {
                    let (left, right) = binary(&mut stack, operation, host)?;
                    let value = match operation {
                        ExpressionOp::Add => left.wrapping_add(right),
                        ExpressionOp::Subtract => left.wrapping_sub(right),
                        ExpressionOp::Multiply => left.wrapping_mul(right),
                        ExpressionOp::Divide if right != 0 => left.wrapping_div(right),
                        ExpressionOp::Remainder if right != 0 => left.wrapping_rem(right),
                        ExpressionOp::ShiftLeft => left.wrapping_shl(right as u32),
                        ExpressionOp::ShiftRight => left.wrapping_shr(right as u32),
                        ExpressionOp::Angle => {
                            let actor = self.entity.ok_or_else(|| {
                                expression_error(
                                    offset,
                                    opcode,
                                    "coordinate angle requires object context",
                                )
                            })?;
                            let position = host
                                .read_object_position(actor)
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                            angle2(
                                host,
                                (left as f32 - position[0]) as i32,
                                (right as f32 - position[2]) as i32,
                                offset,
                                opcode,
                            )?
                        }
                        ExpressionOp::Angle2 => angle2(host, left, right, offset, opcode)?,
                        ExpressionOp::DistanceToPoint => {
                            let actor = self.entity.ok_or_else(|| {
                                expression_error(
                                    offset,
                                    opcode,
                                    "coordinate distance requires object context",
                                )
                            })?;
                            let position = host
                                .read_object_position(actor)
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                            let delta_x = left as f32 - position[0];
                            let delta_z = right as f32 - position[2];
                            (delta_x * delta_x + delta_z * delta_z).sqrt() as i32
                        }
                        ExpressionOp::Divide | ExpressionOp::Remainder => left,
                        ExpressionOp::Less => i32::from(left < right),
                        ExpressionOp::LessOrEqual => i32::from(left <= right),
                        ExpressionOp::Greater => i32::from(left > right),
                        ExpressionOp::GreaterOrEqual => i32::from(left >= right),
                        ExpressionOp::Equal => i32::from(left == right),
                        ExpressionOp::NotEqual => i32::from(left != right),
                        ExpressionOp::BitwiseAnd => left & right,
                        ExpressionOp::BitwiseXor => left ^ right,
                        ExpressionOp::BitwiseOr => left | right,
                        _ => unreachable!(),
                    };
                    stack.push(StackValue {
                        value,
                        target: None,
                    });
                }
                ExpressionOp::LogicalAnd | ExpressionOp::LogicalOr => {
                    let (left, right) = binary(&mut stack, operation, host)?;
                    let value = match operation {
                        ExpressionOp::LogicalAnd => left != 0 && right != 0,
                        ExpressionOp::LogicalOr => left != 0 || right != 0,
                        _ => unreachable!(),
                    };
                    stack.push(StackValue {
                        value: i32::from(value),
                        target: None,
                    });
                }
                ExpressionOp::AddAssign
                | ExpressionOp::SubtractAssign
                | ExpressionOp::MultiplyAssign
                | ExpressionOp::DivideAssign
                | ExpressionOp::RemainderAssign
                | ExpressionOp::ShiftLeftAssign
                | ExpressionOp::ShiftRightAssign
                | ExpressionOp::AndAssign
                | ExpressionOp::XorAssign
                | ExpressionOp::OrAssign => {
                    let operand = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing arithmetic-assignment operand")
                    })?;
                    let operand = read_value(host, operand, offset, opcode)?;
                    let target = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing arithmetic-assignment target")
                    })?;
                    let current = read_value(host, target, offset, opcode)?;
                    // Memoria DIV_LET and REM_LET leave the target untouched on a zero divisor.
                    let write = !(matches!(
                        operation,
                        ExpressionOp::DivideAssign | ExpressionOp::RemainderAssign
                    ) && operand == 0);
                    let value = match operation {
                        ExpressionOp::AddAssign => current.wrapping_add(operand),
                        ExpressionOp::SubtractAssign => current.wrapping_sub(operand),
                        ExpressionOp::MultiplyAssign => current.wrapping_mul(operand),
                        ExpressionOp::DivideAssign if write => current.wrapping_div(operand),
                        ExpressionOp::RemainderAssign if write => current.wrapping_rem(operand),
                        ExpressionOp::DivideAssign | ExpressionOp::RemainderAssign => current,
                        ExpressionOp::ShiftLeftAssign => current.wrapping_shl(operand as u32),
                        ExpressionOp::ShiftRightAssign => current.wrapping_shr(operand as u32),
                        ExpressionOp::AndAssign => current & operand,
                        ExpressionOp::XorAssign => current ^ operand,
                        ExpressionOp::OrAssign => current | operand,
                        _ => unreachable!(),
                    };
                    if write {
                        match target.target {
                            Some(StackTarget::Variable(variable)) => {
                                let normalized = normalize(value, variable.value_type);
                                self.variables
                                    .insert((variable.scope, variable.index), normalized);
                            }
                            Some(StackTarget::Member(member)) => {
                                let object = host
                                    .select_object_member_target(None)
                                    .map_err(|error| VmError::host(error, offset, opcode))?;
                                host.write_object_member(ObjectMemberRef { object, member }, value)
                                    .map_err(|error| VmError::host(error, offset, opcode))?;
                            }
                            Some(StackTarget::SystemList(index)) => {
                                host.write_system_list(index, value)
                                    .map_err(|error| VmError::host(error, offset, opcode))?;
                            }
                            None => {
                                return Err(expression_error(
                                    offset,
                                    opcode,
                                    "arithmetic-assignment target is not writable",
                                ));
                            }
                        }
                    }
                    stack.push(StackValue {
                        value,
                        target: None,
                    });
                }
                ExpressionOp::Assign => {
                    let value = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing assignment value")
                    })?;
                    let value = read_value(host, value, offset, opcode)?;
                    let target = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing assignment target")
                    })?;
                    let normalized = match target.target {
                        Some(StackTarget::Variable(variable)) => {
                            let normalized = normalize(value, variable.value_type);
                            self.variables
                                .insert((variable.scope, variable.index), normalized);
                            normalized
                        }
                        Some(StackTarget::Member(member)) => {
                            let object = host
                                .select_object_member_target(None)
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                            host.write_object_member(ObjectMemberRef { object, member }, value)
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                            value
                        }
                        Some(StackTarget::SystemList(index)) => {
                            host.write_system_list(index, value)
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                            value
                        }
                        // Memoria's SetVariableValue pops a plain-value target (for example the
                        // result of `a && b`) and writes nothing, so the assignment only yields
                        // its value. Real scripts rely on this (EVT_LIND1_TN_LB_LDH_0).
                        None => value,
                    };
                    stack.push(StackValue {
                        value: normalized,
                        target: None,
                    });
                }
                ExpressionOp::PreIncrement
                | ExpressionOp::PostIncrement
                | ExpressionOp::PreDecrement
                | ExpressionOp::PostDecrement => {
                    let target = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing increment/decrement target")
                    })?;
                    let current = read_value(host, target, offset, opcode)?;
                    let delta = if matches!(
                        operation,
                        ExpressionOp::PreIncrement | ExpressionOp::PostIncrement
                    ) {
                        1
                    } else {
                        -1
                    };
                    let changed = match target.target {
                        Some(StackTarget::Variable(variable)) => {
                            let changed =
                                normalize(current.wrapping_add(delta), variable.value_type);
                            self.variables
                                .insert((variable.scope, variable.index), changed);
                            changed
                        }
                        Some(StackTarget::Member(member)) => {
                            let changed = current.wrapping_add(delta);
                            let object = host
                                .select_object_member_target(None)
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                            host.write_object_member(ObjectMemberRef { object, member }, changed)
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                            changed
                        }
                        Some(StackTarget::SystemList(index)) => {
                            let changed = current.wrapping_add(delta);
                            host.write_system_list(index, changed)
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                            changed
                        }
                        None => {
                            return Err(expression_error(
                                offset,
                                opcode,
                                "increment/decrement target is not writable",
                            ));
                        }
                    };
                    stack.push(StackValue {
                        value: if matches!(
                            operation,
                            ExpressionOp::PostIncrement | ExpressionOp::PostDecrement
                        ) {
                            current
                        } else {
                            changed
                        },
                        target: None,
                    });
                }
                ExpressionOp::SystemList(index) => stack.push(StackValue {
                    value: 0,
                    target: Some(StackTarget::SystemList(*index)),
                }),
                ExpressionOp::SystemVariable(variable) => {
                    let value = host
                        .read_system_variable(*variable)
                        .map_err(|error| VmError::host(error, offset, opcode))?;
                    stack.push(StackValue {
                        value,
                        target: None,
                    });
                }
                ExpressionOp::CustomVariable(variable) => {
                    let value = host
                        .read_custom_variable(*variable)
                        .map_err(|error| VmError::host(error, offset, opcode))?;
                    stack.push(StackValue {
                        value,
                        target: None,
                    });
                }
                ExpressionOp::CastI8
                | ExpressionOp::CastU8
                | ExpressionOp::CastI16
                | ExpressionOp::CastU16 => {
                    let value = stack
                        .pop()
                        .ok_or_else(|| expression_error(offset, opcode, "missing cast operand"))?;
                    let value = read_value(host, value, offset, opcode)?;
                    let value = match operation {
                        ExpressionOp::CastI8 => (value as i8) as i32,
                        ExpressionOp::CastU8 => (value as u8) as i32,
                        ExpressionOp::CastI16 => (value as i16) as i32,
                        ExpressionOp::CastU16 => (value as u16) as i32,
                        _ => unreachable!(),
                    };
                    stack.push(StackValue {
                        value,
                        target: None,
                    });
                }
                ExpressionOp::CastList => stack.push(StackValue {
                    value: 0,
                    target: None,
                }),
                ExpressionOp::AssignMembers
                | ExpressionOp::AndAssignMembers
                | ExpressionOp::XorAssignMembers
                | ExpressionOp::OrAssignMembers => {
                    // Memoria `OperatorExtractLet`: the stack ends with [lvalue, mask, member].
                    // The result is the bit set of slots whose member is nonzero; ops 67-69
                    // combine it with the lvalue's current value before storing.
                    let combines = !matches!(operation, ExpressionOp::AssignMembers);
                    if stack.len() < if combines { 3 } else { 2 } {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "member extraction requires a mask and a member input",
                        ));
                    }
                    let Some(StackTarget::Member(_)) = stack[stack.len() - 1].target else {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "member extraction input is not a selected member",
                        ));
                    };
                    let mask = read_value(host, stack[stack.len() - 2], offset, opcode)?;
                    let member = stack[stack.len() - 1];
                    let mut result = 0i32;
                    for slot in 0..8 {
                        result >>= 1;
                        if mask & (1 << slot) != 0 {
                            host.select_object_member_target(Some(slot))
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                            result |=
                                i32::from(read_value(host, member, offset, opcode)? != 0) << 7;
                        }
                    }
                    stack.truncate(stack.len() - 2);
                    let lvalue = stack.pop();
                    if let Some(lvalue) = lvalue {
                        if combines {
                            let current = read_value(host, lvalue, offset, opcode)?;
                            result = match operation {
                                ExpressionOp::AndAssignMembers => result & current,
                                ExpressionOp::XorAssignMembers => result ^ current,
                                _ => result | current,
                            };
                        }
                        match lvalue.target {
                            Some(StackTarget::Variable(variable)) => {
                                let stored = normalize(result, variable.value_type);
                                self.variables
                                    .insert((variable.scope, variable.index), stored);
                            }
                            Some(StackTarget::Member(member)) => {
                                let object = host
                                    .select_object_member_target(None)
                                    .map_err(|error| VmError::host(error, offset, opcode))?;
                                host.write_object_member(
                                    ObjectMemberRef { object, member },
                                    result,
                                )
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                            }
                            Some(StackTarget::SystemList(index)) => {
                                host.write_system_list(index, result)
                                    .map_err(|error| VmError::host(error, offset, opcode))?;
                            }
                            None => {}
                        }
                    } else {
                        self.variables
                            .insert((VariableScope::Global, 0), i32::from(result != 0));
                    }
                    stack.push(StackValue {
                        value: result,
                        target: None,
                    });
                }
                ExpressionOp::AssignAllMembers => {
                    if stack.len() < 3 {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "all-member assignment requires mask, target, and value inputs",
                        ));
                    }
                    let Some(StackTarget::Member(member)) = stack[stack.len() - 2].target else {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "all-member assignment target is not a selected member",
                        ));
                    };
                    let mask = read_value(host, stack[stack.len() - 3], offset, opcode)?;
                    let operand = read_value(host, stack[stack.len() - 1], offset, opcode)?;
                    let mut result = 0;
                    for slot in 0..8 {
                        if mask & (1 << slot) != 0 {
                            let object = host
                                .select_object_member_target(Some(slot))
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                            host.write_object_member(ObjectMemberRef { object, member }, operand)
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                            result = operand;
                        }
                    }
                    stack.truncate(stack.len() - 3);
                    stack.push(StackValue {
                        value: result,
                        target: None,
                    });
                }
                ExpressionOp::XorAssignAllMembers
                | ExpressionOp::AndAssignAllMembers
                | ExpressionOp::OrAssignAllMembers
                | ExpressionOp::ShiftRightAssignAllMembers
                | ExpressionOp::MultiplyAssignAllMembers
                | ExpressionOp::DivideAssignAllMembers
                | ExpressionOp::SubtractAssignAllMembers
                | ExpressionOp::RemainderAssignAllMembers
                | ExpressionOp::PlusAssignAllMembers
                | ExpressionOp::ShiftLeftAssignAllMembers => {
                    if stack.len() < 3 {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "all-member bitwise assignment requires mask, target, and value inputs",
                        ));
                    }
                    let Some(StackTarget::Member(member)) = stack[stack.len() - 2].target else {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "all-member bitwise assignment target is not a selected member",
                        ));
                    };
                    let mask = read_value(host, stack[stack.len() - 3], offset, opcode)?;
                    let operand = read_value(host, stack[stack.len() - 1], offset, opcode)?;
                    let mut result = 0;
                    for slot in 0..8 {
                        if mask & (1 << slot) != 0 {
                            let object = host
                                .select_object_member_target(Some(slot))
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                            let reference = ObjectMemberRef { object, member };
                            let current = host
                                .read_object_member(reference)
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                            result = match operation {
                                ExpressionOp::AndAssignAllMembers => current & operand,
                                ExpressionOp::OrAssignAllMembers => current | operand,
                                ExpressionOp::XorAssignAllMembers => current ^ operand,
                                ExpressionOp::ShiftRightAssignAllMembers => {
                                    current.wrapping_shr(operand as u32)
                                }
                                ExpressionOp::MultiplyAssignAllMembers => {
                                    current.wrapping_mul(operand)
                                }
                                ExpressionOp::DivideAssignAllMembers => {
                                    if operand == 0 {
                                        current
                                    } else {
                                        current.wrapping_div(operand)
                                    }
                                }
                                ExpressionOp::RemainderAssignAllMembers => {
                                    if operand == 0 {
                                        current
                                    } else {
                                        current.wrapping_rem(operand)
                                    }
                                }
                                ExpressionOp::PlusAssignAllMembers => current.wrapping_add(operand),
                                ExpressionOp::ShiftLeftAssignAllMembers => {
                                    current.wrapping_shl(operand as u32)
                                }
                                ExpressionOp::SubtractAssignAllMembers => {
                                    current.wrapping_sub(operand)
                                }
                                _ => unreachable!(),
                            };
                            host.write_object_member(reference, result)
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                        }
                    }
                    stack.truncate(stack.len() - 3);
                    stack.push(StackValue {
                        value: result,
                        target: None,
                    });
                }
                ExpressionOp::ObjectSpecial => stack.push(StackValue {
                    value: 0,
                    target: None,
                }),
                ExpressionOp::ReadObjectMember(reference) => {
                    let value = host
                        .read_object_member(*reference)
                        .map_err(|error| VmError::host(error, offset, opcode))?;
                    stack.push(StackValue {
                        value,
                        target: None,
                    });
                }
                ExpressionOp::Pad => stack.push(StackValue {
                    value: 0,
                    target: None,
                }),
                ExpressionOp::EqualMembers
                | ExpressionOp::NotEqualMembers
                | ExpressionOp::AndMembers
                | ExpressionOp::NandMembers
                | ExpressionOp::XorMembers
                | ExpressionOp::LessMembers
                | ExpressionOp::LessOrEqualMembers
                | ExpressionOp::GreaterOrEqualMembers
                | ExpressionOp::GreaterMembers => {
                    if stack.len() < 3 {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "member extraction requires mask, target, and operand inputs",
                        ));
                    }
                    let Some(StackTarget::Member(_)) = stack[stack.len() - 2].target else {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "member extraction target is not a selected member",
                        ));
                    };
                    let mask = read_value(host, stack[stack.len() - 3], offset, opcode)?;
                    let target = stack[stack.len() - 2];
                    let comparison = read_value(host, stack[stack.len() - 1], offset, opcode)?;
                    let mut result = 0i32;
                    for slot in 0..8 {
                        result >>= 1;
                        if mask & (1 << slot) != 0 {
                            host.select_object_member_target(Some(slot))
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                            let value = read_value(host, target, offset, opcode)?;
                            let matches = match operation {
                                ExpressionOp::EqualMembers => value == comparison,
                                ExpressionOp::NotEqualMembers => value != comparison,
                                ExpressionOp::AndMembers => value & comparison != 0,
                                ExpressionOp::NandMembers => value & comparison == 0,
                                ExpressionOp::XorMembers => value ^ comparison != 0,
                                ExpressionOp::LessMembers => value < comparison,
                                ExpressionOp::LessOrEqualMembers => value <= comparison,
                                ExpressionOp::GreaterOrEqualMembers => value >= comparison,
                                ExpressionOp::GreaterMembers => value > comparison,
                                _ => unreachable!(),
                            };
                            result |= i32::from(matches) << 7;
                        }
                    }
                    stack.truncate(stack.len() - 3);
                    stack.push(StackValue {
                        value: result,
                        target: None,
                    });
                }
                ExpressionOp::NotMembers => {
                    if stack.len() < 2 {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "member logical NOT requires mask and target inputs",
                        ));
                    }
                    let Some(StackTarget::Member(_)) = stack[stack.len() - 1].target else {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "member logical NOT target is not a selected member",
                        ));
                    };
                    let mask = read_value(host, stack[stack.len() - 2], offset, opcode)?;
                    let target = stack[stack.len() - 1];
                    let mut result = 0i32;
                    for slot in 0..8 {
                        result >>= 1;
                        if mask & (1 << slot) != 0 {
                            host.select_object_member_target(Some(slot))
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                            result |=
                                i32::from(read_value(host, target, offset, opcode)? == 0) << 7;
                        }
                    }
                    stack.truncate(stack.len() - 2);
                    stack.push(StackValue {
                        value: result,
                        target: None,
                    });
                }
                ExpressionOp::PreIncrementMembers => {
                    if stack.len() < 2 {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "member pre-increment requires mask and target inputs",
                        ));
                    }
                    let Some(StackTarget::Member(member)) = stack[stack.len() - 1].target else {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "member pre-increment target is not a selected member",
                        ));
                    };
                    let mask = read_value(host, stack[stack.len() - 2], offset, opcode)?;
                    let mut result = 0;
                    for slot in 0..8 {
                        if mask & (1 << slot) != 0 {
                            let object = host
                                .select_object_member_target(Some(slot))
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                            let reference = ObjectMemberRef { object, member };
                            let current = host
                                .read_object_member(reference)
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                            result = current.wrapping_add(1);
                            host.write_object_member(reference, result)
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                        }
                    }
                    stack.truncate(stack.len() - 2);
                    stack.push(StackValue {
                        value: result,
                        target: None,
                    });
                }
                ExpressionOp::PostDecrementMembers => {
                    if stack.len() < 2 {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "member post-decrement requires mask and target inputs",
                        ));
                    }
                    let Some(StackTarget::Member(member)) = stack[stack.len() - 1].target else {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "member post-decrement target is not a selected member",
                        ));
                    };
                    let mask = read_value(host, stack[stack.len() - 2], offset, opcode)?;
                    let mut result = 0;
                    for slot in 0..8 {
                        if mask & (1 << slot) != 0 {
                            let object = host
                                .select_object_member_target(Some(slot))
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                            let reference = ObjectMemberRef { object, member };
                            let current = host
                                .read_object_member(reference)
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                            host.write_object_member(reference, current.wrapping_sub(1))
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                            result = current;
                        }
                    }
                    stack.truncate(stack.len() - 2);
                    stack.push(StackValue {
                        value: result,
                        target: None,
                    });
                }
                ExpressionOp::PostIncrementMembers | ExpressionOp::PreDecrementMembers => {
                    if stack.len() < 2 {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "object-member operation requires selection inputs",
                        ));
                    }
                    if read_value(host, stack[stack.len() - 2], offset, opcode)? != 0 {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "nonzero member selections require the object-member host",
                        ));
                    }
                    stack.truncate(stack.len() - 2);
                    stack.push(StackValue {
                        value: 0,
                        target: None,
                    });
                }
                ExpressionOp::End => break,
            }
            index += 1;
        }
        match stack.last().copied() {
            Some(value) => read_value(host, value, offset, opcode),
            None => Ok(0),
        }
    }

    fn evaluate_u8(
        &mut self,
        host: &mut dyn LegacyEventHost,
        expression: &Expression,
        offset: u32,
        opcode: u32,
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
        opcode: u32,
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

    fn trace(&mut self, tick: Tick, offset: u32, opcode: u32, kind: &InstructionKind) {
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

fn opcode_for(kind: &InstructionKind) -> u32 {
    match kind {
        InstructionKind::Nop => 0x00,
        InstructionKind::NoOp(opcode) => *opcode,
        InstructionKind::Jump(_) => 0x01,
        InstructionKind::JumpIfNot { .. } => 0x02,
        InstructionKind::JumpIf { .. } => 0x03,
        InstructionKind::JumpSwitchEx { .. } => 0x06,
        InstructionKind::JumpSwitch { .. } => 0x0b,
        InstructionKind::JumpSwitchWide { .. } => 0x0d,
        InstructionKind::DeleteObject(_) => 0x1c,
        InstructionKind::SetActorPosition { .. } => 0x1d,
        InstructionKind::SetIdleAnimation { .. } => 0x33,
        InstructionKind::SetWalkAnimation { .. } => 0x34,
        InstructionKind::SetRunAnimation { .. } => 0x35,
        InstructionKind::PlayAnimation { .. } => 0x40,
        InstructionKind::SetAnimationRange { .. } => 0x3d,
        InstructionKind::SetLeftTurnAnimation { .. } => 0x7a,
        InstructionKind::SetRightTurnAnimation { .. } => 0x7b,
        InstructionKind::SetJumpAnimation { .. } => 0x94,
        InstructionKind::SetActorRadii { .. } => 0x4b,
        InstructionKind::SetIdleAnimationSpeeds { .. } => 0x86,
        InstructionKind::SetHeadFocusMasks { .. } => 0x8b,
        InstructionKind::SetHeadFocusFlags { .. } => 0x47,
        InstructionKind::SetWalkTurnSpeed { .. } => 0x55,
        InstructionKind::SetObjectFlags { .. } => 0x93,
        InstructionKind::SetActorFacing { .. } => 0x36,
        InstructionKind::StartActorTurn { .. } => 0x56,
        InstructionKind::Return => 0x04,
        InstructionKind::Evaluate(_) => 0x05,
        InstructionKind::WindowSync { .. } => 0x1f,
        InstructionKind::WindowAsync { .. } => 0x20,
        InstructionKind::CloseWindow(_) => 0x21,
        InstructionKind::Wait(_) => 0x22,
        InstructionKind::WaitAnimation => 0x41,
        InstructionKind::WaitTurn => 0x50,
        InstructionKind::SetActorPositionXzy { .. } => 0xa1,
        InstructionKind::MoveActorXzy { .. } => 0xad,
        InstructionKind::WalkActorMixed { opcode, .. } => u32::from(*opcode),
        InstructionKind::TurnActorToPosition { .. } => 0x9b,
        InstructionKind::WaitSharedScript => 0x44,
        InstructionKind::SetPathing { .. } => 0xa8,
        InstructionKind::WaitWindow(_) => 0x54,
        InstructionKind::AddItem { .. } => 0x48,
        InstructionKind::RemoveItem { .. } => 0x49,
        InstructionKind::AddGil(_) => 0xce,
        InstructionKind::RemoveGil(_) => 0xcf,
        InstructionKind::SetPartyHp { .. } => 0xf1,
        InstructionKind::SetPartyMp { .. } => 0xf2,
        InstructionKind::ClearAbilityAp { .. } => 0xf3,
        InstructionKind::MasterAbility { .. } => 0xf4,
        InstructionKind::WalkActor { .. } => 0x23,
        InstructionKind::MoveActor { .. } => 0xbf,
        InstructionKind::AimAtCamera => 0xb8,
        InstructionKind::SetModelColor { .. } => 0x8f,
        InstructionKind::StartObjectTurn { .. } => 0xbb,
        InstructionKind::PlayObjectAnimation { .. } => 0xbd,
        InstructionKind::PlayTextureAnimation { once: false, .. } => 0xc0,
        InstructionKind::PlayTextureAnimation { once: true, .. } => 0xc1,
        InstructionKind::DisableRun => 0x6a,
        InstructionKind::SetTileCamera { .. } => 0xc3,
        InstructionKind::GameOver => 0xf5,
        InstructionKind::StartCardGame { .. } => 0xae,
        InstructionKind::StartBattle { group: None, .. } => 0x2a,
        InstructionKind::StartBattle { group: Some(_), .. } => 0x8c,
        InstructionKind::ChangeWorldMap { .. } => 0xb6,
        InstructionKind::SyncPartyUid => 0xe9,
        InstructionKind::ExitField => 0x9e,
        InstructionKind::PlayJumpAnimation { landing: false } => 0x9c,
        InstructionKind::PlayJumpAnimation { landing: true } => 0x9d,
        InstructionKind::DisableIdleAnimation => 0x90,
        InstructionKind::StopAnimation => 0x42,
        InstructionKind::SetPathFloorActive { .. } => 0xcb,
        InstructionKind::PretendToBe { .. } => 0xb5,
        InstructionKind::DefinePlayerControl => 0x2c,
        InstructionKind::SetControlDirection { .. } => 0x67,
        InstructionKind::Cinematic { .. } => 0x28,
        InstructionKind::DefineRegion { .. } => 0x29,
        InstructionKind::BackgroundMove { .. } => 0x58,
        InstructionKind::SetTileShadeColor { .. } => 0x59,
        InstructionKind::SetSpsParameters { .. } => 0xb3,
        InstructionKind::InitializeObject { .. } => 0x09,
        InstructionKind::InitializeRegion { .. } => 0x08,
        InstructionKind::InitializeCode { .. } => 0x07,
        InstructionKind::SetTriangleActive { .. } => 0x9a,
        InstructionKind::RunSound { opcode, .. } => u32::from(*opcode),
        InstructionKind::AttachObjectOffset { .. } => 0xd4,
        InstructionKind::ResetWalkState => 0x25,
        InstructionKind::SetMovementSpeed(_) => 0x26,
        InstructionKind::SetActorModel { .. } => 0x2f,
        InstructionKind::SetNavigationAttributeMask(_) => 0x27,
        InstructionKind::Yield(opcode) => u32::from(*opcode),
        InstructionKind::SetShadowEnabled { opcode, .. } => u32::from(*opcode),
        InstructionKind::SetTextVariable { .. } => 0x66,
        InstructionKind::ShowBattleMessage { .. } => 0xd0,
        InstructionKind::SetObjectScale { .. } => 0x9f,
        InstructionKind::SetVibrationFrameRate(_) => 0xfa,
        InstructionKind::PreloadField { .. } => 0xfd,
        InstructionKind::SetRain { .. } => 0xd8,
        InstructionKind::RequestScriptAsync { .. } => 0x10,
        InstructionKind::RequestScriptWhenReady { .. } => 0x12,
        InstructionKind::DefineTileViewport { .. } => 0xc9,
        InstructionKind::PreserveAndHideAll => 0xd5,
        InstructionKind::RestoreAllVisibility => 0xd6,
        InstructionKind::SetTimerRunning(_) => 0x7d,
        InstructionKind::RequestCallerAsync { .. } => 0x16,
        InstructionKind::RequestCallerWhenReady { .. } => 0x18,
        InstructionKind::CalculateExitPosition => 0xa4,
        InstructionKind::WalkToExit => 0xa0,
        InstructionKind::CalculateScreenPosition { .. } => 0xa9,
        InstructionKind::ChangeField { .. } => 0x2b,
        InstructionKind::RequestCallerSync { .. } => 0x1a,
        InstructionKind::RequestScriptSync { .. } => 0x14,
        InstructionKind::SetSimulationFloor { .. } => 0x104,
        InstructionKind::SetSimulationFrameRate { .. } => 0x105,
        InstructionKind::SetTileAnimationFlags { .. } => 0x64,
        InstructionKind::SetTileAnimationRange { .. } => 0x65,
        InstructionKind::EndSharedScript => 0x45,
        InstructionKind::SetMenuControl { opcode, .. } => u32::from(*opcode),
        InstructionKind::SetUserControl { opcode, .. } => u32::from(*opcode),
        InstructionKind::RaiseWindows => 0x8e,
        InstructionKind::CalculateScreenOrigin => 0xea,
        InstructionKind::RunSharedScript { .. } => 0x43,
        InstructionKind::TurnTowardObject { .. } => 0x51,
        InstructionKind::ShowBubbleIcon { .. } => 0x68,
        InstructionKind::SetFieldCamera { .. } => 0x7e,
        InstructionKind::ShowObjectMeshes { .. } => 0x39,
        InstructionKind::HideObjectMeshes { .. } => 0x3a,
        InstructionKind::SetObjectIndex { .. } => 0x3b,
        InstructionKind::SetAnimationFlags { .. } => 0x3f,
        InstructionKind::SetInactiveAnimation { .. } => 0x52,
        InstructionKind::SetFollowFocusTurn { .. } => 0x91,
        InstructionKind::SetAnimationLoopCount { .. } => 0x98,
        InstructionKind::SetAnimationSpeed { .. } => 0x3e,
        InstructionKind::SetCharacterAttribute { set, .. } => {
            if *set {
                0xcc
            } else {
                0xcd
            }
        }
        InstructionKind::SetTurnSpeed { .. } => 0x99,
        InstructionKind::SetRunSpeedLimit { .. } => 0xa6,
        InstructionKind::TurnToAngle { .. } => 0xa7,
        InstructionKind::SetTimerTime { .. } => 0x69,
        InstructionKind::EnableDialogChoices { .. } => 0x7c,
        InstructionKind::SetFieldName { .. } => 0xb0,
        InstructionKind::SetRandomBattles { .. } => 0x3c,
        InstructionKind::SetRandomBattleFrequency { .. } => 0x57,
        InstructionKind::SetBackgroundColor { .. } => 0x6b,
        InstructionKind::AddControllerMask { .. } => 0xb9,
        InstructionKind::RemoveControllerMask { .. } => 0xba,
        InstructionKind::MoveCamera { .. } => 0x6f,
        InstructionKind::ReleaseCamera { .. } => 0x70,
        InstructionKind::EnableCameraServices { .. } => 0x71,
        InstructionKind::SetCameraFollowHeight { .. } => 0x72,
        InstructionKind::SetShadowSize { .. } => 0x81,
        InstructionKind::SetShadowOffset { .. } => 0x82,
        InstructionKind::LockShadowRotation { .. } => 0x83,
        InstructionKind::UnlockShadowRotation => 0x84,
        InstructionKind::SetShadowAmplifier { .. } => 0x85,
        InstructionKind::StartVibration { .. } => 0xf6,
        InstructionKind::SetVibrationActive { .. } => 0xf7,
        InstructionKind::RunVibrationTrack { .. } => 0xf8,
        InstructionKind::ActivateVibrationTrack { .. } => 0xf9,
        InstructionKind::SetVibrationFlags { .. } => 0xfb,
        InstructionKind::SetVibrationRange { .. } => 0xfc,
        InstructionKind::SetTileBlockActive { .. } => 0x5b,
        InstructionKind::SetTileLoop { .. } => 0x5c,
        InstructionKind::SetTileParallax { .. } => 0x5d,
        InstructionKind::SetTileOrigin { .. } => 0x5e,
        InstructionKind::RunTileAnimation { .. } => 0x5f,
        InstructionKind::SetTileAnimationActive { .. } => 0x60,
        InstructionKind::ResetTileAnimation { .. } => 0xca,
        InstructionKind::SetTileAnimationFrameWait { .. } => 0x63,
        InstructionKind::SetTileAnimationRate { .. } => 0x61,
        InstructionKind::IncrementFrogCount => 0xe0,
        InstructionKind::SetTileAnimationFrame { .. } => 0xe7,
        InstructionKind::OpenPartyMenu { .. } => 0xb2,
        InstructionKind::InitializeCharacter { .. } => 0xfe,
        InstructionKind::StopTextureAnimation { .. } => 0xc2,
        InstructionKind::RunBattleCode { .. } => 0x4a,
        InstructionKind::SetBattleMusicContinuation { .. } => 0x1b,
        InstructionKind::RunModelCode { .. } => 0x88,
        InstructionKind::AddBattleStatus { .. } => 0x11d,
        InstructionKind::SetModelLight { .. } => 0x121,
        InstructionKind::ChangeElementalAffinity { .. } => 0x122,
        InstructionKind::SetRunEnabled { opcode, .. } => u32::from(*opcode),
        InstructionKind::SetHereIcon { .. } => 0xef,
        InstructionKind::SetPartyAvailability { .. } => 0xb4,
        InstructionKind::MoveTile { .. } => 0x5a,
        InstructionKind::SetTileScrollWithOffset { .. } => 0xe4,
        InstructionKind::SetTileViewportAlpha { .. } => 0xed,
        InstructionKind::SetTileLoopType { .. } => 0xe6,
        InstructionKind::AttachObject { .. } => 0x4c,
        InstructionKind::DetachObject { .. } => 0x4d,
        InstructionKind::TrackStart { .. } => 0x76,
        InstructionKind::TrackSetLast { .. } => 0x77,
        InstructionKind::TrackAdd => 0x78,
        InstructionKind::SetSoundPosition { .. } => 0x89,
        InstructionKind::SetSoundObjectPosition { .. } => 0x8a,
        InstructionKind::ShowTimer { .. } => 0x8d,
        InstructionKind::RunWorldCode { .. } => 0xc4,
        InstructionKind::EnableVictoryPose { .. } => 0xdb,
        InstructionKind::RemoveParty { .. } => 0xdd,
        InstructionKind::SetPlayerName { .. } => 0xde,
        InstructionKind::SetupJump { .. } => 0xe2,
        InstructionKind::SetDialogSignal { .. } => 0xe3,
        InstructionKind::ApplyFadeFilter { .. } => 0xec,
        InstructionKind::OpenMenu { .. } => 0x75,
        InstructionKind::SetActiveTimeEventIcon { .. } => 0xd7,
        InstructionKind::SetObjectOvalRatio { .. } => 0xdf,
        InstructionKind::TerminateBattle => 0xe1,
        InstructionKind::ResetIdleTimer => 0xee,
        InstructionKind::CloseAllWindows => 0xeb,
        InstructionKind::AlignActorToCameraEye => 0xb7,
        InstructionKind::ProgressJump => 0xdc,
        InstructionKind::WalkTowardObject { .. } => 0x24,
        InstructionKind::PreventWindowInit => 0x53,
        InstructionKind::LockCameraFollow => 0x73,
        InstructionKind::UnlockCameraFollow => 0x74,
        InstructionKind::TurnObjectToAngle { .. } => 0x87,
        InstructionKind::ReturnEntryFunctions { .. } => 0x97,
        InstructionKind::ResetFieldName => 0xb1,
        InstructionKind::DeleteAllCards => 0xaf,
        InstructionKind::WaitObjectTurn { .. } => 0xbc,
        InstructionKind::WaitObjectAnimation { .. } => 0xbe,
        InstructionKind::QueueSpecialAttack { .. } => 0xe5,
        InstructionKind::ChangeDisc { .. } => 0xac,
        InstructionKind::SetSimulationActive { .. } => 0x102,
        InstructionKind::Watch { .. } => 0x4e,
        InstructionKind::SetSimulationFlags { .. } => 0x103,
        InstructionKind::SetSimulationAlgorithm { .. } => 0x106,
        InstructionKind::SetSimulationDelta { .. } => 0x107,
        InstructionKind::SetSimulationAxis { .. } => 0x108,
        InstructionKind::ShowNavigationAnimationFrame { .. } => 0x10a,
        InstructionKind::QueueEnemyAttack { .. } => 0x38,
        InstructionKind::ShowObjectWindow { opcode, .. } => u32::from(*opcode),
        InstructionKind::RunSpsCode { .. } => 0xda,
        InstructionKind::SetCameraBounds { .. } => 0x1e,
        InstructionKind::SetCharacterRow { .. } => 0x62,
        InstructionKind::ClearPartyStatus { .. } => 0xd9,
        InstructionKind::SetActorPitchRoll { .. } => 0x37,
        InstructionKind::AttachTile { .. } => 0x92,
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

fn switch_target(instruction: &Instruction, relative: u16, anchor: u32) -> Result<u32, VmError> {
    instruction
        .offset
        .checked_add(anchor)
        .and_then(|offset| offset.checked_add(u32::from(relative)))
        .ok_or_else(|| {
            VmError::new(
                VmErrorKind::InvalidJump,
                instruction.offset,
                Some(opcode_for(&instruction.kind)),
                "switch target leaves the event program",
            )
        })
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

fn expression_error(offset: u32, opcode: u32, message: impl Into<String>) -> VmError {
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
    opcode: u32,
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
    opcode: u32,
) -> Result<usize, VmError> {
    if end == 0 {
        return Err(expression_error(offset, opcode, "missing logical operand"));
    }
    let index = end - 1;
    match operations[index] {
        ExpressionOp::Constant(_)
        | ExpressionOp::Variable(_)
        | ExpressionOp::ReadObjectMember(_)
        | ExpressionOp::Member(_)
        | ExpressionOp::SystemVariable(_)
        | ExpressionOp::SystemList(_)
        | ExpressionOp::CustomVariable(_)
        | ExpressionOp::ObjectPointer(_)
        | ExpressionOp::ObjectSpecial
        | ExpressionOp::CastList
        | ExpressionOp::Pad => Ok(index),
        ExpressionOp::Negate
        | ExpressionOp::Not
        | ExpressionOp::Complement
        | ExpressionOp::Sine
        | ExpressionOp::SineUnshifted
        | ExpressionOp::Cosine
        | ExpressionOp::CosineShifted
        | ExpressionOp::AngleToObject
        | ExpressionOp::DistanceToObject
        | ExpressionOp::ItemCount
        | ExpressionOp::AnimationFrame
        | ExpressionOp::BgiAnimationFrame
        | ExpressionOp::BgiCharacterTriangle
        | ExpressionOp::BgiCharacterFloor
        | ExpressionOp::ActiveBitCount
        | ExpressionOp::CharacterMaxMp
        | ExpressionOp::PartyContains
        | ExpressionOp::PartyAdd
        | ExpressionOp::SelectRandomMember
        | ExpressionOp::KeyOn
        | ExpressionOp::KeyState(_)
        | ExpressionOp::CharacterStat(_)
        | ExpressionOp::ConsumeAndZero
        | ExpressionOp::PreIncrement
        | ExpressionOp::PreDecrement
        | ExpressionOp::PostIncrement
        | ExpressionOp::PostDecrement
        | ExpressionOp::CastI8
        | ExpressionOp::CastU8
        | ExpressionOp::CastI16
        | ExpressionOp::CastU16 => operand_start(operations, index, offset, opcode),
        ExpressionOp::Sps
        | ExpressionOp::Add
        | ExpressionOp::Subtract
        | ExpressionOp::Multiply
        | ExpressionOp::Divide
        | ExpressionOp::Remainder
        | ExpressionOp::ShiftLeft
        | ExpressionOp::ShiftRight
        | ExpressionOp::Angle
        | ExpressionOp::Angle2
        | ExpressionOp::DistanceToPoint
        | ExpressionOp::LessOrEqual
        | ExpressionOp::Less
        | ExpressionOp::Greater
        | ExpressionOp::GreaterOrEqual
        | ExpressionOp::Equal
        | ExpressionOp::NotEqual
        | ExpressionOp::BitwiseAnd
        | ExpressionOp::BitwiseXor
        | ExpressionOp::BitwiseOr
        | ExpressionOp::Assign
        | ExpressionOp::AddAssign
        | ExpressionOp::SubtractAssign
        | ExpressionOp::MultiplyAssign
        | ExpressionOp::DivideAssign
        | ExpressionOp::RemainderAssign
        | ExpressionOp::ShiftLeftAssign
        | ExpressionOp::ShiftRightAssign
        | ExpressionOp::AndAssign
        | ExpressionOp::XorAssign
        | ExpressionOp::OrAssign
        | ExpressionOp::PostIncrementMembers
        | ExpressionOp::PostDecrementMembers
        | ExpressionOp::PreIncrementMembers
        | ExpressionOp::PreDecrementMembers
        | ExpressionOp::AssignMembers
        | ExpressionOp::NotMembers
        | ExpressionOp::LogicalAnd
        | ExpressionOp::LogicalOr => {
            let right = operand_start(operations, index, offset, opcode)?;
            operand_start(operations, right, offset, opcode)
        }
        ExpressionOp::AndAssignMembers
        | ExpressionOp::XorAssignMembers
        | ExpressionOp::OrAssignMembers
        | ExpressionOp::XorAssignAllMembers
        | ExpressionOp::AndAssignAllMembers
        | ExpressionOp::OrAssignAllMembers
        | ExpressionOp::ShiftRightAssignAllMembers
        | ExpressionOp::MultiplyAssignAllMembers
        | ExpressionOp::DivideAssignAllMembers
        | ExpressionOp::SubtractAssignAllMembers
        | ExpressionOp::RemainderAssignAllMembers
        | ExpressionOp::PlusAssignAllMembers
        | ExpressionOp::ShiftLeftAssignAllMembers
        | ExpressionOp::AssignAllMembers
        | ExpressionOp::PickFirstMember
        | ExpressionOp::EqualMembers
        | ExpressionOp::NotEqualMembers
        | ExpressionOp::AndMembers
        | ExpressionOp::NandMembers
        | ExpressionOp::XorMembers
        | ExpressionOp::LessMembers
        | ExpressionOp::LessOrEqualMembers
        | ExpressionOp::GreaterOrEqualMembers
        | ExpressionOp::GreaterMembers => {
            let value = operand_start(operations, index, offset, opcode)?;
            let mask = operand_start(operations, value, offset, opcode)?;
            operand_start(operations, mask, offset, opcode)
        }
        ExpressionOp::NoOp | ExpressionOp::End => Err(expression_error(
            offset,
            opcode,
            "unsupported stack shape for a short-circuit operand",
        )),
    }
}

fn angle2(
    host: &mut dyn LegacyEventHost,
    delta_x: i32,
    delta_z: i32,
    offset: u32,
    opcode: u32,
) -> Result<i32, VmError> {
    if delta_x == 0 && delta_z == 0 {
        return Ok(0);
    }
    let scaled_z = delta_z.wrapping_shl(10);
    let scaled_x = delta_x.wrapping_shl(10);
    let delta = delta_x.wrapping_sub(delta_z);
    let mut entry = |table_offset| {
        host.read_atan_entry(table_offset)
            .map(i32::from)
            .map_err(|error| VmError::host(error, offset, opcode))
    };
    let fixed = if delta_z >= 0 {
        if delta_x >= 0 {
            if delta >= 0 {
                -1024 - entry(scaled_z.wrapping_div(delta_x).wrapping_shl(1))?
            } else {
                entry(scaled_x.wrapping_div(delta_z).wrapping_shl(1))? - 2048
            }
        } else if delta_x.wrapping_neg().wrapping_sub(delta_z) < 0 {
            2048 - entry(
                scaled_x
                    .wrapping_div(delta_z)
                    .wrapping_shl(1)
                    .wrapping_neg(),
            )?
        } else {
            1024 + entry(
                scaled_z
                    .wrapping_div(delta_x)
                    .wrapping_shl(1)
                    .wrapping_neg(),
            )?
        }
    } else if delta_x >= 0 {
        if delta_x.wrapping_sub(delta_z.wrapping_neg()) < 0 {
            entry(
                scaled_x
                    .wrapping_div(delta_z)
                    .wrapping_shl(1)
                    .wrapping_neg(),
            )?
            .wrapping_neg()
        } else {
            entry(
                scaled_z
                    .wrapping_div(delta_x)
                    .wrapping_shl(1)
                    .wrapping_neg(),
            )? - 1024
        }
    } else if delta < 0 {
        1024 - entry(scaled_z.wrapping_div(delta_x).wrapping_shl(1))?
    } else {
        entry(scaled_x.wrapping_div(delta_z).wrapping_shl(1))?
    };
    Ok(i32::from((fixed as i16) >> 4))
}
