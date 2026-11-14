use std::collections::BTreeSet;
use std::path::PathBuf;
use tantalus_client::animation_adapter::AnimationAdapter;
use tantalus_client::capture::{CaptureConfig, capture_snapshot, synthetic_snapshot};
use tantalus_client::legacy_compositor::LegacyCompositor;
use tantalus_client::scene_adapter::{ClientSceneAdapter, ResolvedAssets};
use tantalus_contracts::*;

#[test]
fn legacy_compositor_orders_background_before_foreground() {
    let mut snapshot = synthetic_snapshot(ScenePresentationMode::Layered2d);
    snapshot.layers.reverse();

    let ordered = LegacyCompositor::ordered_layers(&snapshot);
    assert_eq!(ordered[0].id.as_str(), "fixture:layer/background");
    assert_eq!(ordered[1].id.as_str(), "fixture:layer/foreground");
}

#[test]
fn duplicate_actor_ids_are_rejected_before_scene_mutation() {
    let mut snapshot = synthetic_snapshot(ScenePresentationMode::Layered2d);
    snapshot.actors.push(snapshot.actors[0].clone());
    let mut adapter = ClientSceneAdapter::new();

    let error = adapter
        .apply(&snapshot, &ResolvedAssets::default())
        .unwrap_err();
    assert!(error.context.contains("duplicate presentation actor"));
}

#[test]
fn invalid_dimensions_are_rejected_before_scene_mutation() {
    let mut snapshot = synthetic_snapshot(ScenePresentationMode::Layered2d);
    snapshot.actors[0].size.x = 0;
    let mut adapter = ClientSceneAdapter::new();

    let error = adapter
        .apply(&snapshot, &ResolvedAssets::default())
        .unwrap_err();
    assert!(error.context.contains("dimensions must be positive"));
}

#[test]
fn animation_mapping_preserves_content_identity_and_start_tick() {
    let clip = ContentId::parse("fixture:animation/idle").unwrap();
    let mapped = AnimationAdapter::map(Some(clip.clone()), Tick::new(42)).unwrap();
    assert_eq!(mapped.clip, clip);
    assert_eq!(mapped.started_at, Tick::new(42));
    assert!(AnimationAdapter::map(None, Tick::new(0)).is_none());
}

#[test]
fn scene_snapshots_are_serializable_without_bevy_types() {
    let snapshot = synthetic_snapshot(ScenePresentationMode::Hybrid);
    let encoded = serde_json::to_string(&snapshot).unwrap();
    let restored: PresentationSnapshot = serde_json::from_str(&encoded).unwrap();
    assert_eq!(restored, snapshot);
    assert!(!encoded.contains("Bevy"));
}

#[test]
#[ignore = "Bevy 0.19.1's own headless reference also produces blank captures on this host"]
fn offscreen_layered_and_3d_scenes_produce_nonempty_images() {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../local/captures");
    std::fs::create_dir_all(&directory).unwrap();

    for (name, mode) in [
        ("synthetic-2d", ScenePresentationMode::Layered2d),
        ("synthetic-3d", ScenePresentationMode::Full3d),
    ] {
        let output = directory.join(format!("{name}.png"));
        capture_snapshot(&synthetic_snapshot(mode), &output, CaptureConfig::default()).unwrap();

        let bytes = std::fs::read(&output).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        let image = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(image.width(), 320);
        assert_eq!(image.height(), 180);
        let colors: BTreeSet<_> = image.pixels().map(|pixel| pixel.0).collect();
        assert!(colors.len() > 2, "{name} produced a blank or flat image");
    }
}
