use serde::{Deserialize, Serialize};
use tantalus_contracts::ContentId;
use tantalus_core::{EncounterCommand, EncounterSpaceKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncounterPresentationMode {
    Exploration,
    Transitioning,
    Combat,
    Returning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncounterPresentationAction {
    SuspendInput,
    BeginCameraTransition {
        presentation: ContentId,
        space: EncounterSpaceKind,
    },
    ShowCombatUi,
    HideCombatUiAndReturnCamera,
    RestoreInput,
}

#[derive(Debug)]
pub struct EncounterPresenter {
    mode: EncounterPresentationMode,
    input_suspended: bool,
}

impl Default for EncounterPresenter {
    fn default() -> Self {
        Self {
            mode: EncounterPresentationMode::Exploration,
            input_suspended: false,
        }
    }
}

impl EncounterPresenter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mode(&self) -> EncounterPresentationMode {
        self.mode
    }

    pub fn input_suspended(&self) -> bool {
        self.input_suspended
    }

    pub fn apply(&mut self, commands: &[EncounterCommand]) -> Vec<EncounterPresentationAction> {
        let mut actions = Vec::new();
        for command in commands {
            match command {
                EncounterCommand::SuspendExploration => {
                    self.input_suspended = true;
                    actions.push(EncounterPresentationAction::SuspendInput);
                }
                EncounterCommand::BeginTransition {
                    presentation,
                    space,
                } => {
                    self.mode = EncounterPresentationMode::Transitioning;
                    actions.push(EncounterPresentationAction::BeginCameraTransition {
                        presentation: presentation.clone(),
                        space: *space,
                    });
                }
                EncounterCommand::StartCombat { .. } => {
                    self.mode = EncounterPresentationMode::Combat;
                    actions.push(EncounterPresentationAction::ShowCombatUi);
                }
                EncounterCommand::BeginReturn => {
                    self.mode = EncounterPresentationMode::Returning;
                    actions.push(EncounterPresentationAction::HideCombatUiAndReturnCamera);
                }
                EncounterCommand::ResumeExploration => {
                    self.input_suspended = false;
                    actions.push(EncounterPresentationAction::RestoreInput);
                }
                EncounterCommand::CompleteEncounter
                | EncounterCommand::FailEncounter { .. }
                | EncounterCommand::CancelEncounter { .. } => {
                    self.mode = EncounterPresentationMode::Exploration;
                    self.input_suspended = false;
                    actions.push(EncounterPresentationAction::RestoreInput);
                }
                EncounterCommand::ClaimParticipants { .. }
                | EncounterCommand::AcquireResourceLeases { .. }
                | EncounterCommand::ContinueExploration
                | EncounterCommand::CommitOutcome { .. }
                | EncounterCommand::ReleaseParticipants
                | EncounterCommand::ReleaseResourceLeases => {}
            }
        }
        actions
    }
}
