use bevy::prelude::*;
use tantalus_client::scene_adapter::{ClientSceneAdapter, FieldSpriteIdentity, ResolvedAssets};
use tantalus_contracts::*;

fn sprite(
    overlay: u32,
    index: u32,
    atlas: &ContentId,
    depth: u32,
    active: bool,
) -> FieldSpritePresentation {
    FieldSpritePresentation {
        overlay_index: overlay,
        sprite_index: index,
        atlas: atlas.clone(),
        atlas_rect: [10, 20, 26, 28],
        pixel_size: [16, 8],
        pixel_offset: [100, 50],
        depth,
        alpha: 0,
        transparency: 0,
        active,
        screen_anchored: false,
    }
}

fn field(atlas: &ContentId) -> FieldSceneDescription {
    FieldSceneDescription {
        camera_index: 0,
        camera: LegacyFieldCamera {
            projection: 529,
            rotation: [0; 9],
            translation: Point3::default(),
            center_offset: [0; 2],
            size: [320, 224],
            viewport_x: [0; 2],
            viewport_y: [0; 2],
            depth_offset: 0,
        },
        actor_anchor: Point3::default(),
        navigation: FieldNavigationMesh {
            vertices: Vec::new(),
            triangles: Vec::new(),
            active_floor: 0,
            active_triangle: 0,
            actor_anchor: Point3::default(),
        },
        overlays: vec![FieldOverlayPresentation {
            overlay_index: 0,
            camera_index: 0,
            flags: 2,
            pixel_size: [16, 16],
            current_offset: [0; 2],
            sprites: vec![
                sprite(0, 0, atlas, 20, true),
                sprite(0, 1, atlas, 10, true),
                sprite(0, 2, atlas, 0, false),
            ],
        }],
        scene_offset: [0; 2],
        cameras: Vec::new(),
    }
}

fn resolved(adapter: &mut ClientSceneAdapter, atlas: &ContentId) -> ResolvedAssets {
    adapter.app_mut().init_resource::<Assets<Image>>();
    let image = adapter
        .app_mut()
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(Image::default());
    ResolvedAssets {
        content_revision: ContentRevision::default(),
        images: [(atlas.clone(), image)].into(),
    }
}

#[test]
fn field_adapter_spawns_active_textured_sprites_in_depth_order() {
    let atlas = ContentId::parse("fixture:texture/atlas").unwrap();
    let mut adapter = ClientSceneAdapter::new();
    let assets = resolved(&mut adapter, &atlas);
    adapter.apply_field(&field(&atlas), &assets).unwrap();

    let mut sprites = adapter
        .app_mut()
        .world_mut()
        .query::<(&FieldSpriteIdentity, &Sprite, &Transform, &Visibility)>()
        .iter(adapter.app().world())
        .filter(|(.., visibility)| **visibility != Visibility::Hidden)
        .collect::<Vec<_>>();
    // Farther sprites (larger depth) get lower z and draw first.
    sprites.sort_by(|a, b| a.2.translation.z.total_cmp(&b.2.translation.z));
    // The inactive sprite is spawned hidden, for `BGLACTIVE` to show.
    assert_eq!(sprites.len(), 2);
    assert_eq!(sprites[0].0.sprite, 0);
    assert_eq!(sprites[1].0.sprite, 1);
    assert_eq!(
        sprites[1].1.rect.unwrap(),
        Rect::new(10.0, 20.0, 26.0, 28.0)
    );
    assert_eq!(sprites[1].1.custom_size, Some(Vec2::new(16.0, 8.0)));
    assert_eq!(sprites[1].2.translation.x, 108.0);
    assert_eq!(sprites[1].2.translation.y, -54.0);

    let mut cameras = adapter
        .app_mut()
        .world_mut()
        .query::<(&Camera2d, &Transform)>();
    let (_, camera) = cameras.iter(adapter.app().world()).next().unwrap();
    assert_eq!(camera.translation.x, 0.0);
    assert_eq!(camera.translation.y, 0.0);
}

