//! CPU dialog renderer for decoded `.mes` entries (milestone A8).
//!
//! Every constant below is tied to a source or to install data; anything that is not is
//! listed in [`UNKNOWNS`] and in `audit/text-rendering.md`.
//!
//! Sources:
//! - Glyphs: the install's own `p_fa.mpc` TextAsset (`resources.assets`), a `UnityWeb`
//!   bundle holding the TrueType font `TBUDGoStd-Bold`. `EncryptFontManager.LoadFont`
//!   (`Memoria/Assets/EncryptFontManager.cs`) reads `EmbeddedAsset/FA/<platform>_fa.mpc`,
//!   runs `ByteEncryption.Decryption` and loads `TBUDGoStd-Bold` as the default font for
//!   every `UILabel`.
//! - Label settings: the dialog prefab's `Phrase` UILabel in `sharedassets2.assets`
//!   (path id 338): font size 42, spacing Y 26, left aligned, shadow effect, effect
//!   distance (4, 4), effect color (40, 40, 40) / 255. 42 + 26 = 68 equals
//!   `Dialog.DialogLineHeight`.
//! - Layout: `NGUIText.WrapText` (no width wrap for dialogs: only explicit `\n` breaks a
//!   line), `NGUIText.GenerateTextRender`, `DialogBoxSymbols.ParseSingleGenericTag`,
//!   `Dialog.AutomaticSize`, `Dialog.Width`, `Dialog.LineNumber`.
//! - Frame: `Body` (sprite `dialog_bg`, tiled) and `Border` (sprite `dialog_frame_chat`,
//!   sliced, anchored 18 px outside the body) UISprites of the same prefab
//!   (`sharedassets2.assets` path ids 340 and 339), drawn from `Blue Atlas` or
//!   `Gray Atlas` (`FF9UIDataTool.WindowAtlas`).

use super::mes::{MesEntry, MesToken};
use super::ui_atlas::UiSprite;
use serde::Serialize;
use std::cell::RefCell;
use std::collections::HashMap;

/// `Phrase` UILabel `mFontSize`.
pub const FONT_SIZE: i32 = 42;
/// `Phrase` UILabel `mSpacingY`.
pub const SPACING_Y: i32 = 26;
/// `Phrase` UILabel `mSpacingX` (the field after `mSpacingY` is also 0 in the prefab).
pub const SPACING_X: i32 = 0;
/// `Dialog.DialogLineHeight`; equals `FONT_SIZE + SPACING_Y`.
pub const LINE_HEIGHT: f32 = 68.0;
/// `Dialog.DialogPhraseXPadding`.
pub const PHRASE_X_PADDING: f32 = 16.0;
/// `Dialog.DialogPhraseYPadding`.
pub const PHRASE_Y_PADDING: f32 = 20.0;
/// `Dialog.BorderPadding`; also the `Border` sprite's anchor offset in the prefab.
pub const BORDER_PADDING: i32 = 18;
/// `UIManager.GetUIContentSize` (non-widescreen branch) over `FieldMap.PsxFieldWidth`.
pub const RESOURCE_X_MULTIPLIER: f32 = 1543.0 / 320.0;
/// Same, vertical: 1080 over `PsxFieldHeightNative` (224).
pub const RESOURCE_Y_MULTIPLIER: f32 = 1080.0 / 224.0;
/// `Phrase` UILabel effect color (shadow), 40/255 per channel.
pub const SHADOW_COLOR: [f32; 4] = [0.156_862_75, 0.156_862_75, 0.156_862_75, 1.0];
/// `Phrase` UILabel `mEffectDistance`.
pub const SHADOW_DISTANCE: (f32, f32) = (4.0, 4.0);
/// `FF9TextTool.White`, the label's `DefaultTextColor`.
pub const DEFAULT_TEXT_COLOR: [f32; 4] = [0.784_313_74, 0.784_313_74, 0.784_313_74, 1.0];
/// Body sprite name in the prefab.
pub const BODY_SPRITE: &str = "dialog_bg";
/// Border sprite for `WindowStyleAuto` and `WindowStyleNoTail` (`Dialog.Style`).
pub const BORDER_SPRITE: &str = "dialog_frame_chat";

/// Facts this renderer cannot establish from the pinned source or the install.
pub const UNKNOWNS: &[&str] = &[
    "Glyph pixels come from fontdue (grayscale coverage, no hinting). Unity rasterizes dynamic fonts with FreeType; pixel-exact agreement is not claimed. Advance widths are rounded to integers because Unity's CharacterInfo.advance is an int.",
    "The Windows config `win_type` (Gray vs Blue window atlas) default is not in the pinned source; Blue is the default here.",
    "Auto versus hint dialog sizing: Dialog.CanAutoResize depends on event-engine state (window id, field map, window style); both modes are available, Auto is the default.",
    "Dynamic text (character names, party names, [NUMB], [ITEM], [TEXT]) needs game state; those tags are recorded, not substituted.",
    "Icons, sprites, buttons, choice indents, alignment tags and the tail pointer are recorded, not drawn.",
    "Text speed, wait, time and sound markers are recorded with their glyph index, not executed.",
    "Dialog position on screen (DialogPosition, tail placement) is not modeled; the image is the box alone.",
];

/// One rasterized glyph in Unity `CharacterInfo` terms (y up, relative to the baseline).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlyphBitmap {
    pub advance: i32,
    pub min_x: i32,
    pub min_y: i32,
    pub width: usize,
    pub height: usize,
    /// `width * height` coverage values, top row first.
    pub coverage: Vec<u8>,
}

impl GlyphBitmap {
    pub fn max_y(&self) -> i32 {
        self.min_y + self.height as i32
    }
}

/// Where glyphs come from. `None` means the font has no glyph (Unity's
/// `GetCharacterInfo` fails and `NGUIText` skips the character).
pub trait GlyphSource {
    fn glyph(&self, ch: char) -> Option<GlyphBitmap>;
}

/// `NGUIText.UpdateFontSizes`: the baseline offset measured from `)` (or `A`).
pub fn measure_baseline(source: &dyn GlyphSource, font_size: i32) -> i32 {
    let reference = source
        .glyph(')')
        .filter(|glyph| glyph.max_y() != 0)
        .or_else(|| source.glyph('A'));
    match reference {
        // Mathf.Round rounds half to even.
        Some(glyph) => {
            (((glyph.min_y + glyph.max_y() + font_size) as f32) * 0.5).round_ties_even() as i32
        }
        None => 0,
    }
}

/// `fontdue` backed glyph source with a per-character cache.
pub struct FontdueSource {
    font: fontdue::Font,
    px: f32,
    cache: RefCell<HashMap<char, Option<GlyphBitmap>>>,
}

