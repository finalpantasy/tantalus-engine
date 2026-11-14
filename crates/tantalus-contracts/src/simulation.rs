use crate::{ContentId, ContentRevision, EngineError, ErrorCategory};
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

macro_rules! define_id {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(u64);

        impl $name {
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            pub const fn get(self) -> u64 {
                self.0
            }
        }

        impl Display for $name {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
                write!(formatter, "{}", self.0)
            }
        }
    };
}

define_id!(EntityId);
define_id!(EncounterId);
define_id!(ResourceRequestId);
define_id!(TraceId);

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Tick(u64);

impl Tick {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    pub fn checked_add(self, stride: u32) -> Option<Self> {
        self.0.checked_add(stride.into()).map(Self)
    }
}

impl Display for Tick {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "tick {}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulationRate {
    pub numerator: u32,
    pub denominator: u32,
}

impl SimulationRate {
    pub fn new(numerator: u32, denominator: u32) -> Result<Self, EngineError> {
        if numerator == 0 || denominator == 0 {
            return Err(EngineError::new(
                ErrorCategory::InvalidProperties,
                "simulation rate numerator and denominator must be nonzero",
                "tantalus-contracts/simulation",
                true,
            ));
        }
        Ok(Self {
            numerator,
            denominator,
        })
    }

    pub fn per_second(ticks: u32) -> Result<Self, EngineError> {
        Self::new(ticks, 1)
    }

    pub fn tick_duration_nanoseconds(self) -> u128 {
        1_000_000_000_u128 * u128::from(self.denominator) / u128::from(self.numerator)
    }
}

impl Default for SimulationRate {
    fn default() -> Self {
        Self {
            numerator: 60,
            denominator: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressionMode {
    CompatibilityBarrier,
    ContinuingWorld,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RngConfig {
    pub algorithm: String,
    pub seed: u64,
}

impl Default for RngConfig {
    fn default() -> Self {
        Self {
            algorithm: "none".to_owned(),
            seed: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticPolicy {
    pub hash_every_step: bool,
    pub detailed_traces: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationRequest {
    pub state_digest: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulationConfig {
    pub rate: SimulationRate,
    pub progression: ProgressionMode,
    pub content_revision: ContentRevision,
    pub session_id: u64,
    pub max_step_stride: u32,
    pub rng: RngConfig,
    pub diagnostics: DiagnosticPolicy,
}

impl Default for SimulationConfig {
    fn default() -> Self {
        Self {
            rate: SimulationRate::default(),
            progression: ProgressionMode::ContinuingWorld,
            content_revision: ContentRevision::default(),
            session_id: 0,
            max_step_stride: 16,
            rng: RngConfig::default(),
            diagnostics: DiagnosticPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayTape {
    pub schema_version: u32,
    pub initial_state_digest: StateDigest,
    pub config: SimulationConfig,
    pub inputs: Vec<TickInput>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonTransition {
    Press,
    Release,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineMarker {
    pub tick_offset: u32,
    pub event: ContentId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectTimeline {
    pub duration_ticks: u32,
    pub markers: Vec<TimelineMarker>,
    pub on_complete: ContentId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameCommand {
    ButtonTransition {
        control: String,
        transition: ButtonTransition,
    },
    RequestResource {
        request: ResourceRequestId,
    },
    StartEffect {
        effect: EntityId,
        timeline: EffectTimeline,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalEventKind {
    Ready,
    Failed { code: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedExternalEvent {
    pub tick: Tick,
    pub sequence: u32,
    pub session_id: u64,
    pub content_revision: ContentRevision,
    pub request: ResourceRequestId,
    pub kind: ExternalEventKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TickInput {
    pub tick: Tick,
    pub stride: u32,
    pub commands: Vec<GameCommand>,
    pub external_events: Vec<RecordedExternalEvent>,
    pub verification: VerificationRequest,
}

impl TickInput {
    pub fn new(tick: Tick) -> Self {
        Self {
            tick,
            stride: 1,
            commands: Vec::new(),
            external_events: Vec::new(),
            verification: VerificationRequest::default(),
        }
    }

    pub fn with_stride(tick: Tick, stride: u32) -> Self {
        Self {
            tick,
            stride,
            commands: Vec::new(),
            external_events: Vec::new(),
            verification: VerificationRequest::default(),
        }
    }

    pub fn with_commands(mut self, commands: Vec<GameCommand>) -> Self {
        self.commands = commands;
        self
    }

    pub fn with_external_events(mut self, events: Vec<RecordedExternalEvent>) -> Self {
        self.external_events = events;
        self
    }

    pub fn requesting_digest(mut self) -> Self {
        self.verification.state_digest = true;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainEventKind {
    ButtonPressed {
        control: String,
    },
    ButtonReleased {
        control: String,
    },
    ResourceRequested {
        request: ResourceRequestId,
    },
    ResourceReady {
        request: ResourceRequestId,
    },
    ResourceFailed {
        request: ResourceRequestId,
        code: String,
    },
    EffectStarted {
        effect: EntityId,
    },
    EffectMarker {
        effect: EntityId,
        marker: ContentId,
    },
    EffectCompleted {
        effect: EntityId,
        callback: ContentId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainEvent {
    pub trace_id: TraceId,
    pub tick: Tick,
    pub kind: DomainEventKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceEvent {
    pub id: TraceId,
    pub tick: Tick,
    pub owner: String,
    pub source: Option<ContentId>,
    pub subject: Option<EntityId>,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "content")]
pub enum ActionMarkerKind {
    Effect(ContentId),
    Audio(ContentId),
    Hit,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticMarker {
    pub tick: Tick,
    pub action: ContentId,
    pub kind: ActionMarkerKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectPresentation {
    pub effect: EntityId,
    pub elapsed_ticks: u32,
    pub duration_ticks: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Point3 {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActorPresentation {
    pub entity: EntityId,
    pub position: Point3,
    pub size: Point3,
    pub color: [u8; 4],
    pub animation: Option<ContentId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldLayerPresentation {
    pub id: ContentId,
    pub position: Point3,
    pub size: Point3,
    pub color: [u8; 4],
    pub foreground: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraSnapshot {
    pub position: Point3,
    pub target: Point3,
}

impl Default for CameraSnapshot {
    fn default() -> Self {
        Self {
            position: Point3 { x: 0, y: 0, z: 500 },
            target: Point3::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenePresentationMode {
    #[default]
    Layered2d,
    Hybrid,
    Full3d,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioRequest {
    pub cue: ContentId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiRequest {
    pub screen: ContentId,
    pub visible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationSnapshot {
    pub tick: Tick,
    pub effects: Vec<EffectPresentation>,
    pub actors: Vec<ActorPresentation>,
    pub layers: Vec<FieldLayerPresentation>,
    pub camera: CameraSnapshot,
    pub mode: ScenePresentationMode,
    pub audio_requests: Vec<AudioRequest>,
    pub ui_requests: Vec<UiRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StateDigest(String);

impl StateDigest {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcome {
    Advanced,
    WaitingForReadiness,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TickResult {
    pub tick: Tick,
    pub outcome: StepOutcome,
    pub events: Vec<DomainEvent>,
    pub presentation: PresentationSnapshot,
    pub state_digest: Option<StateDigest>,
}
