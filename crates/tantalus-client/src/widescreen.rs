//! Widescreen (16:9 and wider) as a default, with a 4:3 option.
//!
//! The setting is `--aspect 16:9|16:10|21:9|4:3|auto` (or `aspect = ...` in
//! `tantalus.toml`); `auto`, the default, follows the window or capture size.
//!
//! Memoria's field widescreen (`FieldMap.OnWidescreenSupportChanged`,
//! `BGCAM_DEF.RefreshCache`, `NarrowMapList`, `PSXCameraAspect`) shows more of
//! the pre-rendered art where it is wide enough: the scroll range shrinks by
//! half the extra width on each side so the view stays inside the art, fields
//! whose art is narrower than the view are pillarboxed at the art's width,
//! and a few cameras (`RestrictedCams`) are pillarboxed at a fixed width.
//! Battle and world widen the 3D view only: the vertical field of view is
//! the vanilla one and the horizontal one grows with the aspect. Movies
//! keep their own aspect.
//!
//! The field frame here is 240 rows tall (`window::FIELD_FRAME`), so the
//! view is `240 * aspect` art pixels wide; `4:3` gives the classic 320.

use crate::widescreen_data::{CAMERA_MARGINS, MAP_WIDTHS, RESTRICTED_CAMS, RESTRICTED_SCENES};
use std::sync::atomic::{AtomicI32, AtomicU32, AtomicU64, Ordering};

/// Rows of the field frame (`window::FIELD_FRAME`).
pub const FRAME_HEIGHT: f32 = 240.0;
/// The classic frame width.
pub const CLASSIC_WIDTH: f32 = 320.0;
/// `Graphics.InitializeWidescreenSupport`: below this aspect the classic frame is used.
const MIN_WIDE: f32 = 1.34;
/// Memoria's `PsxScreenHeightNative`.
const PSX_SCREEN_HEIGHT: f32 = 220.0;
/// The widest view drawn (32:9); wider windows are pillarboxed.
const MAX_ASPECT: f32 = 32.0 / 9.0;

/// The `--aspect` setting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AspectSetting {
    /// Follow the window (or capture) size.
    Auto,
    /// A fixed aspect ratio; the picture is letterboxed to it.
    Fixed(f32),
}

impl AspectSetting {
    /// `16:9`, `16:10`, `21:9`, `4:3`, a plain ratio like `1.78`, or `auto`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("auto") {
            return Ok(Self::Auto);
        }
        let ratio = match text.split_once([':', 'x', '/']) {
            Some((w, h)) => w
                .trim()
                .parse::<f32>()
                .ok()
                .zip(h.trim().parse::<f32>().ok())
                .map(|(w, h)| w / h),
            None => text.parse::<f32>().ok(),
        };
        match ratio {
            Some(ratio) if ratio.is_finite() && (1.0..=8.0).contains(&ratio) => {
                Ok(Self::Fixed(ratio))
            }
            _ => Err(format!(
                "aspect wants 16:9, 16:10, 21:9, 4:3 or auto, got `{text}`"
            )),
        }
    }
}

static SETTING: AtomicU32 = AtomicU32::new(0);
/// The target's pixel size, `width << 32 | height`; 0 until a view sets it.
static TARGET: AtomicU64 = AtomicU64::new(0);
/// The current field view's width in art pixels (bits of an `f32`).
static FIELD_WIDTH: AtomicU32 = AtomicU32::new(0);
/// The field being drawn and its effective art width (`NarrowMapList.MapWidth`).
static FIELD_MAP: AtomicI32 = AtomicI32::new(-1);
static FIELD_CAMERA: AtomicI32 = AtomicI32::new(0);
static FIELD_MAP_WIDTH: AtomicI32 = AtomicI32::new(500);
/// `EBin.SC_COUNTER_SVR` of the field drawn, for the story-dependent follow cases.
static FIELD_COUNTER: AtomicI32 = AtomicI32::new(0);

