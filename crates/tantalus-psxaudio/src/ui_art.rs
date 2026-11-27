//! The PS1 game's resident UI art, read from the owner's own discs: the dialog font with its glyph
//! table, the icon sheet (hand cursor, button marks, menu captions) and the window sheet (window
//! grain, frame, battle digits).
//!
//! Layout, worked out from the discs (no public documentation exists for it):
//! - The art is file 1 of the `FF9.IMG` group that holds the resident system data (the group's
//!   id differs between discs). The file is a cluster ([`crate::cluster`]) with one list of type
//!   [`GLYPH_TABLE`] (the font's glyph table) and one of type [`TIMS`] (four PS1 TIM images,
//!   ids 0, 10000, 10001 and 10002).
//! - TIM 0 is the dialog font, a 4-bit 256x120 sheet. TIM 10001 is the icon sheet (13 palettes),
//!   TIM 10002 the window sheet (16 palettes); TIM 10000 is a fixed-cell small font.
//! - The glyph table: a 12-byte header (`u16` glyph count at byte 8), then one 4-byte entry per
//!   glyph: x and y of the glyph's 12-line cell in the font sheet, a byte whose low nibble is
//!   the drawn width and high nibble the left offset, and a byte whose high nibble is the
//!   advance. Glyph `i` is the PS1 text code `i`: `0`-`9`, `+ - = * %`, space, `A`-`Z`,
//!   `( ! ? ” : .`, `a`-`z`, `) , / · ~ &`, then the Japanese set.
//!
//! The PS1 font has its drop shadow drawn into the sheet (the darker of the two text colors).
//!
//! The TIM format is psx-spx "GPU: Texture image (TIM)" (documentation only).

use crate::cluster::{self, ClusterObject};
use crate::disc::{Disc, find_discs};
use crate::img::Img;

/// Cluster list type of the dialog font's glyph table.
pub const GLYPH_TABLE: u8 = 0x0D;
/// Cluster list type of TIM images.
pub const TIMS: u8 = 0x04;
/// TIM ids in the resident UI file.
pub const FONT_TIM: u16 = 0;
pub const ICON_TIM: u16 = 10001;
pub const WINDOW_TIM: u16 = 10002;
/// Lines of one glyph cell in the font sheet.
pub const GLYPH_CELL_HEIGHT: usize = 12;

/// A decoded TIM: its VRAM place, its palettes and its pixels as palette indices (4- and 8-bit
/// images) or direct 15-bit colors (16-bit images).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tim {
    pub vram_x: u16,
    pub vram_y: u16,
    /// Width in pixels (not VRAM halfwords).
    pub width: usize,
    pub height: usize,
    pub bits: u8,
    /// Palette rows of 15-bit colors (empty for a 16-bit image).
    pub cluts: Vec<Vec<u16>>,
    /// One value per pixel, top row first.
    pub pixels: Vec<u16>,
}

fn u16le(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(at..at + 2)?.try_into().ok()?))
}
fn u32le(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

/// A PS1 15-bit color as RGBA8: black (`0x0000`) is the transparent color; the semi-transparency
/// bit is ignored (the UI draws its sprites opaque).
pub fn rgba_of(color: u16) -> [u8; 4] {
    if color == 0 {
        return [0, 0, 0, 0];
    }
    let c = |shift: u16| {
        let v = ((color >> shift) & 31) as u8;
        (v << 3) | (v >> 2)
    };
    [c(0), c(5), c(10), 255]
}

impl Tim {
    /// Parses a TIM (magic `0x10`, flags 8, 9 or 2).
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        if u32le(data, 0) != Some(0x10) {
            return Err("not a TIM (no 0x10 magic)".to_owned());
        }
        let flags = u32le(data, 4).ok_or("TIM header cut short")?;
        let bits = match flags & 7 {
            0 => 4,
            1 => 8,
            2 => 16,
            other => return Err(format!("TIM pixel mode {other} is not supported")),
        };
        let mut at = 8;
        let mut cluts = Vec::new();
        if flags & 8 != 0 {
            let len = u32le(data, at).ok_or("TIM palette cut short")? as usize;
            let w = usize::from(u16le(data, at + 8).ok_or("TIM palette cut short")?);
            let h = usize::from(u16le(data, at + 10).ok_or("TIM palette cut short")?);
            for row in 0..h {
                let colors = (0..w)
                    .map(|i| u16le(data, at + 12 + (row * w + i) * 2))
                    .collect::<Option<Vec<_>>>()
                    .ok_or("TIM palette cut short")?;
                cluts.push(colors);
            }
            at += len;
        }
        let head = |o: usize| u16le(data, at + o).ok_or("TIM image header cut short");
        let (vram_x, vram_y, halfwords, height) = (head(4)?, head(6)?, head(8)?, head(10)?);
        let width = usize::from(halfwords) * 16 / usize::from(bits);
        let height = usize::from(height);
        let body = data
            .get(at + 12..at + 12 + usize::from(halfwords) * 2 * height)
            .ok_or("TIM pixels cut short")?;
        let row_bytes = usize::from(halfwords) * 2;
        let mut pixels = Vec::with_capacity(width * height);
        for y in 0..height {
            let row = &body[y * row_bytes..(y + 1) * row_bytes];
            for x in 0..width {
                pixels.push(match bits {
                    4 => u16::from((row[x / 2] >> ((x & 1) * 4)) & 15),
                    8 => u16::from(row[x]),
                    _ => u16::from_le_bytes([row[x * 2], row[x * 2 + 1]]),
                });
            }
        }
        if bits != 16 && cluts.is_empty() {
            return Err("paletted TIM without a palette".to_owned());
        }
        Ok(Self {
            vram_x,
            vram_y,
            width,
            height,
            bits,
            cluts,
            pixels,
        })
    }

    /// The color of pixel `(x, y)` with palette row `clut` (clamped to the last row).
    pub fn color(&self, x: usize, y: usize, clut: usize) -> u16 {
        let value = self.pixels[y * self.width + x];
        if self.bits == 16 {
            return value;
        }
        let row = &self.cluts[clut.min(self.cluts.len() - 1)];
        row.get(usize::from(value)).copied().unwrap_or(0)
    }

    /// The rectangle `(x, y, w, h)` as RGBA8 with palette row `clut`; parts outside the image are
    /// transparent.
    pub fn rgba(&self, rect: (usize, usize, usize, usize), clut: usize) -> Vec<u8> {
        let (x0, y0, w, h) = rect;
        let mut out = Vec::with_capacity(w * h * 4);
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                let px = if x < self.width && y < self.height {
                    rgba_of(self.color(x, y, clut))
                } else {
                    [0; 4]
                };
                out.extend_from_slice(&px);
            }
        }
        out
    }
}

