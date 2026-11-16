//! Draws the field main menu ([`tantalus_tools::field_menu`]) over the
//! field with the game's font and window art.
//!
//! Each menu window is one picture from `DialogAssets::render_window`, laid
//! out in PSX pixels like a dialog window. The pictures are rebuilt when the
//! menu's revision changes.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::sprite::Anchor;
use tantalus_assets::pc::dialog_render::{
    BORDER_PADDING, DEFAULT_TEXT_COLOR, DialogAssets, RESOURCE_X_MULTIPLIER, RESOURCE_Y_MULTIPLIER,
    WindowAtlasChoice, WindowText,
};
use tantalus_tools::field_menu::{MenuFrame, Tint};
use tantalus_tools::play::PlayGame;

use crate::dialog_view::{FRAME, PSX, TEXELS, WINDOW_Z, downscale};

/// What the menu view has on screen.
#[derive(Default)]
pub struct MenuView {
    root: Option<Entity>,
    revision: Option<u64>,
    assets: Option<Option<DialogAssets>>,
}

fn color(tint: Tint) -> [f32; 4] {
    let [r, g, b, a] = DEFAULT_TEXT_COLOR;
    match tint {
        Tint::White => [r, g, b, a],
        Tint::Dim => [0.45, 0.45, 0.5, 1.0],
        Tint::Yellow => [0.96, 0.82, 0.4, 1.0],
        Tint::Green => [0.45, 0.95, 0.5, 1.0],
        Tint::Red => [1.0, 0.45, 0.4, 1.0],
    }
}

/// Draws the open menu of `game`, or removes the drawing when it closed.
pub fn sync_menu(world: &mut World, game: &PlayGame) {
    let mut view = world.remove_non_send::<MenuView>().unwrap_or_default();
    let shown = game.menu_frame();
    let revision = shown.as_ref().map(|(revision, _)| *revision);
    let mut cameras = world.query_filtered::<Entity, With<Camera2d>>();
    let camera = cameras.iter(world).next();
    let attached = match (view.root, camera) {
        (Some(root), Some(camera)) => world
            .get::<ChildOf>(root)
            .is_some_and(|parent| parent.parent() == camera),
        _ => view.root.is_none(),
    };
    if attached && view.revision == revision {
        world.insert_non_send(view);
        return;
    }
    if let Some(root) = view.root.take()
        && let Ok(entity) = world.get_entity_mut(root)
    {
        entity.despawn();
    }
    view.revision = revision;
    if let (Some((_, frame)), Some(camera)) = (shown, camera) {
        if view.assets.is_none() {
            let path = game.game_data().join("resources.assets");
            view.assets = Some(match DialogAssets::load(&path, WindowAtlasChoice::Blue) {
                Ok(assets) => Some(assets),
                Err(error) => {
                    println!("!! menu font and window art: {error}");
                    None
                }
            });
        }
        if let Some(Some(assets)) = view.assets.as_ref() {
            let root = world
                .spawn((
                    Transform::from_xyz(0.0, 0.0, WINDOW_Z),
                    Visibility::default(),
                    ChildOf(camera),
                ))
                .id();
            spawn_frame(world, assets, root, &frame);
            view.root = Some(root);
        }
    }
    world.insert_non_send(view);
}

fn spawn_frame(world: &mut World, assets: &DialogAssets, root: Entity, frame: &MenuFrame) {
    // PSX (320x224, y down) -> frame (320x240) -> camera-local (y up).
    let frame_y_offset = (FRAME.y - PSX.y) / 2.0;
    let local =
        |x: f32, y: f32| Vec3::new(x - FRAME.x / 2.0, FRAME.y / 2.0 - (y + frame_y_offset), 0.0);
    let border = Vec2::splat(BORDER_PADDING as f32) / RESOURCE_X_MULTIPLIER;
    for (index, menu_window) in frame.windows.iter().enumerate() {
        let texts: Vec<WindowText> = menu_window
            .texts
            .iter()
            .map(|run| WindowText {
                x: run.x * RESOURCE_X_MULTIPLIER,
                y: run.y * RESOURCE_Y_MULTIPLIER,
                text: run.text.clone(),
                color: color(run.tint),
                right: run.right,
            })
            .collect();
        let picture = assets.render_window(
            (menu_window.w * RESOURCE_X_MULTIPLIER).round() as i32,
            (menu_window.h * RESOURCE_Y_MULTIPLIER).round() as i32,
            &texts,
        );
        let step = RESOURCE_X_MULTIPLIER / TEXELS;
        let (width, height) = (
            ((picture.width as f32 / step).round() as usize).max(1),
            ((picture.height as f32 / (RESOURCE_Y_MULTIPLIER / TEXELS)).round() as usize).max(1),
        );
        let data = downscale(&picture.rgba, picture.width, picture.height, width, height);
        let mut image = Image::new(
            Extent3d {
                width: width as u32,
                height: height as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        image.sampler = ImageSampler::linear();
        let handle = world.resource_mut::<Assets<Image>>().add(image);
        world.spawn((
            Sprite {
                image: handle,
                custom_size: Some(Vec2::new(width as f32, height as f32) / TEXELS),
                ..default()
            },
            Anchor::TOP_LEFT,
            Transform::from_translation(
                local(menu_window.x - border.x, menu_window.y - border.y)
                    + Vec3::Z * (index as f32 * 0.1),
            ),
            ChildOf(root),
        ));
    }
    if let Some((x, y)) = frame.pointer {
        // A diamond stands in for the hand pointer.
        world.spawn((
            Sprite::from_color(Color::srgb(1.0, 0.95, 0.6), Vec2::splat(5.0)),
            Transform::from_translation(local(x, y) + Vec3::Z * 5.0)
                .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_4)),
            ChildOf(root),
        ));
    }
}
