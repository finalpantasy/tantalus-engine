//! Script-driven presentation drawn over the field: the speech-bubble icon, the "here" icon,
//! the ATE prompt and the fade filter.
//!
//! The icons are the game's own art, cut from `Icon Atlas` in `sharedassets2.assets`
//! (`balloon_exclamation`, `balloon_question`, `balloon_card`, `balloon_beach`,
//! `cursor_hand_here`, `icon_ate_*`), laid out the way `EIcon` and `BubbleUI` place them: the
//! bubble sits over the controlled actor at `(0, -eye * scaley >> 6 + 50)` in field units
//! (`BubbleMappingInfo.GetActorInfo`) plus `BubbleUI.UIDefaultOffset` (50 UI units) up the
//! screen. `ShowBubbleIcon` raises `BubbleUI.IconType`: 0 question, 1 exclamation, 2
//! exclamation and duel (card), 3 beach, 4 exclamation and beach. The here icon (the pointing
//! hand) shows while `EIcon.sHereIconTimer` is between 1 and 57 and no bubble is up.
//!
//! The fade filter is a full-screen quad that adds or subtracts the fade color per channel,
//! as Memoria's shaders do with `_FadeColor_ABR1/2`. It sits behind the dialog windows.
//!
//! The field clock (`TimerUI`, the Hot and Cold countdown) is `MM:SS` in the `counter_N` and
//! `counter_colon` sprites of `General Atlas` (`TimerUI.SetDisplayTime`), the leading minute
//! digit hidden while it is 0, over a window-colored panel. The digits show while the clock
//! runs and blink twice a second while it is stopped (`TimerUI.UpdateBlinkState`). Where the
//! `Timer Container` prefab puts it is not in the data we read; the upper right stands in.
//!
//! Not matched: the bubble's pop-in animation and idle/act sprite swap, the exact on-screen
//! place of the ATE prompt (its prefab is not in the data we read; the lower left stands in),
//! and the here icon's confirm-button trigger.

use std::collections::BTreeMap;
use std::path::Path;

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use tantalus_assets::pc::dialog_render::{RESOURCE_X_MULTIPLIER, RESOURCE_Y_MULTIPLIER};
use tantalus_assets::pc::ui_atlas::{SpriteImage, load_sprites};
use tantalus_contracts::LegacyFieldCamera;
use tantalus_game::field::world::FieldWorld;

use crate::dialog_view::{FRAME, WINDOW_Z};
use crate::psx_blend::FadeBlendMaterial;
use crate::track::TrackFrame;

/// `BubbleUI.UIDefaultOffset` (50 UI units up), in frame pixels.
const UI_OFFSET: f32 = 50.0 / RESOURCE_Y_MULTIPLIER;
/// `BubbleMappingInfo.GetActorInfo`: the bubble's base height above the actor's eye line.
const BASE_HEIGHT: f32 = 50.0;
/// Gap between two bubble icons, frame pixels.
const ICON_GAP: f32 = 2.0;
/// Behind the dialog windows, in front of the field scene.
const FADE_Z: f32 = -0.1;
const ICON_ATLAS: &str = "Icon Atlas";
const ICON_FILE: &str = "sharedassets2.assets";
const SPRITES: [&str; 8] = [
    "balloon_exclamation",
    "balloon_question",
    "balloon_card",
    "balloon_beach",
    "cursor_hand_here",
    "icon_ate_us_uk_jp",
    "icon_bubble",
    "icon_x",
];

/// The sprites currently drawn (a child of the camera) and the art they come from.
#[derive(Default)]
pub struct PresentationView {
    root: Option<Entity>,
    icons: Option<Option<BTreeMap<String, SpriteImage>>>,
    counters: Option<Option<BTreeMap<String, SpriteImage>>>,
}

const COUNTER_ATLAS: &str = "General Atlas";
const COUNTER_SPRITES: [&str; 11] = [
    "counter_0",
    "counter_1",
    "counter_2",
    "counter_3",
    "counter_4",
    "counter_5",
    "counter_6",
    "counter_7",
    "counter_8",
    "counter_9",
    "counter_colon",
];

