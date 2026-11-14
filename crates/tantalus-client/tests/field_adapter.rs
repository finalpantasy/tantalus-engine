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
            viewport_x: [0; 2],
            viewport_y: [0; 2],
            depth_offset: 0,
        },
        actor_anchor: Point3::default(),
        navigation_triangles: 0,
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
        .query::<(&FieldSpriteIdentity, &Sprite, &Transform)>()
        .iter(adapter.app().world())
        .collect::<Vec<_>>();
    sprites.sort_by_key(|(_, _, transform)| transform.translation.z as i32);
    assert_eq!(sprites.len(), 2);
    assert_eq!(sprites[0].0.sprite, 1);
    assert_eq!(sprites[1].0.sprite, 0);
    assert_eq!(
        sprites[0].1.rect.unwrap(),
        Rect::new(10.0, 20.0, 26.0, 28.0)
    );
    assert_eq!(sprites[0].2.translation.x, 0.1);
    assert_eq!(sprites[0].2.translation.y, -0.05);
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
            .query::<&FieldSpriteIdentity>()
            .iter(adapter.app().world())
            .count(),
        2
    );
}