impl FontdueSource {
    pub fn from_ttf(bytes: &[u8], px: f32) -> Result<Self, String> {
        let font = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
            .map_err(|error| format!("font parse failed: {error}"))?;
        Ok(Self {
            font,
            px,
            cache: RefCell::new(HashMap::new()),
        })
    }
}

impl GlyphSource for FontdueSource {
    fn glyph(&self, ch: char) -> Option<GlyphBitmap> {
        if let Some(hit) = self.cache.borrow().get(&ch) {
            return hit.clone();
        }
        let glyph = if self.font.lookup_glyph_index(ch) == 0 && ch != '\0' {
            None
        } else {
            let (metrics, coverage) = self.font.rasterize(ch, self.px);
            Some(GlyphBitmap {
                advance: metrics.advance_width.round() as i32,
                min_x: metrics.xmin,
                min_y: metrics.ymin,
                width: metrics.width,
                height: metrics.height,
                coverage,
            })
        };
        self.cache.borrow_mut().insert(ch, glyph.clone());
        glyph
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeMode {
    /// `Dialog.AutomaticSize` over the measured pages.
    Auto,
    /// `Dialog.WidthHint` / `LineNumberHint` from the `[STRT=w,l]` tag.
    Hint,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlacedGlyph {
    pub ch: char,
    /// Pen x in label space (pixels from the label's left edge).
    pub x: f32,
    /// Top of the line (plus tag y offsets) in label space.
    pub y: f32,
    pub color: [f32; 4],
    /// `[HSHD]` active: text is nudged by a third of the shadow distance.
    pub high_shadow: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LineInfo {
    pub base_y: f32,
    pub width: f32,
    pub height_below: f32,
}

/// A tag the renderer saw but did not execute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecordedTag {
    pub page: usize,
    /// Number of glyphs placed on this page before the tag.
    pub glyph_index: usize,
    pub code: String,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PageLayout {
    /// Line the `[CHOO]` answers start on, when the page has them.
    pub choice_line: Option<usize>,
    pub glyphs: Vec<PlacedGlyph>,
    pub lines: Vec<LineInfo>,
    /// `TextParser.MaxWidth`.
    pub max_width: f32,
    /// `TextParser.RawHeight`.
    pub raw_height: f32,
    /// `TextParser.DialogLineCount`.
    pub line_count: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DialogLayout {
    pub pages: Vec<PageLayout>,
    pub width_hint: f32,
    pub line_hint: f32,
    pub size_mode: &'static str,
    /// `bodySprite.width` / `.height`.
    pub body_width: i32,
    pub body_height: i32,
    /// Speed, wait, time, sound and similar markers (recorded, not executed).
    pub timing_events: Vec<RecordedTag>,
    /// Tags that need game state or art this renderer does not model.
    pub unhandled_tags: Vec<RecordedTag>,
    /// Tags the parser could not decode at all.
    pub unknown_tags: Vec<String>,
    /// Characters the font has no glyph for (skipped, as `NGUIText` does).
    pub missing_glyphs: Vec<char>,
}

const TIMING_CODES: &[&str] = &[
    "Wait",
    "Speed",
    "Time",
    "IncreaseSignal",
    "IncreaseSignalEx",
    "Signal",
    "Sound",
    "Instantly",
    "NoAnimation",
    "Flash",
    "NoFocus",
    "TurboOff",
    "TagAnimation",
];

const PASSIVE_CODES: &[&str] = &[
    "End",
    "DialogSize",
    "TailPosition",
    "LowerRight",
    "LowerLeft",
    "UpperRight",
    "UpperLeft",
    "LowerCenter",
    "UpperCenter",
    "LowerRightForce",
    "LowerLeftForce",
    "UpperRightForce",
    "UpperLeftForce",
    "DialogPosition",
    "Position",
];

fn hex_component(text: &str) -> Option<f32> {
    u8::from_str_radix(text, 16)
        .ok()
        .map(|value| f32::from(value) * 0.003_921_569)
}

fn named_color(code: &str) -> Option<[f32; 3]> {
    Some(match code {
        "White" => [0.784_313_74, 0.784_313_74, 0.784_313_74],
        // FFIXTextTagCode.Pink maps to FF9TextTool.Magenta, Brown to Red.
        "Pink" => [0.721_568_64, 0.501_960_8, 0.878_431_4],
        "Cyan" => [0.407_843_14, 0.752_941_2, 0.847_058_83],
        "Brown" => [0.815_686_3, 0.376_470_6, 0.313_725_5],
        "Yellow" => [0.784_313_74, 0.690_196_1, 0.250_980_4],
        "Green" => [0.470_588_24, 0.784_313_74, 0.250_980_4],
        "Grey" => [0.564_705_9, 0.564_705_9, 0.564_705_9],
        _ => return None,
    })
}

fn param_f32(params: &[String], index: usize) -> f32 {
    params
        .get(index)
        .and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(0.0)
}

/// Splits an entry into pages the way `DialogBoxSymbols.ParseTextSplitTags` does: a
/// `NewPage` tag starts a page, `End` stops.
fn split_pages(entry: &MesEntry) -> Vec<Vec<&MesToken>> {
    let mut pages: Vec<Vec<&MesToken>> = vec![Vec::new()];
    for token in &entry.tokens {
        if let MesToken::Tag(tag) = token {
            match tag.code.as_str() {
                "NewPage" => {
                    pages.push(Vec::new());
                    continue;
                }
                "End" => break,
                _ => {}
            }
        }
        pages.last_mut().expect("one page").push(token);
    }
    pages
}

struct PageState {
    glyphs: Vec<PlacedGlyph>,
    lines: Vec<LineInfo>,
    colors: Vec<[f32; 4]>,
    high_shadow: bool,
    x: f32,
    y_offset: f32,
    /// Indent every line after `[CHOO]` starts with (`NGUIText.ChoiceIndent`).
    indent: f32,
    /// Line the `[CHOO]` list starts on.
    choice_line: Option<usize>,
}

/// Lays an entry out. Mirrors `NGUIText.WrapText` (dialogs never wrap by width, so the
/// only line breaks are `\n`) and the tag handling of `GenerateTextRender`.
pub fn layout_entry(
    entry: &MesEntry,
    source: &dyn GlyphSource,
    mode: SizeMode,
    min_pages: usize,
) -> DialogLayout {
    let baseline = measure_baseline(source, FONT_SIZE);
    let mut pages = Vec::new();
    let mut timing_events = Vec::new();
    let mut unhandled_tags = Vec::new();
    let mut unknown_tags = Vec::new();
    let mut missing_glyphs: Vec<char> = Vec::new();
    let mut width_hint = 0.0_f32;
    let mut line_hint = 1.0_f32;
    for (page_index, tokens) in split_pages(entry).into_iter().enumerate() {
        let mut state = PageState {
            glyphs: Vec::new(),
            lines: vec![LineInfo {
                base_y: 0.0,
                width: 0.0,
                height_below: LINE_HEIGHT,
            }],
            colors: vec![DEFAULT_TEXT_COLOR],
            high_shadow: false,
            x: 0.0,
            y_offset: 0.0,
            indent: 0.0,
            choice_line: None,
        };
        for token in tokens {
            match token {
                MesToken::Text { text } => {
                    for ch in text.chars() {
                        place_char(ch, source, baseline, &mut state, &mut missing_glyphs);
                    }
                }
                MesToken::Unknown { raw } => unknown_tags.push(raw.clone()),
                MesToken::LoadMes { name } => unhandled_tags.push(RecordedTag {
                    page: page_index,
                    glyph_index: state.glyphs.len(),
                    code: "LoadMes".to_owned(),
                    raw: format!("[LOADMES={name}]"),
                }),
                MesToken::Tag(tag) => {
                    let code = tag.code.as_str();
                    let record = |list: &mut Vec<RecordedTag>, glyphs: usize| {
                        list.push(RecordedTag {
                            page: page_index,
                            glyph_index: glyphs,
                            code: tag.code.clone(),
                            raw: tag.raw.clone(),
                        });
                    };
                    if code == "Choice" {
                        // `DialogBoxSymbols`: the list is indented by
                        // `NGUIText.ChoiceIndent` (16 * ResourceXMultipier).
                        state.indent = 16.0 * RESOURCE_X_MULTIPLIER;
                        state.x += state.indent;
                        state.choice_line = Some(state.lines.len() - 1);
                    } else if code == "DialogSize" {
                        width_hint = param_f32(&tag.params, 0);
                        line_hint = param_f32(&tag.params, 1).max(1.0);
                    } else if code == "TextRGBA" {
                        apply_rgba(&mut state, &tag.params);
                    } else if let Some(rgb) = named_color(code) {
                        let alpha = state.colors.last().map_or(1.0, |color| color[3]);
                        state.colors.push([rgb[0], rgb[1], rgb[2], alpha]);
                    } else if code == "ShadowToggle" {
                        if tag.params.is_empty() {
                            state.high_shadow = !state.high_shadow;
                        }
                    } else if code == "ShadowOff" {
                        state.high_shadow = false;
                    } else if code == "MoveCaret" || code == "Tab" {
                        // OnTab: (dx - 4) * ResourceXMultipier, dy * ResourceYMultipier.
                        let (dx, dy) = if code == "Tab" {
                            (18.0, 0.0)
                        } else {
                            (param_f32(&tag.params, 0), param_f32(&tag.params, 1))
                        };
                        state.x += (dx - 4.0) * RESOURCE_X_MULTIPLIER;
                        state.y_offset += dy * RESOURCE_Y_MULTIPLIER;
                    } else if code == "DialogF" {
                        state.x += param_f32(&tag.params, 0) * RESOURCE_X_MULTIPLIER;
                    } else if code == "DialogX" {
                        state.x = param_f32(&tag.params, 0) * RESOURCE_X_MULTIPLIER;
                    } else if code == "DialogY" {
                        state.y_offset += param_f32(&tag.params, 0) * RESOURCE_Y_MULTIPLIER;
                    } else if TIMING_CODES.contains(&code) {
                        record(&mut timing_events, state.glyphs.len());
                    } else if !PASSIVE_CODES.contains(&code) {
                        record(&mut unhandled_tags, state.glyphs.len());
                    }
                }
            }
        }
        let max_width = state.lines.iter().map(|l| l.width).fold(0.0, f32::max);
        let raw_height = state
            .lines
            .iter()
            .map(|l| l.base_y + l.height_below)
            .fold(0.0, f32::max);
        pages.push(PageLayout {
            choice_line: state.choice_line,
            glyphs: state.glyphs,
            lines: state.lines,
            max_width,
            raw_height,
            line_count: (raw_height / LINE_HEIGHT).ceil() as i32,
        });
    }
    while pages.len() < min_pages {
        pages.push(PageLayout {
            choice_line: None,
            glyphs: Vec::new(),
            lines: Vec::new(),
            max_width: 0.0,
            raw_height: 0.0,
            line_count: 0,
        });
    }
    let (body_width, body_height, size_mode) = match mode {
        SizeMode::Auto => {
            let widest = pages.iter().map(|p| p.max_width).fold(0.0, f32::max);
            let all_pages_width = (widest + PHRASE_X_PADDING * 2.0) / RESOURCE_X_MULTIPLIER;
            let lines = pages
                .iter()
                .map(|p| p.line_count as f32)
                .fold(line_hint, f32::max);
            (
                dialog_width(all_pages_width + 1.0),
                dialog_height(lines),
                "auto",
            )
        }
        SizeMode::Hint => {
            let hint = if width_hint > 0.0 {
                width_hint + 3.0
            } else {
                width_hint
            };
            (dialog_width(hint), dialog_height(line_hint), "hint")
        }
    };
    missing_glyphs.sort_unstable();
    missing_glyphs.dedup();
    DialogLayout {
        pages,
        width_hint,
        line_hint,
        size_mode,
        body_width,
        body_height,
        timing_events,
        unhandled_tags,
        unknown_tags,
        missing_glyphs,
    }
}

/// `Dialog.Width` setter: `bodySprite.width = (int)size.x`.
fn dialog_width(value: f32) -> i32 {
    let window_min_width = (RESOURCE_X_MULTIPLIER * 32.0) as i32;
    let adjust_width = (RESOURCE_X_MULTIPLIER * 3.0) as i32;
    ((value * RESOURCE_X_MULTIPLIER).max((window_min_width - adjust_width) as f32)) as i32
}

/// `Dialog.LineNumber` setter: `size.y = lines * 68 + 20`.
fn dialog_height(lines: f32) -> i32 {
    (lines.trunc() * LINE_HEIGHT + PHRASE_Y_PADDING) as i32
}

fn apply_rgba(state: &mut PageState, params: &[String]) {
    match params.len() {
        0 => {
            if state.colors.len() > 1 {
                state.colors.pop();
            }
        }
        // One parameter is a global alpha (`NGUIText.mAlpha`); not applied here.
        1 => {}
        3 | 4 => {
            let comps: Vec<f32> = params.iter().filter_map(|p| hex_component(p)).collect();
            if comps.len() == params.len() {
                let alpha = comps.get(3).copied().unwrap_or(1.0);
                state.colors.push([comps[0], comps[1], comps[2], alpha]);
            }
        }
        _ => {}
    }
}

fn place_char(
    ch: char,
    source: &dyn GlyphSource,
    baseline: i32,
    state: &mut PageState,
    missing: &mut Vec<char>,
) {
    if ch == '\r' {
        return;
    }
    if ch == '\n' {
        let base_y = state.lines.last().map_or(0.0, |line| line.base_y) + LINE_HEIGHT;
        state.lines.push(LineInfo {
            base_y,
            width: 0.0,
            height_below: LINE_HEIGHT,
        });
        state.x = state.indent;
        state.y_offset = 0.0;
        return;
    }
    if (ch as u32) < 32 {
        return;
    }
    let Some(glyph) = source.glyph(ch) else {
        missing.push(ch);
        return;
    };
    if glyph.advance == 0 {
        return;
    }
    let line = state.lines.last_mut().expect("one line");
    let advance = SPACING_X as f32 + glyph.advance as f32;
    state.glyphs.push(PlacedGlyph {
        ch,
        x: state.x,
        y: line.base_y + state.y_offset,
        color: *state.colors.last().expect("default color"),
        high_shadow: state.high_shadow,
    });
    state.x += advance;
    line.width = line.width.max(state.x);
    // charHeight = -glyph.v1.y = baseline - minY.
    let char_height = (baseline - glyph.min_y) as f32;
    // Mirrors `Math.Max(currentLine.HeightBelow, charHeight + offsetY)`; the line starts
    // at the label's own line height so the page is never shorter than its lines.
    let below = char_height + state.y_offset;
    if below > line.height_below {
        line.height_below = below;
    }
}

/// RGBA8 image, straight alpha, row-major, top row first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

impl Image {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            rgba: vec![0; width * height * 4],
        }
    }

    /// Source-over with straight alpha; `alpha` scales the source coverage.
    pub fn blend(&mut self, x: i32, y: i32, color: [u8; 3], alpha: f32) {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return;
        }
        let index = (y as usize * self.width + x as usize) * 4;
        let source_alpha = alpha.clamp(0.0, 1.0);
        let dest_alpha = f32::from(self.rgba[index + 3]) / 255.0;
        let out_alpha = source_alpha + dest_alpha * (1.0 - source_alpha);
        if out_alpha <= 0.0 {
            return;
        }
        for (channel, value) in color.iter().enumerate() {
            let src = f32::from(*value);
            let dst = f32::from(self.rgba[index + channel]);
            let mixed = (src * source_alpha + dst * dest_alpha * (1.0 - source_alpha)) / out_alpha;
            self.rgba[index + channel] = mixed.round().clamp(0.0, 255.0) as u8;
        }
        self.rgba[index + 3] = (out_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
    }
}

/// A window atlas texture with the two sprites the dialog uses.
pub struct WindowSprites {
    pub texture_width: usize,
    pub texture_height: usize,
    /// RGBA8, top row first.
    pub rgba: Vec<u8>,
    pub body: UiSprite,
    pub border: UiSprite,
}

impl WindowSprites {
    fn pixel(&self, x: i32, y: i32) -> [u8; 4] {
        let x = x.clamp(0, self.texture_width as i32 - 1) as usize;
        let y = y.clamp(0, self.texture_height as i32 - 1) as usize;
        let index = (y * self.texture_width + x) * 4;
        [
            self.rgba[index],
            self.rgba[index + 1],
            self.rgba[index + 2],
            self.rgba[index + 3],
        ]
    }

    /// NGUI sprite `y` counts from the top of the texture in `UISpriteData`; the decoded
    /// texture is top-first too, so rectangles index it directly.
    fn blit(&self, image: &mut Image, src: (i32, i32), dst: (i32, i32), size: (i32, i32)) {
        for dy in 0..size.1 {
            for dx in 0..size.0 {
                let px = self.pixel(src.0 + dx, src.1 + dy);
                image.blend(
                    dst.0 + dx,
                    dst.1 + dy,
                    [px[0], px[1], px[2]],
                    f32::from(px[3]) / 255.0,
                );
            }
        }
    }

    /// `UIBasicSprite.TiledFill`: tiles of the sprite's inner rectangle (rectangle minus
    /// border), grid origin at `birth` (the dialog sets the body center).
    pub fn draw_tiled(&self, image: &mut Image, rect: (i32, i32, i32, i32), birth: (i32, i32)) {
        let sprite = &self.body;
        let inner_x = sprite.x + sprite.border_left;
        let inner_y = sprite.y + sprite.border_top;
        let tile_w = sprite.width - sprite.border_left - sprite.border_right;
        let tile_h = sprite.height - sprite.border_top - sprite.border_bottom;
        if tile_w < 2 || tile_h < 2 {
            return;
        }
        for y in rect.1..rect.1 + rect.3 {
            for x in rect.0..rect.0 + rect.2 {
                let tx = (x - rect.0 - birth.0).rem_euclid(tile_w);
                let ty = (y - rect.1 - birth.1).rem_euclid(tile_h);
                let px = self.pixel(inner_x + tx, inner_y + ty);
                image.blend(x, y, [px[0], px[1], px[2]], f32::from(px[3]) / 255.0);
            }
        }
    }

    /// `UIBasicSprite.SlicedFill` with `UISprite.drawingDimensions` padding.
    pub fn draw_sliced(&self, image: &mut Image, rect: (i32, i32, i32, i32)) {
        let s = &self.border;
        let left = rect.0 + s.padding_left;
        let top = rect.1 + s.padding_top;
        let right = rect.0 + rect.2 - s.padding_right;
        let bottom = rect.1 + rect.3 - s.padding_bottom;
        let xs_dst = [left, left + s.border_left, right - s.border_right, right];
        let ys_dst = [top, top + s.border_top, bottom - s.border_bottom, bottom];
        let xs_src = [
            s.x,
            s.x + s.border_left,
            s.x + s.width - s.border_right,
            s.x + s.width,
        ];
        let ys_src = [
            s.y,
            s.y + s.border_top,
            s.y + s.height - s.border_bottom,
            s.y + s.height,
        ];
        for row in 0..3 {
            for col in 0..3 {
                let dw = xs_dst[col + 1] - xs_dst[col];
                let dh = ys_dst[row + 1] - ys_dst[row];
                let sw = xs_src[col + 1] - xs_src[col];
                let sh = ys_src[row + 1] - ys_src[row];
                if dw <= 0 || dh <= 0 || sw <= 0 || sh <= 0 {
                    continue;
                }
                if dw == sw && dh == sh {
                    self.blit(
                        image,
                        (xs_src[col], ys_src[row]),
                        (xs_dst[col], ys_dst[row]),
                        (dw, dh),
                    );
                    continue;
                }
                for dy in 0..dh {
                    for dx in 0..dw {
                        // Nearest-neighbor stretch.
                        let sx = xs_src[col] + dx * sw / dw;
                        let sy = ys_src[row] + dy * sh / dh;
                        let px = self.pixel(sx, sy);
                        image.blend(
                            xs_dst[col] + dx,
                            ys_dst[row] + dy,
                            [px[0], px[1], px[2]],
                            f32::from(px[3]) / 255.0,
                        );
                    }
                }
            }
        }
    }
}

/// Draws the window (body then border) and one page of text.
///
/// The canvas is the body plus `BORDER_PADDING` on every side. Text starts at the body's
/// left edge plus `PHRASE_X_PADDING` and its top edge plus `PHRASE_Y_PADDING / 2`: the
/// `Phrase` widget has pivot TopLeft and `Dialog.Width`/`LineNumber` place its top-left
/// corner at `(-width / 2, height / 2)` of the body-centered frame.
pub fn render_page(
    layout: &DialogLayout,
    page: usize,
    source: &dyn GlyphSource,
    window: Option<&WindowSprites>,
) -> Option<Image> {
    let page_layout = layout.pages.get(page)?;
    let pad = BORDER_PADDING;
    let canvas_w = layout.body_width + pad * 2;
    let canvas_h = layout.body_height + pad * 2;
    let mut image = Image::new(canvas_w as usize, canvas_h as usize);
    if let Some(window) = window {
        let body = (pad, pad, layout.body_width, layout.body_height);
        window.draw_tiled(
            &mut image,
            body,
            (layout.body_width / 2, layout.body_height / 2),
        );
        window.draw_sliced(&mut image, (0, 0, canvas_w, canvas_h));
    }
    let baseline = measure_baseline(source, FONT_SIZE);
    let origin_x = pad as f32 + PHRASE_X_PADDING;
    let origin_y = pad as f32 + PHRASE_Y_PADDING / 2.0;
    for placed in &page_layout.glyphs {
        let Some(glyph) = source.glyph(placed.ch) else {
            continue;
        };
        if glyph.width == 0 || glyph.height == 0 {
            continue;
        }
        let nudge = if placed.high_shadow {
            (SHADOW_DISTANCE.0 / 3.0, SHADOW_DISTANCE.1 / 3.0)
        } else {
            (0.0, 0.0)
        };
        let left = origin_x + placed.x + glyph.min_x as f32 + nudge.0;
        let top = origin_y + placed.y + (baseline - glyph.max_y()) as f32 + nudge.1;
        let shadow_left = origin_x + placed.x + glyph.min_x as f32 + SHADOW_DISTANCE.0;
        let shadow_top =
            origin_y + placed.y + (baseline - glyph.max_y()) as f32 + SHADOW_DISTANCE.1;
        let to_u8 = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
        draw_coverage(
            &mut image,
            &glyph,
            shadow_left.round() as i32,
            shadow_top.round() as i32,
            [
                to_u8(SHADOW_COLOR[0]),
                to_u8(SHADOW_COLOR[1]),
                to_u8(SHADOW_COLOR[2]),
            ],
            SHADOW_COLOR[3] * placed.color[3],
        );
        draw_coverage(
            &mut image,
            &glyph,
            left.round() as i32,
            top.round() as i32,
            [
                to_u8(placed.color[0]),
                to_u8(placed.color[1]),
                to_u8(placed.color[2]),
            ],
            placed.color[3],
        );
    }
    Some(image)
}

fn draw_coverage(
    image: &mut Image,
    glyph: &GlyphBitmap,
    left: i32,
    top: i32,
    color: [u8; 3],
    alpha: f32,
) {
    for row in 0..glyph.height {
        for col in 0..glyph.width {
            let coverage = f32::from(glyph.coverage[row * glyph.width + col]) / 255.0;
            if coverage > 0.0 {
                image.blend(left + col as i32, top + row as i32, color, coverage * alpha);
            }
        }
    }
}

/// Which window atlas `FF9UIDataTool.WindowAtlas` picks (`win_type == 0` is gray).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowAtlasChoice {
    Blue,
    Gray,
}

impl WindowAtlasChoice {
    pub fn atlas_name(self) -> &'static str {
        match self {
            Self::Blue => "Blue Atlas",
            Self::Gray => "Gray Atlas",
        }
    }
}

/// Install file holding the window atlases (`audit/ui-atlas-inventory.json`).
pub const WINDOW_ATLAS_FILE: &str = "sharedassets2.assets";
/// TextAsset in `resources.assets` carrying the font bundle (`p` = Windows prefix).
pub const FONT_BUNDLE_ASSET: &str = "p_fa.mpc";
/// Font used when `Configuration.Font.Names` is empty (`EncryptFontManager.LoadFont`).
pub const DEFAULT_FONT_NAME: &str = "TBUDGoStd-Bold";

pub struct RenderRequest<'a> {
    /// Path of the install's `resources.assets`; the atlas file is its sibling.
    pub resources_assets: &'a std::path::Path,
    pub mes_name: &'a str,
    /// Index among TextAssets that share `mes_name` (one per language).
    pub variant: usize,
    pub entry_id: i32,
    pub page: usize,
    pub window: WindowAtlasChoice,
    pub size_mode: SizeMode,
}

