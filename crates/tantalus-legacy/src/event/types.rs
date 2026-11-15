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
    Sine,
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    ShiftLeft,
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
    PostIncrement,
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
    NoOp,
    ShiftRightAssignMembers,
    XorAssignMembers,
    AssignMembers,
    EqualMembers,
    LessOrEqualMembers,
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
    NoOp(u32),
    Jump(i16),
    JumpIfNot {
        condition: Expression,
        relative: i16,
    },
    JumpIf {
        condition: Expression,
        relative: i16,
    },
    JumpSwitch {
        base: i16,
        default_relative: u16,
        case_relative: Vec<u16>,
    },
    Evaluate(Expression),
    DeleteObject(u8),
    SetActorPosition {
        x: Expression,
        z: Expression,
    },
    SetIdleAnimation(Expression),
    SetWalkAnimation(Expression),
    SetRunAnimation(Expression),
    SetLeftTurnAnimation(Expression),
    SetRightTurnAnimation(Expression),
    SetActorFacing(Expression),
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
    ResetWalkState,
    SetMovementSpeed(Expression),
    SetActorModel {
        model: Expression,
        eye_height: Expression,
    },
    SetNavigationAttributeMask(Expression),
    Yield(u8),
    SetShadowEnabled(bool),
    SetTextVariable {
        script: Expression,
        value: Expression,
    },
    SetObjectScale {
        actor: Expression,
        x: Expression,
        z: Expression,
        y: Expression,
    },
    SetVibrationFrameRate(Expression),
    RequestScriptAsync {
        level: Expression,
        target: Expression,
        tag: u8,
    },
    DefineTileViewport {
        viewport: Expression,
        x: Expression,
        y: Expression,
        width: Expression,
        height: Expression,
    },
    PreserveAndHideAll,
    RestoreAllVisibility,
    SetTimerRunning(Expression),
    RequestCallerAsync {
        level: Expression,
        tag: u8,
    },
    CalculateExitPosition,
    RequestCallerSync {
        level: Expression,
        tag: u8,
    },
    SetSpsParameters {
        object: Expression,
        operation: Expression,
        arguments: [Expression; 3],
    },
    SetTileShadeColor {
        overlay: Expression,
        red: Expression,
        green: Expression,
        blue: Expression,
    },
    SetSimulationFloor {
        simulation: Expression,
        floor: Expression,
    },
    AttachTile {
        overlay: Expression,
        x: Expression,
        y: Expression,
        filter_mode: Expression,
        red: Expression,
        green: Expression,
        blue: Expression,
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
    ScriptRequest,
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
    DeleteObject {
        target: EntityId,
    },
    SetActorPosition {
        actor: EntityId,
        position: [i32; 3],
    },
    SetIdleAnimation {
        actor: EntityId,
        animation: u16,
    },
    SetWalkAnimation {
        actor: EntityId,
        animation: u16,
    },
    SetRunAnimation {
        actor: EntityId,
        animation: u16,
    },
    SetLeftTurnAnimation {
        actor: EntityId,
        animation: u16,
    },
    SetRightTurnAnimation {
        actor: EntityId,
        animation: u16,
    },
    SetActorFacing {
        actor: EntityId,
        angle: i16,
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
    ResetWalkState {
        actor: EntityId,
    },
    SetMovementSpeed {
        actor: EntityId,
        speed: u8,
    },
    SetActorModel {
        actor: EntityId,
        model: u16,
        eye_height: i16,
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
    SetObjectScale {
        actor: EntityId,
        scale: [i32; 3],
        vertical_scale: u8,
    },
    SetVibrationFrameRate {
        rate: i16,
    },
    RequestScriptAsync {
        level: i32,
        target: EntityId,
        tag: u8,
    },
    DefineTileViewport {
        viewport: i32,
        x: i16,
        y: i16,
        width: i16,
        height: i16,
    },
    PreserveAndHideAll,
    RestoreAllVisibility,
    SetTimerRunning {
        running: bool,
    },
    RequestCallerAsync {
        level: i32,
        tag: u8,
    },
    CalculateExitPosition,
    RequestCallerSync {
        level: i32,
        tag: u8,
    },
    SetSpsParameters {
        object: i32,
        operation: i32,
        arguments: [i16; 3],
    },
    SetTileShadeColor {
        overlay: i32,
        color: [u8; 3],
    },
    SetSimulationFloor {
        simulation: u32,
        floor: u32,
    },
    AttachTile {
        overlay: i32,
        x: i16,
        y: i16,
        filter_mode: i8,
        color: [u8; 3],
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
    pub opcode: Option<u32>,
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
    pub opcode: Option<u32>,
    pub message: String,
}

impl VmError {
    pub fn new(
        kind: VmErrorKind,
        offset: u32,
        opcode: Option<u32>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            offset,
            opcode,
            message: message.into(),
        }
    }

    pub fn host(error: EngineError, offset: u32, opcode: u32) -> Self {
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
    fn script_request_is_complete(&mut self, entity: EntityId) -> Result<bool, EngineError>;
    fn read_custom_variable(&mut self, variable: u16) -> Result<i32, EngineError>;
}