/// The config file's name, read from the working directory.
pub const CONFIG_FILE: &str = "tantalus.toml";

/// The `aspect = "16:9"` entry of a config file's text, if any.
pub fn config_aspect(text: &str) -> Option<Result<AspectSetting, String>> {
    text.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "aspect").then(|| {
            let value = value.split('#').next().unwrap_or("");
            AspectSetting::parse(value.trim().trim_matches(['"', '\'']))
        })
    })
}

/// Settles the aspect setting at startup: `--aspect X` (removed from
/// `args`), else `TANTALUS_ASPECT`, else the `aspect` entry of
/// `tantalus.toml` in the working directory, else auto.
pub fn take_aspect_flag(args: &mut Vec<std::ffi::OsString>) -> Result<(), String> {
    let mut flag = None;
    if let Some(at) = args.iter().position(|arg| arg == "--aspect") {
        let value = args
            .get(at + 1)
            .and_then(|value| value.to_str())
            .ok_or("--aspect wants a value: 16:9, 16:10, 21:9, 4:3 or auto")?
            .to_owned();
        args.drain(at..=at + 1);
        flag = Some(value);
    }
    let setting = if let Some(value) = flag {
        AspectSetting::parse(&value)?
    } else if let Ok(value) = std::env::var("TANTALUS_ASPECT") {
        AspectSetting::parse(&value)?
    } else if let Some(entry) = std::fs::read_to_string(CONFIG_FILE)
        .ok()
        .and_then(|text| config_aspect(&text))
    {
        entry?
    } else {
        AspectSetting::Auto
    };
    set_setting(setting);
    Ok(())
}

/// Sets the `--aspect` setting (the default is auto).
pub fn set_setting(setting: AspectSetting) {
    let bits = match setting {
        AspectSetting::Auto => 0,
        AspectSetting::Fixed(ratio) => ratio.to_bits(),
    };
    SETTING.store(bits, Ordering::Relaxed);
}

pub fn setting() -> AspectSetting {
    match SETTING.load(Ordering::Relaxed) {
        0 => AspectSetting::Auto,
        bits => AspectSetting::Fixed(f32::from_bits(bits)),
    }
}

/// Tells the view code the size in pixels of what it draws to.
pub fn set_target(width: u32, height: u32) {
    TARGET.store(
        u64::from(width) << 32 | u64::from(height),
        Ordering::Relaxed,
    );
}

pub fn target() -> Option<(u32, u32)> {
    let packed = TARGET.load(Ordering::Relaxed);
    let (width, height) = ((packed >> 32) as u32, packed as u32);
    (width > 0 && height > 0).then_some((width, height))
}

fn clamp_aspect(raw: f32) -> f32 {
    if raw < MIN_WIDE {
        4.0 / 3.0
    } else {
        raw.min(MAX_ASPECT)
    }
}

/// The aspect the picture is drawn at: the fixed setting, or the target's
/// (4:3 before a target is known). Below 1.34 it is the classic 4:3, and it
/// stops at 32:9.
pub fn view_aspect() -> f32 {
    clamp_aspect(match setting() {
        AspectSetting::Fixed(ratio) => ratio,
        AspectSetting::Auto => target().map_or(4.0 / 3.0, |(w, h)| w as f32 / h as f32),
    })
}

/// The part of a `width` x `height` target the picture covers: the largest
/// centered rectangle of [`view_aspect`] (the whole target when it already
/// has that aspect). `(x, y, width, height)` in pixels.
pub fn screen_rect(width: u32, height: u32) -> (u32, u32, u32, u32) {
    fit_rect(width, height, view_aspect())
}

/// The largest centered rectangle of `aspect` in `width` x `height`.
pub fn fit_rect(width: u32, height: u32, aspect: f32) -> (u32, u32, u32, u32) {
    let (w, h) = (width as f32, height as f32);
    let (fw, fh) = if w / h > aspect {
        (h * aspect, h)
    } else {
        (w, w / aspect)
    };
    let (fw, fh) = (fw.round().min(w) as u32, fh.round().min(h) as u32);
    ((width - fw) / 2, (height - fh) / 2, fw, fh)
}

