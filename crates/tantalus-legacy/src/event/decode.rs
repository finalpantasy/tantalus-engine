use super::*;
use std::collections::BTreeMap;
use tantalus_contracts::EntityId;

const OP_NOP: u8 = 0x00;
const OP_JUMP: u8 = 0x01;
const OP_JUMP_IF_NOT: u8 = 0x02;
const OP_JUMP_IF: u8 = 0x03;
const OP_RETURN: u8 = 0x04;
const OP_EVALUATE: u8 = 0x05;
const OP_WINDOW_SYNC: u8 = 0x1f;
const OP_WINDOW_ASYNC: u8 = 0x20;
const OP_CLOSE_WINDOW: u8 = 0x21;
const OP_WAIT: u8 = 0x22;
const OP_WAIT_WINDOW: u8 = 0x54;
const OP_MOVE_ACTOR: u8 = 0xbf;
const OP_AIM_AT_CAMERA: u8 = 0xb8;
const OP_DEFINE_PLAYER_CONTROL: u8 = 0x2c;
const OP_SET_CONTROL_DIRECTION: u8 = 0x67;
const OP_BACKGROUND_MOVE: u8 = 0x58;
const OP_INITIALIZE_OBJECT: u8 = 0x09;
const OP_INITIALIZE_REGION: u8 = 0x08;
const OP_INITIALIZE_CODE: u8 = 0x07;
const OP_SET_TRIANGLE_ACTIVE: u8 = 0x9a;
const OP_RUN_SOUND: u8 = 0xc5;
const OP_ATTACH_OBJECT_OFFSET: u8 = 0xd4;
const OP_SET_NAVIGATION_ATTRIBUTE_MASK: u8 = 0x27;
const OP_YIELD: u8 = 0x0e;
const OP_PADDING17: u8 = 0x17;
const OP_SHADOW_ON: u8 = 0x7f;
const OP_SET_TEXT_VARIABLE: u8 = 0x66;
const OP_SET_OBJECT_SCALE: u8 = 0x9f;
const OP_SET_VIBRATION_FRAME_RATE: u8 = 0xfa;
const OP_REQUEST_SCRIPT_ASYNC: u8 = 0x10;
const OP_DEFINE_TILE_VIEWPORT: u8 = 0xc8;
const OP_RESTORE_ALL_VISIBILITY: u8 = 0xd6;
const OP_SET_TIMER_RUNNING: u8 = 0x7d;

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
    let mut decoder = Decoder { bytes, position: 0 };
    let mut instructions = BTreeMap::new();
    while decoder.position < bytes.len() {
        let instruction = decoder.instruction()?;
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
}

impl Decoder<'_> {
    fn instruction(&mut self) -> Result<Instruction, VmError> {
        let offset = self.position as u32;
        let opcode = self.byte()?;
        let kind = match opcode {
            OP_NOP => InstructionKind::Nop,
            OP_JUMP => InstructionKind::Jump(self.i16()?),
            OP_JUMP_IF_NOT => InstructionKind::JumpIfNot {
                condition: self.expression()?,
                relative: self.i16()?,
            },
            OP_JUMP_IF => InstructionKind::JumpIf {
                condition: self.expression()?,
                relative: self.i16()?,
            },
            OP_RETURN => InstructionKind::Return,
            OP_EVALUATE => InstructionKind::Evaluate(self.expression()?),
            OP_WAIT => {
                self.byte()?;
                InstructionKind::Wait(self.expression()?)
            }
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
            OP_BACKGROUND_MOVE => {
                self.byte()?;
                InstructionKind::BackgroundMove {
                    x: self.expression()?,
                    y: self.expression()?,
                    z: self.expression()?,
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
            OP_RUN_SOUND => {
                let flags = self.byte()?;
                let code = if flags & 1 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.i16()?)),
                        ExpressionOp::End,
                    ])
                };
                let sound = if flags & 2 != 0 {
                    self.expression()?
                } else {
                    Expression(vec![
                        ExpressionOp::Constant(i32::from(self.i16()?)),
                        ExpressionOp::End,
                    ])
                };
                InstructionKind::RunSound { code, sound }
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
            OP_YIELD | OP_PADDING17 => {
                self.byte()?;
                InstructionKind::Yield(opcode)
            }
            OP_SHADOW_ON => {
                self.byte()?;
                InstructionKind::SetShadowEnabled(true)
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
                11 => ExpressionOp::PreDecrementMembers,
                15 => ExpressionOp::NotMembers,
                16 => ExpressionOp::Complement,
                59 => ExpressionOp::ShiftRightAssignMembers,
                68 => ExpressionOp::XorAssignMembers,
                70 => ExpressionOp::CastI8,
                71 => ExpressionOp::CastU8,
                72 => ExpressionOp::CastI16,
                73 => ExpressionOp::CastU16,
                74 => ExpressionOp::CastList,
                102 => ExpressionOp::Angle2,
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
                23 => ExpressionOp::ShiftRight,
                EXPR_LESS => ExpressionOp::Less,
                EXPR_GREATER => ExpressionOp::Greater,
                27 => ExpressionOp::GreaterOrEqual,
                EXPR_EQUAL => ExpressionOp::Equal,
                EXPR_NOT_EQUAL => ExpressionOp::NotEqual,
                34 => ExpressionOp::EqualMembers,
                41 => ExpressionOp::Member(self.byte()?),
                39 => ExpressionOp::LogicalAnd,
                40 => ExpressionOp::LogicalOr,
                EXPR_ASSIGN => ExpressionOp::Assign,
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
                        Some(opcode),
                        "custom variable functions are not implemented",
                    ));
                }
                _ if opcode >= 0x80 && opcode & 3 == 3 => {
                    let mut variable = u16::from(self.byte()?);
                    if opcode & 0x20 != 0 {
                        variable |= u16::from(self.byte()?) << 8;
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
                                Some(opcode),
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
                        index |= u16::from(self.byte()?) << 8;
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
                        Some(opcode),
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

    fn i32(&mut self) -> Result<i32, VmError> {
        Ok(i32::from_le_bytes([
            self.byte()?,
            self.byte()?,
            self.byte()?,
            self.byte()?,
        ]))
    }
}
