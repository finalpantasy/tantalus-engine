//! The play window's two UI skins: PS1 (the default) and Steam.
//!
//! Both skins lay out and behave the same (the C# `Dialog`, `BattleHUD` and menu code); they
//! differ in the art. The Steam skin draws the install's font (`TBUDGoStd-Bold`) and window
//! atlas; the PS1 skin draws the PS1 game's dialog font, window sheet and hand cursor, read at
//! run time from the owner's own discs (`tantalus_psxaudio::ui_art`) and cached under the user's
//! data folder (`%APPDATA%/tantalus/psx-ui`, or `TANTALUS_PSX_UI_DIR`).
//!
//! Choice: `--ui-skin ps1|steam`, else `TANTALUS_UI_SKIN`, else `ui_skin = ps1|steam` in the
//! `--config` file, else PS1. The disc folder: `--psx-discs DIR`, else `TANTALUS_PSX_DISCS`,
//! else `psx_discs = DIR` in the config. Without the cache and the discs the PS1 skin falls back
//! to the Steam art with one log line.
//!
//! The PS1 art is drawn at the UI's resource scale (`RESOURCE_Y_MULTIPLIER`, 1080 / 224 resource
//! pixels per PS1 pixel), nearest-neighbor, so the existing layout code places it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tantalus_assets::pc::dialog_render::{
    DialogAssets, GlyphBitmap, GlyphSource, Image, RESOURCE_Y_MULTIPLIER, WindowAtlasChoice,
    WindowSprites,
};
use tantalus_assets::pc::ui_atlas::UiSprite;
use tantalus_psxaudio::ui_art::{GLYPH_CELL_HEIGHT, GlyphCell, Tim, UiArt, glyph_code};

/// Which art the UI draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiSkin {
    #[default]
    Ps1,
    Steam,
}

impl UiSkin {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text.trim().to_ascii_lowercase().as_str() {
            "ps1" | "psx" => Ok(Self::Ps1),
            "steam" | "pc" => Ok(Self::Steam),
            other => Err(format!("ui skin `{other}`: use ps1 or steam")),
        }
    }
}

/// The skin asked for and where the PS1 discs are.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SkinSettings {
    pub skin: UiSkin,
    pub discs: Option<PathBuf>,
}

/// `key = value` of a config file line, if `key` matches.
fn config_value<'a>(file: &'a str, key: &str) -> Option<&'a str> {
    file.lines().find_map(|line| {
        let line = line.split('#').next()?.trim();
        let (k, v) = line.split_once('=')?;
        (k.trim() == key).then(|| v.trim())
    })
}

impl SkinSettings {
    /// A flag beats the environment, which beats the config file.
    pub fn resolve(
        config: Option<&Path>,
        flag_skin: Option<&str>,
        flag_discs: Option<&str>,
    ) -> Result<Self, String> {
        let file = config
            .map(std::fs::read_to_string)
            .transpose()
            .map_err(|error| format!("config: {error}"))?
            .unwrap_or_default();
        let env_skin = std::env::var("TANTALUS_UI_SKIN").ok();
        let env_discs = std::env::var("TANTALUS_PSX_DISCS").ok();
        let skin = flag_skin
            .or(env_skin.as_deref())
            .or(config_value(&file, "ui_skin"))
            .map(UiSkin::parse)
            .transpose()?
            .unwrap_or_default();
        let discs = flag_discs
            .or(env_discs.as_deref())
            .or(config_value(&file, "psx_discs"))
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from);
        Ok(Self { skin, discs })
    }
}

static SETTINGS: OnceLock<SkinSettings> = OnceLock::new();

/// Sets the run's skin; call once before the first window opens. Later calls are ignored.
pub fn set(settings: SkinSettings) {
    if SETTINGS.set(settings).is_err() {
        println!("ui skin: already chosen, the new choice is ignored");
    }
}

