use crate::commands::validate_timeline;
use crate::trace::TraceLog;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use tantalus_contracts::{
    ButtonTransition, ContentId, DomainEvent, DomainEventKind, EffectPresentation, EffectTimeline,
    EngineError, EntityId, ErrorCategory, ExternalEventKind, GameCommand, PresentationSnapshot,
    ProgressionMode, RecordedExternalEvent, ResourceRequestId, SimulationConfig, StateDigest,
    StepOutcome, Tick, TickInput, TickResult, TraceEvent, TraceId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceRequestStatus {
    Pending,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
struct ActiveEffect {
    timeline: EffectTimeline,
    elapsed_ticks: u32,
    next_marker: usize,
}

#[derive(Debug, Clone)]
pub struct Simulation {
    config: SimulationConfig,
    tick: Tick,
    buttons: BTreeMap<String, bool>,
    effects: BTreeMap<EntityId, ActiveEffect>,
    requests: BTreeMap<ResourceRequestId, ResourceRequestStatus>,
    pending_barrier: Option<ResourceRequestId>,
    rng_state: u64,
    next_trace_id: u64,
    traces: TraceLog,
}

impl Simulation {
    pub fn new(config: SimulationConfig) -> Result<Self, EngineError> {
        if config.rate.numerator == 0 || config.rate.denominator == 0 {
            return Err(simulation_error(
                ErrorCategory::InvalidProperties,
                "simulation rate must be nonzero",
                true,
            ));
        }
        if config.max_step_stride == 0 {
            return Err(simulation_error(
                ErrorCategory::InvalidProperties,
                "maximum step stride must be nonzero",
                true,
            ));
        }
        if config.rng.algorithm != "none" {
            return Err(simulation_error(
                ErrorCategory::InvalidProperties,
                format!(
                    "RNG algorithm `{}` is not implemented",
                    config.rng.algorithm
                ),
                true,
            ));
        }
        Ok(Self {
            rng_state: config.rng.seed,
            config,
            tick: Tick::default(),
            buttons: BTreeMap::new(),
            effects: BTreeMap::new(),
            requests: BTreeMap::new(),
            pending_barrier: None,
            next_trace_id: 1,
            traces: TraceLog::default(),
        })
    }

    pub fn tick(&self) -> Tick {
        self.tick
    }

    pub fn pending_barrier(&self) -> Option<ResourceRequestId> {
        self.pending_barrier
    }

    pub fn button_held(&self, control: &str) -> bool {
        self.buttons.get(control).copied().unwrap_or(false)
    }

    pub fn request_status(&self, request: ResourceRequestId) -> Option<ResourceRequestStatus> {
        self.requests.get(&request).copied()
    }

    pub fn traces(&self) -> impl Iterator<Item = &TraceEvent> {
        self.traces.iter()
    }

    pub fn state_digest(&self) -> Result<StateDigest, EngineError> {
        self.digest()
    }

    pub fn step(&mut self, input: TickInput) -> Result<TickResult, EngineError> {
        let mut staged = self.clone();
        let result = staged.step_inner(input)?;
        *self = staged;
        Ok(result)
    }

    fn step_inner(&mut self, input: TickInput) -> Result<TickResult, EngineError> {
        self.validate_input(&input)?;
        let mut external_events = input.external_events.clone();
        external_events.sort_by_key(|event| (event.tick, event.sequence));

        if self.config.progression == ProgressionMode::CompatibilityBarrier
            && let Some(request) = self.pending_barrier
            && !external_events.iter().any(|event| event.request == request)
        {
            return Ok(TickResult {
                tick: self.tick,
                outcome: StepOutcome::WaitingForReadiness,
                events: Vec::new(),
                presentation: self.presentation(),
                state_digest: self.digest()?,
            });
        }

        let mut events = Vec::new();
        for event in external_events {
            self.apply_external(event, &mut events)?;
        }
        self.advance_effects(input.stride, &mut events)?;
        for command in input.commands {
            self.apply_command(command, input.tick, &mut events)?;
        }
        self.tick = input.tick;

        Ok(TickResult {
            tick: self.tick,
            outcome: StepOutcome::Advanced,
            events,
            presentation: self.presentation(),
            state_digest: self.digest()?,
        })
    }

    fn validate_input(&self, input: &TickInput) -> Result<(), EngineError> {
        if input.stride == 0 || input.stride > self.config.max_step_stride {
            return Err(simulation_error(
                ErrorCategory::InvalidProperties,
                format!(
                    "step stride {} is outside 1..={}",
                    input.stride, self.config.max_step_stride
                ),
                true,
            ));
        }
        let expected = self.tick.checked_add(input.stride).ok_or_else(|| {
            simulation_error(ErrorCategory::CapacityExhausted, "tick overflow", false)
        })?;
        if input.tick != expected {
            return Err(simulation_error(
                ErrorCategory::Conflict,
                format!("expected {expected}, received {}", input.tick),
                true,
            ));
        }

        for event in &input.external_events {
            if event.tick <= self.tick || event.tick > input.tick {
                return Err(simulation_error(
                    ErrorCategory::Conflict,
                    format!(
                        "external event for request {} is outside the step's tick range",
                        event.request
                    ),
                    true,
                ));
            }
            if event.session_id != self.config.session_id
                || event.content_revision != self.config.content_revision
            {
                return Err(simulation_error(
                    ErrorCategory::Conflict,
                    format!(
                        "external event for request {} has a stale session or content revision",
                        event.request
                    ),
                    true,
                ));
            }
        }
        Ok(())
    }

    fn apply_external(
        &mut self,
        event: RecordedExternalEvent,
        events: &mut Vec<DomainEvent>,
    ) -> Result<(), EngineError> {
        if self.requests.get(&event.request) != Some(&ResourceRequestStatus::Pending) {
            return Err(simulation_error(
                ErrorCategory::Conflict,
                format!(
                    "external completion for request {} is stale, duplicated, or unknown",
                    event.request
                ),
                true,
            ));
        }

        let kind = match event.kind {
            ExternalEventKind::Ready => {
                self.requests
                    .insert(event.request, ResourceRequestStatus::Ready);
                DomainEventKind::ResourceReady {
                    request: event.request,
                }
            }
            ExternalEventKind::Failed { code } => {
                self.requests
                    .insert(event.request, ResourceRequestStatus::Failed);
                DomainEventKind::ResourceFailed {
                    request: event.request,
                    code,
                }
            }
        };
        if self.pending_barrier == Some(event.request) {
            self.pending_barrier = None;
        }
        self.emit(event.tick, kind, None, None, events)
    }

    fn apply_command(
        &mut self,
        command: GameCommand,
        command_tick: Tick,
        events: &mut Vec<DomainEvent>,
    ) -> Result<(), EngineError> {
        match command {
            GameCommand::ButtonTransition {
                control,
                transition,
            } => {
                if control.is_empty() {
                    return Err(simulation_error(
                        ErrorCategory::InvalidProperties,
                        "button control name cannot be empty",
                        true,
                    ));
                }
                let held = self.button_held(&control);
                match transition {
                    ButtonTransition::Press => {
                        self.buttons.insert(control.clone(), true);
                        if !held {
                            self.emit(
                                command_tick,
                                DomainEventKind::ButtonPressed { control },
                                None,
                                None,
                                events,
                            )?;
                        }
                    }
                    ButtonTransition::Release => {
                        self.buttons.insert(control.clone(), false);
                        if held {
                            self.emit(
                                command_tick,
                                DomainEventKind::ButtonReleased { control },
                                None,
                                None,
                                events,
                            )?;
                        }
                    }
                }
                Ok(())
            }
            GameCommand::RequestResource { request } => {
                if self.requests.contains_key(&request) {
                    return Err(simulation_error(
                        ErrorCategory::Conflict,
                        format!("resource request {request} already exists"),
                        true,
                    ));
                }
                if self.pending_barrier.is_some() {
                    return Err(simulation_error(
                        ErrorCategory::Conflict,
                        "a compatibility readiness barrier is already pending",
                        true,
                    ));
                }
                self.requests
                    .insert(request, ResourceRequestStatus::Pending);
                if self.config.progression == ProgressionMode::CompatibilityBarrier {
                    self.pending_barrier = Some(request);
                }
                self.emit(
                    command_tick,
                    DomainEventKind::ResourceRequested { request },
                    None,
                    None,
                    events,
                )
            }
            GameCommand::StartEffect { effect, timeline } => {
                validate_timeline(&timeline)?;
                if self.effects.contains_key(&effect) {
                    return Err(simulation_error(
                        ErrorCategory::Conflict,
                        format!("effect entity {effect} is already active"),
                        true,
                    ));
                }
                self.effects.insert(
                    effect,
                    ActiveEffect {
                        timeline,
                        elapsed_ticks: 0,
                        next_marker: 0,
                    },
                );
                self.emit(
                    command_tick,
                    DomainEventKind::EffectStarted { effect },
                    None,
                    Some(effect),
                    events,
                )
            }
        }
    }

    fn advance_effects(
        &mut self,
        stride: u32,
        events: &mut Vec<DomainEvent>,
    ) -> Result<(), EngineError> {
        let mut effects = std::mem::take(&mut self.effects);
        let mut pending_events = Vec::new();
        let effect_tick = self.tick.checked_add(stride).unwrap_or(self.tick);

        for (effect_id, effect) in effects.iter_mut() {
            effect.elapsed_ticks = effect
                .elapsed_ticks
                .saturating_add(stride)
                .min(effect.timeline.duration_ticks);

            while effect.next_marker < effect.timeline.markers.len() {
                let marker = &effect.timeline.markers[effect.next_marker];
                if marker.tick_offset > effect.elapsed_ticks {
                    break;
                }
                pending_events.push((
                    effect_tick,
                    DomainEventKind::EffectMarker {
                        effect: *effect_id,
                        marker: marker.event.clone(),
                    },
                    Some(marker.event.clone()),
                    Some(*effect_id),
                ));
                effect.next_marker += 1;
            }

            if effect.elapsed_ticks == effect.timeline.duration_ticks {
                pending_events.push((
                    effect_tick,
                    DomainEventKind::EffectCompleted {
                        effect: *effect_id,
                        callback: effect.timeline.on_complete.clone(),
                    },
                    Some(effect.timeline.on_complete.clone()),
                    Some(*effect_id),
                ));
            }
        }

        effects.retain(|_, effect| effect.elapsed_ticks < effect.timeline.duration_ticks);
        self.effects = effects;
        for (tick, kind, source, subject) in pending_events {
            self.emit(tick, kind, source, subject, events)?;
        }
        Ok(())
    }

    fn emit(
        &mut self,
        tick: Tick,
        kind: DomainEventKind,
        source: Option<ContentId>,
        subject: Option<EntityId>,
        events: &mut Vec<DomainEvent>,
    ) -> Result<(), EngineError> {
        let id = TraceId::new(self.next_trace_id);
        self.next_trace_id = self.next_trace_id.checked_add(1).ok_or_else(|| {
            simulation_error(ErrorCategory::CapacityExhausted, "trace ID overflow", false)
        })?;
        self.traces.push(TraceEvent {
            id,
            tick,
            owner: "tantalus-core/simulation".to_owned(),
            source,
            subject,
            description: format!("{kind:?}"),
        });
        events.push(DomainEvent {
            trace_id: id,
            tick,
            kind,
        });
        Ok(())
    }

    fn presentation(&self) -> PresentationSnapshot {
        PresentationSnapshot {
            tick: self.tick,
            effects: self
                .effects
                .iter()
                .map(|(effect, state)| EffectPresentation {
                    effect: *effect,
                    elapsed_ticks: state.elapsed_ticks,
                    duration_ticks: state.timeline.duration_ticks,
                })
                .collect(),
        }
    }

    fn digest(&self) -> Result<StateDigest, EngineError> {
        #[derive(Serialize)]
        struct AuthoritativeState<'a> {
            config: &'a SimulationConfig,
            tick: Tick,
            buttons: &'a BTreeMap<String, bool>,
            effects: &'a BTreeMap<EntityId, ActiveEffect>,
            requests: &'a BTreeMap<ResourceRequestId, ResourceRequestStatus>,
            pending_barrier: Option<ResourceRequestId>,
            rng_state: u64,
            next_trace_id: u64,
        }

        let state = AuthoritativeState {
            config: &self.config,
            tick: self.tick,
            buttons: &self.buttons,
            effects: &self.effects,
            requests: &self.requests,
            pending_barrier: self.pending_barrier,
            rng_state: self.rng_state,
            next_trace_id: self.next_trace_id,
        };
        let bytes = serde_json::to_vec(&state).map_err(|error| {
            simulation_error(
                ErrorCategory::Internal,
                format!("could not serialize authoritative state: {error}"),
                false,
            )
        })?;
        Ok(StateDigest::new(format!("{:x}", Sha256::digest(bytes))))
    }
}

fn simulation_error(
    category: ErrorCategory,
    context: impl Into<String>,
    recoverable: bool,
) -> EngineError {
    EngineError::new(category, context, "tantalus-core/simulation", recoverable)
}
