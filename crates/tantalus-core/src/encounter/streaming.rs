use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tantalus_contracts::{ContentId, EngineError, ErrorCategory};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessStage {
    SourceDecoded,
    ReferencesResolved,
    RigsPrepared,
    GpuUploaded,
    PipelinesPrepared,
    AudioReady,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceRequirement {
    pub resource: ContentId,
    pub stages: Vec<ReadinessStage>,
}

#[derive(Debug, Clone, Default)]
pub struct ReadinessTracker {
    pending: BTreeMap<(ContentId, ReadinessStage), ()>,
}

impl ReadinessTracker {
    pub fn new(requirements: &[ResourceRequirement]) -> Result<Self, EngineError> {
        let mut tracker = Self::default();
        for requirement in requirements {
            if requirement.stages.is_empty() {
                return Err(streaming_error(format!(
                    "resource `{}` has no required readiness stages",
                    requirement.resource
                )));
            }
            for stage in &requirement.stages {
                if tracker
                    .pending
                    .insert((requirement.resource.clone(), *stage), ())
                    .is_some()
                {
                    return Err(streaming_error(format!(
                        "resource `{}` duplicates readiness stage {:?}",
                        requirement.resource, stage
                    )));
                }
            }
        }
        Ok(tracker)
    }

    pub fn complete(
        &mut self,
        resource: &ContentId,
        stage: ReadinessStage,
    ) -> Result<bool, EngineError> {
        if self.pending.remove(&(resource.clone(), stage)).is_none() {
            return Err(streaming_error(format!(
                "readiness stage {:?} for `{}` is stale, duplicated, or undeclared",
                stage, resource
            )));
        }
        Ok(self.pending.is_empty())
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }
}

fn streaming_error(context: impl Into<String>) -> EngineError {
    EngineError::new(
        ErrorCategory::Conflict,
        context,
        "tantalus-core/encounter/streaming",
        true,
    )
}
