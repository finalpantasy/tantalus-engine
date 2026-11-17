//! FF9 dialog windows drawn over the play view.
//!
//! Each open window is rendered on the CPU with the game's own font and window
//! art (`tantalus_assets::pc::dialog_render`) and shown as a sprite that is a
//! child of the field camera, so it stays fixed on the screen while the camera
//! scrolls. Placement follows `Dialog.InitializeDialogTransition`,
//! `Dialog.setPositionX/Y` and `ETb.GetMesPos`; sizes are PSX pixels
//! (320x224) mapped onto the 320x240 field frame the camera projects.
//!
//! Not matched: the tail is a plain marker, not the game's tail sprite;
//! `Dialog.CheckDialogOverlap`; the speaker's eye height (a fixed aim height
//! stands in); window open/close animation; icons, buttons and sprites in the
//! text.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::sprite::Anchor;
use tantalus_assets::pc::dialog_render::{
    BORDER_PADDING, DialogAssets, LINE_HEIGHT, PHRASE_Y_PADDING, RESOURCE_X_MULTIPLIER,
    RESOURCE_Y_MULTIPLIER, SizeMode, WindowAtlasChoice, WindowText,
};
use tantalus_contracts::LegacyFieldCamera;
use tantalus_tools::play::{OpenDialog, PlayGame};

use crate::track::TrackFrame;

/// Field frame the camera projects (`window::FIELD_FRAME`).
pub(crate) const FRAME: Vec2 = Vec2::new(320.0, 240.0);
/// The PSX screen sits centered in the frame vertically.
pub(crate) const PSX: Vec2 = Vec2::new(320.0, 224.0);
/// Texels per frame pixel of the drawn window (the UI art is ~4.8 per PSX
/// pixel; this keeps it sharp at 2x and 3x windows).
pub(crate) const TEXELS: f32 = 3.0;
/// In front of the field scene, which the camera sees up to its own z.
pub(crate) const WINDOW_Z: f32 = -1.0;
/// `Dialog.DialogLimitLeft/Right/Top/Bottom`, PSX pixels.
const LIMIT_LEFT: f32 = 8.0;
const LIMIT_RIGHT: f32 = 312.0;
const LIMIT_BOTTOM: f32 = 208.0;
/// `Dialog.kMargin`, `kUpperOffset`, `kLowerOffset`, `kLimitTop`,
/// `DialogTailLeftRightOffset`, `TailMagicNumber1/2`, `InitialMagicNum`.
const MARGIN: f32 = 5.0;
const UPPER_OFFSET: f32 = 16.0;
const LOWER_OFFSET: f32 = 24.0;
const LIMIT_TOP: f32 = 5.0;
const TAIL_SIDE: f32 = 6.0;
const MAGIC_1: f32 = 24.0;
const MAGIC_2: f32 = 8.0;
const INITIAL_MAGIC: f32 = 32.0;
/// `Dialog.PosYOffset` (-15 UI units) in PSX pixels.
const POS_Y_OFFSET: f32 = -3.0;
/// Height of the speaker's mouth above their feet, projected for the tail.
const SPEAKER_HEIGHT: f32 = crate::scene_adapter::CHAR_AIM_HEIGHT;

/// `ETb` window flags.
const FLAG_NO_TAIL: u8 = 4;
/// `ETb.winMOG`: the Mognet window.
const FLAG_MOG: u8 = 8;
const FLAG_TRANSPARENT: u8 = 16;
/// `ETb.winATE`: the Active Time Event window.
const FLAG_ATE: u8 = 64;
const FLAG_CHAT: u8 = 128;

/// The caption a window's flags give it (`ETb.FlagsToStyles`: a window that
/// is not chat style is `Dialog.CaptionType.Mognet` for `winMOG` or
/// `ActiveTimeEvent` for `winATE`; `Dialog.Caption` is
/// `FF9TextTool.GetDialogCaptionText`, the localized name of the type).
fn caption(flags: u8) -> Option<&'static str> {
    if flags & FLAG_CHAT != 0 {
        None
    } else if flags & FLAG_MOG != 0 {
        Some("Mognet")
    } else if flags & FLAG_ATE != 0 {
        Some("Active Time Event")
    } else {
        None
    }
}