pub struct RenderedDialog {
    pub image: Image,
    pub layout: DialogLayout,
    pub variant_count: usize,
    pub font_name: String,
    pub atlas_name: &'static str,
    pub entry_source: String,
}

/// The font and window sprites of an install, loaded once so many entries can be
/// laid out and drawn (the dialogs of a play session).
pub struct DialogAssets {
    glyphs: FontdueSource,
    window: WindowSprites,
}

impl DialogAssets {
    /// Loads `TBUDGoStd-Bold` from `resources.assets` and the window atlas next to it.
    pub fn load(
        resources_assets: &std::path::Path,
        window: WindowAtlasChoice,
    ) -> Result<Self, crate::install::ImportError> {
        use super::font_bundle::inventory_encrypted;
        use super::serialized_inventory::StandaloneSerialized;
        use super::ui_atlas::load_atlas;
        use crate::install::ImportError;
        let source_name = resources_assets.display().to_string();
        let mut file = StandaloneSerialized::open(resources_assets)?;
        let assets = file.text_assets()?;
        let font_object = assets
            .iter()
            .find(|(name, _)| name == FONT_BUNDLE_ASSET)
            .map(|(_, object)| object.clone())
            .ok_or_else(|| {
                ImportError::invalid(&source_name, format!("no TextAsset `{FONT_BUNDLE_ASSET}`"))
            })?;
        let bundle = file.read_text_asset(&font_object)?;
        let (inventory, blobs) = inventory_encrypted(&bundle.script)?;
        let (_, blob) = inventory
            .fonts
            .iter()
            .zip(&blobs)
            .find(|(font, blob)| font.name == DEFAULT_FONT_NAME && !blob.font_data.is_empty())
            .ok_or_else(|| {
                ImportError::invalid(&source_name, format!("no font `{DEFAULT_FONT_NAME}`"))
            })?;
        let glyphs = FontdueSource::from_ttf(&blob.font_data, FONT_SIZE as f32)
            .map_err(|error| ImportError::invalid(&source_name, error))?;
        let atlas_path = resources_assets
            .parent()
            .map(|dir| dir.join(WINDOW_ATLAS_FILE))
            .unwrap_or_else(|| std::path::PathBuf::from(WINDOW_ATLAS_FILE));
        let atlas_name = window.atlas_name();
        let (atlas, width, height, rgba) = load_atlas(&atlas_path, atlas_name)?;
        let sprite = |name: &str| {
            atlas
                .sprites
                .iter()
                .find(|sprite| sprite.name == name)
                .cloned()
                .ok_or_else(|| {
                    ImportError::invalid(
                        &source_name,
                        format!("{atlas_name} has no sprite `{name}`"),
                    )
                })
        };
        let window = WindowSprites {
            texture_width: width as usize,
            texture_height: height as usize,
            rgba,
            body: sprite(BODY_SPRITE)?,
            border: sprite(BORDER_SPRITE)?,
        };
        Ok(Self { glyphs, window })
    }

