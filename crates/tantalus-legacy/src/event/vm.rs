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
                InstructionKind::JumpSwitch {
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
                    self.pc = switch_target(&instruction, relative)?;
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
                InstructionKind::RunSound { code, sound } => {
                    let code = self.evaluate(host, &code, instruction.offset, opcode)?;
                    let sound = self.evaluate(host, &sound, instruction.offset, opcode)?;
                    self.pc = next;
                    events.push(HostEvent::RunSound {
                        code,
                        sound,
                        arguments: [0, 0, 0],
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
                InstructionKind::CalculateExitPosition => {
                    self.pc = next;
                    events.push(HostEvent::CalculateExitPosition);
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
                ExpressionOp::Add
                | ExpressionOp::Subtract
                | ExpressionOp::Multiply
                | ExpressionOp::Divide
                | ExpressionOp::Remainder
                | ExpressionOp::ShiftLeft
                | ExpressionOp::ShiftRight
                | ExpressionOp::Angle2
                | ExpressionOp::Less
                | ExpressionOp::Greater
                | ExpressionOp::GreaterOrEqual
                | ExpressionOp::Equal
                | ExpressionOp::NotEqual => {
                    let (left, right) = binary(&mut stack, operation, host)?;
                    let value = match operation {
                        ExpressionOp::Add => left.wrapping_add(right),
                        ExpressionOp::Subtract => left.wrapping_sub(right),
                        ExpressionOp::Multiply => left.wrapping_mul(right),
                        ExpressionOp::Divide if right != 0 => left.wrapping_div(right),
                        ExpressionOp::Remainder if right != 0 => left.wrapping_rem(right),
                        ExpressionOp::ShiftLeft => left.wrapping_shl(right as u32),
                        ExpressionOp::ShiftRight => left.wrapping_shr(right as u32),
                        ExpressionOp::Angle2 => angle2(host, left, right, offset, opcode)?,
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
                ExpressionOp::AddAssign | ExpressionOp::SubtractAssign => {
                    let operand = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing arithmetic-assignment operand")
                    })?;
                    let operand = read_value(host, operand, offset, opcode)?;
                    let target = stack.pop().ok_or_else(|| {
                        expression_error(offset, opcode, "missing arithmetic-assignment target")
                    })?;
                    let current = read_value(host, target, offset, opcode)?;
                    let value = match operation {
                        ExpressionOp::AddAssign => current.wrapping_add(operand),
                        ExpressionOp::SubtractAssign => current.wrapping_sub(operand),
                        _ => unreachable!(),
                    };
                    let value = match target.target {
                        Some(StackTarget::Variable(variable)) => {
                            let value = normalize(value, variable.value_type);
                            self.variables
                                .insert((variable.scope, variable.index), value);
                            value
                        }
                        Some(StackTarget::Member(member)) => {
                            let object = host
                                .select_object_member_target(None)
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                            host.write_object_member(ObjectMemberRef { object, member }, value)
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                            value
                        }
                        None => {
                            return Err(expression_error(
                                offset,
                                opcode,
                                "arithmetic-assignment target is not writable",
                            ));
                        }
                    };
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
                        None => {
                            return Err(expression_error(
                                offset,
                                opcode,
                                "assignment target is not writable",
                            ));
                        }
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
                ExpressionOp::ShiftRightAssignMembers => {
                    if stack.len() < 3 {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "member shift assignment requires mask and value inputs",
                        ));
                    }
                    let mask = read_value(host, stack[stack.len() - 3], offset, opcode)?;
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
                        target: None,
                    });
                }
                ExpressionOp::AssignMembers => {
                    if stack.len() < 2 {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "member assignment requires target and mask inputs",
                        ));
                    }
                    let Some(StackTarget::Member(_)) = stack[stack.len() - 2].target else {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "member assignment input is not a selected member",
                        ));
                    };
                    let mask = read_value(host, stack[stack.len() - 1], offset, opcode)?;
                    let target = stack[stack.len() - 2];
                    let mut result = 0i32;
                    for slot in 0..8 {
                        result >>= 1;
                        if mask & (1 << slot) != 0 {
                            host.select_object_member_target(Some(slot))
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                            result |=
                                i32::from(read_value(host, target, offset, opcode)? != 0) << 7;
                        }
                    }
                    stack.truncate(stack.len() - 2);
                    if let Some(target) = stack.pop() {
                        match target.target {
                            Some(StackTarget::Variable(variable)) => {
                                let value = normalize(result, variable.value_type);
                                self.variables
                                    .insert((variable.scope, variable.index), value);
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
                ExpressionOp::AndAssignMembers | ExpressionOp::XorAssignMembers => {
                    if stack.len() < 3 {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "member bitwise assignment requires target, mask, and value inputs",
                        ));
                    }
                    let Some(StackTarget::Member(member)) = stack[stack.len() - 3].target else {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "member bitwise assignment target is not a selected member",
                        ));
                    };
                    let mask = read_value(host, stack[stack.len() - 2], offset, opcode)?;
                    let operand = read_value(host, stack[stack.len() - 1], offset, opcode)?;
                    let target = stack[stack.len() - 3];
                    let mut selected_values = 0i32;
                    for slot in 0..8 {
                        selected_values >>= 1;
                        if mask & (1 << slot) != 0 {
                            host.select_object_member_target(Some(slot))
                                .map_err(|error| VmError::host(error, offset, opcode))?;
                            selected_values |=
                                i32::from(read_value(host, target, offset, opcode)? != 0) << 7;
                        }
                    }
                    let object = host
                        .select_object_member_target(None)
                        .map_err(|error| VmError::host(error, offset, opcode))?;
                    let reference = ObjectMemberRef { object, member };
                    let value = match operation {
                        ExpressionOp::AndAssignMembers => selected_values & operand,
                        ExpressionOp::XorAssignMembers => selected_values ^ operand,
                        _ => unreachable!(),
                    };
                    host.write_object_member(reference, value)
                        .map_err(|error| VmError::host(error, offset, opcode))?;
                    stack.truncate(stack.len() - 3);
                    stack.push(StackValue {
                        value,
                        target: None,
                    });
                }
                ExpressionOp::XorAssignAllMembers | ExpressionOp::AndAssignAllMembers => {
                    if stack.len() < 3 {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "all-member bitwise assignment requires target, mask, and value inputs",
                        ));
                    }
                    let Some(StackTarget::Member(member)) = stack[stack.len() - 3].target else {
                        return Err(expression_error(
                            offset,
                            opcode,
                            "all-member bitwise assignment target is not a selected member",
                        ));
                    };
                    let mask = read_value(host, stack[stack.len() - 2], offset, opcode)?;
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
                                ExpressionOp::XorAssignAllMembers => current ^ operand,
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
                | ExpressionOp::AndMembers
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
                                ExpressionOp::AndMembers => value & comparison != 0,
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
        InstructionKind::JumpSwitch { .. } => 0x0b,
        InstructionKind::DeleteObject(_) => 0x1c,
        InstructionKind::SetActorPosition { .. } => 0x1d,
        InstructionKind::SetIdleAnimation { .. } => 0x33,
        InstructionKind::SetWalkAnimation { .. } => 0x34,
        InstructionKind::SetRunAnimation { .. } => 0x35,
        InstructionKind::SetLeftTurnAnimation { .. } => 0x7a,
        InstructionKind::SetRightTurnAnimation { .. } => 0x7b,
        InstructionKind::SetJumpAnimation { .. } => 0x94,
        InstructionKind::SetActorRadii { .. } => 0x4b,
        InstructionKind::SetIdleAnimationSpeeds { .. } => 0x86,
        InstructionKind::SetHeadFocusMasks { .. } => 0x8b,
        InstructionKind::SetObjectFlags { .. } => 0x93,
        InstructionKind::SetActorFacing { .. } => 0x36,
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
        InstructionKind::BackgroundMove { .. } => 0x58,
        InstructionKind::SetTileShadeColor { .. } => 0x59,
        InstructionKind::SetSpsParameters { .. } => 0xb3,
        InstructionKind::InitializeObject { .. } => 0x09,
        InstructionKind::InitializeRegion { .. } => 0x08,
        InstructionKind::InitializeCode { .. } => 0x07,
        InstructionKind::SetTriangleActive { .. } => 0x9a,
        InstructionKind::RunSound { .. } => 0xc5,
        InstructionKind::AttachObjectOffset { .. } => 0xd4,
        InstructionKind::ResetWalkState => 0x25,
        InstructionKind::SetMovementSpeed(_) => 0x26,
        InstructionKind::SetActorModel { .. } => 0x2f,
        InstructionKind::SetNavigationAttributeMask(_) => 0x27,
        InstructionKind::Yield(opcode) => u32::from(*opcode),
        InstructionKind::SetShadowEnabled { opcode, .. } => u32::from(*opcode),
        InstructionKind::SetTextVariable { .. } => 0x66,
        InstructionKind::SetObjectScale { .. } => 0x9f,
        InstructionKind::SetVibrationFrameRate(_) => 0xfa,
        InstructionKind::RequestScriptAsync { .. } => 0x10,
        InstructionKind::DefineTileViewport { .. } => 0xc8,
        InstructionKind::PreserveAndHideAll => 0xd5,
        InstructionKind::RestoreAllVisibility => 0xd6,
        InstructionKind::SetTimerRunning(_) => 0x7d,
        InstructionKind::RequestCallerAsync { .. } => 0x16,
        InstructionKind::CalculateExitPosition => 0xa4,
        InstructionKind::RequestCallerSync { .. } => 0x14,
        InstructionKind::SetSimulationFloor { .. } => 0x104,
        InstructionKind::SetSimulationFrameRate { .. } => 0x105,
        InstructionKind::SetTileAnimationFrame { .. } => 0xe7,
        InstructionKind::OpenPartyMenu { .. } => 0xb2,
        InstructionKind::InitializeCharacter { .. } => 0xfe,
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