/// A rendered window kept for reuse: the picture and the sizes the placement needs.
struct Rendered {
    image: Handle<Image>,
    /// Picture size in texels.
    texels: Vec2,
    /// Window body size in PSX pixels.
    body: Vec2,
    /// Top of each text line below the label top, UI units.
    line_tops: Vec<f32>,
    choice_line: Option<usize>,
    /// Glyphs on the first page.
    glyphs: usize,
}

/// What the view has on screen and what it needs to draw more.
#[derive(Default)]
pub struct DialogView {
    root: Option<Entity>,
    shown: Vec<(u8, u16, usize, [i32; 2], usize)>,
    /// The game tick each open window first appeared on, for the typing speed.
    opened: HashMap<(u8, u16), u64>,
    assets: Option<Option<DialogAssets>>,
    cache: HashMap<String, Rendered>,
}

/// Draws the open dialog windows of `game`, rebuilding them only when the
/// open windows, the choice cursor or an anchor position change.
/// The dialog font and window art loading on a background thread, started by
/// [`preload`] so the first dialog does not stall a frame.
static PRELOAD: std::sync::Mutex<Option<std::thread::JoinHandle<Result<DialogAssets, String>>>> =
    std::sync::Mutex::new(None);

/// Starts loading the dialog assets for [`sync_dialogs`] to pick up.
pub fn preload(game_data: &std::path::Path) {
    let path = game_data.join("resources.assets");
    if let Ok(mut slot) = PRELOAD.lock() {
        *slot = Some(std::thread::spawn(move || {
            DialogAssets::load(&path, WindowAtlasChoice::Blue).map_err(|error| error.to_string())
        }));
    }
}

pub fn sync_dialogs(
    world: &mut World,
    game: &PlayGame,
    scene_camera: &LegacyFieldCamera,
    frame: &TrackFrame,
) {
    let mut dialogs = game.open_dialogs();
    // Test aid: `TANTALUS_DIALOG_FLAGS=8` replaces every dialog's window flags,
    // to see the Mognet caption without playing up to a Mognet window.
    if let Some(extra) = std::env::var("TANTALUS_DIALOG_FLAGS")
        .ok()
        .and_then(|value| value.parse::<u8>().ok())
    {
        for dialog in &mut dialogs {
            dialog.flags = extra;
        }
    }
    let mut cameras = world.query_filtered::<(Entity, &Transform), With<Camera2d>>();
    let camera = cameras
        .iter(world)
        .next()
        .map(|(entity, transform)| (entity, transform.translation.truncate()));
    let mut view = world.remove_non_send::<DialogView>().unwrap_or_default();
    let cursor = usize::try_from(game.cursor.max(0)).unwrap_or(0);
    let anchors: Vec<Option<[f32; 2]>> = dialogs
        .iter()
        .map(|dialog| {
            let (_, view_center) = camera?;
            speaker_screen(dialog, scene_camera, frame, view_center)
        })
        .collect();
    if view.assets.is_none() {
        let path = game.game_data().join("resources.assets");
        let loaded = match PRELOAD.lock().ok().and_then(|mut slot| slot.take()) {
            Some(handle) => handle
                .join()
                .unwrap_or_else(|_| Err("the preload thread failed".to_owned())),
            None => DialogAssets::load(&path, WindowAtlasChoice::Blue)
                .map_err(|error| error.to_string()),
        };
        view.assets = Some(match loaded {
            Ok(assets) => Some(assets),
            Err(error) => {
                println!("!! dialog font and window art: {error}");
                None
            }
        });
    }
    // Text types in at the appearance speed of the text modifier: three times the animation
    // tick of the field message setting, in glyphs a second at 30 ticks a second.
    let per_second = 3.0 * message_tick(game.world().settings.fld_msg);
    view.opened
        .retain(|key, _| dialogs.iter().any(|d| (d.window, d.text) == *key));
    let mut shown: Vec<(u8, u16, usize, [i32; 2], usize)> = Vec::new();
    for (dialog, anchor) in dialogs.iter().zip(&anchors) {
        let at = anchor.map_or([0, 0], |[x, y]| [x.round() as i32, y.round() as i32]);
        let total = {
            let mut images = world.resource_mut::<Assets<Image>>();
            render(&mut view, &mut images, dialog)
                .map_or(0, |key| view.cache.get(&key).map_or(0, |r| r.glyphs))
        };
        let opened = *view
            .opened
            .entry((dialog.window, dialog.text))
            .or_insert(game.ticks);
        let typed = ((game.ticks - opened) as f32 / 30.0 * per_second) as usize;
        shown.push((dialog.window, dialog.text, cursor, at, typed.min(total)));
    }
    let attached = match (view.root, camera) {
        (Some(root), Some((camera, _))) => world
            .get::<ChildOf>(root)
            .is_some_and(|parent| parent.parent() == camera),
        _ => false,
    };
    if attached && view.shown == shown {
        world.insert_non_send(view);
        return;
    }
    if let Some(root) = view.root.take()
        && let Ok(entity) = world.get_entity_mut(root)
    {
        entity.despawn();
    }
    view.shown = shown;
    if let (Some((camera, _)), false) = (camera, dialogs.is_empty()) {
        let root = world
            .spawn((
                Transform::from_xyz(0.0, 0.0, WINDOW_Z),
                Visibility::default(),
                ChildOf(camera),
            ))
            .id();
        for (index, (dialog, anchor)) in dialogs.iter().zip(&anchors).enumerate() {
            let visible = view.shown[index].4;
            spawn_window(
                world,
                &mut view,
                root,
                dialog,
                *anchor,
                (cursor, visible),
                index as f32 * 0.1,
            );
        }
        view.root = Some(root);
    }
    world.insert_non_send(view);
}

