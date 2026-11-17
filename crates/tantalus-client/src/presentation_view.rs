//! Script-driven presentation drawn over the field: the speech-bubble icon, the "here" icon
//! and the fade filter.
//!
//! All three are sprites that are children of the field camera, so they stay fixed on the
//! screen while the field scrolls. The icons follow `EIcon` (`ShowBubbleIcon` raises
//! `BubbleUI.IconType`: 0 question, 1 exclamation, 2 exclamation and duel, 3 beach, 4
//! exclamation and beach; `SetHereIcon` shows the marker over the player).
//!
//! Not matched: the game's icon art (a small procedural bubble and arrow stand in), the icon
//! pop-in animation, and the fade filter's per-channel add and subtract blend (a single
//! translucent tint of the same strength stands in for it).

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use tantalus_contracts::LegacyFieldCamera;
use tantalus_game::field::world::FieldWorld;

use crate::dialog_view::{FRAME, WINDOW_Z};
use crate::track::TrackFrame;

/// Height above the player's feet the icons float at (field units).
const ICON_HEIGHT: f32 = 380.0;
/// Frame pixels one icon texel covers.
const ICON_SCALE: f32 = 1.0;
/// In front of dialogs: the fade covers the whole picture.
const FADE_Z: f32 = WINDOW_Z + 0.5;

/// The sprites currently drawn (a child of the camera).
#[derive(Default)]
pub struct PresentationView {
    root: Option<Entity>,
}

/// `fldmcf`-style 5x7 glyphs for the bubble: `?` and `!`.
const QUESTION: [&str; 7] = [
    ".###.", "#...#", "....#", "..##.", "..#..", ".....", "..#..",
];
const EXCLAMATION: [&str; 7] = [
    "..#..", "..#..", "..#..", "..#..", "..#..", ".....", "..#..",
];

/// A speech bubble 16x14 with a tail, the glyph inside it in `ink`.
fn bubble_image(glyph: &[&str; 7], fill: [u8; 4], ink: [u8; 4]) -> Image {
    const W: usize = 16;
    const H: usize = 14;
    let mut rgba = vec![0u8; W * H * 4];
    let mut put = |x: usize, y: usize, color: [u8; 4]| {
        let at = (y * W + x) * 4;
        rgba[at..at + 4].copy_from_slice(&color);
    };
    let outline = [20, 20, 40, 255];
    for y in 0..11 {
        for x in 0..W {
            let corner = (x == 0 || x == W - 1) && (y == 0 || y == 10);
            if corner {
                continue;
            }
            let edge = x == 0 || x == W - 1 || y == 0 || y == 10;
            put(x, y, if edge { outline } else { fill });
        }
    }
    // The tail.
    for (y, xs) in [(11usize, 5usize..9), (12, 6..8)] {
        for x in xs {
            let edge = (x == 5 || x == 8) && y == 11 || y == 12;
            put(x, y, if edge { outline } else { fill });
        }
    }
    for (row, line) in glyph.iter().enumerate() {
        for (column, cell) in line.bytes().enumerate() {
            if cell == b'#' {
                put(5 + column, 2 + row, ink);
            }
        }
    }
    image(W, H, rgba)
}

/// A downward arrow, 11x9.
fn here_image() -> Image {
    const W: usize = 11;
    const H: usize = 9;
    let mut rgba = vec![0u8; W * H * 4];
    for y in 0..H {
        for x in y..W - y {
            let edge = x == y || x == W - y - 1 || y == 0;
            let color = if edge {
                [40, 30, 0, 255]
            } else {
                [255, 214, 64, 255]
            };
            let at = (y * W + x) * 4;
            rgba[at..at + 4].copy_from_slice(&color);
        }
    }
    image(W, H, rgba)
}

fn image(width: usize, height: usize, rgba: Vec<u8>) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: width as u32,
            height: height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::nearest();
    image
}

/// The color and strength of the fade filter at `progress` (0..=256): `FadeMode.Add` lightens
/// toward the color, `FadeMode.Sub` darkens by it (`SceneDirector.InitFade`).
pub fn fade_tint(mode: i32, color: [i32; 3], progress: i32) -> [f32; 4] {
    let strength = color
        .iter()
        .map(|channel| (*channel).clamp(0, 255))
        .max()
        .unwrap_or(0) as f32
        / 255.0;
    let alpha = strength * progress.clamp(0, 256) as f32 / 256.0;
    if mode & 2 != 0 {
        [0.0, 0.0, 0.0, alpha]
    } else {
        let [r, g, b] = color.map(|channel| channel.clamp(0, 255) as f32 / 255.0);
        let top = r.max(g).max(b).max(f32::EPSILON);
        [r / top, g / top, b / top, alpha]
    }
}