fn settings() -> &'static SkinSettings {
    SETTINGS.get_or_init(|| {
        SkinSettings::resolve(None, None, None).unwrap_or_else(|error| {
            println!("ui skin: {error}; using PS1");
            SkinSettings::default()
        })
    })
}

/// The cache folder of the PS1 UI art.
pub fn cache_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("TANTALUS_PSX_UI_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return PathBuf::from(appdata).join("tantalus").join("psx-ui");
    }
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home).join(".tantalus").join("psx-ui")
}

/// The cached copy of the discs' resident UI file.
const CACHE_FILE: &str = "ui-art.bin";

/// The PS1 art from the cache, else from the discs (and then cached).
fn load_ps1_art(discs: Option<&Path>) -> Result<UiArt, String> {
    let cache = cache_dir().join(CACHE_FILE);
    if let Ok(bytes) = std::fs::read(&cache) {
        match UiArt::parse(&bytes) {
            Ok(art) => return Ok(art),
            Err(error) => println!(
                "ui skin: cached PS1 art {} unusable: {error}",
                cache.display()
            ),
        }
    }
    let Some(discs) = discs else {
        return Err(format!(
            "no cached PS1 art at {} and no disc folder (set psx_discs in the config, \
             --psx-discs or TANTALUS_PSX_DISCS)",
            cache.display()
        ));
    };
    let bytes = tantalus_psxaudio::ui_art::read_ui_file(discs)?;
    let art = UiArt::parse(&bytes)?;
    if let Err(error) =
        std::fs::create_dir_all(cache_dir()).and_then(|()| std::fs::write(&cache, &bytes))
    {
        println!(
            "ui skin: cannot cache the PS1 art at {}: {error}",
            cache.display()
        );
    }
    Ok(art)
}

static PS1_ART: OnceLock<Option<UiArt>> = OnceLock::new();

/// The PS1 art when the PS1 skin is chosen and its art is found.
fn ps1_art() -> Option<&'static UiArt> {
    PS1_ART
        .get_or_init(|| {
            let settings = settings();
            if settings.skin != UiSkin::Ps1 {
                return None;
            }
            match load_ps1_art(settings.discs.as_deref()) {
                Ok(art) => {
                    println!("ui skin: PS1");
                    Some(art)
                }
                Err(error) => {
                    println!("ui skin: PS1 art not found ({error}); using the Steam art");
                    None
                }
            }
        })
        .as_ref()
}

/// The skin the UI draws with (Steam when the PS1 art is missing).
pub fn active() -> UiSkin {
    if ps1_art().is_some() {
        UiSkin::Ps1
    } else {
        UiSkin::Steam
    }
}

/// Resource pixels per PS1 pixel.
const S: f32 = RESOURCE_Y_MULTIPLIER;

/// A rectangle of a TIM scaled by [`S`], nearest-neighbor, as RGBA8.
fn scaled(tim: &Tim, rect: (usize, usize, usize, usize), clut: usize) -> Image {
    let rgba = tim.rgba(rect, clut);
    let (w, h) = (
        (rect.2 as f32 * S).round() as usize,
        (rect.3 as f32 * S).round() as usize,
    );
    let mut out = Image::new(w.max(1), h.max(1));
    for y in 0..h {
        let sy = ((y as f32 / S) as usize).min(rect.3 - 1);
        for x in 0..w {
            let sx = ((x as f32 / S) as usize).min(rect.2 - 1);
            let from = (sy * rect.2 + sx) * 4;
            let to = (y * w + x) * 4;
            out.rgba[to..to + 4].copy_from_slice(&rgba[from..from + 4]);
        }
    }
    out
}

/// The PS1 dialog font as a glyph source: the sheet's text pixels (the brighter of its two ink
/// colors) as coverage, scaled to resource pixels; the drop shadow is the renderer's own.
struct Ps1Font {
    sheet: Tim,
    cells: Vec<GlyphCell>,
    cache: RefCell<HashMap<char, Option<GlyphBitmap>>>,
}

