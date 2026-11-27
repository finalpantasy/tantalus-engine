//! The title screen's picture (`TitleUI.MenuPanelObject`) behind its menu while
//! [`PlayGame::title_open`]: the PS1 title (resident file 60) in the PS1 skin, the Steam
//! `title_bg` and `title_logo` in the Steam skin ([`crate::ui_skin::title_picture`]).
//!
//! The picture fills the screen's height at its own shape, centered, on the title's own camera;
//! the menu window ([`crate::menu_view`]) draws over it on that camera.

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use tantalus_tools::play::PlayGame;

use crate::dialog_view::FRAME;

/// The title's render layer and camera order: its own camera, over the field, world map and
/// battle cameras, so the title shows with no field behind it (the first launch's field 70 has
/// no background and so no field camera).
const TITLE_LAYER: usize = 29;
const TITLE_ORDER: isize = 20;

/// The title picture's image and its size in pixels.
pub(crate) type Picture = (Handle<Image>, (u32, u32));

/// What the title view has on screen.
#[derive(Default)]
pub struct TitleView {
    camera: Option<Entity>,
    /// The picture, read once, and its size in pixels.
    picture: Option<Option<Picture>>,
}

/// The picture's size in frame pixels for a `width` x `height` image: the screen's height and
/// the picture's own aspect.
pub fn picture_size(width: u32, height: u32) -> Vec2 {
    Vec2::new(
        FRAME.y * width.max(1) as f32 / height.max(1) as f32,
        FRAME.y,
    )
}

/// Draws the title picture while the title is up, or removes it. Call before
/// [`crate::menu_view::sync_menu`] so the menu draws on the title's camera.
pub fn sync_title(world: &mut World, game: &PlayGame) {
    let mut view = world.remove_non_send::<TitleView>().unwrap_or_default();
    if !game.title_open() {
        if let Some(camera) = view.camera.take()
            && let Ok(entity) = world.get_entity_mut(camera)
        {
            entity.despawn();
        }
        world.insert_non_send(view);
        return;
    }
    if view.picture.is_none() {
        let picture = crate::ui_skin::title_picture(game.game_data()).map(|picture| {
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
    // The PS1 screen's shape when there is no picture.
    let size = match &view.picture {
        Some(Some((_, size))) => *size,
        _ => (320, 224),
    };
    if view.camera.is_none() {
        // A recording window renders to an image; draw where the other cameras do.
        let mut targets = world.query_filtered::<&RenderTarget, With<Camera2d>>();
        let target = targets.iter(world).next().cloned();
        let layers = RenderLayers::layer(TITLE_LAYER);
        let camera = world
            .spawn((
                Camera2d,
                Camera {
                    order: TITLE_ORDER,
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
            world.spawn((
                Sprite {
                    image: handle.clone(),
                    custom_size: Some(picture_size(size.0, size.1)),
                    ..default()
                },
                Transform::default(),
                layers,
                ChildOf(camera),
            ));
        }
        view.camera = Some(camera);
    }
    if let Some(camera) = view.camera {
        // Each frame: the window may have changed shape.
        crate::window::fit_frame_camera(world, camera, size);
    }
    world.insert_non_send(view);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_picture_is_the_screen_height() {
        let size = picture_size(320, 224);
        assert_eq!(size.y, FRAME.y);
        assert!((size.x - FRAME.y * 320.0 / 224.0).abs() < 1e-3);
    }
}