/// Draws the bubble icon, the here icon and the fade filter of `field`, replacing last
/// frame's.
pub fn sync_presentation(
    world: &mut World,
    field: &FieldWorld,
    scene_camera: &LegacyFieldCamera,
    frame: &TrackFrame,
) {
    let mut view = world
        .remove_non_send::<PresentationView>()
        .unwrap_or_default();
    if let Some(root) = view.root.take()
        && let Ok(entity) = world.get_entity_mut(root)
    {
        entity.despawn();
    }
    let mut cameras = world.query_filtered::<(Entity, &Transform), With<Camera2d>>();
    let camera = cameras
        .iter(world)
        .next()
        .map(|(entity, transform)| (entity, transform.translation.truncate()));
    let Some((camera, view_center)) = camera else {
        world.insert_non_send(view);
        return;
    };
    let fade = field
        .fade_filter
        .as_ref()
        .map(|fade| fade_tint(fade.mode, fade.color, fade.progress()))
        .filter(|tint| tint[3] > 0.0);
    let player = crate::live::player_position(frame);
    let head = player.and_then(|position| {
        let [x, y] = crate::projection::project_field_point(
            scene_camera,
            [position[0], position[1] + ICON_HEIGHT, position[2]],
        )?;
        Some(Vec3::new(x - view_center.x, y - view_center.y, 0.0))
    });
    if fade.is_none() && head.is_none() {
        world.insert_non_send(view);
        return;
    }
    let bubble = field.bubble_icon.filter(|_| head.is_some());
    let here = (field.here_icon != 0 && head.is_some()).then_some(());
    if fade.is_none() && bubble.is_none() && here.is_none() {
        world.insert_non_send(view);
        return;
    }
    let root = world
        .spawn((
            Transform::from_xyz(0.0, 0.0, WINDOW_Z),
            Visibility::default(),
            ChildOf(camera),
        ))
        .id();
    if let (Some(icon), Some(head)) = (bubble, head) {
        let (glyph, fill, ink) = match icon {
            0 => (&QUESTION, [255, 255, 255, 255], [30, 30, 90, 255]),
            2 => (&EXCLAMATION, [255, 214, 214, 255], [200, 20, 20, 255]),
            3 => (&QUESTION, [214, 244, 255, 255], [20, 90, 160, 255]),
            4 => (&EXCLAMATION, [214, 244, 255, 255], [20, 90, 160, 255]),
            _ => (&EXCLAMATION, [255, 255, 255, 255], [200, 20, 20, 255]),
        };
        let handle = world
            .resource_mut::<Assets<Image>>()
            .add(bubble_image(glyph, fill, ink));
        world.spawn((
            Sprite {
                image: handle,
                custom_size: Some(Vec2::new(16.0, 14.0) * ICON_SCALE),
                ..default()
            },
            Transform::from_translation(head + Vec3::new(0.0, 12.0, 0.3)),
            ChildOf(root),
        ));
    }
    if let (Some(()), Some(head)) = (here, head) {
        let handle = world.resource_mut::<Assets<Image>>().add(here_image());
        world.spawn((
            Sprite {
                image: handle,
                custom_size: Some(Vec2::new(11.0, 9.0) * ICON_SCALE),
                ..default()
            },
            Transform::from_translation(head + Vec3::new(0.0, 0.0, 0.2)),
            ChildOf(root),
        ));
    }
    if let Some([r, g, b, a]) = fade {
        world.spawn((
            Sprite {
                color: Color::srgba(r, g, b, a),
                custom_size: Some(FRAME),
                ..default()
            },
            Transform::from_xyz(0.0, 0.0, FADE_Z - WINDOW_Z),
            ChildOf(root),
        ));
    }
    view.root = Some(root);
    world.insert_non_send(view);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtract_darkens_and_add_lightens() {
        assert_eq!(fade_tint(2, [255, 0, 0], 256), [0.0, 0.0, 0.0, 1.0]);
        let add = fade_tint(0, [128, 0, 0], 128);
        assert_eq!(add[..3], [1.0, 0.0, 0.0]);
        assert!((add[3] - 128.0 / 255.0 * 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_fade_that_has_not_started_is_invisible() {
        assert_eq!(fade_tint(0, [255, 255, 255], 0)[3], 0.0);
    }
}