    /// The font, at [`FONT_SIZE`] (other UI text, such as the battle HUD).
    pub fn glyphs(&self) -> &FontdueSource {
        &self.glyphs
    }

    /// The window sprites.
    pub fn window(&self) -> &WindowSprites {
        &self.window
    }

    /// Lays out and draws the first page of `entry`.
    pub fn render(
        &self,
        entry: &MesEntry,
        mode: SizeMode,
        framed: bool,
    ) -> Option<(Image, DialogLayout)> {
        let layout = layout_entry(entry, &self.glyphs, mode, 0);
        let window = framed.then_some(&self.window);
        let image = render_page(&layout, 0, &self.glyphs, window)?;
        Some((image, layout))
    }
}

/// One run of text for [`DialogAssets::render_window`]: position in
/// resource pixels from the window body's top left (`y` is the line top).
#[derive(Debug, Clone, PartialEq)]
pub struct WindowText {
    pub x: f32,
    pub y: f32,
    pub text: String,
    pub color: [f32; 4],
    /// `x` is the right edge of the text.
    pub right: bool,
}

impl DialogAssets {
    /// Draws a framed window of `body_width` x `body_height` resource pixels
    /// with `texts` in the game's font, the way a dialog page is drawn.
    pub fn render_window(&self, body_width: i32, body_height: i32, texts: &[WindowText]) -> Image {
        let pad = BORDER_PADDING;
        let canvas_w = body_width + pad * 2;
        let canvas_h = body_height + pad * 2;
        let mut image = Image::new(canvas_w.max(1) as usize, canvas_h.max(1) as usize);
        self.window.draw_tiled(
            &mut image,
            (pad, pad, body_width, body_height),
            (body_width / 2, body_height / 2),
        );
        self.window
            .draw_sliced(&mut image, (0, 0, canvas_w, canvas_h));
        let baseline = measure_baseline(&self.glyphs, FONT_SIZE);
        let to_u8 = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
        for run in texts {
            let width: i32 = run
                .text
                .chars()
                .filter_map(|ch| self.glyphs.glyph(ch))
                .map(|glyph| glyph.advance + SPACING_X)
                .sum();
            let mut pen = pad as f32
                + PHRASE_X_PADDING
                + if run.right {
                    run.x - width as f32
                } else {
                    run.x
                };
            let top = pad as f32 + PHRASE_Y_PADDING / 2.0 + run.y;
            for ch in run.text.chars() {
                let Some(glyph) = self.glyphs.glyph(ch) else {
                    continue;
                };
                if glyph.width > 0 && glyph.height > 0 {
                    let left = pen + glyph.min_x as f32;
                    let y = top + (baseline - glyph.max_y()) as f32;
                    draw_coverage(
                        &mut image,
                        &glyph,
                        (left + SHADOW_DISTANCE.0).round() as i32,
                        (y + SHADOW_DISTANCE.1).round() as i32,
                        [
                            to_u8(SHADOW_COLOR[0]),
                            to_u8(SHADOW_COLOR[1]),
                            to_u8(SHADOW_COLOR[2]),
                        ],
                        SHADOW_COLOR[3] * run.color[3],
                    );
                    draw_coverage(
                        &mut image,
                        &glyph,
                        left.round() as i32,
                        y.round() as i32,
                        [
                            to_u8(run.color[0]),
                            to_u8(run.color[1]),
                            to_u8(run.color[2]),
                        ],
                        run.color[3],
                    );
                }
                pen += (glyph.advance + SPACING_X) as f32;
            }
        }
        image
    }
}

