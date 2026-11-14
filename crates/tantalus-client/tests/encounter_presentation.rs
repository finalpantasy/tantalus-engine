use tantalus_client::encounter_presentation::*;
use tantalus_contracts::*;
use tantalus_core::*;

#[test]
fn presentation_enters_and_leaves_combat_without_owning_outcomes() {
    let mut presenter = EncounterPresenter::new();
    let actions = presenter.apply(&[
        EncounterCommand::SuspendExploration,
        EncounterCommand::BeginTransition {
            presentation: ContentId::parse("fixture:presentation/zoom").unwrap(),
            space: EncounterSpaceKind::CurrentScene,
        },
        EncounterCommand::StartCombat {
            rules: ContentId::parse("fixture:rules/atb").unwrap(),
        },
        EncounterCommand::BeginReturn,
        EncounterCommand::ResumeExploration,
        EncounterCommand::CompleteEncounter,
    ]);

    assert_eq!(presenter.mode(), EncounterPresentationMode::Exploration);
    assert!(!presenter.input_suspended());
    assert!(actions.iter().any(|action| matches!(
        action,
        EncounterPresentationAction::BeginCameraTransition { .. }
    )));
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, EncounterPresentationAction::ShowCombatUi))
    );
    assert!(actions.iter().any(|action| matches!(
        action,
        EncounterPresentationAction::HideCombatUiAndReturnCamera
    )));
    assert_eq!(
        actions.last(),
        Some(&EncounterPresentationAction::RestoreInput)
    );
}

#[test]
fn cancelled_encounter_restores_input() {
    let mut presenter = EncounterPresenter::new();
    presenter.apply(&[EncounterCommand::SuspendExploration]);
    presenter.apply(&[EncounterCommand::CancelEncounter {
        reason: "test".to_owned(),
    }]);
    assert!(!presenter.input_suspended());
}
