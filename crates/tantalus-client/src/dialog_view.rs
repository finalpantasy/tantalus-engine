//! FF9-style dialog windows drawn over the play view.
//!
//! The windows are children of the field camera, so they stay fixed on the
//! screen while the camera scrolls. Coordinates are field-frame pixels (the
//! 320x240 frame the camera projects), with the origin at the top left.
//!
//! What is not matched yet: windows are not anchored to the speaking actor
//! and have no tail (`Dialog.setAutoPosition`, `ETb.GetMesPos`; the script's
//! window flags are not kept in the world), and the text uses Bevy's built-in
//! font, not the game's.

use bevy::prelude::*;
use bevy::sprite::Anchor;
use tantalus_tools::field_data::TextLayout;
use tantalus_tools::play::PlayGame;

/// Field frame the camera projects (`window::FIELD_FRAME`).
const FRAME: Vec2 = Vec2::new(320.0, 240.0);
/// `Dialog.DialogLimitLeft/Right/Bottom` margins, in PSX pixels.
const MARGIN: f32 = 8.0;
/// `Dialog.DialogLineHeight` (68 UI units at a 4x UI scale).
const LINE_HEIGHT: f32 = 17.0;
/// Space between the window edge and its text.
const PAD: f32 = 8.0;
const FONT_SIZE: f32 = 12.0;
/// Width of one glyph of the fallback font at `FONT_SIZE`.
const GLYPH_WIDTH: f32 = 7.2;
const CHOICE_INDENT: f32 = 14.0;
/// In front of the field scene, which the camera sees up to its own z.
const WINDOW_Z: f32 = -1.0;

/// The window entities currently drawn and what they show.
#[derive(Resource, Default)]
pub struct DialogView {
    root: Option<Entity>,
    shown: Vec<(u8, u16, usize)>,
}

/// Draws the open dialog windows of `game`, rebuilding them only when the
/// open windows or the choice cursor change.
pub fn sync_dialogs(world: &mut World, game: &PlayGame) {
    let dialogs = game.open_dialogs();
    let mut cameras = world.query_filtered::<Entity, With<Camera2d>>();
    let camera = cameras.iter(world).next();
    let mut view = world.remove_resource::<DialogView>().unwrap_or_default();
    let shown: Vec<(u8, u16, usize)> = dialogs
        .iter()
        .map(|(window, text, layout)| {
            let choices = choice_count(layout);
            let cursor = usize::try_from(game.cursor.max(0)).unwrap_or(0);
            (*window, *text, cursor.min(choices.saturating_sub(1)))
        })
        .collect();
    let attached = match (view.root, camera) {
        (Some(root), Some(camera)) => world
            .get::<ChildOf>(root)
            .is_some_and(|parent| parent.parent() == camera),
        _ => false,
    };
    if attached && view.shown == shown {
        world.insert_resource(view);
        return;
    }
    if let Some(root) = view.root.take()
        && let Ok(entity) = world.get_entity_mut(root)
    {
        entity.despawn();
    }
    view.shown = shown.clone();
    if let (Some(camera), false) = (camera, dialogs.is_empty()) {
        let root = world
            .spawn((
                Transform::from_xyz(0.0, 0.0, WINDOW_Z),
                Visibility::default(),
                ChildOf(camera),
            ))
            .id();
        let mut stacked = 0.0;
        for ((_, _, layout), (_, _, cursor)) in dialogs.iter().zip(&shown) {
            let height = spawn_window(world, root, layout, *cursor, stacked);
            if layout.position.is_none() {
                stacked += height + 4.0;
            }
        }
        view.root = Some(root);
    }
    world.insert_resource(view);
}

fn choice_count(layout: &TextLayout) -> usize {
    layout
        .choice_row
        .map_or(0, |start| layout.rows.len().saturating_sub(start))
}