/// Reads font, `.mes` entry and window atlas from an install and renders one page.
pub fn render_from_install(
    request: &RenderRequest<'_>,
) -> Result<RenderedDialog, crate::install::ImportError> {
    use super::font_bundle::inventory_encrypted;
    use super::mes::{MesFile, parse_mes_bytes};
    use super::serialized_inventory::StandaloneSerialized;
    use super::ui_atlas::load_atlas;
    use crate::install::ImportError;
    let source_name = request.resources_assets.display().to_string();
    let mut file = StandaloneSerialized::open(request.resources_assets)?;
    let assets = file.text_assets()?;
    let font_object = assets
        .iter()
        .find(|(name, _)| name == FONT_BUNDLE_ASSET)
        .map(|(_, object)| object.clone())
        .ok_or_else(|| {
            ImportError::invalid(&source_name, format!("no TextAsset `{FONT_BUNDLE_ASSET}`"))
        })?;
    let bundle = file.read_text_asset(&font_object)?;
    let (inventory, blobs) = inventory_encrypted(&bundle.script)?;
    let (font_entry, blob) = inventory
        .fonts
        .iter()
        .zip(&blobs)
        .find(|(font, blob)| font.name == DEFAULT_FONT_NAME && !blob.font_data.is_empty())
        .ok_or_else(|| {
            ImportError::invalid(&source_name, format!("no font `{DEFAULT_FONT_NAME}`"))
        })?;
    let glyphs = FontdueSource::from_ttf(&blob.font_data, FONT_SIZE as f32)
        .map_err(|error| ImportError::invalid(&source_name, error))?;
    let candidates: Vec<_> = assets
        .iter()
        .filter(|(name, _)| name == request.mes_name)
        .map(|(_, object)| object.clone())
        .collect();
    let object = candidates.get(request.variant).ok_or_else(|| {
        ImportError::invalid(
            &source_name,
            format!(
                "`{}` has {} variants; variant {} requested",
                request.mes_name,
                candidates.len(),
                request.variant
            ),
        )
    })?;
    let asset = file.read_text_asset(object)?;
    let mes: MesFile = parse_mes_bytes(&asset.script, request.mes_name)?;
    let entry = mes
        .entries
        .iter()
        .find(|entry| entry.id == request.entry_id)
        .ok_or_else(|| {
            ImportError::invalid(
                &source_name,
                format!("`{}` has no entry {}", request.mes_name, request.entry_id),
            )
        })?;
    let atlas_path = request
        .resources_assets
        .parent()
        .map(|dir| dir.join(WINDOW_ATLAS_FILE))
        .unwrap_or_else(|| std::path::PathBuf::from(WINDOW_ATLAS_FILE));
    let atlas_name = request.window.atlas_name();
    let (atlas, width, height, rgba) = load_atlas(&atlas_path, atlas_name)?;
    let sprite = |name: &str| {
        atlas
            .sprites
            .iter()
            .find(|sprite| sprite.name == name)
            .cloned()
            .ok_or_else(|| {
                ImportError::invalid(&source_name, format!("{atlas_name} has no sprite `{name}`"))
            })
    };
    let window = WindowSprites {
        texture_width: width as usize,
        texture_height: height as usize,
        rgba,
        body: sprite(BODY_SPRITE)?,
        border: sprite(BORDER_SPRITE)?,
    };
    let layout = layout_entry(entry, &glyphs, request.size_mode, 0);
    let image = render_page(&layout, request.page, &glyphs, Some(&window)).ok_or_else(|| {
        ImportError::invalid(
            &source_name,
            format!(
                "page {} requested but the entry has {} pages",
                request.page,
                layout.pages.len()
            ),
        )
    })?;
    Ok(RenderedDialog {
        image,
        layout,
        variant_count: candidates.len(),
        font_name: font_entry.name.clone(),
        atlas_name,
        entry_source: MesFile::entry_source(entry),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pc::mes::{MesTag, MesToken};

    /// Every printable character is a solid 10 x 20 box with advance 12 whose bottom sits
    /// on the baseline. `)` and `A` share that box so the baseline is measurable.
    struct BoxFont;

    impl GlyphSource for BoxFont {
        fn glyph(&self, ch: char) -> Option<GlyphBitmap> {
            if ch == '\u{fffd}' {
                return None;
            }
            let blank = ch == ' ';
            Some(GlyphBitmap {
                advance: 12,
                min_x: 1,
                min_y: 0,
                width: if blank { 0 } else { 10 },
                height: if blank { 0 } else { 20 },
                coverage: if blank { Vec::new() } else { vec![255; 200] },
            })
        }
    }

    fn text(value: &str) -> MesToken {
        MesToken::Text {
            text: value.to_owned(),
        }
    }

    fn tag(code: &str, params: &[&str], raw: &str) -> MesToken {
        MesToken::Tag(MesTag {
            code: code.to_owned(),
            params: params.iter().map(|p| (*p).to_owned()).collect(),
            closing: false,
            raw: raw.to_owned(),
        })
    }

    fn entry(tokens: Vec<MesToken>) -> MesEntry {
        MesEntry { id: 0, tokens }
    }

    #[test]
    fn baseline_follows_the_nguitext_formula() {
        // `)` spans y 0..20: round((0 + 20 + 42) / 2) = 31.
        assert_eq!(measure_baseline(&BoxFont, 42), 31);
    }

    #[test]
    fn newline_advances_one_line_height_and_width_is_widest_line() {
        let layout = layout_entry(&entry(vec![text("ab\ncde")]), &BoxFont, SizeMode::Auto, 0);
        let page = &layout.pages[0];
        assert_eq!(page.lines.len(), 2);
        assert_eq!(page.lines[1].base_y, 68.0);
        assert_eq!(page.max_width, 36.0);
        assert_eq!(page.glyphs[2].y, 68.0);
        assert_eq!(page.glyphs[3].x, 12.0);
    }

    #[test]
    fn auto_size_matches_dialog_width_and_line_formulas() {
        let layout = layout_entry(&entry(vec![text("ab\ncde")]), &BoxFont, SizeMode::Auto, 0);
        // (36 + 32) / m + 1, times m, with the 140 px minimum.
        assert_eq!(layout.body_width, 140);
        assert_eq!(layout.body_height, 2 * 68 + 20);
        let wide = layout_entry(
            &entry(vec![text(&"x".repeat(30))]),
            &BoxFont,
            SizeMode::Auto,
            0,
        );
        let expected = (360.0 + 32.0 + RESOURCE_X_MULTIPLIER) as i32;
        assert!((wide.body_width - expected).abs() <= 1);
        assert_eq!(wide.body_height, 68 + 20);
    }

    #[test]
    fn hint_size_uses_strt_parameters() {
        let layout = layout_entry(
            &entry(vec![
                tag("DialogSize", &["100", "3"], "[STRT=100,3]"),
                text("a"),
            ]),
            &BoxFont,
            SizeMode::Hint,
            0,
        );
        assert_eq!(layout.width_hint, 100.0);
        assert_eq!(layout.body_width, (103.0 * RESOURCE_X_MULTIPLIER) as i32);
        assert_eq!(layout.body_height, 3 * 68 + 20);
    }

    #[test]
    fn new_page_splits_and_end_stops() {
        let layout = layout_entry(
            &entry(vec![
                text("a"),
                tag("NewPage", &[], "[PAGE]"),
                text("bb"),
                tag("End", &[], "[ENDN]"),
                text("zzz"),
            ]),
            &BoxFont,
            SizeMode::Auto,
            0,
        );
        assert_eq!(layout.pages.len(), 2);
        assert_eq!(layout.pages[0].glyphs.len(), 1);
        assert_eq!(layout.pages[1].glyphs.len(), 2);
    }

    #[test]
    fn color_tags_push_and_pop() {
        let layout = layout_entry(
            &entry(vec![
                text("a"),
                tag("TextRGBA", &["FF", "00", "80"], "[FF0080]"),
                text("b"),
                tag("TextRGBA", &[], "[-]"),
                text("c"),
            ]),
            &BoxFont,
            SizeMode::Auto,
            0,
        );
        let glyphs = &layout.pages[0].glyphs;
        assert_eq!(glyphs[0].color, DEFAULT_TEXT_COLOR);
        assert!((glyphs[1].color[0] - 1.0).abs() < 1e-6);
        assert!((glyphs[1].color[2] - 128.0 / 255.0).abs() < 1e-3);
        assert_eq!(glyphs[2].color, DEFAULT_TEXT_COLOR);
    }

    #[test]
    fn speed_wait_are_recorded_not_executed_and_names_are_unhandled() {
        let layout = layout_entry(
            &entry(vec![
                text("a"),
                tag("Speed", &["2"], "[SPED=2]"),
                tag("Wait", &["30"], "[WAIT=30]"),
                tag("CharacterName", &["ZDNE"], "[ZDNE]"),
                text("b"),
            ]),
            &BoxFont,
            SizeMode::Auto,
            0,
        );
        assert_eq!(layout.timing_events.len(), 2);
        assert_eq!(layout.timing_events[0].glyph_index, 1);
        assert_eq!(layout.unhandled_tags.len(), 1);
        assert_eq!(layout.unhandled_tags[0].code, "CharacterName");
        assert_eq!(layout.pages[0].glyphs.len(), 2);
    }

    #[test]
    fn tab_offsets_the_pen_by_dx_minus_four_times_multiplier() {
        let layout = layout_entry(
            &entry(vec![
                tag("MoveCaret", &["18", "0"], "[MOVE=18,0]"),
                text("a"),
            ]),
            &BoxFont,
            SizeMode::Auto,
            0,
        );
        let expected = 14.0 * RESOURCE_X_MULTIPLIER;
        assert!((layout.pages[0].glyphs[0].x - expected).abs() < 1e-4);
    }

    #[test]
    fn missing_glyphs_are_skipped_and_reported() {
        let layout = layout_entry(
            &entry(vec![text("a\u{fffd}b")]),
            &BoxFont,
            SizeMode::Auto,
            0,
        );
        assert_eq!(layout.pages[0].glyphs.len(), 2);
        assert_eq!(layout.missing_glyphs, vec!['\u{fffd}']);
    }

    #[test]
    fn render_draws_shadow_then_glyph_inside_the_canvas() {
        let layout = layout_entry(&entry(vec![text("a")]), &BoxFont, SizeMode::Auto, 0);
        let image = render_page(&layout, 0, &BoxFont, None).expect("page");
        assert_eq!(image.width as i32, layout.body_width + 36);
        assert_eq!(image.height as i32, layout.body_height + 36);
        // Glyph box: x = 18 + 16 + 1, y = 18 + 10 + (31 - 20).
        let (gx, gy) = (35, 39);
        let at = |x: usize, y: usize| {
            let i = (y * image.width + x) * 4;
            [
                image.rgba[i],
                image.rgba[i + 1],
                image.rgba[i + 2],
                image.rgba[i + 3],
            ]
        };
        assert_eq!(at(gx, gy), [200, 200, 200, 255]);
        // Shadow-only pixel: 4 px right of the glyph's right edge, inside the shadow box.
        assert_eq!(at(gx + 10 + 3, gy + 4), [40, 40, 40, 255]);
        assert_eq!(at(gx - 1, gy - 1)[3], 0);
    }

    fn synthetic_window() -> WindowSprites {
        // 8 x 8 texture: left half red tile source, right half blue frame source.
        let mut rgba = vec![0_u8; 8 * 8 * 4];
        for y in 0..8 {
            for x in 0..8 {
                let i = (y * 8 + x) * 4;
                let color = if x < 4 {
                    [255, 0, 0, 255]
                } else {
                    [0, 0, 255, 255]
                };
                rgba[i..i + 4].copy_from_slice(&color);
            }
        }
        let sprite = |name: &str, x, border| UiSprite {
            name: name.to_owned(),
            x,
            y: 0,
            width: 4,
            height: 8,
            border_left: border,
            border_right: border,
            border_top: border,
            border_bottom: border,
            padding_left: 0,
            padding_right: 0,
            padding_top: 0,
            padding_bottom: 0,
        };
        WindowSprites {
            texture_width: 8,
            texture_height: 8,
            rgba,
            body: sprite(BODY_SPRITE, 0, 1),
            border: sprite(BORDER_SPRITE, 4, 1),
        }
    }

    #[test]
    fn window_draws_tiled_body_and_sliced_border() {
        let layout = layout_entry(&entry(vec![text("a")]), &BoxFont, SizeMode::Auto, 0);
        let window = synthetic_window();
        let image = render_page(&layout, 0, &BoxFont, Some(&window)).expect("page");
        let at = |x: usize, y: usize| {
            let i = (y * image.width + x) * 4;
            [
                image.rgba[i],
                image.rgba[i + 1],
                image.rgba[i + 2],
                image.rgba[i + 3],
            ]
        };
        // Canvas corner is border (blue); body interior away from text is tiled red.
        assert_eq!(at(0, 0), [0, 0, 255, 255]);
        assert_eq!(at(image.width - 5, image.height - 5), [0, 0, 255, 255]);
    }

    #[test]
    fn real_install_dialog_renders_when_present() {
        let path = std::path::Path::new(
            "E:/SteamLibrary/steamapps/common/FINAL FANTASY IX/x64/FF9_Data/resources.assets",
        );
        if !path.is_file() {
            eprintln!("skipping: FFIX install not present");
            return;
        }
        let rendered = render_from_install(&RenderRequest {
            resources_assets: path,
            mes_name: "94.mes",
            variant: 1,
            entry_id: 7,
            page: 0,
            window: WindowAtlasChoice::Blue,
            size_mode: SizeMode::Auto,
        })
        .expect("render");
        assert_eq!(rendered.font_name, DEFAULT_FONT_NAME);
        assert!(rendered.layout.missing_glyphs.is_empty());
        assert!(rendered.image.rgba.iter().any(|b| *b != 0));
    }

    #[test]
    fn page_out_of_range_is_none() {
        let layout = layout_entry(&entry(vec![text("a")]), &BoxFont, SizeMode::Auto, 0);
        assert!(render_page(&layout, 3, &BoxFont, None).is_none());
    }
}
