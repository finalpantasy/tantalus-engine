use tantalus_client::effects::{EffectPresenter, PresentationEffectCommand};
use tantalus_contracts::*;

fn marker(tick: u64, kind: ActionMarkerKind) -> SemanticMarker {
    SemanticMarker {
        tick: Tick::new(tick),
        action: ContentId::parse("fixture:attack/ruin").unwrap(),
        kind,
    }
}

#[test]
fn presentation_consumes_effect_audio_hit_and_cleanup_markers() {
    let mut presenter = EffectPresenter::new();
    let commands = presenter
        .apply(&[
            marker(
                1,
                ActionMarkerKind::Effect(ContentId::parse("fixture:effect/ruin").unwrap()),
            ),
            marker(
                2,
                ActionMarkerKind::Audio(ContentId::parse("fixture:audio/cast").unwrap()),
            ),
            marker(3, ActionMarkerKind::Hit),
            marker(4, ActionMarkerKind::Hit),
            marker(6, ActionMarkerKind::Complete),
        ])
        .unwrap();

    assert_eq!(commands.len(), 5);
    assert!(matches!(
        commands[0],
        PresentationEffectCommand::StartEffect { .. }
    ));
    assert!(matches!(
        commands[1],
        PresentationEffectCommand::PlayAudio { .. }
    ));
    assert_eq!(presenter.active_count(), 0);
}

#[test]
fn completion_without_start_is_an_explicit_error() {
    let mut presenter = EffectPresenter::new();
    let error = presenter
        .apply(&[marker(1, ActionMarkerKind::Complete)])
        .unwrap_err();
    assert!(error.context.contains("without starting"));
}