/// Where the window's speaker is on the PSX screen, pixels from the top left
/// (`ETb.GetMesPos` then `World2Screen`). `None` for windows with no speaker
/// to follow.
fn speaker_screen(
    dialog: &OpenDialog,
    scene_camera: &LegacyFieldCamera,
    frame: &TrackFrame,
    view_center: Vec2,
) -> Option<[f32; 2]> {
    if dialog.flags & FLAG_CHAT == 0 {
        return None;
    }
    let speaker = dialog.speaker?;
    let (_, position) = frame.actors.iter().find(|(id, _)| *id == speaker.get())?;
    let [x, y] = crate::projection::project_field_point(
        scene_camera,
        [position[0], position[1] + SPEAKER_HEIGHT, position[2]],
    )?;
    // Camera space (y up, centered on the view) -> PSX screen (y down).
    Some([
        PSX.x / 2.0 + (x - view_center.x),
        PSX.y / 2.0 - (y - view_center.y),
    ])
}

/// `NGUIText.ChoiceIndent`-style conversion of UI units to PSX pixels.
fn to_psx(ui: Vec2) -> Vec2 {
    Vec2::new(ui.x / RESOURCE_X_MULTIPLIER, ui.y / RESOURCE_Y_MULTIPLIER)
}

fn render(
    view: &mut DialogView,
    images: &mut Assets<Image>,
    dialog: &OpenDialog,
) -> Option<String> {
    let key = render_key(dialog);
    if view.cache.contains_key(&key) {
        return Some(key);
    }
    let assets = view.assets.as_ref()?.as_ref()?;
    // `[STRT]` widths are hints; measured sizing never clips the text.
    let mode = SizeMode::Auto;
    let framed = dialog.flags & FLAG_TRANSPARENT == 0;
    let (picture, layout) = assets.render(&dialog.entry, mode, framed)?;
    // Box-filter down to `TEXELS` per frame pixel.
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
    let page = layout.pages.first()?;
    let rendered = Rendered {
        image: images.add(image),
        texels: Vec2::new(width as f32, height as f32),
        body: to_psx(Vec2::new(
            layout.body_width as f32,
            layout.body_height as f32,
        )),
        line_tops: page.lines.iter().map(|line| line.base_y).collect(),
        choice_line: page.choice_line,
        glyphs: page.glyphs.len(),
    };
    view.cache.insert(key.clone(), rendered);
    Some(key)
}

fn render_key(dialog: &OpenDialog) -> String {
    format!(
        "{:?}|{}",
        dialog.entry.tokens,
        dialog.flags & FLAG_TRANSPARENT
    )
}

/// The animation tick of field message speed `fld_msg`; the last entry past the table.
pub fn message_tick(fld_msg: u64) -> f32 {
    let at = usize::try_from(fld_msg)
        .unwrap_or(usize::MAX)
        .min(tantalus_legacy::party_menu::config::DIALOG_TEXT_TICK.len() - 1);
    f32::from(tantalus_legacy::party_menu::config::DIALOG_TEXT_TICK[at])
}

