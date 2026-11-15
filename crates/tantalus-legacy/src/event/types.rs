use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{Display, Formatter};
use tantalus_contracts::{EngineError, EntityId, Tick};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VariableScope {
    Global,
    Map,
    Instance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VariableType {
    SignedBit,
    Bit,
    Int24,
    UInt24,
    SignedByte,
    Byte,
    Int16,
    UInt16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariableRef {
    pub scope: VariableScope,
    pub value_type: VariableType,
    pub index: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpressionOp {
    Constant(i32),
    Variable(VariableRef),
    Negate,
    Not,
    Complement,
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    ShiftRight,
    Angle2,
    Less,
    Greater,
    GreaterOrEqual,
    Equal,
    NotEqual,
    Assign,
    PostIncrementMembers,
    PostDecrementMembers,
    PreDecrementMembers,
    PreIncrement,
    PreDecrement,
    PostDecrement,
    ReadObjectMember(ObjectMemberRef),
    Member(u8),
    SystemVariable(u8),
    CustomVariable(u16),
    CastI8,
    CastU8,
    CastI16,
    CastU16,
    CastList,
    ShiftRightAssignMembers,
    XorAssignMembers,
    AssignMembers,
    EqualMembers,
    NotMembers,
    LogicalAnd,
    LogicalOr,
    Pad,
    End,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Expression(pub Vec<ExpressionOp>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstructionKind {
    Nop,
    Jump(i16),
    JumpIfNot {
        condition: Expression,
        relative: i16,
    },
    JumpIf {
        condition: Expression,
        relative: i16,
    },
    Evaluate(Expression),
    Return,
    Wait(Expression),
    WindowSync {
        window: Expression,
        flags: Expression,
        text: Expression,
    },
    WindowAsync {
        window: Expression,
        flags: Expression,
        text: Expression,
    },
    WaitWindow(Expression),
    CloseWindow(Expression),
    MoveActor {
        actor: Expression,
        x: Expression,
        z: Expression,
    },
    AimAtCamera,
    DefinePlayerControl,
    SetControlDirection {
        arrow: Expression,
        analog: Expression,
    },
    BackgroundMove {
        x: Expression,
        y: Expression,
        z: Expression,
    },
    InitializeObject {
        source: u8,
        unique_id: u8,
    },
    InitializeRegion {
        source: u8,
        unique_id: u8,
    },
    InitializeCode {
        source: u8,
        unique_id: u8,
    },
    SetTriangleActive {
        triangle: Expression,
        active: Expression,
    },
    RunSound {
        code: Expression,
        sound: Expression,
    },
    AttachObjectOffset {
        x: Expression,
        y: Expression,
        z: Expression,
    },
    SetNavigationAttributeMask(Expression),
    Yield(u8),
    SetShadowEnabled(bool),
    SetTextVariable {
        script: Expression,
        value: Expression,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instruction {
    pub offset: u32,
    pub length: u32,
    pub kind: InstructionKind,
}

impl Instruction {
    pub fn next_offset(&self) -> u32 {
        self.offset + self.length
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventProgram {
    pub instructions: BTreeMap<u32, Instruction>,
    pub source_length: u32,
}

impl EventProgram {
    pub fn instruction(&self, offset: u32) -> Option<&Instruction> {
        self.instructions.get(&offset)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VmStatus {
    Running,
    Waiting,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitState {
    Frames(u8),
    Window(u8),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostEvent {
    ShowWindow {
        window: u8,
        flags: u8,
        text: u16,
    },
    CloseWindow {
        window: u8,
    },
    MoveActor {
        actor: EntityId,
        x: i32,
        z: i32,
    },
    AimAtCamera,
    DefinePlayerControl {
        actor: EntityId,
    },
    SetControlDirection {
        arrow: i16,
        analog: i16,
    },
    InitializeObject {
        source: u8,
        unique_id: u8,
    },
    InitializeRegion {
        source: u8,
        unique_id: u8,
    },
    InitializeCode {
        source: u8,
        unique_id: u8,
    },
    SetTriangleActive {
        triangle: u32,
        active: bool,
    },
    RunSound {
        code: i32,
        sound: i32,
        arguments: [i32; 3],
    },
    AttachObjectOffset {
        actor: EntityId,
        x: i32,
        y: i32,
        z: i32,
    },
    SetNavigationAttributeMask {
        mask: u8,
    },
    SetShadowEnabled {
        actor: EntityId,
        enabled: bool,
    },
    SetTextVariable {
        script: i32,
        value: i32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VmStepReport {
    pub tick: Tick,
    pub status: VmStatus,
    pub operations: u32,
    pub events: Vec<HostEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VmTraceEvent {
    pub tick: Tick,
    pub offset: u32,
    pub opcode: Option<u8>,
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VmErrorKind {
    Decode,
    UnsupportedOpcode,
    InvalidExpression,
    InvalidJump,
    InstructionLimit,
    Host,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VmError {
    pub kind: VmErrorKind,
    pub offset: u32,
    pub opcode: Option<u8>,
    pub message: String,
}

impl VmError {
    pub fn new(
        kind: VmErrorKind,
        offset: u32,
        opcode: Option<u8>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            offset,
            opcode,
            message: message.into(),
        }
    }

    pub fn host(error: EngineError, offset: u32, opcode: u8) -> Self {
        Self::new(VmErrorKind::Host, offset, Some(opcode), error.to_string())
    }
}

impl Display for VmError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self.opcode {
            Some(opcode) => write!(
                formatter,
                "event VM error at byte {} (opcode 0x{opcode:02x}): {}",
                self.offset, self.message
            ),
            None => write!(
                formatter,
                "event VM error at byte {}: {}",
                self.offset, self.message
            ),
        }
    }
}

impl Error for VmError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ObjectMemberRef {
    pub object: EntityId,
    pub member: u8,
}

pub trait LegacyEventHost {
    fn apply(&mut self, events: &[HostEvent]) -> Result<(), EngineError>;
    fn window_is_open(&self, window: u8) -> bool;
    fn read_object_member(&mut self, reference: ObjectMemberRef) -> Result<i32, EngineError>;
    fn select_object_member_target(&mut self, slot: Option<u8>) -> Result<EntityId, EngineError>;
    fn write_object_member(
        &mut self,
        reference: ObjectMemberRef,
        value: i32,
    ) -> Result<(), EngineError>;
    fn read_system_variable(&mut self, variable: u8) -> Result<i32, EngineError>;
    fn read_atan_entry(&mut self, offset: i32) -> Result<u16, EngineError>;
    fn read_custom_variable(&mut self, variable: u16) -> Result<i32, EngineError>;
}