#[test]
fn missing_atlas_does_not_clear_the_previous_field() {
    let atlas = ContentId::parse("fixture:texture/atlas").unwrap();
    let mut adapter = ClientSceneAdapter::new();
    let assets = resolved(&mut adapter, &atlas);
    adapter.apply_field(&field(&atlas), &assets).unwrap();

    let missing = ContentId::parse("fixture:texture/missing").unwrap();
    let error = adapter.apply_field(&field(&missing), &assets).unwrap_err();
    assert!(error.context.contains("not resolved"));
    assert_eq!(
        adapter
            .app_mut()
            .world_mut()
            .query::<(&FieldSpriteIdentity, &Visibility)>()
            .iter(adapter.app().world())
            .filter(|(_, visibility)| **visibility != Visibility::Hidden)
            .count(),
        2
    );
}

#[test]
fn atlas_resolution_does_not_change_native_sprite_placement() {
    let atlas = ContentId::parse("fixture:texture/atlas").unwrap();
    let mut adapter = ClientSceneAdapter::new();
    let assets = resolved(&mut adapter, &atlas);
    let mut scene = field(&atlas);
    scene.overlays[0].sprites[0].atlas_rect = [10, 20, 74, 84];
    adapter.apply_field(&scene, &assets).unwrap();

    let mut sprites = adapter
        .app_mut()
        .world_mut()
        .query::<(&FieldSpriteIdentity, &Transform)>();
    let (_, transform) = sprites
        .iter(adapter.app().world())
        .find(|(identity, _)| identity.sprite == 0)
        .unwrap();
    assert_eq!(transform.translation.x, 108.0);
    assert_eq!(transform.translation.y, -54.0);
}

#[test]
fn inactive_cameras_do_not_contribute_field_sprites() {
    let atlas = ContentId::parse("fixture:texture/atlas").unwrap();
    let mut adapter = ClientSceneAdapter::new();
    let assets = resolved(&mut adapter, &atlas);
    let mut scene = field(&atlas);
    scene.overlays.push(FieldOverlayPresentation {
        overlay_index: 1,
        camera_index: 1,
        flags: 2,
        pixel_size: [16, 16],
        current_offset: [0; 2],
        sprites: vec![sprite(1, 0, &atlas, 0, true)],
    });
    adapter.apply_field(&scene, &assets).unwrap();

    assert_eq!(
        adapter
            .app_mut()
            .world_mut()
            .query::<(&FieldSpriteIdentity, &Visibility)>()
            .iter(adapter.app().world())
            .filter(|(_, visibility)| **visibility != Visibility::Hidden)
            .count(),
        2
    );
}

#[test]
fn an_attached_tile_block_follows_the_player_aim_point() {
    use tantalus_game::field::world::OverlayChange;
    let atlas = ContentId::parse("fixture:texture/atlas").unwrap();
    let mut scene = field(&atlas);
    scene.camera.rotation = [4096, 0, 0, 0, 4096, 0, 0, 0, 4096];
    scene.camera.translation = Point3 {
        x: 0,
        y: 0,
        z: 1000,
    };
    scene.camera.projection = 1000;
    scene.camera.viewport_x = [160, 320];
    scene.camera.viewport_y = [112, 208];
    let changes = [(
        0u16,
        OverlayChange {
            attached: Some([240, 136]),
            ..OverlayChange::default()
        },
    )]
    .into();
    // Aimed 324 above [10, -324, 0], the player projects to (10, 0).
    let placements = tantalus_client::scene_adapter::overlay_placements(
        &scene,
        &changes,
        Some([10.0, -324.0, 0.0]),
    );
    // `BgAttachService`: 10 - 0 - 240 + 160 and -0 - 0 - 136 + 112.
    assert_eq!(placements[&0], (None, [-70.0, -24.0]));
    // With no player the block stays put.
    let still = tantalus_client::scene_adapter::overlay_placements(&scene, &changes, None);
    assert_eq!(still[&0], (None, [0.0, 0.0]));
}