static LIVE: AtomicU64 = AtomicU64::new(0);

fn computed_live_size() -> (u32, u32) {
    let width = (480.0 * view_aspect()).round() as u32;
    (width & !1, 480)
}

/// The size of the battle and world frame: 480 rows, as wide as the aspect
/// asks (640x480 at 4:3, the size `LIVE_SIZE` used to be). Fixed by
/// [`latch_live_size`] while a live view is up, so a resized window does not
/// change the frame under it.
pub fn live_size() -> (u32, u32) {
    let packed = LIVE.load(Ordering::Relaxed);
    if packed == 0 {
        return computed_live_size();
    }
    ((packed >> 32) as u32, packed as u32)
}

/// Fixes [`live_size`] at the current aspect: call when a live battle or
/// world view starts.
pub fn latch_live_size() {
    let (w, h) = computed_live_size();
    LIVE.store(u64::from(w) << 32 | u64::from(h), Ordering::Relaxed);
}

/// Whether the window's aspect has moved on from the one [`latch_live_size`]
/// fixed, so a live battle or world view should redraw at the new one.
pub fn live_size_stale() -> bool {
    let packed = LIVE.load(Ordering::Relaxed);
    packed != 0 && live_size() != computed_live_size()
}

/// The width of the battle and world picture in PSX pixels for a frame of
/// `width` x `height`: 320 at 4:3 (`PsxScreenWidthNative`), growing with the
/// aspect while the 220 rows stay (`PsxScreenWidth = 220 * aspect`, kept
/// proportional to the 4:3 baseline).
pub fn psx_width(width: u32, height: u32) -> f32 {
    match setting() {
        AspectSetting::Fixed(_) => 240.0 * view_aspect(),
        AspectSetting::Auto => 240.0 * clamp_aspect(width as f32 / height.max(1) as f32),
    }
}

/// `NarrowMapList.MapWidth`'s fixed list, 500 for a field not listed.
fn listed_width(map: i32) -> i32 {
    MAP_WIDTHS
        .iter()
        .find(|(field, _)| *field == map)
        .map_or(500, |(_, width)| *width)
}

/// What the scripts' globals say about the scene, for the story-dependent
/// narrow scenes (`NarrowMapList.ConditionalForceNarrow`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SceneState {
    /// `EBin.MAP_INDEX_SVR`: the global `Int16` at byte 2.
    pub map_index: i32,
    /// `EBin.SC_COUNTER_SVR`: the scenario counter.
    pub counter: i32,
}

impl SceneState {
    /// Reads the two globals from the game's global memory bytes.
    pub fn from_globals(bytes: &[u8]) -> Self {
        let word = |at: usize| bytes.get(at..at + 2).map_or([0, 0], |b| [b[0], b[1]]);
        Self {
            counter: i32::from(u16::from_le_bytes(word(0))),
            map_index: i32::from(i16::from_le_bytes(word(2))),
        }
    }
}

fn forced_narrow(map: i32, state: SceneState, screen_width: f32) -> bool {
    RESTRICTED_SCENES
        .iter()
        .any(|&(field, max, index, counter)| {
            field == map
                && (screen_width > max as f32 || max == -5)
                && (index == state.map_index || index == -5)
                && (counter == state.counter || counter == -5)
        })
}