/// Rows of a glyph cell above the baseline.
const BASELINE_ROW: usize = 10;
/// Text pixels are the palette colors at least this bright (sum of 5-bit channels); the
/// shadow color of the font is far darker.
const INK_THRESHOLD: u16 = 48;

impl Ps1Font {
    fn cell(&self, ch: char) -> Option<GlyphCell> {
        match ch {
            // The font has no single quote: the left mark of `”` stands in for it.
            '\'' | '’' | '‘' => self.cells.get(glyph_code('”')?).map(|c| GlyphCell {
                width: 3,
                advance: 3,
                ..*c
            }),
            '—' | '–' => self.cells.get(glyph_code('-')?).copied(),
            _ => self.cells.get(glyph_code(ch)?).copied(),
        }
    }

    fn ink(&self, x: usize, y: usize) -> bool {
        if x >= self.sheet.width || y >= self.sheet.height {
            return false;
        }
        let c = self.sheet.color(x, y, 0);
        c != 0 && (c & 31) + ((c >> 5) & 31) + ((c >> 10) & 31) >= INK_THRESHOLD
    }

    fn rasterize(&self, cell: GlyphCell) -> GlyphBitmap {
        let w = (f32::from(cell.width) * S).round() as usize;
        let h = (GLYPH_CELL_HEIGHT as f32 * S).round() as usize;
        let mut coverage = vec![0u8; w * h];
        for y in 0..h {
            let sy = ((y as f32 / S) as usize).min(GLYPH_CELL_HEIGHT - 1);
            for x in 0..w {
                let sx = ((x as f32 / S) as usize).min(usize::from(cell.width).max(1) - 1);
                if self.ink(usize::from(cell.x) + sx, usize::from(cell.y) + sy) {
                    coverage[y * w + x] = 255;
                }
            }
        }
        GlyphBitmap {
            advance: (f32::from(cell.advance) * S).round() as i32,
            min_x: (f32::from(cell.offset) * S).round() as i32,
            min_y: -(((GLYPH_CELL_HEIGHT - BASELINE_ROW) as f32 * S).round() as i32),
            width: w,
            height: h,
            coverage,
        }
    }
}

impl GlyphSource for Ps1Font {
    fn glyph(&self, ch: char) -> Option<GlyphBitmap> {
        if let Some(hit) = self.cache.borrow().get(&ch) {
            return hit.clone();
        }
        let glyph = self.cell(ch).map(|cell| self.rasterize(cell));
        self.cache.borrow_mut().insert(ch, glyph.clone());
        glyph
    }
}

/// Window sheet palettes: rows 0-3 color the frame (indices 1-7), rows 4-10 the grain (indices
/// 1-14); row 7 is the gray-blue grain of the gray window, row 5 the blue one.
const FRAME_CLUT: usize = 0;
const GRAIN_CLUT: usize = 7;
/// The window grain tile and the frame piece in the window sheet (PS1 pixels).
const GRAIN: (usize, usize, usize, usize) = (32, 0, 96, 32);
const FRAME: (usize, usize, usize, usize) = (128, 0, 24, 24);
/// The frame's corner size (PS1 pixels).
const FRAME_CORNER: usize = 8;

fn sprite(name: &str, rect: (i32, i32, i32, i32), border: i32) -> UiSprite {
    UiSprite {
        name: name.to_owned(),
        x: rect.0,
        y: rect.1,
        width: rect.2,
        height: rect.3,
        border_left: border,
        border_right: border,
        border_top: border,
        border_bottom: border,
        padding_left: 0,
        padding_right: 0,
        padding_top: 0,
        padding_bottom: 0,
    }
}