/// What `TimerUI.SetDisplayTime` shows for `time` seconds: the four digits, minutes first,
/// and whether the leading one is drawn (`digit3Enabled`).
pub fn timer_digits(time: f32) -> ([u8; 4], bool) {
    let whole = time.max(0.0) as u32;
    let minutes = (whole / 60) % 100;
    let seconds = whole % 60;
    (
        [
            (minutes / 10) as u8,
            (minutes % 10) as u8,
            (seconds / 10) as u8,
            (seconds % 10) as u8,
        ],
        minutes / 10 != 0,
    )
}

/// `TimerUI.UpdateBlinkState`: a running clock always shows its digits, a stopped one blinks.
pub fn timer_digits_visible(running: bool, tick: u64) -> bool {
    running || (tick / 15) & 1 == 0
}

fn sprite_image(sprite: &SpriteImage) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: sprite.width,
            height: sprite.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        sprite.rgba.clone(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::linear();
    image
}

/// The sprites `ShowBubbleIcon` icon type `icon` shows, left to right (`EIcon.GetBubbleFlagData`).
pub fn bubble_sprites(icon: i32) -> &'static [&'static str] {
    match icon {
        0 => &["balloon_question"],
        2 => &["balloon_exclamation", "balloon_card"],
        3 => &["balloon_beach"],
        4 => &["balloon_exclamation", "balloon_beach"],
        _ => &["balloon_exclamation"],
    }
}

/// The color the fade filter adds or subtracts, 0..=1 per channel.
pub fn fade_color(color: [i32; 3]) -> Vec4 {
    let [r, g, b] = color.map(|channel| channel.clamp(0, 255) as f32 / 255.0);
    Vec4::new(r, g, b, 1.0)
}

/// `BubbleMappingInfo.GetActorInfo`: the bubble's height above the actor's feet.
pub fn bubble_height(eye: i16, vertical_scale: u8) -> f32 {
    let scale = if vertical_scale == 0 {
        64
    } else {
        i32::from(vertical_scale)
    };
    (((-i32::from(eye)) * scale) >> 6) as f32 + BASE_HEIGHT
}