/// The width in art pixels of the view of field `map` on camera `camera` at
/// `aspect`: the whole window width where the art is wide enough, else the
/// art's width (pillarboxed), never below the classic 320.
pub fn field_view_width(map: i32, camera: i32, state: SceneState, aspect: f32) -> f32 {
    if aspect < MIN_WIDE {
        return CLASSIC_WIDTH;
    }
    let wide = FRAME_HEIGHT * aspect;
    // Memoria measures against its 224 and 220 row frames.
    let screen_width = PSX_SCREEN_HEIGHT * aspect;
    let mut art = if forced_narrow(map, state, screen_width) {
        320
    } else {
        listed_width(map)
    };
    if (3000..=3012).contains(&map) {
        art = 320;
    }
    let mut width = if art as f32 <= wide {
        (art as f32).max(CLASSIC_WIDTH)
    } else {
        wide
    };
    // `PSXCameraAspect.LateUpdate`: these cameras are narrower than a 16:9 screen.
    if screen_width < 401.0 {
        for &(field, cam, restricted) in RESTRICTED_CAMS {
            if field == map && cam == camera {
                width = width.min(restricted as f32);
            }
        }
    }
    width
}

/// Memoria's `ActualPsxScreenWidth` (220 rows times the aspect) when the
/// widescreen support is on, else `None`: what the layer fixes in
/// `widescreen_layers` read.
pub fn layer_screen_width() -> Option<f32> {
    let aspect = view_aspect();
    (aspect >= MIN_WIDE).then_some(PSX_SCREEN_HEIGHT * aspect)
}

/// `BGVPORT` for field 2220 (Desert Palace) when the view is wide: Memoria moves the camera's
/// left bound from 154 to 544 so the wide view does not see the hidden path too soon
/// (`EventEngine.DoEventCode.cs`, `BGVPORT`; `ActualPsxScreenWidth > 390`).
pub fn camera_min_x(requested: i16) -> i16 {
    if FIELD_MAP.load(Ordering::Relaxed) == 2220
        && requested == 154
        && PSX_SCREEN_HEIGHT * view_aspect() > 390.0
    {
        544
    } else {
        requested
    }
}

/// Notes the field being drawn, so the scroll range and the camera agree.
/// Call before [`crate::view_scroll::ViewScroll::view`] each frame.
pub fn set_field(map: i32, camera: i32, state: SceneState) {
    let aspect = view_aspect();
    let width = field_view_width(map, camera, state, aspect);
    FIELD_WIDTH.store(width.to_bits(), Ordering::Relaxed);
    FIELD_MAP.store(map, Ordering::Relaxed);
    FIELD_CAMERA.store(camera, Ordering::Relaxed);
    FIELD_COUNTER.store(state.counter, Ordering::Relaxed);
    FIELD_MAP_WIDTH.store(
        effective_map_width(map, state, PSX_SCREEN_HEIGHT * aspect),
        Ordering::Relaxed,
    );
}

/// `NarrowMapList.MapWidth`: 320 for a scene forced narrow (and the ending
/// fields), else the listed width.
fn effective_map_width(map: i32, state: SceneState, screen_width: f32) -> i32 {
    if forced_narrow(map, state, screen_width) || (3000..=3012).contains(&map) {
        320
    } else {
        listed_width(map)
    }
}

/// The width of the current field view (320 until a field is set).
pub fn field_width() -> f32 {
    match FIELD_WIDTH.load(Ordering::Relaxed) {
        0 => CLASSIC_WIDTH,
        bits => f32::from_bits(bits),
    }
}

/// `BGCAM_DEF.RefreshCache`: how much the scroll range shrinks on each side
/// for a view `width` wide, given the range's own span.
pub fn range_delta(width: f32, span: f32) -> f32 {
    ((width - CLASSIC_WIDTH).min(span) / 2.0).max(0.0)
}

/// `NarrowMapList.mapCameraMargin`: pixels cropped on each side of a field's scroll range.
pub fn camera_margin(map: i32) -> f32 {
    CAMERA_MARGINS
        .iter()
        .find(|(field, _)| *field == map)
        .map_or(0.0, |(_, margin)| *margin as f32)
}

/// Whether a field's follow position is set outright rather than clamped
/// to the scroll range (`FieldMap.CenterCameraOnPlayer`'s final `switch`).
pub fn follow_is_fixed() -> bool {
    matches!(FIELD_MAP.load(Ordering::Relaxed), 456 | 505 | 1153) && view_aspect() >= MIN_WIDE
}