/// The PS1 window as dialog window sprites: one texture with the grain tile on the left and
/// the frame piece (its middle cleared) on the right.
fn ps1_window(art: &UiArt) -> WindowSprites {
    let grain = scaled(&art.window, GRAIN, GRAIN_CLUT);
    let mut frame = scaled(&art.window, FRAME, FRAME_CLUT);
    let corner = (FRAME_CORNER as f32 * S).round() as usize;
    for y in corner..frame.height.saturating_sub(corner) {
        for x in corner..frame.width.saturating_sub(corner) {
            let at = (y * frame.width + x) * 4;
            frame.rgba[at..at + 4].fill(0);
        }
    }
    let width = grain.width + frame.width;
    let height = grain.height.max(frame.height);
    let mut rgba = vec![0u8; width * height * 4];
    for (image, left) in [(&grain, 0), (&frame, grain.width)] {
        for y in 0..image.height {
            let from = y * image.width * 4;
            let to = (y * width + left) * 4;
            rgba[to..to + image.width * 4]
                .copy_from_slice(&image.rgba[from..from + image.width * 4]);
        }
    }
    WindowSprites {
        texture_width: width,
        texture_height: height,
        rgba,
        body: sprite(
            "ps1_window_grain",
            (0, 0, grain.width as i32, grain.height as i32),
            0,
        ),
        border: sprite(
            "ps1_window_frame",
            (
                grain.width as i32,
                0,
                frame.width as i32,
                frame.height as i32,
            ),
            corner as i32,
        ),
    }
}

/// The font and window art of the active skin.
pub fn dialog_assets(game_data: &Path) -> Result<DialogAssets, String> {
    if let Some(art) = ps1_art() {
        let font = Ps1Font {
            sheet: art.font.clone(),
            cells: art.glyphs.clone(),
            cache: RefCell::new(HashMap::new()),
        };
        return Ok(DialogAssets::from_parts(Box::new(font), ps1_window(art)));
    }
    DialogAssets::load(&game_data.join("resources.assets"), WindowAtlasChoice::Blue)
        .map_err(|error| error.to_string())
}

/// The hand cursor in the PS1 icon sheet (PS1 pixels, palette 0: the white glove).
const PS1_HAND: (usize, usize, usize, usize) = (0, 32, 24, 14);

/// `FF9UIDataTool.IconSpriteName[1]`: the Steam hand cursor, in the Icon Atlas.
const HAND_FILE: &str = "sharedassets2.assets";
const HAND_ATLAS: &str = "Icon Atlas";
const HAND_SPRITE: &str = "cursor_hand_choice";
/// The Steam hand's width in UI pixels (`PointerManager.PointerSize`, 114 by 62).
const STEAM_HAND_UI_WIDTH: f32 = 114.0;

/// The hand cursor pointing right and its drawn width in UI (resource) pixels.
pub fn hand(game_data: &Path) -> Option<(Image, f32)> {
    if let Some(art) = ps1_art() {
        let image = Image {
            width: PS1_HAND.2,
            height: PS1_HAND.3,
            rgba: art.icons.rgba(PS1_HAND, 0),
        };
        return Some((image, PS1_HAND.2 as f32 * S));
    }
    let mut sprites = tantalus_assets::pc::ui_atlas::load_sprites(
        &game_data.join(HAND_FILE),
        HAND_ATLAS,
        &[HAND_SPRITE],
    )
    .map_err(|error| println!("!! hand cursor art: {error}"))
    .ok()?;
    let sprite = sprites.remove(HAND_SPRITE)?;
    let image = Image {
        width: sprite.width as usize,
        height: sprite.height as usize,
        rgba: sprite.rgba,
    };
    Some((image, STEAM_HAND_UI_WIDTH))
}

/// The PS1 battle digits of the window sheet: 8 by 8 cells, `0`-`7` on line 32, `8` and `9`
/// on line 40, then `MP` and `Miss`; `Death` on line 48.
pub struct BattleDigits {
    pub digits: Vec<Image>,
    pub miss: Image,
    /// UI pixels a digit pixel is drawn (1080 / 224).
    pub ui_per_pixel: f32,
}

