use super::*;
use std::collections::BTreeMap;
use tantalus_contracts::EntityId;

const OP_NOP: u32 = 0x00;
const OP_UNUSED_PRINT: u32 = 0x6c;
const OP_JUMP: u32 = 0x01;
const OP_JUMP_IF_NOT: u32 = 0x02;
const OP_JUMP_IF: u32 = 0x03;
const OP_RETURN: u32 = 0x04;
const OP_JUMP_SWITCH_EX: u32 = 0x06;
const OP_JUMP_SWITCH: u32 = 0x0b;
const OP_JUMP_SWITCH_WIDE: u32 = 0x0d;
const OP_DELETE_OBJECT: u32 = 0x1c;
const OP_SET_ACTOR_POSITION: u32 = 0x1d;
const OP_SET_IDLE_ANIMATION: u32 = 0x33;
const OP_SET_WALK_ANIMATION: u32 = 0x34;
const OP_SET_RUN_ANIMATION: u32 = 0x35;
const OP_PLAY_ANIMATION: u32 = 0x40;
const OP_SET_ANIMATION_RANGE: u32 = 0x3d;
const OP_PRINT_QUAD: u32 = 0x78;
const OP_SET_LEFT_TURN_ANIMATION: u32 = 0x7a;
const OP_SET_RIGHT_TURN_ANIMATION: u32 = 0x7b;
const OP_SET_JUMP_ANIMATION: u32 = 0x94;
const OP_SET_ACTOR_RADII: u32 = 0x4b;
const OP_SET_IDLE_ANIMATION_SPEEDS: u32 = 0x86;
const OP_SET_HEAD_FOCUS_MASKS: u32 = 0x8b;
const OP_SET_HEAD_FOCUS_FLAGS: u32 = 0x47;
const OP_PADDING10: u32 = 0x0a;
const OP_SET_WALK_TURN_SPEED: u32 = 0x55;
const OP_SET_OBJECT_FLAGS: u32 = 0x93;
const OP_SET_ACTOR_FACING: u32 = 0x36;
const OP_START_ACTOR_TURN: u32 = 0x56;
const OP_EVALUATE: u32 = 0x05;
const OP_WINDOW_SYNC: u32 = 0x1f;
const OP_WINDOW_ASYNC: u32 = 0x20;
const OP_CLOSE_WINDOW: u32 = 0x21;
const OP_WAIT: u32 = 0x22;
const OP_WAIT_ANIMATION: u32 = 0x41;
const OP_WAIT_TURN: u32 = 0x50;
const OP_WAIT_WINDOW: u32 = 0x54;
const OP_WALK_ACTOR: u32 = 0x23;
const OP_MOVE_ACTOR: u32 = 0xbf;
const OP_AIM_AT_CAMERA: u32 = 0xb8;
const OP_DEFINE_PLAYER_CONTROL: u32 = 0x2c;
const OP_SET_CONTROL_DIRECTION: u32 = 0x67;
const OP_CINEMATIC: u32 = 0x28;
const OP_DEFINE_REGION: u32 = 0x29;
const OP_BACKGROUND_MOVE: u32 = 0x58;
const OP_SET_TILE_SHADE_COLOR: u32 = 0x59;
const OP_SET_SPS_PARAMETERS: u32 = 0xb3;
const OP_INITIALIZE_OBJECT: u32 = 0x09;
const OP_INITIALIZE_REGION: u32 = 0x08;
const OP_INITIALIZE_CODE: u32 = 0x07;
const OP_SET_TRIANGLE_ACTIVE: u32 = 0x9a;
const OP_RUN_SOUND: u32 = 0xc5;
const OP_RUN_SOUND1: u32 = 0xc6;
const OP_RUN_SOUND2: u32 = 0xc7;
const OP_RUN_SOUND3: u32 = 0xc8;
const OP_ATTACH_OBJECT_OFFSET: u32 = 0xd4;
const OP_RESET_WALK_STATE: u32 = 0x25;
const OP_SET_MOVEMENT_SPEED: u32 = 0x26;
const OP_SET_ACTOR_MODEL: u32 = 0x2f;
const OP_SET_NAVIGATION_ATTRIBUTE_MASK: u32 = 0x27;
const OP_YIELD: u32 = 0x0e;
const OP_PADDING15: u32 = 0x0f;
const OP_PADDING17: u32 = 0x17;
const OP_SHADOW_ON: u32 = 0x7f;
const OP_SHADOW_OFF: u32 = 0x80;
const OP_SET_TEXT_VARIABLE: u32 = 0x66;
const OP_SET_OBJECT_SCALE: u32 = 0x9f;
const OP_SET_VIBRATION_FRAME_RATE: u32 = 0xfa;
const OP_PRELOAD_FIELD: u32 = 0xfd;
const OP_SET_RAIN: u32 = 0xd8;
const OP_SET_HERE_ICON: u32 = 0xef;
const OP_REQUEST_SCRIPT_ASYNC: u32 = 0x10;
const OP_REQUEST_SCRIPT_WHEN_READY: u32 = 0x12;
const OP_DEFINE_TILE_VIEWPORT: u32 = 0xc9;
const OP_SET_TILE_VIEWPORT_ALPHA: u32 = 0xe6;
const OP_BATTLE_MESSAGE: u32 = 0xd0;
const OP_GLOBAL_CLEAR: u32 = 0xd1;
const OP_DEBUG_SAVE: u32 = 0xd2;
const OP_DEBUG_LOAD: u32 = 0xd3;
const OP_PRESERVE_AND_HIDE_ALL: u32 = 0xd5;
const OP_RESTORE_ALL_VISIBILITY: u32 = 0xd6;
const OP_SET_TIMER_RUNNING: u32 = 0x7d;
const OP_REQUEST_CALLER_ASYNC: u32 = 0x16;
const OP_REQUEST_CALLER_WHEN_READY: u32 = 0x18;
const OP_CALCULATE_EXIT_POSITION: u32 = 0xa4;
const OP_WALK_TO_EXIT: u32 = 0xa0;
const OP_CALCULATE_SCREEN_POSITION: u32 = 0xa9;
const OP_CHANGE_FIELD: u32 = 0x2b;
const OP_REQUEST_CALLER_SYNC: u32 = 0x14;
const OP_ATTACH_TILE: u32 = 0x92;
const OP_BATTLE_SCENE_START: u32 = 0x100;
const OP_SET_SIMULATION_FLOOR: u32 = 0x104;
const OP_SET_SIMULATION_FRAME_RATE: u32 = 0x105;
const OP_SET_TILE_ANIMATION_FRAME_WAIT: u32 = 0x62;
const OP_ADD_ITEM: u32 = 0x48;
const OP_REMOVE_ITEM: u32 = 0x49;
const OP_INCREMENT_FROG_COUNT: u32 = 0xe0;
const OP_SET_TILE_ANIMATION_FLAGS: u32 = 0x63;
const OP_SET_TILE_ANIMATION_FRAME: u32 = 0xe7;
const OP_OPEN_PARTY_MENU: u32 = 0xb2;
const OP_INITIALIZE_CHARACTER: u32 = 0xfe;
const OP_STOP_TEXTURE_ANIMATION: u32 = 0xc2;
const OP_RUN_BATTLE_CODE: u32 = 0x4a;
const OP_SET_BATTLE_MUSIC_CONTINUATION: u32 = 0x1b;
const OP_RUN_MODEL_CODE: u32 = 0x88;
const OP_ADD_BATTLE_STATUS: u32 = 0x11d;
const OP_SET_MODEL_LIGHT: u32 = 0x121;
const OP_CHANGE_ELEMENTAL_AFFINITY: u32 = 0x122;
const OP_DASH_ON: u32 = 0xf0;
const OP_FULL_MEMBER: u32 = 0xb4;
const OP_MOVE_TILE: u32 = 0x5a;
const OP_SET_TILE_SCROLL_WITH_OFFSET: u32 = 0xe4;
const OP_SET_CAMERA_BOUNDS: u32 = 0x1e;
const OP_SET_CHARACTER_ROW: u32 = 0x61;
const OP_CLEAR_PARTY_STATUS: u32 = 0xd9;
const OP_SET_ACTOR_PITCH_ROLL: u32 = 0x37;