/// The window with its first `visible` glyphs drawn, not cached.
fn render_typing(
    view: &DialogView,
    images: &mut Assets<Image>,
    dialog: &OpenDialog,
    visible: usize,
) -> Option<Rendered> {
    let full = view.cache.get(&render_key(dialog))?;
    let assets = view.assets.as_ref()?.as_ref()?;
    let framed = dialog.flags & FLAG_TRANSPARENT == 0;
    let (picture, _) = assets.render_limited(&dialog.entry, SizeMode::Auto, framed, visible)?;
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
    Some(Rendered {
        image: images.add(image),
        texels: Vec2::new(width as f32, height as f32),
        body: full.body,
        line_tops: full.line_tops.clone(),
        choice_line: full.choice_line,
        glyphs: full.glyphs,
    })
}

/// Area-average resample of straight-alpha RGBA8 (averaged premultiplied).
pub(crate) fn downscale(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    let mut out = vec![0u8; dw * dh * 4];
    for dy in 0..dh {
        let y0 = dy * sh / dh;
        let y1 = ((dy + 1) * sh / dh).clamp(y0 + 1, sh);
        for dx in 0..dw {
            let x0 = dx * sw / dw;
            let x1 = ((dx + 1) * sw / dw).clamp(x0 + 1, sw);
            let mut sum = [0.0f32; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = (y * sw + x) * 4;
                    let alpha = f32::from(src[i + 3]) / 255.0;
                    for c in 0..3 {
                        sum[c] += f32::from(src[i + c]) * alpha;
                    }
                    sum[3] += alpha;
                }
            }
            let count = ((y1 - y0) * (x1 - x0)) as f32;
            let o = (dy * dw + dx) * 4;
            if sum[3] > 0.0 {
                for c in 0..3 {
                    out[o + c] = (sum[c] / sum[3]).round().clamp(0.0, 255.0) as u8;
                }
                out[o + 3] = (sum[3] / count * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// `Dialog.setPositionX`: left edge of a window whose tail is at `x`.
fn place_x(x: f32, width: f32, left: bool) -> (f32, f32) {
    let (tail, min, max) = if left {
        let tail = (x - TAIL_SIDE).clamp(LIMIT_LEFT + MAGIC_1, LIMIT_RIGHT - MAGIC_2);
        (tail, tail + MAGIC_2 - width, tail - MAGIC_1)
    } else {
        let tail = (x + TAIL_SIDE).clamp(LIMIT_LEFT + MAGIC_2, LIMIT_RIGHT - MAGIC_1);
        (tail, tail + MAGIC_1 - width, tail - MAGIC_2)
    };
    let min = min.max(LIMIT_LEFT);
    let max = max.min(LIMIT_RIGHT - width);
    ((min + max) / 2.0, tail)
}

/// `Dialog.setPositionY`: bottom edge (y up) of a window; flips the side
/// when it does not fit.
fn place_y(y: f32, height: f32, upper: &mut bool) -> f32 {
    let limit_bottom = LIMIT_BOTTOM - 4.0;
    let mut bottom;
    if *upper {
        bottom = y - height - UPPER_OFFSET;
        if bottom < LIMIT_TOP {
            *upper = false;
            bottom = y + LOWER_OFFSET;
        }
    } else {
        bottom = y + LOWER_OFFSET;
        if bottom > limit_bottom - height {
            *upper = true;
            bottom = y - height - UPPER_OFFSET;
        }
    }
    bottom.clamp(LIMIT_TOP, limit_bottom - height)
}

/// Top-left of the window in PSX pixels, and the tail (x, at-top) when it has one.
fn place(dialog: &OpenDialog, size: Vec2, anchor: Option<[f32; 2]>) -> (Vec2, Option<(f32, bool)>) {
    if let Some((x, y)) = dialog.layout.position
        && (x != 0 || y != 0)
    {
        return (Vec2::new(x as f32, y as f32), None);
    }
    let tag = |name: &str| {
        dialog.entry.tokens.iter().any(
            |token| matches!(token, tantalus_assets::pc::mes::MesToken::Tag(tag) if tag.code == name),
        )
    };
    let tail_tag = [
        ("LowerRight", (false, false)),
        ("LowerLeft", (false, true)),
        ("UpperRight", (true, false)),
        ("UpperLeft", (true, true)),
    ]
    .into_iter()
    .find(|(name, _)| tag(name))
    .map(|(_, bits)| bits);
    if let Some([ax, ay]) = anchor {
        // Bottom-origin y, as the reference works in.
        let (px, py) = (ax, PSX.y - ay + POS_Y_OFFSET);
        let (mut upper, mut left) = tail_tag.unwrap_or((true, false));
        if tail_tag.is_none() {
            // Without a listener the tail is upper and right
            // (`TailPosition.AutoPosition`, no `importantObj`).
            upper = true;
            left = false;
        }
        if tail_tag.is_none() {
            let toggle = if left {
                px < LIMIT_LEFT + INITIAL_MAGIC
            } else {
                px > LIMIT_RIGHT - INITIAL_MAGIC
            };
            left ^= toggle;
        }
        let (x, tail) = place_x(px, size.x, left);
        let bottom = place_y(py, size.y, &mut upper);
        return (Vec2::new(x, PSX.y - bottom - size.y), Some((tail, upper)));
    }
    // Fixed windows (`Dialog` else-branch of `InitializeDialogTransition`).
    let (cx, cy) = if tag("LowerRight") {
        (
            LIMIT_RIGHT - MARGIN - size.x / 2.0,
            8.0 + MARGIN + size.y / 2.0,
        )
    } else if tag("LowerLeft") {
        (
            LIMIT_LEFT + MARGIN + size.x / 2.0,
            8.0 + MARGIN + size.y / 2.0,
        )
    } else if tag("UpperRight") {
        (
            LIMIT_RIGHT - MARGIN - size.x / 2.0,
            LIMIT_BOTTOM - MARGIN - size.y / 2.0,
        )
    } else if tag("UpperLeft") {
        (
            LIMIT_LEFT + MARGIN + size.x / 2.0,
            LIMIT_BOTTOM - MARGIN - size.y / 2.0,
        )
    } else if tag("LowerCenter") {
        (PSX.x / 2.0, 8.0 + MARGIN + size.y / 2.0)
    } else if tag("UpperCenter") {
        (PSX.x / 2.0, LIMIT_BOTTOM - MARGIN - size.y / 2.0)
    } else if tag("DialogPosition") {
        (PSX.x / 2.0, 150.0)
    } else {
        (PSX.x / 2.0, PSX.y / 2.0)
    };
    (
        Vec2::new(cx - size.x / 2.0, PSX.y - cy - size.y / 2.0),
        None,
    )
}

fn spawn_window(
    world: &mut World,
    view: &mut DialogView,
    root: Entity,
    dialog: &OpenDialog,
    anchor: Option<[f32; 2]>,
    (cursor, visible): (usize, usize),
    z: f32,
) {
    let key = {
        let mut images = world.resource_mut::<Assets<Image>>();
        render(view, &mut images, dialog)
    };
    let Some(full) = key.and_then(|key| view.cache.get(&key)) else {
        return;
    };
    // Mid-typing the picture is drawn fresh with the glyphs so far (freed when replaced).
    let typing = if visible < full.glyphs {
        let mut images = world.resource_mut::<Assets<Image>>();
        render_typing(view, &mut images, dialog, visible)
    } else {
        None
    };
    let Some(full) = view.cache.get(&render_key(dialog)) else {
        return;
    };
    let rendered = typing.as_ref().unwrap_or(full);
    let border = Vec2::splat(BORDER_PADDING as f32) / RESOURCE_X_MULTIPLIER;
    let size = rendered.body;
    let (top_left, tail) = place(dialog, size, anchor);
    // PSX (320x224, y down) -> frame (320x240) -> camera-local (y up).
    let frame_y_offset = (FRAME.y - PSX.y) / 2.0;
    let local =
        |x: f32, y: f32| Vec3::new(x - FRAME.x / 2.0, FRAME.y / 2.0 - (y + frame_y_offset), 0.0);
    // The picture includes the border, which hangs outside the body.
    let picture_size = rendered.texels / TEXELS;
    world.spawn((
        Sprite {
            image: rendered.image.clone(),
            custom_size: Some(picture_size),
            ..default()
        },
        Anchor::TOP_LEFT,
        Transform::from_translation(
            local(top_left.x - border.x, top_left.y - border.y) + Vec3::Z * z,
        ),
        ChildOf(root),
    ));
    if let Some(text) = caption(dialog.flags)
        && let Some(Some(assets)) = view.assets.as_ref()
    {
        // The caption tab: a small window of the caption text on the
        // top left corner of the window, over its border.
        let texts = [WindowText {
            x: 0.0,
            y: 0.0,
            text: text.to_owned(),
            color: [1.0; 4],
            right: false,
        }];
        let picture = assets.render_window(
            assets.text_window_width(text),
            (LINE_HEIGHT + PHRASE_Y_PADDING) as i32,
            &texts,
        );
        let step = RESOURCE_X_MULTIPLIER / TEXELS;
        let (pw, ph) = (
            ((picture.width as f32 / step).round() as usize).max(1),
            ((picture.height as f32 / (RESOURCE_Y_MULTIPLIER / TEXELS)).round() as usize).max(1),
        );
        let data = downscale(&picture.rgba, picture.width, picture.height, pw, ph);
        let mut image = Image::new(
            Extent3d {
                width: pw as u32,
                height: ph as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        image.sampler = ImageSampler::linear();
        let handle = world.resource_mut::<Assets<Image>>().add(image);
        let tab = Vec2::new(pw as f32, ph as f32) / TEXELS;
        world.spawn((
            Sprite {
                image: handle,
                custom_size: Some(tab),
                ..default()
            },
            Anchor::TOP_LEFT,
            Transform::from_translation(
                // Its bottom edge on the window's top border.
                local(top_left.x - border.x, top_left.y - tab.y + border.y * 2.0)
                    + Vec3::Z * (z + 0.03),
            ),
            ChildOf(root),
        ));
    }
    if let Some((tail_x, upper)) = tail
        && dialog.flags & (FLAG_NO_TAIL | FLAG_TRANSPARENT) == 0
    {
        // A marker for the tail, on the edge facing the speaker.
        let y = if upper {
            top_left.y
        } else {
            top_left.y + size.y
        };
        world.spawn((
            Sprite::from_color(Color::srgb(0.85, 0.88, 1.0), Vec2::splat(6.0)),
            Transform::from_translation(local(tail_x, y) + Vec3::Z * (z + 0.01))
                .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_4)),
            ChildOf(root),
        ));
    }
    if let Some(start) = rendered.choice_line.filter(|_| dialog.choosing) {
        let lines = rendered.line_tops.len();
        let row = (start + cursor).min(lines.saturating_sub(1));
        // Label top is `PHRASE_Y_PADDING / 2` below the body top; a line is
        // `LINE_HEIGHT` tall.
        let ui_y = PHRASE_Y_PADDING / 2.0 + rendered.line_tops[row] + LINE_HEIGHT / 2.0;
        let y = top_left.y + ui_y / RESOURCE_Y_MULTIPLIER;
        // A diamond stands in for the hand pointer.
        world.spawn((
            Sprite::from_color(Color::srgb(1.0, 0.95, 0.6), Vec2::splat(5.0)),
            Transform::from_translation(local(top_left.x + 4.0, y) + Vec3::Z * (z + 0.02))
                .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_4)),
            ChildOf(root),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::{FLAG_ATE, FLAG_CHAT, FLAG_MOG, caption, message_tick};

    #[test]
    fn mognet_and_ate_windows_get_a_caption_unless_chat_style() {
        // `ETb.FlagsToStyles`.
        assert_eq!(caption(FLAG_MOG), Some("Mognet"));
        assert_eq!(caption(FLAG_ATE), Some("Active Time Event"));
        assert_eq!(caption(FLAG_MOG | FLAG_ATE), Some("Mognet"));
        assert_eq!(caption(FLAG_MOG | FLAG_CHAT), None);
        assert_eq!(caption(0), None);
    }

    #[test]
    fn the_message_speed_picks_the_animation_tick() {
        // `Dialog.DialogTextAnimationTick`: 4, 7, 10, 16, 24, 40, 64, 64.
        assert_eq!(message_tick(0), 4.0);
        assert_eq!(message_tick(3), 16.0);
        assert_eq!(message_tick(6), 64.0);
        assert_eq!(message_tick(99), 64.0);
    }
}