const DIGIT_CELL: usize = 8;
const MISS: (usize, usize, usize, usize) = (32, 40, 24, 8);

/// The PS1 skin's damage digits (none for the Steam skin, which draws the font).
pub fn battle_digits() -> Option<BattleDigits> {
    let art = ps1_art()?;
    let cell = |rect| Image {
        width: DIGIT_CELL,
        height: DIGIT_CELL,
        rgba: art.window.rgba(rect, FRAME_CLUT),
    };
    let digits = (0..10)
        .map(|d| {
            let (x, y) = if d < 8 {
                (d * DIGIT_CELL, 32)
            } else {
                ((d - 8) * DIGIT_CELL, 40)
            };
            cell((x, y, DIGIT_CELL, DIGIT_CELL))
        })
        .collect();
    Some(BattleDigits {
        digits,
        miss: Image {
            width: MISS.2,
            height: MISS.3,
            rgba: art.window.rgba(MISS, FRAME_CLUT),
        },
        ui_per_pixel: S,
    })
}

/// `Dialog.SetTailPosition` pointer sprites of the window atlas.
const TAIL_SPRITES: [&str; 4] = [
    "dialog_pointer_topleft",
    "dialog_pointer_topright",
    "dialog_pointer_downleft",
    "dialog_pointer_downright",
];

/// The dialog tail pointers of the skin, by sprite name (in UI pixels). The Steam skin's come
/// from the window atlas the dialog uses; the PS1 skin has none (its tail is a notch drawn into
/// the frame, not a separate sprite), so the dialog draws a marker.
pub fn tails(game_data: &Path) -> Vec<(&'static str, Image)> {
    if active() == UiSkin::Ps1 {
        return Vec::new();
    }
    let atlas = WindowAtlasChoice::Blue.atlas_name();
    let mut sprites = match tantalus_assets::pc::ui_atlas::load_sprites(
        &game_data.join(tantalus_assets::pc::dialog_render::WINDOW_ATLAS_FILE),
        atlas,
        &TAIL_SPRITES,
    ) {
        Ok(sprites) => sprites,
        Err(error) => {
            println!("!! dialog tail art: {error}");
            return Vec::new();
        }
    };
    TAIL_SPRITES
        .iter()
        .filter_map(|name| {
            let sprite = sprites.remove(*name)?;
            Some((
                *name,
                Image {
                    width: sprite.width as usize,
                    height: sprite.height as usize,
                    rgba: sprite.rgba,
                },
            ))
        })
        .collect()
}

/// `BattleParameters.csv` `AvatarSprite` of each character (`FF9UIDataTool.AvatarSpriteName`):
/// Zidane, Vivi, Garnet (long hair), Steiner, Freya, Quina, Eiko, Amarant, Cinna, Marcus,
/// Blank, Beatrix.
const FACE_SPRITES: [&str; 12] = [
    "face00", "face01", "face02", "face04", "face07", "face05", "face06", "face08", "face09",
    "face10", "face11", "face12",
];
/// `GraphicResources.FaceAtlasName`, in `sharedassets2.assets`.
const FACE_ATLAS: &str = "Face Atlas";

/// The member portraits by character id (`FF9UIDataTool.DisplayCharacterAvatar`). Both skins
/// draw the install's `Face Atlas`: the PS1 portraits were not found in the discs' resident
/// files.
pub fn faces(game_data: &Path) -> HashMap<i32, Image> {
    let sprites = match tantalus_assets::pc::ui_atlas::load_sprites(
        &game_data.join(HAND_FILE),
        FACE_ATLAS,
        &FACE_SPRITES,
    ) {
        Ok(sprites) => sprites,
        Err(error) => {
            println!("!! portrait art: {error}");
            return HashMap::new();
        }
    };
    FACE_SPRITES
        .iter()
        .enumerate()
        .filter_map(|(character, name)| {
            let sprite = sprites.get(*name)?;
            Some((
                character as i32,
                Image {
                    width: sprite.width as usize,
                    height: sprite.height as usize,
                    rgba: sprite.rgba.clone(),
                },
            ))
        })
        .collect()
}