const EXPR_POST_INCREMENT_MEMBERS: u8 = 8;
const EXPR_NEGATE: u8 = 13;
const EXPR_NOT: u8 = 14;
const EXPR_MULTIPLY: u8 = 17;
const EXPR_DIVIDE: u8 = 18;
const EXPR_REMAINDER: u8 = 19;
const EXPR_ADD: u8 = 20;
const EXPR_SUBTRACT: u8 = 21;
const EXPR_LESS: u8 = 24;
const EXPR_GREATER: u8 = 25;
const EXPR_EQUAL: u8 = 32;
const EXPR_NOT_EQUAL: u8 = 33;
const EXPR_ASSIGN: u8 = 44;
const EXPR_CONST_I16: u8 = 125;
const EXPR_CONST_I32: u8 = 126;
const EXPR_END: u8 = 127;

pub fn decode_program(bytes: &[u8]) -> Result<EventProgram, VmError> {
    let mut decoder = Decoder {
        bytes,
        position: 0,
        current_opcode: None,
    };
    let mut instructions = BTreeMap::new();
    while decoder.position < bytes.len() {
        let offset = decoder.position as u32;
        let instruction = decoder.instruction().map_err(|mut error| {
            let preview = decoder
                .bytes
                .get(offset as usize..)
                .unwrap_or_default()
                .iter()
                .take(16)
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<_>>()
                .join(" ");
            error.message = format!(
                "instruction at {offset} (opcode {:?}, bytes {preview}): {}",
                decoder.current_opcode, error.message
            );
            error
        })?;
        instructions.insert(instruction.offset, instruction);
    }
    Ok(EventProgram {
        instructions,
        source_length: bytes.len() as u32,
    })
}

struct Decoder<'a> {
    bytes: &'a [u8],
    position: usize,
    current_opcode: Option<u32>,
}