/// One glyph of the dialog font: its cell in the font sheet and how it is spaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlyphCell {
    pub x: u8,
    pub y: u8,
    /// Pixels drawn from the cell (shadow included).
    pub width: u8,
    /// Pixels the glyph is drawn right of the pen.
    pub offset: u8,
    /// Pixels the pen moves.
    pub advance: u8,
}

/// Parses the glyph table object.
pub fn parse_glyph_table(data: &[u8]) -> Result<Vec<GlyphCell>, String> {
    let count = usize::from(u16le(data, 8).ok_or("glyph table header cut short")?);
    (0..count)
        .map(|i| {
            let e = data
                .get(12 + i * 4..16 + i * 4)
                .ok_or("glyph table cut short")?;
            Ok(GlyphCell {
                x: e[0],
                y: e[1],
                width: e[2] & 15,
                offset: e[2] >> 4,
                advance: e[3] >> 4,
            })
        })
        .collect()
}

/// The PS1 text code of a character in the English dialog font, if the font has it.
pub fn glyph_code(ch: char) -> Option<usize> {
    const PUNCT: [(char, usize); 18] = [
        ('+', 10),
        ('-', 11),
        ('=', 12),
        ('*', 13),
        ('%', 14),
        (' ', 15),
        ('(', 42),
        ('!', 43),
        ('?', 44),
        ('”', 45),
        ('"', 45),
        ('“', 45),
        (':', 46),
        ('.', 47),
        (')', 74),
        (',', 75),
        ('/', 76),
        ('&', 79),
    ];
    match ch {
        '0'..='9' => Some(ch as usize - '0' as usize),
        'A'..='Z' => Some(16 + ch as usize - 'A' as usize),
        'a'..='z' => Some(48 + ch as usize - 'a' as usize),
        '·' => Some(77),
        '~' => Some(78),
        '…' => Some(82),
        _ => PUNCT.iter().find(|(c, _)| *c == ch).map(|(_, code)| *code),
    }
}

/// The resident UI art.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiArt {
    pub font: Tim,
    pub glyphs: Vec<GlyphCell>,
    pub icons: Tim,
    pub window: Tim,
}

fn object(objects: &[ClusterObject], list_type: u8, id: u16) -> Option<&ClusterObject> {
    objects
        .iter()
        .find(|o| o.list_type == list_type && o.id == id)
}

impl UiArt {
    /// Parses the resident UI file (the cluster described in the module header).
    pub fn parse(file: &[u8]) -> Result<Self, String> {
        let objects = cluster::objects(file);
        let get = |list_type: u8, id: u16| {
            object(&objects, list_type, id)
                .map(|o| &file[o.range.clone()])
                .ok_or(format!("UI file has no object {id} of type {list_type:#x}"))
        };
        Ok(Self {
            font: Tim::parse(get(TIMS, FONT_TIM)?)?,
            glyphs: parse_glyph_table(get(GLYPH_TABLE, 0)?)?,
            icons: Tim::parse(get(TIMS, ICON_TIM)?)?,
            window: Tim::parse(get(TIMS, WINDOW_TIM)?)?,
        })
    }