/// `FieldMap.CenterCameraOnPlayer`'s widescreen block for the field drawn
/// ([`set_field`]): the view's center x `center` (adapter space, 0 at the
/// middle of a classic 320 frame) of a camera whose art is `art` wide, in
/// the position Memoria's hand-tuned cases give it. Memoria's number is the
/// view's left edge in art pixels, `center + 160 - view / 2` here.
pub fn follow_center(center: f32, art: f32) -> f32 {
    let field = (
        FIELD_MAP.load(Ordering::Relaxed),
        FIELD_CAMERA.load(Ordering::Relaxed),
        FIELD_MAP_WIDTH.load(Ordering::Relaxed) as f32,
    );
    follow_center_in(field, (view_aspect(), field_width()), center, art)
}

/// `FieldMap.CenterCameraOnPlayer`'s widescreen cases on the camera's y for the field drawn
/// ([`set_field`]). `y` is the view's center y in adapter space (0 with the frame's top at
/// the art's top, down negative), which is Memoria's `CamPositionY` (`centerOffset[1] -
/// charOffset.y`, that is `HalfFieldHeight - realVrp.y`).
pub fn follow_y(y: f32) -> f32 {
    follow_y_in(
        FIELD_MAP.load(Ordering::Relaxed),
        FIELD_COUNTER.load(Ordering::Relaxed),
        view_aspect(),
        y,
    )
}

/// [`follow_y`] for field `map` at scenario counter `counter` and `aspect`.
fn follow_y_in(map: i32, counter: i32, aspect: f32, y: f32) -> f32 {
    if aspect < MIN_WIDE {
        return y;
    }
    match map {
        // Lindblum Castle/Castle Bridge, except the scene at counter 3110: the camera climbs
        // along a parabola from the bottom of the art.
        609 | 2159 if counter != 3110 => (y + 224.0) * (y + 224.0) / 2.0 - 224.0,
        // Pandemonium: Kuja's descent, the camera kept from rising too high.
        2716 => y.min(0.0),
        _ => y,
    }
}