impl Decoder<'_> {
    fn instruction(&mut self) -> Result<Instruction, VmError> {
        let offset = self.position as u32;
        let mut code = 0u32;
        let mut code_byte = self.byte()?;
        while code_byte == 0xff {
            code += 0x100;
            code_byte = self.byte()?;
        }
        let opcode = code + u32::from(code_byte);
        self.current_opcode = Some(opcode);
        let kind = match opcode {
            OP_NOP => {
                self.byte()?;
                InstructionKind::Nop
            }
            OP_UNUSED_PRINT => InstructionKind::NoOp(opcode),
            OP_PRINT_QUAD => {
                self.byte()?;
                InstructionKind::NoOp(opcode)
            }
            OP_JUMP => InstructionKind::Jump(self.i16()?),
            OP_JUMP_IF_NOT => InstructionKind::JumpIfNot {
                condition: self.expression()?,
                relative: self.i16()?,
            },
            OP_JUMP_IF => InstructionKind::JumpIf {
                condition: self.expression()?,
                relative: self.i16()?,
            },
            OP_JUMP_SWITCH_EX => {
                let case_count = self.byte()? as usize;
                let default_relative = self.i16()? as u16;
                let mut cases = Vec::with_capacity(case_count);
                for _ in 0..case_count {
                    cases.push([self.i16()? as u16, self.i16()? as u16]);
                }
                InstructionKind::JumpSwitchEx {
                    default_relative,
                    cases,
                }
            }
            OP_JUMP_SWITCH | OP_JUMP_SWITCH_WIDE => {
                let case_count = if opcode == OP_JUMP_SWITCH_WIDE {
                    self.i16()? as u16 as usize
                } else {
                    self.byte()? as usize
                };
                let base = self.i16()?;
                let default_relative = self.i16()? as u16;
                let mut case_relative = Vec::with_capacity(case_count);
                for _ in 0..case_count {
                    case_relative.push(self.i16()? as u16);
                }
                if opcode == OP_JUMP_SWITCH_WIDE {
                    InstructionKind::JumpSwitchWide {
                        base,
                        default_relative,
                        case_relative,
                    }
                } else {
                    InstructionKind::JumpSwitch {
                        base,
                        default_relative,
                        case_relative,
                    }
                }
            }
            OP_DELETE_OBJECT => InstructionKind::DeleteObject(self.byte()?),
            OP_SET_ACTOR_POSITION => {
                let flags = self.byte()?;
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.i16()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::SetActorPosition {
                    x: argument(1)?,
                    z: argument(2)?,
                }
            }
            OP_SET_ACTOR_RADII => {
                let flags = self.byte()?;
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.byte()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::SetActorRadii {
                    pathing_size: argument(1)?,
                    collision_radius: argument(2)?,
                    talk_radius: argument(4)?,
                }
            }
            OP_SET_IDLE_ANIMATION_SPEEDS => {
                let flags = self.byte()?;
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.byte()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::SetIdleAnimationSpeeds([
                    argument(1)?,
                    argument(2)?,
                    argument(4)?,
                    argument(8)?,
                ])
            }
            OP_SET_HEAD_FOCUS_MASKS => {
                let flags = self.byte()?;
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.byte()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::SetHeadFocusMasks {
                    self_mask: argument(1)?,
                    target_mask: argument(2)?,
                }
            }
            OP_SET_HEAD_FOCUS_FLAGS => {
                let flags = self.byte()?;
                let enabled = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.byte()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::SetHeadFocusFlags(enabled)
            }
            OP_SET_WALK_TURN_SPEED => {
                let flags = self.byte()?;
                let speed = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.byte()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::SetWalkTurnSpeed(speed)
            }
            OP_SET_OBJECT_FLAGS => {
                let flags = self.byte()?;
                let object_flags = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.byte()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::SetObjectFlags(object_flags)
            }
            OP_SET_RIGHT_TURN_ANIMATION => {
                let flags = self.byte()?;
                let animation = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.i16()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::SetRightTurnAnimation(animation)
            }
            OP_SET_JUMP_ANIMATION => {
                let flags = self.byte()?;
                let mut argument = |bit: u8, short: bool| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        let value = if short {
                            i32::from(self.i16()?)
                        } else {
                            i32::from(self.byte()?)
                        };
                        Ok(Expression(vec![
                            ExpressionOp::Constant(value),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::SetJumpAnimation {
                    animation: argument(1, true)?,
                    parameters: [argument(2, false)?, argument(4, false)?],
                }
            }
            OP_SET_LEFT_TURN_ANIMATION => {
                let flags = self.byte()?;
                let animation = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.i16()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::SetLeftTurnAnimation(animation)
            }
            OP_SET_RUN_ANIMATION => {
                let flags = self.byte()?;
                let animation = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.i16()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::SetRunAnimation(animation)
            }
            OP_PLAY_ANIMATION => {
                let flags = self.byte()?;
                let animation = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.i16()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::PlayAnimation(animation)
            }
            OP_SET_ANIMATION_RANGE => {
                let flags = self.byte()?;
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.byte()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::SetAnimationRange {
                    start: argument(1)?,
                    end: argument(2)?,
                }
            }
            OP_SET_WALK_ANIMATION => {
                let flags = self.byte()?;
                let animation = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.i16()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::SetWalkAnimation(animation)
            }
            OP_SET_IDLE_ANIMATION => {
                let flags = self.byte()?;
                let animation = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.i16()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::SetIdleAnimation(animation)
            }
            OP_SET_ACTOR_FACING => {
                let flags = self.byte()?;
                let angle = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.byte()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::SetActorFacing(angle)
            }
            OP_START_ACTOR_TURN => {
                let flags = self.byte()?;
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.byte()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::StartActorTurn {
                    angle: argument(1)?,
                    speed: argument(2)?,
                }
            }
            OP_RETURN => InstructionKind::Return,
            OP_EVALUATE => InstructionKind::Evaluate(self.expression()?),
            OP_WAIT => {
                self.byte()?;
                InstructionKind::Wait(self.expression()?)
            }
            OP_WAIT_ANIMATION => InstructionKind::WaitAnimation,
            OP_WAIT_TURN => InstructionKind::WaitTurn,
            OP_WINDOW_SYNC | OP_WINDOW_ASYNC => {
                self.byte()?;
                let window = self.expression()?;
                let flags = self.expression()?;
                let text = self.expression()?;
                if opcode == OP_WINDOW_SYNC {
                    InstructionKind::WindowSync {
                        window,
                        flags,
                        text,
                    }
                } else {
                    InstructionKind::WindowAsync {
                        window,
                        flags,
                        text,
                    }
                }
            }
            OP_WAIT_WINDOW => {
                self.byte()?;
                InstructionKind::WaitWindow(self.expression()?)
            }
            OP_CLOSE_WINDOW => {
                self.byte()?;
                InstructionKind::CloseWindow(self.expression()?)
            }
            OP_WALK_ACTOR => {
                let flags = self.byte()?;
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.i16()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::WalkActor {
                    x: argument(1)?,
                    z: argument(2)?,
                }
            }
            OP_MOVE_ACTOR => {
                self.byte()?;
                InstructionKind::MoveActor {
                    actor: self.expression()?,
                    x: self.expression()?,
                    z: self.expression()?,
                }
            }
            OP_AIM_AT_CAMERA => {
                self.byte()?;
                InstructionKind::AimAtCamera
            }
            OP_DEFINE_PLAYER_CONTROL => {
                self.byte()?;
                InstructionKind::DefinePlayerControl
            }
            OP_SET_CONTROL_DIRECTION => {
                self.byte()?;
                InstructionKind::SetControlDirection {
                    arrow: self.expression()?,
                    analog: self.expression()?,
                }
            }
            OP_CINEMATIC => {
                let flags = self.byte()?;
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.byte()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::Cinematic {
                    operation: argument(1)?,
                    argument: argument(2)?,
                }
            }
            OP_DEFINE_REGION => {
                let flags = u16::from(self.byte()?);
                if flags & 1 != 0 {
                    return Err(VmError::new(
                        VmErrorKind::UnsupportedOpcode,
                        offset,
                        Some(opcode),
                        "expression region vertex counts require runtime-length decoding",
                    ));
                }
                let vertex_count = self.byte()? as usize;
                if vertex_count > 8 {
                    return Err(VmError::new(
                        VmErrorKind::Decode,
                        offset,
                        Some(opcode),
                        "region exceeds the eight-vertex Quad capacity",
                    ));
                }
                let mut vertices = Vec::with_capacity(vertex_count);
                for index in 0..vertex_count {
                    let mut argument = |bit: u16| {
                        if flags & bit != 0 {
                            self.expression()
                        } else {
                            Ok(Expression(vec![
                                ExpressionOp::Constant(i32::from(self.i16()?)),
                                ExpressionOp::End,
                            ]))
                        }
                    };
                    let x_bit = 1 << (1 + index * 2);
                    vertices.push([argument(x_bit)?, argument(x_bit << 1)?]);
                }
                InstructionKind::DefineRegion { vertices }
            }
            OP_BACKGROUND_MOVE => {
                self.byte()?;
                InstructionKind::BackgroundMove {
                    x: self.expression()?,
                    y: self.expression()?,
                    z: self.expression()?,
                }
            }
            OP_SET_TILE_SHADE_COLOR => {
                let flags = self.byte()?;
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.byte()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::SetTileShadeColor {
                    overlay: argument(1)?,
                    red: argument(2)?,
                    green: argument(4)?,
                    blue: argument(8)?,
                }
            }
            OP_SET_SPS_PARAMETERS => {
                let flags = self.byte()?;
                let mut byte_argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.byte()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                let object = byte_argument(1)?;
                let operation = byte_argument(2)?;
                let mut wide_argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.i16()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::SetSpsParameters {
                    object,
                    operation,
                    arguments: [wide_argument(4)?, wide_argument(8)?, wide_argument(16)?],
                }
            }
            OP_INITIALIZE_OBJECT => InstructionKind::InitializeObject {
                source: self.byte()?,
                unique_id: self.byte()?,
            },
            OP_INITIALIZE_REGION => InstructionKind::InitializeRegion {
                source: self.byte()?,
                unique_id: self.byte()?,
            },
            OP_INITIALIZE_CODE => InstructionKind::InitializeCode {
                source: self.byte()?,
                unique_id: self.byte()?,
            },
            OP_SET_TRIANGLE_ACTIVE => {
                let flags = self.byte()?;
                let triangle = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.i16()?)),
                        ExpressionOp::End,
                    ])
                };
                let active = if flags & 2 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.byte()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::SetTriangleActive { triangle, active }
            }
            OP_RUN_SOUND | OP_RUN_SOUND1 | OP_RUN_SOUND2 | OP_RUN_SOUND3 => {
                let flags = self.byte()?;
                let mut argument = |bit: u8, width: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        let value = match width {
                            1 => i32::from(self.byte()?),
                            2 => i32::from(self.i16()?),
                            3 => self.i24()?,
                            _ => unreachable!(),
                        };
                        Ok(Expression(vec![
                            ExpressionOp::Constant(value),
                            ExpressionOp::End,
                        ]))
                    }
                };
                let zero = || Expression(vec![ExpressionOp::Constant(0), ExpressionOp::End]);
                InstructionKind::RunSound {
                    code: argument(1, 2)?,
                    sound: argument(2, 2)?,
                    arguments: [
                        if opcode == OP_RUN_SOUND {
                            zero()
                        } else {
                            argument(4, 3)?
                        },
                        if opcode == OP_RUN_SOUND2 || opcode == OP_RUN_SOUND3 {
                            argument(8, 1)?
                        } else {
                            zero()
                        },
                        if opcode == OP_RUN_SOUND3 {
                            argument(16, 1)?
                        } else {
                            zero()
                        },
                    ],
                    opcode: opcode as u8,
                }
            }
            OP_ATTACH_OBJECT_OFFSET => {
                let flags = self.byte()?;
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.i16()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::AttachObjectOffset {
                    x: argument(1)?,
                    y: argument(2)?,
                    z: argument(4)?,
                }
            }
            OP_RESET_WALK_STATE => {
                self.byte()?;
                InstructionKind::ResetWalkState
            }
            OP_SET_MOVEMENT_SPEED => {
                let flags = self.byte()?;
                let speed = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.byte()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::SetMovementSpeed(speed)
            }
            OP_SET_ACTOR_MODEL => {
                let flags = self.byte()?;
                let model = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.i16()?)),
                        ExpressionOp::End,
                    ])
                };
                let eye_height = if flags & 2 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.byte()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::SetActorModel { model, eye_height }
            }
            OP_SET_NAVIGATION_ATTRIBUTE_MASK => {
                let flags = self.byte()?;
                let mask = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.byte()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::SetNavigationAttributeMask(mask)
            }
            OP_YIELD | OP_PADDING10 | OP_PADDING15 | OP_PADDING17 => {
                self.byte()?;
                InstructionKind::Yield(opcode as u8)
            }
            OP_SHADOW_ON | OP_SHADOW_OFF => {
                self.byte()?;
                InstructionKind::SetShadowEnabled {
                    enabled: opcode == OP_SHADOW_ON,
                    opcode: opcode as u8,
                }
            }
            OP_SET_TEXT_VARIABLE => {
                let flags = self.byte()?;
                let script = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.byte()?)),
                        ExpressionOp::End,
                    ])
                };
                let value = if flags & 2 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.i16()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::SetTextVariable { script, value }
            }
            OP_SET_OBJECT_SCALE => {
                let flags = self.byte()?;
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.byte()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::SetObjectScale {
                    actor: argument(1)?,
                    x: argument(2)?,
                    z: argument(4)?,
                    y: argument(8)?,
                }
            }
            OP_SET_VIBRATION_FRAME_RATE => {
                let flags = self.byte()?;
                let rate = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.i16()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::SetVibrationFrameRate(rate)
            }
            OP_PRELOAD_FIELD => {
                let flags = self.byte()?;
                let hint = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.byte()?)),
                        ExpressionOp::End,
                    ])
                };
                let field = if flags & 2 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.i16()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::PreloadField { hint, field }
            }
            OP_ADD_ITEM => {
                let flags = self.byte()?;
                let mut argument = |bit: u8, short: bool| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        let value = if short {
                            i32::from(self.i16()?)
                        } else {
                            i32::from(self.byte()?)
                        };
                        Ok(Expression(vec![
                            ExpressionOp::Constant(value),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::AddItem {
                    item: argument(1, true)?,
                    count: argument(2, false)?,
                }
            }
            OP_REMOVE_ITEM => {
                let flags = self.byte()?;
                let mut argument = |bit: u8, short: bool| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        let value = if short {
                            i32::from(self.i16()?)
                        } else {
                            i32::from(self.byte()?)
                        };
                        Ok(Expression(vec![
                            ExpressionOp::Constant(value),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::RemoveItem {
                    item: argument(1, true)?,
                    count: argument(2, false)?,
                }
            }
            OP_SET_RAIN => {
                let flags = self.byte()?;
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.byte()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::SetRain {
                    strength: argument(1)?,
                    speed: argument(2)?,
                }
            }
            OP_REQUEST_SCRIPT_ASYNC => {
                let flags = self.byte()?;
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.byte()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::RequestScriptAsync {
                    level: argument(1)?,
                    target: argument(2)?,
                    tag: self.byte()?,
                }
            }
            OP_REQUEST_SCRIPT_WHEN_READY => {
                let flags = self.byte()?;
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.byte()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::RequestScriptWhenReady {
                    level: argument(1)?,
                    target: argument(2)?,
                    tag: self.byte()?,
                }
            }
            OP_DEFINE_TILE_VIEWPORT => {
                let flags = self.byte()?;
                let viewport = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.byte()?)),
                        ExpressionOp::End,
                    ])
                };
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.i16()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::DefineTileViewport {
                    viewport,
                    x: argument(2)?,
                    y: argument(4)?,
                    width: argument(8)?,
                    height: argument(16)?,
                }
            }
            OP_SET_TILE_VIEWPORT_ALPHA => {
                let flags = self.byte()?;
                let viewport = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.byte()?)),
                        ExpressionOp::End,
                    ])
                };
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.i16()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::SetTileViewportAlpha {
                    viewport,
                    alpha: [argument(2)?, argument(4)?],
                }
            }
            OP_BATTLE_MESSAGE => {
                let flags = self.byte()?;
                let text = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.i16()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::ShowBattleMessage(text)
            }
            OP_GLOBAL_CLEAR | OP_DEBUG_SAVE | OP_DEBUG_LOAD => {
                self.byte()?;
                InstructionKind::NoOp(opcode)
            }
            OP_PRESERVE_AND_HIDE_ALL => {
                self.byte()?;
                InstructionKind::PreserveAndHideAll
            }
            OP_RESTORE_ALL_VISIBILITY => {
                self.byte()?;
                InstructionKind::RestoreAllVisibility
            }
            OP_SET_TIMER_RUNNING => {
                let flags = self.byte()?;
                let running = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.byte()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::SetTimerRunning(running)
            }
            OP_REQUEST_CALLER_ASYNC => {
                let flags = self.byte()?;
                let level = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.byte()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::RequestCallerAsync {
                    level,
                    tag: self.byte()?,
                }
            }
            OP_REQUEST_CALLER_WHEN_READY => {
                let flags = self.byte()?;
                let level = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.byte()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::RequestCallerWhenReady {
                    level,
                    tag: self.byte()?,
                }
            }
            OP_CALCULATE_EXIT_POSITION => {
                self.byte()?;
                InstructionKind::CalculateExitPosition
            }
            OP_WALK_TO_EXIT => InstructionKind::WalkToExit,
            OP_CALCULATE_SCREEN_POSITION => {
                let flags = self.byte()?;
                let object = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.byte()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::CalculateScreenPosition(object)
            }
            OP_CHANGE_FIELD => {
                let flags = self.byte()?;
                let field = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.i16()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::ChangeField(field)
            }
            OP_REQUEST_CALLER_SYNC => {
                let flags = self.byte()?;
                let level = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.byte()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::RequestCallerSync {
                    level,
                    tag: self.byte()?,
                }
            }
            OP_BATTLE_SCENE_START => {
                let flags = self.byte()?;
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.byte()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                argument(1)?;
                argument(2)?;
                InstructionKind::NoOp(opcode)
            }
            OP_SET_SIMULATION_FLOOR => {
                let flags = self.byte()?;
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.byte()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::SetSimulationFloor {
                    simulation: argument(1)?,
                    floor: argument(2)?,
                }
            }
            OP_SET_SIMULATION_FRAME_RATE => {
                let flags = self.byte()?;
                let mut argument = |bit: u8, short: bool| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        let value = if short {
                            i32::from(self.i16()?)
                        } else {
                            i32::from(self.byte()?)
                        };
                        Ok(Expression(vec![
                            ExpressionOp::Constant(value),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::SetSimulationFrameRate {
                    simulation: argument(1, false)?,
                    frame_rate: argument(2, true)?,
                }
            }
            OP_SET_TILE_ANIMATION_FLAGS => {
                let flags = self.byte()?;
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.byte()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::SetTileAnimationFlags {
                    animation: argument(1)?,
                    flags: argument(2)?,
                }
            }
            OP_SET_TILE_ANIMATION_FRAME_WAIT => {
                let flags = self.byte()?;
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.byte()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::SetTileAnimationFrameWait {
                    animation: argument(1)?,
                    frame: argument(2)?,
                    wait: argument(4)?,
                }
            }
            OP_INCREMENT_FROG_COUNT => InstructionKind::IncrementFrogCount,
            OP_SET_TILE_ANIMATION_FRAME => {
                let flags = self.byte()?;
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.byte()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::SetTileAnimationFrame {
                    animation: argument(1)?,
                    frame: argument(2)?,
                }
            }
            OP_OPEN_PARTY_MENU => {
                let flags = self.byte()?;
                let mut argument = |bit: u8, short: bool| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        let value = if short {
                            i32::from(self.i16()?)
                        } else {
                            i32::from(self.byte()?)
                        };
                        Ok(Expression(vec![
                            ExpressionOp::Constant(value),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::OpenPartyMenu {
                    minimum_party_size: argument(1, false)?,
                    fixed_members: argument(2, true)?,
                }
            }
            OP_INITIALIZE_CHARACTER => {
                let flags = self.byte()?;
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.byte()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::InitializeCharacter {
                    character: argument(1)?,
                    enable_leveling: argument(2)?,
                    equipment_set: argument(4)?,
                    category: argument(8)?,
                    menu_preset: argument(16)?,
                }
            }
            OP_STOP_TEXTURE_ANIMATION => {
                let flags = self.byte()?;
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.byte()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::StopTextureAnimation {
                    object: argument(1)?,
                    animation: argument(2)?,
                }
            }
            OP_SET_BATTLE_MUSIC_CONTINUATION => {
                let flags = self.byte()?;
                let enabled = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.byte()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::SetBattleMusicContinuation(enabled)
            }
            OP_RUN_BATTLE_CODE => {
                let flags = self.byte()?;
                let mut argument = |bit: u8, short: bool| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        let value = if short {
                            i32::from(self.i16()?)
                        } else {
                            i32::from(self.byte()?)
                        };
                        Ok(Expression(vec![
                            ExpressionOp::Constant(value),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::RunBattleCode {
                    code: argument(1, false)?,
                    value: argument(2, true)?,
                }
            }
            OP_RUN_MODEL_CODE => {
                let flags = self.byte()?;
                let mut argument = |bit: u8, short: bool| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        let value = if short {
                            i32::from(self.i16()?)
                        } else {
                            i32::from(self.byte()?)
                        };
                        Ok(Expression(vec![
                            ExpressionOp::Constant(value),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::RunModelCode {
                    code: argument(1, false)?,
                    arguments: [argument(2, true)?, argument(4, true)?, argument(8, true)?],
                }
            }
            OP_ADD_BATTLE_STATUS => {
                let flags = self.byte()?;
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(self.i24()?),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::AddBattleStatus {
                    target: argument(1)?,
                    status: argument(2)?,
                    permanent: argument(4)?,
                    arguments: [argument(8)?, argument(16)?, argument(32)?],
                }
            }
            OP_SET_MODEL_LIGHT => {
                let flags = self.byte()?;
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(self.i24()?),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::SetModelLight {
                    model: argument(1)?,
                    color: [argument(2)?, argument(4)?, argument(8)?],
                    shadow_intensity: argument(16)?,
                    shadow_scale: argument(32)?,
                }
            }
            OP_CHANGE_ELEMENTAL_AFFINITY => {
                let flags = self.byte()?;
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(self.i24()?),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::ChangeElementalAffinity {
                    target: argument(1)?,
                    kind: argument(2)?,
                    element: argument(4)?,
                    add: argument(8)?,
                }
            }
            OP_SET_HERE_ICON => {
                let flags = self.byte()?;
                let mode = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.byte()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::SetHereIcon(mode)
            }
            OP_FULL_MEMBER => {
                let flags = self.byte()?;
                let mask = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.i16()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::SetPartyAvailability(mask)
            }
            OP_DASH_ON => {
                self.byte()?;
                InstructionKind::SetRunEnabled {
                    enabled: true,
                    opcode: opcode as u8,
                }
            }
            OP_MOVE_TILE => {
                let flags = self.byte()?;
                let mut argument = |bit: u8, short: bool| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        let value = if short {
                            i32::from(self.i16()?)
                        } else {
                            i32::from(self.byte()?)
                        };
                        Ok(Expression(vec![
                            ExpressionOp::Constant(value),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::MoveTile {
                    overlay: argument(1, false)?,
                    x: argument(2, true)?,
                    y: argument(4, true)?,
                    depth: argument(8, true)?,
                }
            }
            OP_SET_TILE_SCROLL_WITH_OFFSET => {
                let flags = self.byte()?;
                let mut argument = |bit: u8, short: bool| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        let value = if short {
                            i32::from(self.i16()?)
                        } else {
                            i32::from(self.byte()?)
                        };
                        Ok(Expression(vec![
                            ExpressionOp::Constant(value),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::SetTileScrollWithOffset {
                    overlay: argument(1, false)?,
                    enabled: argument(2, false)?,
                    speed: argument(4, true)?,
                    offset: argument(8, true)?,
                    is_x_offset: argument(16, false)?,
                }
            }
            OP_SET_CAMERA_BOUNDS => {
                let flags = self.byte()?;
                let mut argument = |bit: u8, short: bool| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        let value = if short {
                            i32::from(self.i16()?)
                        } else {
                            i32::from(self.byte()?)
                        };
                        Ok(Expression(vec![
                            ExpressionOp::Constant(value),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::SetCameraBounds {
                    camera: argument(1, false)?,
                    bounds: [
                        argument(2, true)?,
                        argument(4, true)?,
                        argument(8, true)?,
                        argument(16, true)?,
                    ],
                }
            }
            OP_SET_CHARACTER_ROW => {
                let flags = self.byte()?;
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.byte()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::SetCharacterRow {
                    character: argument(1)?,
                    row: argument(2)?,
                }
            }
            OP_CLEAR_PARTY_STATUS => {
                let flags = self.byte()?;
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.byte()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::ClearPartyStatus {
                    character: argument(1)?,
                    statuses: argument(2)?,
                }
            }
            OP_SET_ACTOR_PITCH_ROLL => {
                let flags = self.byte()?;
                let mut argument = |bit: u8| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        Ok(Expression(vec![
                            ExpressionOp::Constant(i32::from(self.byte()?)),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::SetActorPitchRoll {
                    pitch: argument(1)?,
                    roll: argument(2)?,
                }
            }
            OP_ATTACH_TILE => {
                let flags = self.byte()?;
                let mut argument = |bit: u8, short: bool| {
                    if flags & bit != 0 {
                        self.expression()
                    } else {
                        let value = if short {
                            i32::from(self.i16()?)
                        } else {
                            i32::from(self.byte()?)
                        };
                        Ok(Expression(vec![
                            ExpressionOp::Constant(value),
                            ExpressionOp::End,
                        ]))
                    }
                };
                InstructionKind::AttachTile {
                    overlay: argument(1, false)?,
                    x: argument(2, true)?,
                    y: argument(4, false)?,
                    filter_mode: argument(8, false)?,
                    red: argument(16, false)?,
                    green: argument(32, false)?,
                    blue: argument(64, false)?,
                }
            }
            _ => {
                return Err(VmError::new(
                    VmErrorKind::UnsupportedOpcode,
                    offset,
                    Some(opcode),
                    "unsupported event opcode",
                ));
            }
        };
        Ok(Instruction {
            offset,
            length: (self.position as u32) - offset,
            kind,
        })
    }

    fn expression(&mut self) -> Result<Expression, VmError> {
        let start = self.position as u32;
        let mut operations = Vec::new();
        loop {
            let opcode_position = self.position as u32;
            let opcode = self.byte()?;
            let operation = match opcode {
                0..=3 | 103..=105 => ExpressionOp::Pad,
                4 => ExpressionOp::PostIncrement,
                5 => ExpressionOp::PostDecrement,
                6 => ExpressionOp::PreIncrement,
                7 => ExpressionOp::PreDecrement,
                9 => ExpressionOp::PostDecrementMembers,
                10 => ExpressionOp::PreIncrementMembers,
                11 => ExpressionOp::PreDecrementMembers,
                15 => ExpressionOp::NotMembers,
                16 => ExpressionOp::Complement,
                54 => ExpressionOp::MultiplyAssignAllMembers,
                55 => ExpressionOp::DivideAssignAllMembers,
                58 => ExpressionOp::SubtractAssignAllMembers,
                59 => ExpressionOp::ShiftRightAssignMembers,
                60 => ExpressionOp::ShiftRightAssignAllMembers,
                64 => ExpressionOp::AndAssignAllMembers,
                65 => ExpressionOp::XorAssignAllMembers,
                67 => ExpressionOp::AndAssignMembers,
                68 => ExpressionOp::XorAssignMembers,
                69 => ExpressionOp::OrAssignMembers,
                70 => ExpressionOp::CastI8,
                71 => ExpressionOp::CastU8,
                72 => ExpressionOp::CastI16,
                73 => ExpressionOp::CastU16,
                74 => ExpressionOp::CastList,
                77 => ExpressionOp::SelectRandomMember,
                78 => ExpressionOp::ObjectSpecial,
                79 => ExpressionOp::KeyOn,
                80 => ExpressionOp::SineUnshifted,
                81 => ExpressionOp::Cosine,
                84 => ExpressionOp::AndMembers,
                85 => ExpressionOp::NandMembers,
                86 => ExpressionOp::XorMembers,
                90 | 92 => ExpressionOp::ConsumeAndZero,
                93 => ExpressionOp::Angle,
                95 => ExpressionOp::ObjectPointer(self.byte()?),
                96 => ExpressionOp::AngleToObject,
                97 => ExpressionOp::DistanceToObject,
                98 => ExpressionOp::Sine,
                99 => ExpressionOp::CosineShifted,
                100 => ExpressionOp::ItemCount,
                101 => ExpressionOp::BgiAnimationFrame,
                102 => ExpressionOp::Angle2,
                108 => ExpressionOp::Sps,
                109 => ExpressionOp::PartyAdd,
                106 => ExpressionOp::AnimationFrame,
                107 => ExpressionOp::PartyContains,
                111 => ExpressionOp::CharacterMaxMp,
                112 => ExpressionOp::BgiCharacterTriangle,
                114 => ExpressionOp::NoOp,
                EXPR_CONST_I16 => ExpressionOp::Constant(i32::from(self.i16()?)),
                EXPR_CONST_I32 => ExpressionOp::Constant(self.i32()?),
                EXPR_POST_INCREMENT_MEMBERS => ExpressionOp::PostIncrementMembers,
                EXPR_NEGATE => ExpressionOp::Negate,
                EXPR_NOT => ExpressionOp::Not,
                EXPR_ADD => ExpressionOp::Add,
                EXPR_SUBTRACT => ExpressionOp::Subtract,
                EXPR_MULTIPLY => ExpressionOp::Multiply,
                EXPR_DIVIDE => ExpressionOp::Divide,
                EXPR_REMAINDER => ExpressionOp::Remainder,
                22 => ExpressionOp::ShiftLeft,
                23 => ExpressionOp::ShiftRight,
                EXPR_LESS => ExpressionOp::Less,
                EXPR_GREATER => ExpressionOp::Greater,
                27 => ExpressionOp::GreaterOrEqual,
                EXPR_EQUAL => ExpressionOp::Equal,
                EXPR_NOT_EQUAL => ExpressionOp::NotEqual,
                36 => ExpressionOp::BitwiseAnd,
                37 => ExpressionOp::BitwiseXor,
                38 => ExpressionOp::BitwiseOr,
                28 => ExpressionOp::LessMembers,
                29 => ExpressionOp::GreaterMembers,
                30 => ExpressionOp::LessOrEqualMembers,
                31 => ExpressionOp::GreaterOrEqualMembers,
                34 => ExpressionOp::EqualMembers,
                35 => ExpressionOp::NotEqualMembers,
                41 => ExpressionOp::Member(self.byte()?),
                39 => ExpressionOp::LogicalAnd,
                40 => ExpressionOp::LogicalOr,
                EXPR_ASSIGN => ExpressionOp::Assign,
                47 => ExpressionOp::MultiplyAssign,
                45 => ExpressionOp::AssignAllMembers,
                50 => ExpressionOp::AddAssign,
                51 => ExpressionOp::SubtractAssign,
                46 => ExpressionOp::AssignMembers,
                120 => ExpressionOp::ReadObjectMember(ObjectMemberRef {
                    object: EntityId::new(self.byte()? as u64),
                    member: self.byte()?,
                }),
                122 => ExpressionOp::SystemVariable(self.byte()?),
                EXPR_END => {
                    operations.push(ExpressionOp::End);
                    break;
                }
                0xd3 => {
                    return Err(VmError::new(
                        VmErrorKind::UnsupportedOpcode,
                        opcode_position,
                        Some(u32::from(opcode)),
                        "custom variable functions are not implemented",
                    ));
                }
                _ if opcode >= 0x80 && opcode & 3 == 3 => {
                    let mut variable = u16::from(self.byte()?);
                    if opcode & 0x20 != 0 {
                        let high = self.byte().map_err(|mut error| {
                            error.opcode = Some(u32::from(opcode));
                            error.message = format!(
                                "truncated two-byte custom variable index (expression starts at {start})"
                            );
                            error
                        })?;
                        variable |= u16::from(high) << 8;
                    }
                    ExpressionOp::CustomVariable(variable)
                }
                _ if opcode >= 0x80 => {
                    let scope = match opcode & 3 {
                        0 => VariableScope::Global,
                        1 => VariableScope::Map,
                        2 => VariableScope::Instance,
                        value => {
                            return Err(VmError::new(
                                VmErrorKind::UnsupportedOpcode,
                                opcode_position,
                                Some(u32::from(opcode)),
                                format!("unsupported variable source {value}"),
                            ));
                        }
                    };
                    let value_type = match (opcode >> 2) & 7 {
                        0 => VariableType::SignedBit,
                        1 => VariableType::Bit,
                        2 => VariableType::Int24,
                        3 => VariableType::UInt24,
                        4 => VariableType::SignedByte,
                        5 => VariableType::Byte,
                        6 => VariableType::Int16,
                        7 => VariableType::UInt16,
                        _ => unreachable!(),
                    };
                    let mut index = u16::from(self.byte()?);
                    if opcode & 0x20 != 0 {
                        let high = self.byte().map_err(|mut error| {
                            error.opcode = Some(u32::from(opcode));
                            error.message = format!(
                                "truncated two-byte variable index (expression starts at {start})"
                            );
                            error
                        })?;
                        index |= u16::from(high) << 8;
                    }
                    ExpressionOp::Variable(VariableRef {
                        scope,
                        value_type,
                        index,
                    })
                }
                _ => {
                    return Err(VmError::new(
                        VmErrorKind::UnsupportedOpcode,
                        opcode_position,
                        Some(u32::from(opcode)),
                        format!("unsupported expression operation (expression starts at {start})"),
                    ));
                }
            };
            operations.push(operation);
        }
        Ok(Expression(operations))
    }

    fn byte(&mut self) -> Result<u8, VmError> {
        let offset = self.position as u32;
        let Some(value) = self.bytes.get(self.position).copied() else {
            return Err(VmError::new(
                VmErrorKind::Decode,
                offset,
                None,
                "unexpected end of event bytecode",
            ));
        };
        self.position += 1;
        Ok(value)
    }

    fn i16(&mut self) -> Result<i16, VmError> {
        Ok(i16::from_le_bytes([self.byte()?, self.byte()?]))
    }

    fn i24(&mut self) -> Result<i32, VmError> {
        let value = u32::from(self.byte()?)
            | (u32::from(self.byte()?) << 8)
            | (u32::from(self.byte()?) << 16);
        Ok(((value << 8) as i32) >> 8)
    }

    fn i32(&mut self) -> Result<i32, VmError> {
        Ok(i32::from_le_bytes([
            self.byte()?,
            self.byte()?,
            self.byte()?,
            self.byte()?,
        ]))
    }
}
