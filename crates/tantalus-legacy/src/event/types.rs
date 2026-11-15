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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CharacterStat {
    CurrentHp,
    MaxHp,
    CurrentMp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyState {
    Off,
    Held,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpressionOp {
    /// `B_CURHP` 82, `B_MAXHP` 83 and `B_CURMP` 110: a party member's stat by character id.
    CharacterStat(CharacterStat),
    /// `B_KEYOFF` 88 and `B_KEY` 89: the mask intersected with a host input state.
    KeyState(KeyState),
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
    /// `B_DISTANCE` 94: distance from the current actor to an X/Z point.
    DistanceToPoint,
    AngleToObject,
    DistanceToObject,
    ObjectPointer(u8),
    ItemCount,
    AnimationFrame,
    BgiAnimationFrame,
    BgiCharacterTriangle,
    /// `B_BGIFLOOR` 113: the floor index of a character's walkmesh position.
    BgiCharacterFloor,
    /// `B_LE` 26.
    LessOrEqual,
    /// `B_COUNT` 42: number of active member bits in the operand.
    ActiveBitCount,
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
    /// `B_DIV_LET` 48: divide-assign; a zero divisor leaves the target unchanged.
    DivideAssign,
    /// `B_REM_LET` 49: remainder-assign; a zero divisor leaves the target unchanged.
    RemainderAssign,
    /// `B_SHIFT_LEFT_LET` 52.
    ShiftLeftAssign,
    /// `B_SHIFT_RIGHT_LET` 53.
    ShiftRightAssign,
    /// `B_AND_LET` 61.
    AndAssign,
    /// `B_XOR_LET` 62.
    XorAssign,
    /// `B_OR_LET` 63.
    OrAssign,
    /// `B_SYSLIST` 121: an element of the engine system list (battle script state).
    SystemList(u8),
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
    /// Memoria `beq`: pops the previous `Evaluate` result; jumps forward by an
    /// unsigned offset when it is zero.
    JumpIfNot {
        relative: u16,
    },
    /// Memoria `bne`: pops the previous `Evaluate` result; jumps by a signed
    /// offset when it is not zero.
    JumpIf {
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
    AddItem {
        item: Expression,
        count: Expression,
    },
    RemoveItem {
        item: Expression,
        count: Expression,
    },
    AddGil(Expression),
    RemoveGil(Expression),
    SetPartyHp {
        character: Expression,
        value: Expression,
    },
    SetPartyMp {
        character: Expression,
        value: Expression,
    },
    ClearAbilityAp {
        character: Expression,
        ability: Expression,
    },
    MasterAbility {
        character: Expression,
        ability: Expression,
    },
    WalkActor {
        x: Expression,
        z: Expression,
    },
    MoveActor {
        actor: Expression,
        x: Expression,
        z: Expression,
    },
    AimAtCamera,
    /// `ENCOUNT` 0x2A (`group: None`) and `ENCOUNT2` 0x8C: start a scripted battle.
    StartBattle {
        rush: Expression,
        group: Option<Expression>,
        battle: Expression,
    },
    /// `WMAPJUMP` 0xB6.
    ChangeWorldMap {
        destination: Expression,
    },
    /// `SYNCPARTY` 0xE9.
    SyncPartyUid,
    /// `MOVQ` 0x9E: walk the controlled character to the field exit and flush the field.
    ExitField,
    /// `PREJUMP` 0x9C (`landing: false`) and `POSTJUMP` 0x9D (`landing: true`).
    PlayJumpAnimation {
        landing: bool,
    },
    /// `SLEEPINH` 0x90.
    DisableIdleAnimation,
    /// `ENDANIM` 0x42.
    StopAnimation,
    /// `BGIACTIVEF` 0xCB.
    SetPathFloorActive {
        floor: Expression,
        active: Expression,
    },
    /// `PRETEND` 0xB5.
    PretendToBe {
        target: Expression,
    },
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
    RequestScriptSync {
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
    /// `POS3` 0xA1: arguments in file order X, Z (negated at execution), Y.
    SetActorPositionXzy {
        x: Expression,
        z: Expression,
        y: Expression,
    },
    /// `DPOS3` 0xAD: object entry then X, Z (negated at execution), Y.
    MoveActorXzy {
        actor: Expression,
        x: Expression,
        z: Expression,
        y: Expression,
    },
    /// `MOVE3` 0xA2, `MOVE3H` 0xE8 and `MOVH` 0xA5: the `MoveToward_mixed(x, y, z)` form.
    /// `y` is negated at execution; `MOVH` supplies a constant zero `y`.
    WalkActorMixed {
        opcode: u8,
        x: Expression,
        y: Expression,
        z: Expression,
    },
    /// `TURNTO` 0x9B.
    TurnActorToPosition {
        x: Expression,
        z: Expression,
    },
    /// `WAITSEQ` 0x44.
    WaitSharedScript,
    /// `BGI` 0xA8.
    SetPathing {
        enabled: Expression,
    },
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
    SetTileBlockActive {
        block: Expression,
        active: Expression,
    },
    SetTileLoop {
        block: Expression,
        enabled: Expression,
        x_speed: Expression,
        y_speed: Expression,
    },
    SetTileParallax {
        block: Expression,
        enabled: Expression,
        x_speed: Expression,
        y_speed: Expression,
    },
    SetTileOrigin {
        block: Expression,
        x: Expression,
        y: Expression,
    },
    RunTileAnimation {
        animation: Expression,
        start_frame: Expression,
    },
    SetTileAnimationActive {
        animation: Expression,
        active: Expression,
    },
    ResetTileAnimation {
        animation: Expression,
        show_first_frame: Expression,
    },
    StartVibration {
        frame: Expression,
    },
    SetVibrationActive {
        active: Expression,
    },
    RunVibrationTrack {
        track: Expression,
        sample: Expression,
        active: Expression,
    },
    ActivateVibrationTrack {
        track: Expression,
        sample: Expression,
        active: Expression,
    },
    SetVibrationFlags {
        flags: Expression,
    },
    SetVibrationRange {
        start: Expression,
        end: Expression,
    },
    SetShadowSize {
        x: Expression,
        z: Expression,
    },
    SetShadowOffset {
        x: Expression,
        z: Expression,
    },
    LockShadowRotation {
        rotation: Expression,
    },
    UnlockShadowRotation,
    SetShadowAmplifier {
        factor: Expression,
    },
    MoveCamera {
        x: Expression,
        y: Expression,
        duration: Expression,
        mode: Expression,
    },
    ReleaseCamera {
        duration: Expression,
        mode: Expression,
    },
    EnableCameraServices {
        active: Expression,
        frame_count: Expression,
        mode: Expression,
    },
    SetCameraFollowHeight {
        height: Expression,
    },
    SetRandomBattles {
        pattern: Expression,
        scene_0: Expression,
        scene_1: Expression,
        scene_2: Expression,
        scene_3: Expression,
    },
    SetRandomBattleFrequency {
        frequency: Expression,
    },
    SetBackgroundColor {
        red: Expression,
        green: Expression,
        blue: Expression,
    },
    AddControllerMask {
        pad: Expression,
        buttons: Expression,
    },
    RemoveControllerMask {
        pad: Expression,
        buttons: Expression,
    },
    ShowObjectMeshes {
        object: Expression,
        mesh: Expression,
    },
    HideObjectMeshes {
        object: Expression,
        mesh: Expression,
    },
    SetObjectIndex {
        index: Expression,
    },
    SetAnimationFlags {
        flags: Expression,
        loop_count: Expression,
    },
    SetInactiveAnimation {
        animation: Expression,
    },
    SetFollowFocusTurn {
        enabled: Expression,
    },
    SetAnimationLoopCount {
        count: Expression,
    },
    SetAnimationSpeed {
        speed: Expression,
    },
    SetCharacterAttribute {
        set: bool,
        flags: Expression,
    },
    SetTurnSpeed {
        speed: Expression,
    },
    SetRunSpeedLimit {
        limit: Expression,
    },
    TurnToAngle {
        angle: Expression,
    },
    SetTimerTime {
        seconds: Expression,
    },
    EnableDialogChoices {
        choices: Expression,
        default_choice: Expression,
    },
    SetFieldName {
        text: Expression,
    },
    AttachObject {
        attached: Expression,
        carrying: Expression,
        bone: Expression,
    },
    DetachObject {
        attached: Expression,
    },
    TrackStart {
        x: Expression,
        z: Expression,
    },
    TrackSetLast {
        x: Expression,
        z: Expression,
    },
    TrackAdd,
    SetSoundPosition {
        x: Expression,
        z: Expression,
        y: Expression,
        volume: Expression,
    },
    SetSoundObjectPosition {
        object: Expression,
        volume: Expression,
    },
    ShowTimer {
        show: Expression,
    },
    RunWorldCode {
        code: Expression,
        value: Expression,
    },
    EnableVictoryPose {
        character: Expression,
        enabled: Expression,
    },
    RemoveParty {
        character: Expression,
    },
    SetPlayerName {
        character: Expression,
        text: Expression,
    },
    SetupJump {
        x: Expression,
        z: Expression,
        y: Expression,
        steps: Expression,
    },
    SetDialogSignal {
        signal: Expression,
    },
    SetTileLoopType {
        overlay: Expression,
        screen_anchored: Expression,
    },
    ApplyFadeFilter {
        mode: Expression,
        frame: Expression,
        unknown: Expression,
        cyan: Expression,
        magenta: Expression,
        yellow: Expression,
    },
    OpenMenu {
        menu: Expression,
        subject: Expression,
    },
    SetActiveTimeEventIcon {
        mode: Expression,
    },
    SetObjectOvalRatio {
        ratio: Expression,
    },
    TerminateBattle,
    ResetIdleTimer,
    CloseAllWindows,
    AlignActorToCameraEye,
    ProgressJump,
    SetUserControl {
        enabled: bool,
        opcode: u8,
    },
    RaiseWindows,
    CalculateScreenOrigin,
    RunSharedScript {
        entry: Expression,
    },
    TurnTowardObject {
        target: Expression,
        speed: Expression,
    },
    ShowBubbleIcon {
        icon: Expression,
    },
    SetFieldCamera {
        camera: Expression,
    },
    EndSharedScript,
    SetMenuControl {
        enabled: bool,
        opcode: u8,
    },
    SetTileAnimationRate {
        animation: Expression,
        frame_rate: Expression,
    },
    SetTileAnimationFrameWait {
        animation: Expression,
        frame: Expression,
        wait: Expression,
    },
    SetTileAnimationFlags {
        animation: Expression,
        flags: Expression,
    },
    SetTileAnimationRange {
        animation: Expression,
        first_frame: Expression,
        last_frame: Expression,
    },
    IncrementFrogCount,
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
    AddBattleStatus {
        target: Expression,
        status: Expression,
        permanent: Expression,
        arguments: [Expression; 3],
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
    SetHereIcon(Expression),
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
    ActorMovement {
        next: u32,
    },
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
    SharedScript,
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
    AddItem {
        item: i32,
        count: i32,
    },
    RemoveItem {
        item: i32,
        count: i32,
    },
    /// Raw unsigned amount; the host clamps to 9,999,999 gil.
    AddGil {
        amount: u32,
    },
    /// Raw unsigned amount; the host clamps underflow to zero and owns achievement side effects.
    RemoveGil {
        amount: u32,
    },
    /// Raw character and HP operands; slot mapping, the 9999 full-heal sentinel, clamping,
    /// substitute-member mirroring, and heal propagation stay host-owned.
    SetPartyHp {
        character: i32,
        hp: i32,
    },
    /// Raw character and MP operands; the 999 full-restore sentinel and clamping are host-owned.
    SetPartyMp {
        character: i32,
        mp: i32,
    },
    ClearAbilityAp {
        character: i32,
        ability: i32,
    },
    MasterAbility {
        character: i32,
        ability: i32,
    },
    WalkActor {
        actor: EntityId,
        destination: [i16; 2],
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
    StartBattle {
        rush: i32,
        group: Option<i32>,
        battle: i32,
        steiner_state: bool,
    },
    ChangeWorldMap {
        destination: i16,
    },
    SyncPartyUid,
    ExitField,
    PlayJumpAnimation {
        actor: EntityId,
        landing: bool,
    },
    DisableIdleAnimation,
    StopAnimation {
        actor: EntityId,
    },
    SetPathFloorActive {
        floor: i32,
        active: i32,
    },
    PretendToBe {
        actor: EntityId,
        target: EntityId,
    },
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
    RequestScriptSync {
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
    SetTileBlockActive {
        block: i32,
        active: bool,
    },
    SetTileLoop {
        block: i32,
        enabled: i32,
        x_speed: i16,
        y_speed: i16,
    },
    SetTileParallax {
        block: i32,
        enabled: i32,
        x_speed: i16,
        y_speed: i16,
    },
    SetTileOrigin {
        block: i32,
        x: i16,
        y: i16,
    },
    RunTileAnimation {
        animation: i32,
        start_frame: i32,
    },
    SetTileAnimationActive {
        animation: i32,
        active: bool,
    },
    ResetTileAnimation {
        animation: i32,
        show_first_frame: i32,
    },
    StartVibration {
        frame: i16,
    },
    SetVibrationActive {
        active: bool,
    },
    RunVibrationTrack {
        track: i32,
        sample: i32,
        active: bool,
    },
    ActivateVibrationTrack {
        track: u32,
        sample: u32,
        active: bool,
    },
    SetVibrationFlags {
        flags: i16,
    },
    SetVibrationRange {
        start: i16,
        end: i16,
    },
    SetShadowSize {
        actor: EntityId,
        x: i32,
        z: i32,
    },
    SetShadowOffset {
        actor: EntityId,
        x: i16,
        z: i16,
    },
    LockShadowRotation {
        actor: EntityId,
        rotation: i32,
    },
    UnlockShadowRotation {
        actor: EntityId,
    },
    SetShadowAmplifier {
        actor: EntityId,
        factor: i32,
    },
    MoveCamera {
        x: i16,
        y: i16,
        duration: u16,
        mode: u32,
    },
    ReleaseCamera {
        duration: i32,
        mode: u32,
    },
    EnableCameraServices {
        active: u32,
        frame_count: i32,
        mode: u32,
    },
    SetCameraFollowHeight {
        height: i16,
    },
    SetRandomBattles {
        pattern: u8,
        scenes: [u16; 4],
    },
    SetRandomBattleFrequency {
        frequency: u8,
    },
    SetBackgroundColor {
        red: i32,
        green: i32,
        blue: i32,
    },
    AddControllerMask {
        pad: i32,
        buttons: u32,
    },
    RemoveControllerMask {
        pad: i32,
        buttons: u32,
    },
    ShowObjectMeshes {
        object: EntityId,
        mesh: i32,
    },
    HideObjectMeshes {
        object: EntityId,
        mesh: i32,
    },
    SetObjectIndex {
        actor: EntityId,
        index: u8,
    },
    SetAnimationFlags {
        actor: EntityId,
        flags: u8,
        loop_count: u8,
    },
    SetInactiveAnimation {
        actor: EntityId,
        animation: u16,
    },
    SetFollowFocusTurn {
        actor: EntityId,
        enabled: bool,
    },
    SetAnimationLoopCount {
        actor: EntityId,
        count: u8,
    },
    MoveActorXzy {
        actor: EntityId,
        position: [i32; 3],
    },
    WalkActorMixed {
        actor: EntityId,
        destination: [i32; 3],
        mode: u8,
    },
    TurnActorToPosition {
        actor: EntityId,
        x: i32,
        z: i32,
    },
    SetPathing {
        actor: EntityId,
        enabled: i32,
    },
    SetAnimationSpeed {
        actor: EntityId,
        speed: u8,
    },
    SetCharacterAttribute {
        actor: EntityId,
        set: bool,
        flags: i32,
    },
    SetTurnSpeed {
        actor: EntityId,
        speed: u8,
    },
    SetRunSpeedLimit {
        actor: EntityId,
        limit: u8,
    },
    TurnToAngle {
        actor: EntityId,
        angle: i16,
    },
    SetTimerTime {
        seconds: i32,
    },
    EnableDialogChoices {
        choices: i32,
        default_choice: i32,
    },
    SetFieldName {
        text: i32,
    },
    AttachObject {
        attached: EntityId,
        carrying: EntityId,
        bone: i32,
    },
    DetachObject {
        attached: EntityId,
    },
    TrackStart {
        actor: EntityId,
        x: i16,
        z: i16,
    },
    TrackSetLast {
        actor: EntityId,
        x: i16,
        z: i16,
    },
    TrackAdd {
        actor: EntityId,
    },
    SetSoundPosition {
        position: [i32; 3],
        volume: i32,
    },
    SetSoundObjectPosition {
        object: EntityId,
        volume: i32,
    },
    ShowTimer {
        show: bool,
    },
    RunWorldCode {
        code: i32,
        value: i32,
    },
    EnableVictoryPose {
        character: i32,
        enabled: bool,
    },
    RemoveParty {
        character: i32,
    },
    SetPlayerName {
        character: i32,
        text: i32,
    },
    SetupJump {
        actor: EntityId,
        x: i16,
        z: i16,
        y: i16,
        steps: u8,
    },
    SetDialogSignal {
        signal: i32,
    },
    SetTileLoopType {
        overlay: i32,
        screen_anchored: bool,
    },
    ApplyFadeFilter {
        mode: i32,
        frame: i32,
        cyan: i32,
        magenta: i32,
        yellow: i32,
    },
    OpenMenu {
        menu: i32,
        subject: i32,
    },
    SetActiveTimeEventIcon {
        mode: i32,
    },
    SetObjectOvalRatio {
        actor: EntityId,
        ratio: i32,
    },
    TerminateBattle,
    ResetIdleTimer,
    CloseAllWindows,
    AlignActorToCameraEye {
        actor: EntityId,
    },
    ProgressJump {
        actor: EntityId,
    },
    SetUserControl {
        enabled: bool,
    },
    RaiseWindows,
    CalculateScreenOrigin,
    RunSharedScript {
        actor: EntityId,
        entry: i32,
    },
    TurnTowardObject {
        actor: EntityId,
        target: i32,
        speed: i32,
    },
    ShowBubbleIcon {
        icon: i32,
    },
    SetFieldCamera {
        camera: i32,
    },
    EndSharedScript {
        actor: EntityId,
    },
    SetMenuControl {
        enabled: bool,
    },
    SetTileAnimationRate {
        animation: i32,
        frame_rate: i16,
    },
    SetTileAnimationFrameWait {
        animation: i32,
        frame: i32,
        wait: i32,
    },
    SetTileAnimationFlags {
        animation: i32,
        flags: i32,
    },
    SetTileAnimationRange {
        animation: i32,
        first_frame: i32,
        last_frame: i32,
    },
    IncrementFrogCount,
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
    AddBattleStatus {
        target: u16,
        status: i32,
        permanent: bool,
        arguments: [i32; 3],
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
    SetHereIcon {
        mode: i32,
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
    fn read_system_list(&mut self, index: u8) -> Result<i32, EngineError>;
    fn write_system_list(&mut self, index: u8, value: i32) -> Result<(), EngineError>;
    fn read_atan_entry(&mut self, offset: i32) -> Result<u16, EngineError>;
    fn resolve_object_uid(&mut self, uid: i32) -> Result<EntityId, EngineError>;
    fn find_object_uid(&mut self, uid: u8) -> Result<Option<i32>, EngineError>;
    fn read_object_position(&mut self, object: EntityId) -> Result<[f32; 3], EngineError>;
    fn read_item_count(&mut self, item: i32) -> Result<i32, EngineError>;
    fn read_animation_frame(&mut self, actor: EntityId, animation: i32)
    -> Result<i32, EngineError>;
    fn read_bgi_animation_frame(&mut self, animation: u32) -> Result<i32, EngineError>;
    fn read_bgi_character_triangle(&mut self, character: i32) -> Result<i32, EngineError>;
    fn read_bgi_character_floor(&mut self, character: i32) -> Result<i32, EngineError>;
    fn read_character_max_mp(&mut self, character: i32) -> Result<i32, EngineError>;
    fn party_contains(&mut self, character: i32) -> Result<bool, EngineError>;
    fn add_party_member(&mut self, character: i32) -> Result<bool, EngineError>;
    fn party_menu_is_open(&self) -> bool;
    fn actor_turn_is_active(&mut self, actor: EntityId) -> Result<bool, EngineError>;
    fn shared_script_is_active(&mut self, actor: EntityId) -> Result<bool, EngineError>;
    fn actor_animation_is_active(&mut self, actor: EntityId) -> Result<bool, EngineError>;
    fn field_actor_movement_is_active(&mut self, actor: EntityId) -> Result<bool, EngineError>;
    fn field_exit_movement_is_active(&mut self, actor: EntityId) -> Result<bool, EngineError>;
    fn actor_jump_is_active(&mut self, actor: EntityId) -> Result<bool, EngineError>;
    fn read_random_u8(&mut self) -> Result<u8, EngineError>;
    fn read_key_on(&mut self) -> Result<i32, EngineError>;
    fn read_key_state(&mut self, state: KeyState) -> Result<i32, EngineError>;
    fn read_character_stat(
        &mut self,
        character: i32,
        stat: CharacterStat,
    ) -> Result<i32, EngineError>;
    fn caller_request_acceptable(&mut self, level: i32) -> Result<bool, EngineError>;
    fn script_request_acceptable(
        &mut self,
        target: EntityId,
        level: i32,
    ) -> Result<bool, EngineError>;
    fn script_request_is_complete(&mut self, entity: EntityId) -> Result<bool, EngineError>;
    fn read_custom_variable(&mut self, variable: u16) -> Result<i32, EngineError>;
}
