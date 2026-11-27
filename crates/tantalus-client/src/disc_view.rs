//! The disc-change picture (`SceneDirector` with `DisplayPSXDiscChanges`): Memoria's
//! `changetodisc<N>.png` over the whole screen while [`PlayGame::disc_picture`] has one.
//!
//! `SceneDirector.cs` draws it `Screen.height / 0.7` wide and the screen's height, centered,
//! or over the whole screen when the picture is wider than that, fading in over 100 frames,
//! held until a button is pressed and fading out over 100 frames.

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use tantalus_tools::play::PlayGame;

use crate::dialog_view::{FRAME, WINDOW_Z};

/// What the disc view has on screen.
#[derive(Default)]
pub struct DiscView {
    /// Its own black camera while the disc change lasts: the field before it may have no
    /// camera (a field without a background, as at the end of disc 1), and the save screen
    /// draws on the topmost camera.
    camera: Option<Entity>,
    root: Option<Entity>,
    sprite: Option<Entity>,
    path: Option<std::path::PathBuf>,
}

/// The picture's size in frame pixels for a `width` x `height` image (`SceneDirector.cs`).
pub fn picture_size(width: u32, height: u32) -> Vec2 {
    if width == 0 || (height as f32 / width as f32) < 0.698 {
        Vec2::new(
            FRAME.y * width.max(1) as f32 / height.max(1) as f32,
            FRAME.y,
        )
    } else {
        Vec2::new(FRAME.y / 0.7, FRAME.y)
    }
}

fn clear(world: &mut World, view: &mut DiscView) {
    if let Some(root) = view.root.take()
        && let Ok(entity) = world.get_entity_mut(root)
    {
        entity.despawn();
    }
    view.sprite = None;
    view.path = None;
}

/// The disc change's render layer and camera order: over the field's and the world map's
/// cameras, under the game over and title cameras.
const DISC_LAYER: usize = 27;
const DISC_ORDER: isize = 18;

/// A black camera over the whole window for the disc change.
fn open_camera(world: &mut World) -> Entity {
    // A recording window renders to an image; draw where the other cameras do.
    let mut targets = world.query_filtered::<&RenderTarget, With<Camera2d>>();
    let target = targets.iter(world).next().cloned();
    let camera = world
        .spawn((
            Camera2d,
            Camera {
                order: DISC_ORDER,
                clear_color: ClearColorConfig::Custom(Color::BLACK),
                ..default()
            },
            Transform::default(),
            RenderLayers::layer(DISC_LAYER),
        ))
        .id();
    if let Some(target) = target {
        world.entity_mut(camera).insert(target);
    }
    camera
}

/// Draws the disc picture of `game`, or removes it.
pub fn sync_disc(world: &mut World, game: &PlayGame) {
    let mut view = world.remove_non_send::<DiscView>().unwrap_or_default();
    if game.disc_change().is_none() {
        clear(world, &mut view);
        if let Some(camera) = view.camera.take()
            && let Ok(entity) = world.get_entity_mut(camera)
        {
            entity.despawn();
        }
        world.insert_non_send(view);
        return;
    }
    let camera = *view.camera.get_or_insert_with(|| open_camera(world));
    let Some((path, alpha)) = game.disc_picture() else {
        clear(world, &mut view);
        world.insert_non_send(view);
        return;
    };
    if view.path.as_ref() != Some(&path) {
        clear(world, &mut view);
        let layers = RenderLayers::layer(DISC_LAYER);
        match image::open(&path) {
            Ok(picture) => {
                let picture = picture.to_rgba8();
                let size = picture_size(picture.width(), picture.height());
                let image = Image::new(
                    Extent3d {
                        width: picture.width(),
                        height: picture.height(),
                        depth_or_array_layers: 1,
                    },
                    TextureDimension::D2,
                    picture.into_raw(),
                    TextureFormat::Rgba8UnormSrgb,
                    RenderAssetUsages::default(),
                );
                let handle = world.resource_mut::<Assets<Image>>().add(image);
                let root = world
                    .spawn((
                        Transform::from_xyz(0.0, 0.0, WINDOW_Z + 0.5),
                        Visibility::default(),
                        ChildOf(camera),
                    ))
                    .id();
                // Black behind the picture, as the screen is between scenes.
                world.spawn((
                    Sprite::from_color(Color::BLACK, Vec2::new(FRAME.x * 4.0, FRAME.y * 2.0)),
                    Transform::default(),
                    layers.clone(),
                    ChildOf(root),
                ));
                let sprite = world
                    .spawn((
                        Sprite {
                            image: handle,
                            custom_size: Some(size),
                            ..default()
                        },
                        Transform::from_xyz(0.0, 0.0, 0.1),
                        layers,
                        ChildOf(root),
                    ))
                    .id();
                view.root = Some(root);
                view.sprite = Some(sprite);
            }
            Err(error) => println!("!! disc picture {}: {error}", path.display()),
        }
        view.path = Some(path);
    }
    if let Some(sprite) = view.sprite
        && let Some(mut sprite) = world.get_mut::<Sprite>(sprite)
    {
        sprite.color = Color::srgba(1.0, 1.0, 1.0, alpha);
    }
    world.insert_non_send(view);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_disc_picture_is_the_screen_height_and_a_fixed_width() {
        // `Screen.height / 0.7` wide unless the picture is wider than that.
        let size = picture_size(1440, 1080);
        assert_eq!(size.y, FRAME.y);
        assert!((size.x - FRAME.y / 0.7).abs() < 1e-3);
        let wide = picture_size(1920, 1080);
        assert!((wide.x - FRAME.y * 1920.0 / 1080.0).abs() < 1e-3);
    }
}
