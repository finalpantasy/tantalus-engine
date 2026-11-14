use tantalus_contracts::*;
use tantalus_core::*;

fn id(text: &str) -> ContentId {
    ContentId::parse(text).unwrap()
}

fn plan(space: EncounterSpaceKind, pause: EncounterPausePolicy) -> EncounterPlan {
    EncounterPlan {
        source_anchor: Point3 {
            x: 1_000,
            y: 0,
            z: 2_000,
        },
        rules: id("fixture:rules/atb"),
        space,
        presentation: id("fixture:presentation/camera_zoom"),
        pause_policy: pause,
        participants: vec![EntityId::new(1), EntityId::new(2)],
        resources: vec![ResourceRequirement {
            resource: id("fixture:arena/opening"),
            stages: vec![
                ReadinessStage::SourceDecoded,
                ReadinessStage::ReferencesResolved,
                ReadinessStage::GpuUploaded,
                ReadinessStage::AudioReady,
            ],
        }],
        source_to_arena: SpaceTransform::identity(),
        session_id: 11,
        content_revision: ContentRevision::from_u64(7),
        generation: 3,
    }
}

fn context() -> ExternalEventContext {
    ExternalEventContext {
        session_id: 11,
        content_revision: ContentRevision::from_u64(7),
        generation: 3,
    }
}

fn ready(encounter: EncounterId, stage: ReadinessStage) -> EncounterEvent {
    EncounterEvent::ResourceReady {
        encounter,
        resource: id("fixture:arena/opening"),
        stage,
        context: context(),
    }
}

fn finish_ready(coordinator: &mut EncounterCoordinator, encounter: EncounterId) {
    for stage in [
        ReadinessStage::SourceDecoded,
        ReadinessStage::ReferencesResolved,
        ReadinessStage::GpuUploaded,
        ReadinessStage::AudioReady,
    ] {
        coordinator.update(ready(encounter, stage)).unwrap();
    }
}

fn drive_to_returning(coordinator: &mut EncounterCoordinator, encounter: EncounterId) {
    finish_ready(coordinator, encounter);
    coordinator
        .update(EncounterEvent::TransitionCompleted {
            encounter,
            generation: 3,
        })
        .unwrap();
    coordinator
        .update(EncounterEvent::CombatFinished {
            encounter,
            generation: 3,
            outcome: EncounterOutcome::Victory,
        })
        .unwrap();
}

#[test]
fn classic_lifecycle_commits_and_returns_exactly_once() {
    let mut coordinator = EncounterCoordinator::new();
    let encounter = coordinator
        .request(plan(
            EncounterSpaceKind::OriginalArena,
            EncounterPausePolicy::SuspendField,
        ))
        .unwrap();
    assert_eq!(coordinator.state(encounter).unwrap().pending_readiness, 4);

    let commands = finish_and_collect(&mut coordinator, encounter);
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, EncounterCommand::ClaimParticipants { .. }))
    );
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, EncounterCommand::SuspendExploration))
    );
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, EncounterCommand::BeginTransition { .. }))
    );

    coordinator
        .update(EncounterEvent::TransitionCompleted {
            encounter,
            generation: 3,
        })
        .unwrap();
    let outcome = coordinator
        .update(EncounterEvent::CombatFinished {
            encounter,
            generation: 3,
            outcome: EncounterOutcome::Victory,
        })
        .unwrap();
    assert!(outcome.contains(&EncounterCommand::CommitOutcome {
        outcome: EncounterOutcome::Victory,
        resolution_id: 1,
    }));
    assert_eq!(
        coordinator.state(encounter).unwrap().phase,
        EncounterPhase::Returning
    );

    assert!(
        coordinator
            .update(EncounterEvent::CombatFinished {
                encounter,
                generation: 3,
                outcome: EncounterOutcome::Victory,
            })
            .is_err()
    );
    assert_eq!(coordinator.state(encounter).unwrap().resolution_id, Some(1));

    let returned = coordinator
        .update(EncounterEvent::ReturnCompleted {
            encounter,
            generation: 3,
        })
        .unwrap();
    assert!(returned.contains(&EncounterCommand::ReleaseParticipants));
    assert!(returned.contains(&EncounterCommand::ReleaseResourceLeases));
    assert!(returned.contains(&EncounterCommand::ResumeExploration));
    assert_eq!(
        coordinator.state(encounter).unwrap().phase,
        EncounterPhase::Completed
    );
    assert_eq!(coordinator.active(), None);
}

