use super::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use tantalus_contracts::{
    ContentId, ContentRevision, EncounterId, EngineError, EntityId, ErrorCategory, Point3,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncounterPausePolicy {
    SuspendField,
    ContinueWorld,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncounterPlan {
    pub source_anchor: Point3,
    pub rules: ContentId,
    pub space: EncounterSpaceKind,
    pub presentation: ContentId,
    pub pause_policy: EncounterPausePolicy,
    pub participants: Vec<EntityId>,
    pub resources: Vec<ResourceRequirement>,
    pub source_to_arena: SpaceTransform,
    pub session_id: u64,
    pub content_revision: ContentRevision,
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncounterPhase {
    Preparing,
    Transitioning,
    Fighting,
    Returning,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncounterOutcome {
    Victory,
    Defeat,
    Fled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalEventContext {
    pub session_id: u64,
    pub content_revision: ContentRevision,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncounterEvent {
    Poll,
    ResourceReady {
        encounter: EncounterId,
        resource: ContentId,
        stage: ReadinessStage,
        context: ExternalEventContext,
    },
    ResourceFailed {
        encounter: EncounterId,
        resource: ContentId,
        stage: ReadinessStage,
        context: ExternalEventContext,
        code: String,
    },
    TransitionCompleted {
        encounter: EncounterId,
        generation: u64,
    },
    CombatFinished {
        encounter: EncounterId,
        generation: u64,
        outcome: EncounterOutcome,
    },
    ReturnCompleted {
        encounter: EncounterId,
        generation: u64,
    },
    Cancel {
        encounter: EncounterId,
        generation: u64,
        reason: String,
    },
    SceneChanged {
        encounter: EncounterId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncounterCommand {
    ClaimParticipants {
        participants: Vec<EntityId>,
    },
    AcquireResourceLeases {
        resources: Vec<ContentId>,
    },
    SuspendExploration,
    ContinueExploration,
    BeginTransition {
        presentation: ContentId,
        space: EncounterSpaceKind,
    },
    StartCombat {
        rules: ContentId,
    },
    CommitOutcome {
        outcome: EncounterOutcome,
        resolution_id: u64,
    },
    BeginReturn,
    ReleaseParticipants,
    ReleaseResourceLeases,
    ResumeExploration,
    CompleteEncounter,
    FailEncounter {
        code: String,
    },
    CancelEncounter {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncounterStateView {
    pub phase: EncounterPhase,
    pub pending_readiness: usize,
    pub outcome: Option<EncounterOutcome>,
    pub resolution_id: Option<u64>,
}

#[derive(Debug, Clone)]
struct EncounterRecord {
    plan: EncounterPlan,
    phase: EncounterPhase,
    readiness: ReadinessTracker,
    claimed: bool,
    leases: bool,
    suspended: bool,
    outcome: Option<EncounterOutcome>,
    resolution_id: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct EncounterCoordinator {
    encounters: BTreeMap<EncounterId, EncounterRecord>,
    active: Option<EncounterId>,
    next_encounter: u64,
    next_resolution: u64,
}

impl EncounterCoordinator {
    pub fn new() -> Self {
        Self {
            next_encounter: 1,
            next_resolution: 1,
            ..Self::default()
        }
    }

    pub fn request(&mut self, plan: EncounterPlan) -> Result<EncounterId, EngineError> {
        validate_plan(&plan)?;
        if self.active.is_some_and(|id| {
            self.encounters
                .get(&id)
                .is_some_and(|record| !is_terminal(record.phase))
        }) {
            return Err(coordinator_error(
                "another encounter is already active",
                true,
            ));
        }

        let id = EncounterId::new(self.next_encounter);
        self.next_encounter = self
            .next_encounter
            .checked_add(1)
            .ok_or_else(|| coordinator_error("encounter ID space exhausted", false))?;
        let readiness = ReadinessTracker::new(&plan.resources)?;
        self.encounters.insert(
            id,
            EncounterRecord {
                plan,
                phase: EncounterPhase::Preparing,
                readiness,
                claimed: false,
                leases: false,
                suspended: false,
                outcome: None,
                resolution_id: None,
            },
        );
        self.active = Some(id);
        Ok(id)
    }

    pub fn state(&self, encounter: EncounterId) -> Option<EncounterStateView> {
        self.encounters
            .get(&encounter)
            .map(|record| EncounterStateView {
                phase: record.phase,
                pending_readiness: record.readiness.len(),
                outcome: record.outcome,
                resolution_id: record.resolution_id,
            })
    }

    pub fn active(&self) -> Option<EncounterId> {
        self.active.filter(|id| {
            self.encounters
                .get(id)
                .is_some_and(|record| !is_terminal(record.phase))
        })
    }

    pub fn update(&mut self, event: EncounterEvent) -> Result<Vec<EncounterCommand>, EngineError> {
        let mut staged = self.clone();
        let result = staged.update_inner(event)?;
        *self = staged;
        Ok(result)
    }

    fn update_inner(
        &mut self,
        event: EncounterEvent,
    ) -> Result<Vec<EncounterCommand>, EngineError> {
        match event {
            EncounterEvent::Poll => {
                let id = self
                    .active
                    .ok_or_else(|| coordinator_error("no encounter can be polled", true))?;
                let mut commands = Vec::new();
                let record = self.record_mut(id)?;
                if record.phase == EncounterPhase::Preparing && record.readiness.is_empty() {
                    begin_transition(record, &mut commands);
                }
                Ok(commands)
            }
            EncounterEvent::ResourceReady {
                encounter,
                resource,
                stage,
                context,
            } => {
                self.validate_external(encounter, context)?;
                let mut commands = Vec::new();
                let record = self.record_mut(encounter)?;
                require_phase(record, EncounterPhase::Preparing)?;
                if record.readiness.complete(&resource, stage)? {
                    begin_transition(record, &mut commands);
                }
                Ok(commands)
            }
            EncounterEvent::ResourceFailed {
                encounter,
                resource,
                stage,
                context,
                code,
            } => {
                self.validate_external(encounter, context)?;
                let record = self.record_mut(encounter)?;
                require_phase(record, EncounterPhase::Preparing)?;
                record.readiness.complete(&resource, stage)?;
                record.phase = EncounterPhase::Failed;
                let mut commands = vec![EncounterCommand::FailEncounter { code }];
                release_cleanup(record, &mut commands);
                Ok(commands)
            }
            EncounterEvent::TransitionCompleted {
                encounter,
                generation,
            } => {
                let mut commands = Vec::new();
                let record = self.record_mut(encounter)?;
                validate_generation(record, generation)?;
                require_phase(record, EncounterPhase::Transitioning)?;
                record.phase = EncounterPhase::Fighting;
                commands.push(EncounterCommand::StartCombat {
                    rules: record.plan.rules.clone(),
                });
                Ok(commands)
            }
            EncounterEvent::CombatFinished {
                encounter,
                generation,
                outcome,
            } => {
                let mut commands = Vec::new();
                let next_resolution = self.next_resolution;
                let record = self.record_mut(encounter)?;
                validate_generation(record, generation)?;
                require_phase(record, EncounterPhase::Fighting)?;
                record.outcome = Some(outcome);
                record.resolution_id = Some(next_resolution);
                record.phase = EncounterPhase::Returning;
                self.next_resolution = self.next_resolution.checked_add(1).ok_or_else(|| {
                    coordinator_error("encounter resolution ID space exhausted", false)
                })?;
                commands.push(EncounterCommand::CommitOutcome {
                    outcome,
                    resolution_id: next_resolution,
                });
                commands.push(EncounterCommand::BeginReturn);
                Ok(commands)
            }
            EncounterEvent::ReturnCompleted {
                encounter,
                generation,
            } => {
                let mut commands = Vec::new();
                let record = self.record_mut(encounter)?;
                validate_generation(record, generation)?;
                require_phase(record, EncounterPhase::Returning)?;
                record.phase = EncounterPhase::Completed;
                release_cleanup(record, &mut commands);
                commands.push(EncounterCommand::CompleteEncounter);
                Ok(commands)
            }
            EncounterEvent::Cancel {
                encounter,
                generation,
                reason,
            } => {
                let record = self.record_mut(encounter)?;
                validate_generation(record, generation)?;
                if matches!(
                    record.phase,
                    EncounterPhase::Returning | EncounterPhase::Completed
                ) {
                    return Err(coordinator_error(
                        "a committed outcome cannot be cancelled as a generic load operation",
                        true,
                    ));
                }
                if matches!(
                    record.phase,
                    EncounterPhase::Cancelled | EncounterPhase::Failed
                ) {
                    return Err(coordinator_error("encounter is already terminal", true));
                }
                record.phase = EncounterPhase::Cancelled;
                let mut commands = vec![EncounterCommand::CancelEncounter { reason }];
                release_cleanup(record, &mut commands);
                Ok(commands)
            }
            EncounterEvent::SceneChanged { encounter } => {
                let record = self.record_mut(encounter)?;
                if matches!(
                    record.phase,
                    EncounterPhase::Returning | EncounterPhase::Completed
                ) {
                    return Err(coordinator_error(
                        "scene change after commitment requires an explicit recovery checkpoint",
                        true,
                    ));
                }
                if matches!(
                    record.phase,
                    EncounterPhase::Cancelled | EncounterPhase::Failed
                ) {
                    return Err(coordinator_error("encounter is already terminal", true));
                }
                record.phase = EncounterPhase::Cancelled;
                let mut commands = vec![EncounterCommand::CancelEncounter {
                    reason: "scene_changed".to_owned(),
                }];
                release_cleanup(record, &mut commands);
                Ok(commands)
            }
        }
    }

    fn validate_external(
        &self,
        encounter: EncounterId,
        context: ExternalEventContext,
    ) -> Result<(), EngineError> {
        let record = self.record(encounter)?;
        if record.plan.session_id != context.session_id
            || record.plan.content_revision != context.content_revision
            || record.plan.generation != context.generation
        {
            return Err(coordinator_error(
                "external encounter event has a stale session, content revision, or generation",
                true,
            ));
        }
        Ok(())
    }

    fn record(&self, encounter: EncounterId) -> Result<&EncounterRecord, EngineError> {
        self.encounters
            .get(&encounter)
            .ok_or_else(|| coordinator_error(format!("unknown encounter {encounter}"), true))
    }

    fn record_mut(&mut self, encounter: EncounterId) -> Result<&mut EncounterRecord, EngineError> {
        self.encounters
            .get_mut(&encounter)
            .ok_or_else(|| coordinator_error(format!("unknown encounter {encounter}"), true))
    }
}

fn validate_plan(plan: &EncounterPlan) -> Result<(), EngineError> {
    if plan.participants.is_empty() {
        return Err(coordinator_error("encounter requires participants", true));
    }
    let mut participants = BTreeSet::new();
    for participant in &plan.participants {
        if !participants.insert(*participant) {
            return Err(coordinator_error("duplicate encounter participant", true));
        }
    }
    if plan.source_to_arena.scale_millionths == 0 {
        return Err(coordinator_error(
            "encounter arena scale must be nonzero",
            true,
        ));
    }
    Ok(())
}

fn begin_transition(record: &mut EncounterRecord, commands: &mut Vec<EncounterCommand>) {
    record.phase = EncounterPhase::Transitioning;
    record.claimed = true;
    record.leases = true;
    record.suspended = record.plan.pause_policy == EncounterPausePolicy::SuspendField;
    commands.push(EncounterCommand::ClaimParticipants {
        participants: record.plan.participants.clone(),
    });
    commands.push(EncounterCommand::AcquireResourceLeases {
        resources: record
            .plan
            .resources
            .iter()
            .map(|resource| resource.resource.clone())
            .collect(),
    });
    commands.push(if record.suspended {
        EncounterCommand::SuspendExploration
    } else {
        EncounterCommand::ContinueExploration
    });
    commands.push(EncounterCommand::BeginTransition {
        presentation: record.plan.presentation.clone(),
        space: record.plan.space,
    });
}

fn release_cleanup(record: &mut EncounterRecord, commands: &mut Vec<EncounterCommand>) {
    if record.claimed {
        commands.push(EncounterCommand::ReleaseParticipants);
        record.claimed = false;
    }
    if record.leases {
        commands.push(EncounterCommand::ReleaseResourceLeases);
        record.leases = false;
    }
    if record.suspended {
        commands.push(EncounterCommand::ResumeExploration);
        record.suspended = false;
    }
}

fn require_phase(record: &EncounterRecord, phase: EncounterPhase) -> Result<(), EngineError> {
    if record.phase != phase {
        return Err(coordinator_error(
            format!("encounter event is invalid during {:?}", record.phase),
            true,
        ));
    }
    Ok(())
}

fn validate_generation(record: &EncounterRecord, generation: u64) -> Result<(), EngineError> {
    if record.plan.generation != generation {
        return Err(coordinator_error(
            "encounter event generation is stale",
            true,
        ));
    }
    Ok(())
}

fn is_terminal(phase: EncounterPhase) -> bool {
    matches!(
        phase,
        EncounterPhase::Completed | EncounterPhase::Cancelled | EncounterPhase::Failed
    )
}

fn coordinator_error(context: impl Into<String>, recoverable: bool) -> EngineError {
    EngineError::new(
        ErrorCategory::Conflict,
        context,
        "tantalus-core/encounter/coordinator",
        recoverable,
    )
}
