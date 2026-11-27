//! The game over screen of a field script's `GAMEOVER` ([`PlayGame::game_over_alpha`]):
//! `GameOverUI`'s picture (the PS1 one in the PS1 skin, [`crate::ui_skin::game_over_picture`])
//! over black, the screen's height at its own shape, centered, fading in and out with the
//! screen. A battle's game over draws in the battle HUD instead.

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use tantalus_tools::play::PlayGame;

/// Its own render layer and camera, over the field's (and under the title's, which follows it).
const GAME_OVER_LAYER: usize = 28;
const GAME_OVER_ORDER: isize = 19;

/// What the game over view has on screen.
#[derive(Default)]
pub struct GameOverView {
    camera: Option<Entity>,
    sprite: Option<Entity>,
    /// The picture, read once, and its size in pixels.
    picture: Option<Option<crate::title_view::Picture>>,
}

/// Draws the game over picture while the screen is up, or removes it.
pub fn sync_game_over(world: &mut World, game: &PlayGame) {
    let mut view = world.remove_non_send::<GameOverView>().unwrap_or_default();
    let Some(alpha) = game.game_over_alpha() else {
        if let Some(camera) = view.camera.take()
            && let Ok(entity) = world.get_entity_mut(camera)
        {
            entity.despawn();
        }
        view.sprite = None;
        world.insert_non_send(view);
        return;
    };
    if view.picture.is_none() {
        let picture = crate::ui_skin::game_over_picture(game.game_data()).map(|picture| {
            let size = (picture.width as u32, picture.height as u32);
            let image = Image::new(
                Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                picture.rgba,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::default(),
            );
            (world.resource_mut::<Assets<Image>>().add(image), size)
        });
        view.picture = Some(picture);
    }
    let size = match &view.picture {
        Some(Some((_, size))) => *size,
        _ => (320, 224),
    };
    if view.camera.is_none() {
        // A recording window renders to an image; draw where the other cameras do.
        let mut targets = world.query_filtered::<&RenderTarget, With<Camera2d>>();
        let target = targets.iter(world).next().cloned();
        let layers = RenderLayers::layer(GAME_OVER_LAYER);
        let camera = world
            .spawn((
                Camera2d,
                Camera {
                    order: GAME_OVER_ORDER,
                    clear_color: ClearColorConfig::Custom(Color::BLACK),
                    ..default()
                },
                Transform::default(),
                layers.clone(),
            ))
            .id();
        if let Some(target) = target {
            world.entity_mut(camera).insert(target);
        }
        if let Some(Some((handle, _))) = &view.picture {
            let sprite = world
                .spawn((
                    Sprite {
                        image: handle.clone(),
                        custom_size: Some(crate::title_view::picture_size(size.0, size.1)),
                        ..default()
                    },
                    Transform::default(),
                    layers,
                    ChildOf(camera),
                ))
                .id();
            view.sprite = Some(sprite);
        }
        view.camera = Some(camera);
    }
    if let Some(sprite) = view.sprite
        && let Some(mut sprite) = world.get_mut::<Sprite>(sprite)
    {
        sprite.color = Color::srgba(1.0, 1.0, 1.0, alpha);
    }
    if let Some(camera) = view.camera {
        crate::window::fit_frame_camera(world, camera, size);
    }
    world.insert_non_send(view);
}