fn finish_and_collect(
    coordinator: &mut EncounterCoordinator,
    encounter: EncounterId,
) -> Vec<EncounterCommand> {
    let mut commands = Vec::new();
    for stage in [
        ReadinessStage::SourceDecoded,
        ReadinessStage::ReferencesResolved,
        ReadinessStage::GpuUploaded,
        ReadinessStage::AudioReady,
    ] {
        commands.extend(coordinator.update(ready(encounter, stage)).unwrap());
    }
    commands
}

#[test]
fn seamless_and_prepared_paths_preserve_the_same_combat_outcome() {
    for space in [
        EncounterSpaceKind::CurrentScene,
        EncounterSpaceKind::PreparedArena,
    ] {
        let mut coordinator = EncounterCoordinator::new();
        let encounter = coordinator
            .request(plan(space, EncounterPausePolicy::ContinueWorld))
            .unwrap();
        drive_to_returning(&mut coordinator, encounter);
        assert_eq!(
            coordinator.state(encounter).unwrap().outcome,
            Some(EncounterOutcome::Victory)
        );
        assert_eq!(coordinator.state(encounter).unwrap().resolution_id, Some(1));
        coordinator
            .update(EncounterEvent::ReturnCompleted {
                encounter,
                generation: 3,
            })
            .unwrap();
        assert_eq!(
            coordinator.state(encounter).unwrap().phase,
            EncounterPhase::Completed
        );
    }
}

#[test]
fn current_scene_without_external_assets_can_begin_on_poll() {
    let mut coordinator = EncounterCoordinator::new();
    let mut current = plan(
        EncounterSpaceKind::CurrentScene,
        EncounterPausePolicy::ContinueWorld,
    );
    current.resources.clear();
    let encounter = coordinator.request(current).unwrap();

    let commands = coordinator.update(EncounterEvent::Poll).unwrap();
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, EncounterCommand::BeginTransition { .. }))
    );
    assert_eq!(
        coordinator.state(encounter).unwrap().phase,
        EncounterPhase::Transitioning
    );
}

#[test]
fn cancellation_rejects_late_stale_completions() {
    let mut coordinator = EncounterCoordinator::new();
    let encounter = coordinator
        .request(plan(
            EncounterSpaceKind::PreparedArena,
            EncounterPausePolicy::SuspendField,
        ))
        .unwrap();
    let cancelled = coordinator
        .update(EncounterEvent::Cancel {
            encounter,
            generation: 3,
            reason: "test_cancel".to_owned(),
        })
        .unwrap();
    assert!(
        cancelled
            .iter()
            .any(|command| matches!(command, EncounterCommand::CancelEncounter { .. }))
    );

    assert!(
        coordinator
            .update(ready(encounter, ReadinessStage::SourceDecoded))
            .is_err()
    );
    assert_eq!(
        coordinator.state(encounter).unwrap().phase,
        EncounterPhase::Cancelled
    );
}

#[test]
fn scene_edit_during_loading_cannot_resurrect_the_encounter() {
    let mut coordinator = EncounterCoordinator::new();
    let encounter = coordinator
        .request(plan(
            EncounterSpaceKind::CurrentScene,
            EncounterPausePolicy::SuspendField,
        ))
        .unwrap();
    coordinator
        .update(EncounterEvent::SceneChanged { encounter })
        .unwrap();

    assert!(
        coordinator
            .update(EncounterEvent::TransitionCompleted {
                encounter,
                generation: 3
            })
            .is_err()
    );
    assert_eq!(coordinator.active(), None);
}