fn switch_target(instruction: &Instruction, relative: u16) -> Result<u32, VmError> {
    instruction
        .offset
        .checked_add(1)
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
        | ExpressionOp::CustomVariable(_)
        | ExpressionOp::CastList
        | ExpressionOp::Pad => Ok(index),
        ExpressionOp::Negate
        | ExpressionOp::Not
        | ExpressionOp::Complement
        | ExpressionOp::Sine
        | ExpressionOp::AngleToObject
        | ExpressionOp::ItemCount
        | ExpressionOp::AnimationFrame
        | ExpressionOp::BgiAnimationFrame
        | ExpressionOp::PartyContains
        | ExpressionOp::ConsumeAndZero
        | ExpressionOp::PreIncrement
        | ExpressionOp::PreDecrement
        | ExpressionOp::PostIncrement
        | ExpressionOp::PostDecrement
        | ExpressionOp::CastI8
        | ExpressionOp::CastU8
        | ExpressionOp::CastI16
        | ExpressionOp::CastU16 => operand_start(operations, index, offset, opcode),
        ExpressionOp::Add
        | ExpressionOp::Subtract
        | ExpressionOp::Multiply
        | ExpressionOp::Divide
        | ExpressionOp::Remainder
        | ExpressionOp::ShiftLeft
        | ExpressionOp::ShiftRight
        | ExpressionOp::Angle2
        | ExpressionOp::Less
        | ExpressionOp::Greater
        | ExpressionOp::GreaterOrEqual
        | ExpressionOp::Equal
        | ExpressionOp::NotEqual
        | ExpressionOp::Assign
        | ExpressionOp::AddAssign
        | ExpressionOp::SubtractAssign
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
        | ExpressionOp::XorAssignAllMembers
        | ExpressionOp::AndAssignAllMembers
        | ExpressionOp::EqualMembers
        | ExpressionOp::AndMembers
        | ExpressionOp::LessOrEqualMembers
        | ExpressionOp::GreaterOrEqualMembers
        | ExpressionOp::GreaterMembers => {
            let value = operand_start(operations, index, offset, opcode)?;
            let mask = operand_start(operations, value, offset, opcode)?;
            operand_start(operations, mask, offset, opcode)
        }
        ExpressionOp::NoOp | ExpressionOp::ShiftRightAssignMembers | ExpressionOp::End => {
            Err(expression_error(
                offset,
                opcode,
                "unsupported stack shape for a short-circuit operand",
            ))
        }
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
    Ok(i32::from((fixed as i16) >> 8))
}