/// Draws the bubble icon, the here icon, the ATE prompt and the fade filter of `field`,
/// replacing last frame's. `game_data` is the `FF9_Data` folder holding the icon art.
pub fn sync_presentation(
    world: &mut World,
    field: &FieldWorld,
    game_data: &Path,
    scene_camera: &LegacyFieldCamera,
    frame: &TrackFrame,
    tick: u64,
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
        .map(|fade| (fade.mode, fade.color_now()))
        .filter(|(_, color)| color.iter().any(|channel| *channel > 0));
    let player = field.player();
    let look = player
        .and_then(|player| field.actor_presentation.get(&player))
        .copied()
        .unwrap_or_default();
    let anchor = crate::live::player_position(frame).and_then(|position| {
        let height = bubble_height(look.eye, look.vertical_scale);
        let [x, y] = crate::projection::project_field_point(
            scene_camera,
            [position[0], position[1] + height, position[2]],
        )?;
        Some(Vec3::new(
            x - view_center.x,
            y - view_center.y + UI_OFFSET,
            0.0,
        ))
    });
    // `EIcon.ProcessHereIcon`: from the 58th tick down, while no bubble is up.
    let here = field.here_icon_timer > 0
        && field.here_icon_timer < 58
        && field.bubble_icon.is_none()
        && anchor.is_some();
    let bubble = field.bubble_icon.filter(|_| anchor.is_some());
    // `EIcon.ProcessAIcon`: blue while on, gray and blinking every 15 ticks in mode 2.
    let ate = match field.ate_icon_mode {
        mode if mode > 0 && (mode & 4 != 0 || field.user_control) => {
            if mode & 3 != 2 {
                Some(Color::WHITE)
            } else if (tick / 15) & 1 == 1 {
                Some(Color::srgb(0.55, 0.55, 0.55))
            } else {
                None
            }
        }
        _ => None,
    };
    let timer = Some(field.timer)
        .filter(|timer| timer.displayed)
        .map(|timer| (timer.time(), timer.running));
    if fade.is_none() && bubble.is_none() && !here && ate.is_none() && timer.is_none() {
        world.insert_non_send(view);
        return;
    }
    if view.icons.is_none() {
        let path = game_data.join(ICON_FILE);
        view.icons = Some(match load_sprites(&path, ICON_ATLAS, &SPRITES) {
            Ok(sprites) => Some(sprites),
            Err(error) => {
                println!("!! field icon art: {error}");
                None
            }
        });
    }
    if timer.is_some() && view.counters.is_none() {
        view.counters = Some(
            match load_sprites(&game_data.join(ICON_FILE), COUNTER_ATLAS, &COUNTER_SPRITES) {
                Ok(sprites) => Some(sprites),
                Err(error) => {
                    println!("!! field clock art: {error}");
                    None
                }
            },
        );
    }
    let root = world
        .spawn((
            Transform::from_xyz(0.0, 0.0, WINDOW_Z),
            Visibility::default(),
            ChildOf(camera),
        ))
        .id();
    let sprites = view.icons.as_ref().and_then(Option::as_ref);
    let icon = |world: &mut World, name: &str, at: Vec3, color: Color| {
        let Some(art) = sprites.and_then(|sprites| sprites.get(name)) else {
            return;
        };
        let handle = world.resource_mut::<Assets<Image>>().add(sprite_image(art));
        world.spawn((
            Sprite {
                image: handle,
                color,
                custom_size: Some(Vec2::new(
                    art.width as f32 / RESOURCE_X_MULTIPLIER,
                    art.height as f32 / RESOURCE_Y_MULTIPLIER,
                )),
                ..default()
            },
            Transform::from_translation(at),
            ChildOf(root),
        ));
    };
    if let (Some(kind), Some(anchor)) = (bubble, anchor) {
        let names = bubble_sprites(kind);
        let width = 116.0 / RESOURCE_X_MULTIPLIER;
        let total = names.len() as f32 * width + (names.len() as f32 - 1.0) * ICON_GAP;
        for (index, name) in names.iter().enumerate() {
            let x = -total / 2.0 + width / 2.0 + index as f32 * (width + ICON_GAP);
            icon(world, name, anchor + Vec3::new(x, 0.0, 0.3), Color::WHITE);
        }
    }
    if let (true, Some(anchor)) = (here, anchor) {
        icon(
            world,
            "cursor_hand_here",
            anchor + Vec3::new(0.0, 0.0, 0.2),
            Color::WHITE,
        );
    }
    if let Some(color) = ate {
        let at = Vec3::new(-FRAME.x / 2.0 + 34.0, -FRAME.y / 2.0 + 22.0, 0.1);
        icon(world, "icon_ate_us_uk_jp", at, color);
    }
    if let Some((time, running)) = timer {
        draw_timer(
            world,
            root,
            view.counters.as_ref().and_then(Option::as_ref),
            time,
            timer_digits_visible(running, tick),
        );
    }
    if let Some((mode, color)) = fade {
        let material = world
            .resource_mut::<Assets<FadeBlendMaterial>>()
            .add(FadeBlendMaterial {
                color: fade_color(color),
                mode: if mode & 2 != 0 { 2 } else { 1 },
            });
        let mesh = world
            .resource_mut::<Assets<Mesh>>()
            .add(Rectangle::new(FRAME.x, FRAME.y));
        world.spawn((
            Mesh2d(mesh),
            MeshMaterial2d(material),
            Transform::from_xyz(0.0, 0.0, FADE_Z),
            ChildOf(root),
        ));
    }
    view.root = Some(root);
    world.insert_non_send(view);
}