#[test]
fn failed_resource_cleans_up_without_an_outcome_commit() {
    let mut coordinator = EncounterCoordinator::new();
    let encounter = coordinator
        .request(plan(
            EncounterSpaceKind::PreparedArena,
            EncounterPausePolicy::SuspendField,
        ))
        .unwrap();
    let commands = coordinator
        .update(EncounterEvent::ResourceFailed {
            encounter,
            resource: id("fixture:arena/opening"),
            stage: ReadinessStage::SourceDecoded,
            context: context(),
            code: "missing_mesh".to_owned(),
        })
        .unwrap();

    assert!(commands.iter().any(|command| matches!(command, EncounterCommand::FailEncounter { code } if code == "missing_mesh")));
    assert_eq!(
        coordinator.state(encounter).unwrap().phase,
        EncounterPhase::Failed
    );
    assert_eq!(coordinator.state(encounter).unwrap().resolution_id, None);
}

#[test]
fn committed_outcome_requires_the_return_path_not_generic_cancellation() {
    let mut coordinator = EncounterCoordinator::new();
    let encounter = coordinator
        .request(plan(
            EncounterSpaceKind::CurrentScene,
            EncounterPausePolicy::SuspendField,
        ))
        .unwrap();
    drive_to_returning(&mut coordinator, encounter);

    assert!(
        coordinator
            .update(EncounterEvent::Cancel {
                encounter,
                generation: 3,
                reason: "late_cancel".to_owned(),
            })
            .is_err()
    );
    assert_eq!(
        coordinator.state(encounter).unwrap().phase,
        EncounterPhase::Returning
    );
    coordinator
        .update(EncounterEvent::ReturnCompleted {
            encounter,
            generation: 3,
        })
        .unwrap();
    assert_eq!(
        coordinator.state(encounter).unwrap().phase,
        EncounterPhase::Completed
    );
}

#[test]
fn duplicate_or_stale_readiness_is_rejected_atomically() {
    let mut coordinator = EncounterCoordinator::new();
    let encounter = coordinator
        .request(plan(
            EncounterSpaceKind::PreparedArena,
            EncounterPausePolicy::SuspendField,
        ))
        .unwrap();
    coordinator
        .update(ready(encounter, ReadinessStage::SourceDecoded))
        .unwrap();
    let before = coordinator.state(encounter).unwrap();

    assert!(
        coordinator
            .update(ready(encounter, ReadinessStage::SourceDecoded))
            .is_err()
    );
    assert_eq!(coordinator.state(encounter).unwrap(), before);
}

#[test]
fn arena_transform_maps_scale_and_translation_with_checked_ranges() {
    let transform = SpaceTransform {
        scale_millionths: 500_000,
        translation: Point3 {
            x: 100,
            y: -50,
            z: 0,
        },
    };
    assert_eq!(
        transform
            .map(Point3 {
                x: 2_000,
                y: 1_000,
                z: -2_000
            })
            .unwrap(),
        Point3 {
            x: 1_100,
            y: 450,
            z: -1_000
        }
    );
    assert!(
        SpaceTransform {
            scale_millionths: 0,
            ..transform
        }
        .map(Point3::default())
        .is_err()
    );
}

#[test]
fn invalid_participants_and_active_conflicts_are_rejected() {
    let mut coordinator = EncounterCoordinator::new();
    let mut invalid = plan(
        EncounterSpaceKind::CurrentScene,
        EncounterPausePolicy::SuspendField,
    );
    invalid.participants.push(invalid.participants[0]);
    assert!(coordinator.request(invalid).is_err());

    coordinator
        .request(plan(
            EncounterSpaceKind::CurrentScene,
            EncounterPausePolicy::SuspendField,
        ))
        .unwrap();
    assert!(
        coordinator
            .request(plan(
                EncounterSpaceKind::CurrentScene,
                EncounterPausePolicy::SuspendField
            ))
            .is_err()
    );
}