/// Window size in pixels for `layout` (`[STRT]` width when the text gives
/// one, else wide enough for its longest row).
fn window_size(layout: &TextLayout) -> Vec2 {
    let longest = layout
        .rows
        .iter()
        .enumerate()
        .map(|(row, text)| {
            let indent = if layout.choice_row.is_some_and(|start| row >= start) {
                CHOICE_INDENT
            } else {
                0.0
            };
            text.chars().count() as f32 * GLYPH_WIDTH + indent
        })
        .fold(0.0, f32::max);
    let width = layout
        .size
        .map_or(0.0, |(width, _)| width as f32)
        .max(longest + 2.0 * PAD)
        .clamp(48.0, FRAME.x - 2.0 * MARGIN);
    let lines = layout.rows.len().max(1) as f32;
    Vec2::new(width, lines * LINE_HEIGHT + 2.0 * PAD - 4.0)
}

/// Spawns one window under `root` and returns its height. `stacked` lifts a
/// window that has no fixed position above the ones already drawn.
fn spawn_window(
    world: &mut World,
    root: Entity,
    layout: &TextLayout,
    cursor: usize,
    stacked: f32,
) -> f32 {
    let size = window_size(layout);
    // Windows without `[MPOS]` sit at the bottom center, as `LowerCenter`
    // places a window with no speaker (`Dialog.setAutoPosition`).
    let (left, top) = match layout.position {
        Some((x, y)) => (x as f32, y as f32),
        None => (
            (FRAME.x - size.x) / 2.0,
            FRAME.y - 16.0 - MARGIN - size.y - stacked,
        ),
    };
    let left = left.clamp(MARGIN, (FRAME.x - MARGIN - size.x).max(MARGIN));
    let top = top.clamp(MARGIN, (FRAME.y - MARGIN - size.y).max(MARGIN));
    // Field-frame top-left pixel -> camera-local position.
    let window = world
        .spawn((
            Transform::from_xyz(left - FRAME.x / 2.0, FRAME.y / 2.0 - top, 0.0),
            Visibility::default(),
            ChildOf(root),
        ))
        .id();
    let part = |world: &mut World, color: Color, from: Vec2, to: Vec2, z: f32| {
        world.spawn((
            Sprite::from_color(color, to - from),
            Anchor::TOP_LEFT,
            Transform::from_xyz(from.x, -from.y, z),
            ChildOf(window),
        ));
    };
    // White frame, then the blue body as strips from dark to light (the
    // window's gradient, `Dialog` body sprite).
    part(world, Color::srgb(0.92, 0.92, 0.95), Vec2::ZERO, size, 0.0);
    let body = size - Vec2::splat(2.0);
    let strips = 8;
    for strip in 0..strips {
        let t = strip as f32 / (strips - 1) as f32;
        let from = Vec2::new(1.0, 1.0 + body.y * strip as f32 / strips as f32);
        let to = Vec2::new(
            1.0 + body.x,
            1.0 + body.y * (strip + 1) as f32 / strips as f32,
        );
        part(
            world,
            Color::srgb(0.05 + 0.12 * t, 0.07 + 0.14 * t, 0.30 + 0.28 * t),
            from,
            to,
            0.01,
        );
    }
    for (row, text) in layout.rows.iter().enumerate() {
        let choosing = layout.choice_row.is_some_and(|start| row >= start);
        let x = PAD + if choosing { CHOICE_INDENT } else { 0.0 };
        let y = PAD - 2.0 + row as f32 * LINE_HEIGHT + LINE_HEIGHT / 2.0;
        world.spawn((
            Text2d::new(text.clone()),
            TextFont {
                font_size: FontSize::Px(FONT_SIZE),
                ..default()
            },
            TextColor(Color::WHITE),
            Anchor::CENTER_LEFT,
            Transform::from_xyz(x, -y, 0.02),
            ChildOf(window),
        ));
    }
    if let Some(start) = layout.choice_row {
        // A diamond stands in for the hand pointer (`Dialog` choice cursor).
        let row = start + cursor;
        let y = PAD - 2.0 + row as f32 * LINE_HEIGHT + LINE_HEIGHT / 2.0;
        world.spawn((
            Sprite::from_color(Color::srgb(1.0, 0.95, 0.6), Vec2::splat(6.0)),
            Transform::from_xyz(PAD + 3.0, -y, 0.03)
                .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_4)),
            ChildOf(window),
        ));
    }
    size.y
}
