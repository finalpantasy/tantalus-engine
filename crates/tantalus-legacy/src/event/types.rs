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
    SineUnshifted,
    Cosine,
    CosineShifted,
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    ShiftLeft,
    ShiftRight,
    Angle,
    Angle2,
    AngleToObject,
    DistanceToObject,
    ObjectPointer(u8),
    ItemCount,
    AnimationFrame,
    BgiAnimationFrame,
    BgiCharacterTriangle,
    CharacterMaxMp,
    PartyContains,
    PartyAdd,
    SelectRandomMember,
    Sps,
    KeyOn,
    Less,
    Greater,
    GreaterOrEqual,
    Equal,
    NotEqual,
    BitwiseAnd,
    BitwiseXor,
    BitwiseOr,
    Assign,
    AddAssign,
    SubtractAssign,
    MultiplyAssign,
    PostIncrementMembers,
    PostDecrementMembers,
    PreIncrementMembers,
    PreDecrementMembers,
    PreIncrement,
    PreDecrement,
    PostIncrement,
    PostDecrement,
    ReadObjectMember(ObjectMemberRef),
    ObjectSpecial,
    Member(u8),
    SystemVariable(u8),
    CustomVariable(u16),
    CastI8,
    CastU8,
    CastI16,
    CastU16,
    CastList,
    NoOp,
    ConsumeAndZero,
    ShiftRightAssignMembers,
    ShiftRightAssignAllMembers,
    MultiplyAssignAllMembers,
    DivideAssignAllMembers,
    SubtractAssignAllMembers,
    AndAssignMembers,
    XorAssignMembers,
    OrAssignMembers,
    XorAssignAllMembers,
    AndAssignAllMembers,
    AssignAllMembers,
    AssignMembers,
    EqualMembers,
    NotEqualMembers,
    AndMembers,
    NandMembers,
    XorMembers,
    LessMembers,
    LessOrEqualMembers,
    GreaterOrEqualMembers,
    GreaterMembers,
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
    JumpSwitchWide {
        base: i16,
        default_relative: u16,
        case_relative: Vec<u16>,
    },
    JumpSwitchEx {
        default_relative: u16,
        cases: Vec<[u16; 2]>,
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
    PlayAnimation(Expression),
    SetAnimationRange {
        start: Expression,
        end: Expression,
    },
    SetLeftTurnAnimation(Expression),
    SetRightTurnAnimation(Expression),
    SetJumpAnimation {
        animation: Expression,
        parameters: [Expression; 2],
    },
    SetActorRadii {
        pathing_size: Expression,
        collision_radius: Expression,
        talk_radius: Expression,
    },
    SetIdleAnimationSpeeds([Expression; 4]),
    SetHeadFocusMasks {
        self_mask: Expression,
        target_mask: Expression,
    },
    SetHeadFocusFlags(Expression),
    SetWalkTurnSpeed(Expression),
    SetObjectFlags(Expression),
    SetActorFacing(Expression),
    StartActorTurn {
        angle: Expression,
        speed: Expression,
    },
    Return,
    Wait(Expression),
    WaitTurn,
    WaitAnimation,
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
    Cinematic {
        operation: Expression,
        argument: Expression,
    },
    DefineRegion {
        vertices: Vec<[Expression; 2]>,
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
        arguments: [Expression; 3],
        opcode: u8,
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
    SetShadowEnabled {
        enabled: bool,
        opcode: u8,
    },
    SetTextVariable {
        script: Expression,
        value: Expression,
    },
    ShowBattleMessage(Expression),
    SetObjectScale {
        actor: Expression,
        x: Expression,
        z: Expression,
        y: Expression,
    },
    SetVibrationFrameRate(Expression),
    PreloadField {
        hint: Expression,
        field: Expression,
    },
    SetRain {
        strength: Expression,
        speed: Expression,
    },
    RequestScriptAsync {
        level: Expression,
        target: Expression,
        tag: u8,
    },
    RequestScriptWhenReady {
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
    RequestCallerWhenReady {
        level: Expression,
        tag: u8,
    },
    CalculateExitPosition,
    WalkToExit,
    CalculateScreenPosition(Expression),
    ChangeField(Expression),
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
    SetSimulationFrameRate {
        simulation: Expression,
        frame_rate: Expression,
    },
    SetTileAnimationFrameWait {
        animation: Expression,
        frame: Expression,
        wait: Expression,
    },
    SetTileAnimationFrame {
        animation: Expression,
        frame: Expression,
    },
    OpenPartyMenu {
        minimum_party_size: Expression,
        fixed_members: Expression,
    },
    InitializeCharacter {
        character: Expression,
        enable_leveling: Expression,
        equipment_set: Expression,
        category: Expression,
        menu_preset: Expression,
    },
    StopTextureAnimation {
        object: Expression,
        animation: Expression,
    },
    RunBattleCode {
        code: Expression,
        value: Expression,
    },
    SetBattleMusicContinuation(Expression),
    RunModelCode {
        code: Expression,
        arguments: [Expression; 3],
    },
    SetModelLight {
        model: Expression,
        color: [Expression; 3],
        shadow_intensity: Expression,
        shadow_scale: Expression,
    },
    ChangeElementalAffinity {
        target: Expression,
        kind: Expression,
        element: Expression,
        add: Expression,
    },
    SetRunEnabled {
        enabled: bool,
        opcode: u8,
    },
    SetPartyAvailability(Expression),
    MoveTile {
        overlay: Expression,
        x: Expression,
        y: Expression,
        depth: Expression,
    },
    SetTileScrollWithOffset {
        overlay: Expression,
        enabled: Expression,
        speed: Expression,
        offset: Expression,
        is_x_offset: Expression,
    },
    SetTileViewportAlpha {
        viewport: Expression,
        alpha: [Expression; 2],
    },
    SetCameraBounds {
        camera: Expression,
        bounds: [Expression; 4],
    },
    SetCharacterRow {
        character: Expression,
        row: Expression,
    },
    ClearPartyStatus {
        character: Expression,
        statuses: Expression,
    },
    SetActorPitchRoll {
        pitch: Expression,
        roll: Expression,
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
    PartyMenu,
    ActorTurn,
    ActorAnimation,
    CallerRequest {
        level: i32,
        tag: u8,
    },
    ScriptReadiness {
        level: i32,
        target: EntityId,
        tag: u8,
    },
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
    MoveActorToExit {
        actor: EntityId,
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
    PlayAnimation {
        actor: EntityId,
        animation: u16,
    },
    SetAnimationRange {
        actor: EntityId,
        start: u8,
        end: u8,
    },
    SetLeftTurnAnimation {
        actor: EntityId,
        animation: u16,
    },
    SetRightTurnAnimation {
        actor: EntityId,
        animation: u16,
    },
    SetJumpAnimation {
        actor: EntityId,
        animation: u16,
        parameters: [u8; 2],
    },
    SetActorRadii {
        actor: EntityId,
        pathing_size: i32,
        collision_radius: u8,
        talk_radius: u8,
    },
    SetIdleAnimationSpeeds {
        actor: EntityId,
        speeds: [u8; 4],
    },
    SetHeadFocusMasks {
        actor: EntityId,
        self_mask: u8,
        target_mask: u8,
    },
    SetHeadFocusFlags {
        actor: EntityId,
        flags: u8,
    },
    SetWalkTurnSpeed {
        actor: EntityId,
        speed: u8,
    },
    SetObjectFlags {
        object: EntityId,
        flags: u8,
    },
    SetActorFacing {
        actor: EntityId,
        angle: i16,
    },
    StartActorTurn {
        actor: EntityId,
        angle: i16,
        speed: i32,
    },
    AimAtCamera,
    DefinePlayerControl {
        actor: EntityId,
    },
    SetControlDirection {
        arrow: i16,
        analog: i16,
    },
    DispatchCinematic {
        operation: i32,
        argument: i32,
        auxiliary: i32,
    },
    DefineRegion {
        region: EntityId,
        vertices: Vec<[i16; 2]>,
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
    ShowBattleMessage {
        text: i32,
    },
    SetObjectScale {
        actor: EntityId,
        scale: [i32; 3],
        vertical_scale: u8,
    },
    SetVibrationFrameRate {
        rate: i16,
    },
    PreloadField {
        hint: i32,
        field: i16,
    },
    SetRain {
        strength: i32,
        speed: i32,
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
    CalculateScreenPosition {
        object: i32,
    },
    ChangeField {
        field: i16,
    },
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
    SetSimulationFrameRate {
        simulation: u32,
        frame_rate: i16,
    },
    SetTileAnimationFrameWait {
        animation: i32,
        frame: i32,
        wait: i32,
    },
    SetTileAnimationFrame {
        animation: i32,
        frame: i32,
    },
    OpenPartyMenu {
        minimum_party_size: i32,
        fixed_members: i32,
    },
    InitializeCharacter {
        character: i32,
        enable_leveling: bool,
        equipment_set: i32,
        category: i32,
        menu_preset: i32,
    },
    StopTextureAnimation {
        object: i32,
        animation: i32,
    },
    RunBattleCode {
        code: u32,
        value: i32,
    },
    SetBattleMusicContinuation {
        enabled: bool,
    },
    RunModelCode {
        object: EntityId,
        code: i32,
        arguments: [i16; 3],
    },
    SetModelLight {
        model: i32,
        color: [u8; 3],
        shadow_intensity: i8,
        shadow_scale: i8,
    },
    ChangeElementalAffinity {
        target: u16,
        affinity_kind: i32,
        element: i32,
        add: bool,
    },
    SetRunEnabled {
        enabled: bool,
    },
    SetPartyAvailability {
        mask: i32,
    },
    MoveTile {
        overlay: i32,
        offset: [i16; 3],
    },
    SetTileScrollWithOffset {
        overlay: i32,
        enabled: i32,
        speed: i16,
        offset: i16,
        is_x_offset: i32,
    },
    SetTileViewportAlpha {
        viewport: i32,
        alpha: [i16; 2],
    },
    SetCameraBounds {
        camera: i32,
        bounds: [i16; 4],
    },
    SetCharacterRow {
        character: i32,
        row: u8,
    },
    ClearPartyStatus {
        character: i32,
        statuses: i32,
    },
    SetActorPitchRoll {
        actor: EntityId,
        pitch: i16,
        roll: i16,
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
    fn resolve_object_uid(&mut self, uid: i32) -> Result<EntityId, EngineError>;
    fn find_object_uid(&mut self, uid: u8) -> Result<Option<i32>, EngineError>;
    fn read_object_position(&mut self, object: EntityId) -> Result<[f32; 3], EngineError>;
    fn read_item_count(&mut self, item: i32) -> Result<i32, EngineError>;
    fn read_animation_frame(&mut self, actor: EntityId, animation: i32)
    -> Result<i32, EngineError>;
    fn read_bgi_animation_frame(&mut self, animation: u32) -> Result<i32, EngineError>;
    fn read_bgi_character_triangle(&mut self, character: i32) -> Result<i32, EngineError>;
    fn read_character_max_mp(&mut self, character: i32) -> Result<i32, EngineError>;
    fn party_contains(&mut self, character: i32) -> Result<bool, EngineError>;
    fn add_party_member(&mut self, character: i32) -> Result<bool, EngineError>;
    fn party_menu_is_open(&self) -> bool;
    fn actor_turn_is_active(&mut self, actor: EntityId) -> Result<bool, EngineError>;
    fn actor_animation_is_active(&mut self, actor: EntityId) -> Result<bool, EngineError>;
    fn field_exit_movement_is_active(&mut self, actor: EntityId) -> Result<bool, EngineError>;
    fn read_random_u8(&mut self) -> Result<u8, EngineError>;
    fn read_key_on(&mut self) -> Result<i32, EngineError>;
    fn caller_request_acceptable(&mut self, level: i32) -> Result<bool, EngineError>;
    fn script_request_acceptable(
        &mut self,
        target: EntityId,
        level: i32,
    ) -> Result<bool, EngineError>;
    fn script_request_is_complete(&mut self, entity: EntityId) -> Result<bool, EngineError>;
    fn read_custom_variable(&mut self, variable: u16) -> Result<i32, EngineError>;
}