/// The clock: a panel and `MM:SS` in the upper right of the frame.
fn draw_timer(
    world: &mut World,
    root: Entity,
    counters: Option<&BTreeMap<String, SpriteImage>>,
    time: f32,
    visible: bool,
) {
    let Some(counters) = counters else {
        return;
    };
    let (digits, leading) = timer_digits(time);
    let size = |name: &str| {
        counters.get(name).map_or(Vec2::ZERO, |art| {
            Vec2::new(
                art.width as f32 / RESOURCE_X_MULTIPLIER,
                art.height as f32 / RESOURCE_Y_MULTIPLIER,
            )
        })
    };
    let digit = size("counter_0");
    let colon = size("counter_colon");
    let pad = Vec2::new(8.0, 5.0);
    let width = digit.x * 4.0 + colon.x;
    let panel = Vec2::new(width, digit.y) + pad * 2.0;
    let center = Vec3::new(
        FRAME.x / 2.0 - panel.x / 2.0 - 6.0,
        FRAME.y / 2.0 - panel.y / 2.0 - 6.0,
        0.15,
    );
    world.spawn((
        Sprite {
            color: Color::srgba(0.07, 0.1, 0.4, 0.9),
            custom_size: Some(panel),
            ..default()
        },
        Transform::from_translation(center),
        ChildOf(root),
    ));
    if !visible {
        return;
    }
    // Left edge of the digits, then minutes tens, ones, the colon, seconds tens, ones.
    let mut x = center.x - width / 2.0;
    let slots = [
        Some(digits[0]).filter(|_| leading),
        Some(digits[1]),
        None,
        Some(digits[2]),
        Some(digits[3]),
    ];
    for (index, slot) in slots.iter().enumerate() {
        let name = match (index, slot) {
            (2, _) => "counter_colon".to_owned(),
            (_, Some(value)) => format!("counter_{value}"),
            (_, None) => {
                x += digit.x;
                continue;
            }
        };
        let step = if index == 2 { colon.x } else { digit.x };
        let Some(art) = counters.get(&name) else {
            continue;
        };
        let handle = world.resource_mut::<Assets<Image>>().add(sprite_image(art));
        world.spawn((
            Sprite {
                image: handle,
                custom_size: Some(size(&name)),
                ..default()
            },
            Transform::from_xyz(x + step / 2.0, center.y, center.z + 0.05),
            ChildOf(root),
        ));
        x += step;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_shows_minutes_and_seconds_and_hides_a_zero_tens_digit() {
        assert_eq!(timer_digits(60.99), ([0, 1, 0, 0], false));
        assert_eq!(timer_digits(5.0), ([0, 0, 0, 5], false));
        assert_eq!(timer_digits(754.0), ([1, 2, 3, 4], true));
        assert!(timer_digits_visible(true, 20));
        assert!(!timer_digits_visible(false, 20));
        assert!(timer_digits_visible(false, 5));
    }

    #[test]
    fn each_icon_type_shows_the_flags_eicon_maps_it_to() {
        assert_eq!(bubble_sprites(0), ["balloon_question"]);
        assert_eq!(bubble_sprites(1), ["balloon_exclamation"]);
        assert_eq!(bubble_sprites(2), ["balloon_exclamation", "balloon_card"]);
        assert_eq!(bubble_sprites(4), ["balloon_exclamation", "balloon_beach"]);
    }

    #[test]
    fn the_bubble_floats_above_the_eye_line() {
        // Eye height 100 is stored as -400; scale 64 leaves it, plus the 50 base.
        assert_eq!(bubble_height(-400, 64), 450.0);
        assert_eq!(bubble_height(-400, 0), 450.0);
        assert_eq!(bubble_height(-400, 128), 850.0);
    }

    #[test]
    fn fade_colors_clamp_to_the_byte_range() {
        assert_eq!(fade_color([255, 0, 300]), Vec4::new(1.0, 0.0, 1.0, 1.0));
    }
}