/// [`follow_center`] for `(map, camera, map width)` at `(aspect, view width)`.
fn follow_center_in(
    (map, camera, map_width): (i32, i32, f32),
    (aspect, view): (f32, f32),
    center: f32,
    art: f32,
) -> f32 {
    if aspect < MIN_WIDE {
        return center;
    }
    let to_left = view / 2.0 - CLASSIC_WIDTH / 2.0;
    let mut left = center - to_left;
    if map == 931 {
        return center;
    }
    let ten = (aspect - 1.6).abs() < 0.01;
    let mut thresh = (art - view).min(0.0);
    match map {
        507 => left += 1.0, // Cargo Ship/Deck
        1205 | 1652 | 2552 | 154 | 1215 | 1807 | 1055 => {
            // Alexandria Castle/Chapel, Iifa Tree/Roots, Earth Shrine, grand hall.
            if map == 1652 && camera == 0 {
                thresh += 16.0; // Iifa Tree/Roots
            }
            if map == 1055 {
                thresh += 1.0;
            }
            let right = art - view - thresh;
            left = left.max(thresh).min(right);
        }
        _ if map_width > view => {
            if CAMERA_MARGINS.iter().any(|(field, _)| *field == map) {
                thresh = camera_margin(map);
            }
            let mut right = art - view - thresh;
            match map {
                103 | 1853 | 2053 | 2606 => thresh += 16.0, // Alexandria center, Brahne's fleet
                2903 => right -= 32.0,                      // Memoria castle
                2923 => thresh += 20.0,                     // Crystal World
                _ => {}
            }
            left = left.max(thresh).min(right);
        }
        _ if map_width <= view && map_width > CLASSIC_WIDTH => {
            left = ((art - map_width) / 2.0).floor();
        }
        _ => {}
    }
    match map {
        312 => left += 1.0, // Ice Cavern exit (white line left)
        456 => left = if ten { 195.0 } else { 160.0 }, // Dali Mountain/Summit
        505 => left = if ten { 70.0 } else { 105.0 }, // Cargo Ship
        1153 => left = if ten { 140.0 } else { 175.0 }, // Rose Rouge cockpit
        2903 if PSX_SCREEN_HEIGHT * aspect > 510.0 => left = 0.0, // Memoria castle
        _ => {}
    }
    left + to_left
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aspects_parse() {
        assert_eq!(AspectSetting::parse("auto"), Ok(AspectSetting::Auto));
        let Ok(AspectSetting::Fixed(ratio)) = AspectSetting::parse("16:9") else {
            panic!()
        };
        assert!((ratio - 16.0 / 9.0).abs() < 1e-6);
        assert!(AspectSetting::parse("wide").is_err());
    }

    #[test]
    fn field_widths_follow_memoria() {
        let any = SceneState::default();
        let a169 = 16.0 / 9.0;
        // Unlisted field: the whole view.
        assert!((field_view_width(101, 0, any, a169) - 240.0 * a169).abs() < 0.01);
        // A restricted camera of a wide field is pillarboxed at 352.
        assert_eq!(field_view_width(154, 0, any, a169), 352.0);
        // Classic aspect is 320 whatever the field.
        assert_eq!(field_view_width(101, 0, any, 4.0 / 3.0), 320.0);
        // A restricted camera on 16:9.
        assert_eq!(field_view_width(116, 1, any, a169), 320.0);
        assert!(field_view_width(116, 0, any, a169) > 400.0);
    }

    #[test]
    fn config_entry_is_read() {
        let text = "# video
aspect = \"21:9\" # ultrawide
";
        let Some(Ok(AspectSetting::Fixed(ratio))) = config_aspect(text) else {
            panic!()
        };
        assert!((ratio - 21.0 / 9.0).abs() < 1e-6);
        assert!(config_aspect("other = 1").is_none());
    }

    #[test]
    fn follow_cases_place_the_view() {
        let a169 = 16.0 / 9.0;
        let view = 240.0 * a169;
        // Field 456 (Dali Mountain/Summit): the view's left edge is fixed at art x 160.
        let at = follow_center_in((456, 0, 398.0), (a169, view), 0.0, 700.0);
        assert!((at - view / 2.0).abs() < 0.01, "{at} {view}");
        // Field 507 gets one pixel to the right; 4:3 changes nothing.
        let moved = follow_center_in((507, 0, 500.0), (a169, view), 0.0, 700.0);
        assert!((moved - 1.0).abs() < 0.01, "{moved}");
        assert_eq!(
            follow_center_in((456, 0, 398.0), (4.0 / 3.0, 320.0), 7.0, 700.0),
            7.0
        );
    }

    #[test]
    fn follow_y_cases() {
        let a169 = 16.0 / 9.0;
        // 609 and 2159 (Castle Bridge): the bottom of the art holds, then the parabola climbs.
        assert_eq!(follow_y_in(609, 0, a169, -224.0), -224.0);
        assert_eq!(follow_y_in(2159, 0, a169, -220.0), -216.0);
        assert_eq!(follow_y_in(609, 3110, a169, -220.0), -220.0);
        assert_eq!(follow_y_in(609, 0, 4.0 / 3.0, -220.0), -220.0);
        // 2716: never above the top of the art.
        assert_eq!(follow_y_in(2716, 0, a169, 16.0), 0.0);
        assert_eq!(follow_y_in(2716, 0, a169, -40.0), -40.0);
    }

    #[test]
    fn rects_letterbox() {
        assert_eq!(fit_rect(1280, 720, 4.0 / 3.0), (160, 0, 960, 720));
        assert_eq!(fit_rect(640, 480, 16.0 / 9.0), (0, 60, 640, 360));
    }
}
