//! Draws the field main menu ([`tantalus_tools::field_menu`]) over the
//! field with the game's font and window art.
//!
//! Each menu window is one picture from `DialogAssets::render_window`, laid
//! out in PSX pixels like a dialog window. The pictures are rebuilt when the
//! menu's revision changes.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::sprite::Anchor;
use std::collections::HashMap;
use tantalus_assets::pc::dialog_render::{
    BORDER_PADDING, DEFAULT_TEXT_COLOR, DialogAssets, RESOURCE_X_MULTIPLIER, RESOURCE_Y_MULTIPLIER,
    WindowText,
};

use tantalus_tools::field_menu::{MenuArt, MenuFrame, Tint};
use tantalus_tools::play::PlayGame;

use crate::dialog_view::{FRAME, PSX, TEXELS, WINDOW_Z, downscale};

/// What the menu view has on screen.
#[derive(Default)]
pub struct MenuView {
    root: Option<Entity>,
    revision: Option<u64>,
    assets: Option<Option<DialogAssets>>,
    /// The skin's hand cursor and its size in PSX pixels.
    hand: Option<Option<(Handle<Image>, Vec2)>>,
    /// Portraits by character id, with their pixel size.
    faces: Option<FaceArt>,
}

type FaceArt = HashMap<i32, (Handle<Image>, Vec2)>;

fn color(tint: Tint) -> [f32; 4] {
    let [r, g, b, a] = DEFAULT_TEXT_COLOR;
    match tint {
        Tint::White => [r, g, b, a],
        Tint::Dim => [0.45, 0.45, 0.5, 1.0],
        Tint::Yellow => [0.96, 0.82, 0.4, 1.0],
        Tint::Green => [0.45, 0.95, 0.5, 1.0],
        Tint::Red => [1.0, 0.45, 0.4, 1.0],
        Tint::Cyan => [0.45, 0.9, 0.95, 1.0],
        Tint::Magenta => [0.95, 0.5, 0.9, 1.0],
    }
}

/// Draws the open menu of `game`, or removes the drawing when it closed.
pub fn sync_menu(world: &mut World, game: &PlayGame) {
    let mut view = world.remove_non_send::<MenuView>().unwrap_or_default();
    // The world script's windows (the moogle's) draw as a frame when no screen is up.
    let shown = game.menu_frame().or_else(|| game.world_dialog_frame());
    let revision = shown.as_ref().map(|(revision, _)| *revision);
    // The topmost 2D camera: the field's, or the world map's frame camera over it.
    let mut cameras =
        world.query_filtered::<(Entity, &Camera, Option<&RenderLayers>), With<Camera2d>>();
    let top = cameras
        .iter(world)
        .max_by_key(|(_, camera, _)| camera.order)
        .map(|(entity, _, layers)| (entity, layers.cloned()));
    let camera = top.as_ref().map(|(entity, _)| *entity);
    let layers = top.and_then(|(_, layers)| layers);
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
            view.assets = Some(match crate::ui_skin::dialog_assets(game.game_data()) {
                Ok(assets) => Some(assets),
                Err(error) => {
                    println!("!! menu font and window art: {error}");
                    None
                }
            });
        }
        if view.hand.is_none() {
            let mut images = world.resource_mut::<Assets<Image>>();
            view.hand = Some(crate::dialog_view::load_hand(game.game_data(), &mut images));
        }
        if view.faces.is_none() && !frame.images.is_empty() {
            let mut images = world.resource_mut::<Assets<Image>>();
            view.faces = Some(load_faces(game.game_data(), &mut images));
        }
        if let Some(Some(assets)) = view.assets.as_ref() {
            // The world map's camera sits with its frame picture at z 0; draw in front of it.
            let z = if layers.is_some() { 10.0 } else { WINDOW_Z };
            let root = world
                .spawn((
                    Transform::from_xyz(0.0, 0.0, z),
                    Visibility::default(),
                    ChildOf(camera),
                ))
                .id();
            let art = Art {
                hand: view.hand.clone().flatten(),
                faces: view.faces.as_ref(),
            };
            spawn_frame(world, assets, root, &frame, layers.as_ref(), &art);
            view.root = Some(root);
        }
    }
    world.insert_non_send(view);
}

/// The pictures a menu frame can draw besides its windows.
struct Art<'a> {
    hand: Option<(Handle<Image>, Vec2)>,
    faces: Option<&'a FaceArt>,
}

fn spawn_frame(
    world: &mut World,
    assets: &DialogAssets,
    root: Entity,
    frame: &MenuFrame,
    layers: Option<&RenderLayers>,
    art: &Art,
) {
    let layers = layers.cloned().unwrap_or_default();
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
            layers.clone(),
            ChildOf(root),
        ));
    }
    let over = frame.windows.len() as f32 * 0.1 + 1.0;
    for (index, image) in frame.images.iter().enumerate() {
        match image.art {
            MenuArt::Face { character, dim } => {
                let Some((handle, size)) = art.faces.and_then(|faces| faces.get(&character)) else {
                    continue;
                };
                let shown = fit(*size, image.w, image.h);
                // Centered in its box.
                let x = image.x + (image.w - shown.x) / 2.0;
                let y = image.y + (image.h - shown.y) / 2.0;
                world.spawn((
                    Sprite {
                        image: handle.clone(),
                        custom_size: Some(shown),
                        color: Color::srgba(1.0, 1.0, 1.0, if dim { 0.5 } else { 1.0 }),
                        ..default()
                    },
                    Anchor::TOP_LEFT,
                    Transform::from_translation(
                        local(x, y) + Vec3::Z * (over + index as f32 * 0.01),
                    ),
                    layers.clone(),
                    ChildOf(root),
                ));
            }
        }
    }
    if let Some((x, y)) = frame.pointer {
        match art.hand.clone() {
            // The hand's center at the pointer.
            Some((hand, size)) => {
                world.spawn((
                    Sprite {
                        image: hand,
                        custom_size: Some(size),
                        ..default()
                    },
                    Anchor::CENTER,
                    Transform::from_translation(local(x, y) + Vec3::Z * 5.0),
                    layers,
                    ChildOf(root),
                ));
            }
            // A diamond stands in for the hand without the art.
            None => {
                world.spawn((
                    Sprite::from_color(Color::srgb(1.0, 0.95, 0.6), Vec2::splat(5.0)),
                    Transform::from_translation(local(x, y) + Vec3::Z * 5.0)
                        .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_4)),
                    layers,
                    ChildOf(root),
                ));
            }
        }
    }
}

/// The skin's portraits (`ui_skin::faces`) as images by character id.
fn load_faces(game_data: &std::path::Path, images: &mut Assets<Image>) -> FaceArt {
    crate::ui_skin::faces(game_data)
        .into_iter()
        .map(|(character, face)| {
            let size = Vec2::new(face.width as f32, face.height as f32);
            let mut image = Image::new(
                Extent3d {
                    width: face.width as u32,
                    height: face.height as u32,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                face.rgba,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::default(),
            );
            image.sampler = ImageSampler::linear();
            (character, (images.add(image), size))
        })
        .collect()
}

/// `w` by `h` fitted to the box `bw` by `bh`, keeping the picture's shape.
fn fit(size: Vec2, bw: f32, bh: f32) -> Vec2 {
    let scale = (bw / size.x.max(1.0)).min(bh / size.y.max(1.0));
    size * scale
}