    /// Whether `file` is the resident UI file (has the glyph table and the three sheets).
    pub fn is_ui_file(file: &[u8]) -> bool {
        let objects = cluster::objects(file);
        object(&objects, GLYPH_TABLE, 0).is_some()
            && [FONT_TIM, ICON_TIM, WINDOW_TIM]
                .iter()
                .all(|id| object(&objects, TIMS, *id).is_some())
    }
}

/// File of the resident group (the group holding the UI file) with the game over picture: a
/// bare 16-bit TIM of 320 by 224 pixels, the whole PS1 screen (the Steam `gameover` sprite's
/// source).
pub const GAME_OVER_FILE: u32 = 11;

/// Reads the resident UI file from the first disc image found in `disc_dir`.
pub fn read_ui_file(disc_dir: &std::path::Path) -> Result<Vec<u8>, String> {
    read_resident_file(disc_dir, 1)
}

/// Reads file `id` of the resident group (the group whose file 1 is the UI file) from the first
/// disc image found in `disc_dir`.
pub fn read_resident_file(disc_dir: &std::path::Path, id: u32) -> Result<Vec<u8>, String> {
    let discs = find_discs(disc_dir);
    let (_, path) = discs
        .first()
        .ok_or_else(|| format!("no FF9 disc images in {}", disc_dir.display()))?;
    let mut disc = Disc::open(path)?;
    let img = Img::open(&mut disc)?;
    // File 1 of a group, small, a cluster with the font table and the TIMs.
    for file in img.files.iter().filter(|f| f.id == 1 && f.blocks <= 64) {
        let Ok(data) = img.read(&mut disc, file) else {
            continue;
        };
        if !UiArt::is_ui_file(&data) {
            continue;
        }
        if id == 1 {
            return Ok(data);
        }
        let wanted = img
            .group(file.group)
            .find(|f| f.id == id)
            .copied()
            .ok_or_else(|| format!("{}: the resident group has no file {id}", path.display()))?;
        return img.read(&mut disc, &wanted);
    }
    Err(format!("{} has no resident UI file", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4-bit TIM, 4x2 pixels, two palette rows.
    fn tim4() -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&0x10u32.to_le_bytes());
        d.extend_from_slice(&8u32.to_le_bytes());
        // Palette block: 12 + 2 rows * 16 colors * 2 bytes.
        d.extend_from_slice(&(12u32 + 64).to_le_bytes());
        d.extend_from_slice(&[0, 0, 0, 0]);
        d.extend_from_slice(&16u16.to_le_bytes());
        d.extend_from_slice(&2u16.to_le_bytes());
        for row in 0..2u16 {
            for i in 0..16u16 {
                d.extend_from_slice(&(i * 2 + row).to_le_bytes());
            }
        }
        // Image block: 4 pixels = 1 halfword wide, 2 high.
        d.extend_from_slice(&(12u32 + 4).to_le_bytes());
        d.extend_from_slice(&896u16.to_le_bytes());
        d.extend_from_slice(&256u16.to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&2u16.to_le_bytes());
        d.extend_from_slice(&[0x21, 0x43, 0x65, 0x87]);
        d
    }

    #[test]
    fn decodes_a_4bit_tim_low_nibble_first() {
        let tim = Tim::parse(&tim4()).unwrap();
        assert_eq!(
            (tim.vram_x, tim.vram_y, tim.width, tim.height),
            (896, 256, 4, 2)
        );
        assert_eq!(tim.pixels, vec![1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(tim.color(1, 0, 0), 4);
        assert_eq!(tim.color(1, 0, 1), 5);
        // A palette row past the last clamps.
        assert_eq!(tim.color(1, 0, 9), 5);
    }

    #[test]
    fn black_is_transparent_and_five_bits_widen() {
        assert_eq!(rgba_of(0), [0, 0, 0, 0]);
        assert_eq!(rgba_of(0x7FFF), [255, 255, 255, 255]);
        assert_eq!(rgba_of(31), [255, 0, 0, 255]);
    }

    #[test]
    fn glyph_entries_split_their_nibbles() {
        let mut d = vec![0u8; 12];
        d[8] = 2;
        d.extend_from_slice(&[8, 0, 0x24, 0x80, 106, 0, 0x0C, 0xB0]);
        let cells = parse_glyph_table(&d).unwrap();
        assert_eq!(
            cells[0],
            GlyphCell {
                x: 8,
                y: 0,
                width: 4,
                offset: 2,
                advance: 8
            }
        );
        assert_eq!((cells[1].width, cells[1].advance), (12, 11));
    }

    #[test]
    fn english_characters_map_to_ps1_codes() {
        assert_eq!(glyph_code('0'), Some(0));
        assert_eq!(glyph_code(' '), Some(15));
        assert_eq!(glyph_code('A'), Some(16));
        assert_eq!(glyph_code('Z'), Some(41));
        assert_eq!(glyph_code('a'), Some(48));
        assert_eq!(glyph_code('z'), Some(73));
        assert_eq!(glyph_code('&'), Some(79));
        assert_eq!(glyph_code('\''), None);
    }
}
