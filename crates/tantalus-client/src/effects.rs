use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use tantalus_contracts::{ActionMarkerKind, ContentId, EngineError, ErrorCategory, SemanticMarker};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationEffectCommand {
    StartEffect {
        action: ContentId,
        effect: ContentId,
    },
    PlayAudio {
        action: ContentId,
        cue: ContentId,
    },
    Hit {
        action: ContentId,
    },
    Complete {
        action: ContentId,
    },
}

#[derive(Debug, Default)]
pub struct EffectPresenter {
    active_actions: BTreeSet<ContentId>,
}

impl EffectPresenter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply(
        &mut self,
        markers: &[SemanticMarker],
    ) -> Result<Vec<PresentationEffectCommand>, EngineError> {
        let mut commands = Vec::new();
        for marker in markers {
            let command = match &marker.kind {
                ActionMarkerKind::Effect(effect) => {
                    if !self.active_actions.insert(marker.action.clone()) {
                        return Err(effect_error("effect action is already active"));
                    }
                    PresentationEffectCommand::StartEffect {
                        action: marker.action.clone(),
                        effect: effect.clone(),
                    }
                }
                ActionMarkerKind::Audio(cue) => PresentationEffectCommand::PlayAudio {
                    action: marker.action.clone(),
                    cue: cue.clone(),
                },
                ActionMarkerKind::Hit => PresentationEffectCommand::Hit {
                    action: marker.action.clone(),
                },
                ActionMarkerKind::Complete => {
                    if !self.active_actions.remove(&marker.action) {
                        return Err(effect_error("effect action completed without starting"));
                    }
                    PresentationEffectCommand::Complete {
                        action: marker.action.clone(),
                    }
                }
            };
            commands.push(command);
        }
        Ok(commands)
    }

    pub fn active_count(&self) -> usize {
        self.active_actions.len()
    }
}

fn effect_error(context: &str) -> EngineError {
    EngineError::new(
        ErrorCategory::Conflict,
        context,
        "tantalus-client/effects",
        true,
    )
}