/// The cached copy of the discs' game over picture.
const GAME_OVER_CACHE: &str = "game-over.tim";

/// The PS1 game over picture (`tantalus_psxaudio::ui_art::GAME_OVER_FILE`), from the cache or
/// the discs.
fn ps1_game_over() -> Result<Image, String> {
    let cache = cache_dir().join(GAME_OVER_CACHE);
    let bytes = match std::fs::read(&cache) {
        Ok(bytes) => bytes,
        Err(_) => {
            let discs = settings()
                .discs
                .as_deref()
                .ok_or("no cached game over picture and no disc folder")?;
            let bytes = tantalus_psxaudio::ui_art::read_resident_file(
                discs,
                tantalus_psxaudio::ui_art::GAME_OVER_FILE,
            )?;
            if let Err(error) =
                std::fs::create_dir_all(cache_dir()).and_then(|()| std::fs::write(&cache, &bytes))
            {
                println!("ui skin: cannot cache the game over picture: {error}");
            }
            bytes
        }
    };
    let tim = Tim::parse(&bytes)?;
    Ok(Image {
        width: tim.width,
        height: tim.height,
        rgba: tim.rgba((0, 0, tim.width, tim.height), 0),
    })
}

/// The Steam game over picture: the `gameover` texture of `GameOverUI.GameOverSprite`.
fn steam_game_over(game_data: &Path) -> Result<Image, String> {
    use tantalus_assets::pc::serialized_inventory::StandaloneSerialized;
    let mut file = StandaloneSerialized::open(&game_data.join(HAND_FILE))
        .map_err(|error| error.to_string())?;
    let textures: Vec<_> = file
        .objects()
        .iter()
        .filter(|object| object.class_id == tantalus_assets::pc::ui_atlas::TEXTURE_2D_CLASS_ID)
        .cloned()
        .collect();
    for object in textures {
        if file.object_name(&object).ok().as_deref() != Some("gameover") {
            continue;
        }
        let bytes = file
            .read_object(&object)
            .map_err(|error| error.to_string())?;
        let texture = tantalus_assets::pc::texture::Texture2D::parse(&bytes, "gameover")
            .map_err(|error| error.to_string())?;
        let rgba = texture.rgba_bytes().map_err(|error| error.to_string())?;
        return Ok(Image {
            width: texture.width.max(0) as usize,
            height: texture.height.max(0) as usize,
            rgba,
        });
    }
    Err("no `gameover` texture".to_owned())
}

/// The game over picture of the skin (`GameOverUI`), the PS1 one when the PS1 skin draws.
pub fn game_over_picture(game_data: &Path) -> Option<Image> {
    if active() == UiSkin::Ps1 {
        match ps1_game_over() {
            Ok(image) => return Some(image),
            Err(error) => println!("ui skin: PS1 game over picture: {error}; using the Steam one"),
        }
    }
    steam_game_over(game_data)
        .map_err(|error| println!("!! game over picture: {error}"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skin_names_parse() {
        assert_eq!(UiSkin::parse("PS1"), Ok(UiSkin::Ps1));
        assert_eq!(UiSkin::parse(" steam "), Ok(UiSkin::Steam));
        assert!(UiSkin::parse("gba").is_err());
    }

    #[test]
    fn config_lines_give_the_skin_and_discs() {
        let file = "fps_cap = 60\nui_skin = steam # the PC look\npsx_discs = D:/discs\n";
        assert_eq!(config_value(file, "ui_skin"), Some("steam"));
        assert_eq!(config_value(file, "psx_discs"), Some("D:/discs"));
        assert_eq!(config_value(file, "missing"), None);
    }

    #[test]
    fn the_default_skin_is_ps1() {
        assert_eq!(UiSkin::default(), UiSkin::Ps1);
    }
}
