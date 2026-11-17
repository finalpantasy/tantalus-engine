//! The VRAM side of the plugin's loader: what `SFX_InitSystem` and the
//! records of an effect package upload, in order.
//!
//! Translated from Memoria's rewrite: `SfxVramInitUpload` (u0110),
//! `SfxVramPagesUploadOrRestore` (u0003), `SfxLoaderStep` and its record
//! handlers (u0070: types 0, 1, 4 and 9) and `SfxUploadTexturePages`
//! (u0067). The original copies each record's sectors to a buffer in PS1
//! memory and uploads from there; here the uploads read the record bytes
//! directly, which is the same data.

use crate::image::PluginImage;
use crate::vram::{Rect, Vram};
use tantalus_assets::pc::special_effect::EffectFile;

/// One `LoadImage` (host callback 0x64).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upload {
    pub rect: Rect,
    pub pixels: Vec<u16>,
}

fn words(bytes: &[u8]) -> Vec<u16> {
    bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect()
}

fn upload(rect: Rect, bytes: &[u8]) -> Upload {
    let count = (i32::from(rect.w).max(0) * i32::from(rect.h).max(0)) as usize;
    let mut pixels = words(
        bytes
            .get(..(2 * count).min(bytes.len()))
            .unwrap_or_default(),
    );
    pixels.resize(count, 0);
    Upload { rect, pixels }
}

/// RVAs of the templates `SfxVramInitUpload` copies (`unk_18004F020`,
/// `unk_180064C60`, `unk_180060060`, `unk_18004F860`).
const BLOCK_TEMPLATES: [(u32, usize); 3] = [(0x4F020, 2048), (0x64C60, 8192), (0x60060, 14336)];
const PAGE_TEMPLATE: (u32, usize) = (0x4F860, 67584);

/// `SfxVramInitUpload`: the 64x192 block at (768, 320), the four 64x128
/// pages at x 768/832, y 256/384, and the 256x2 strip at (0, 240) (the
/// page buffer's bytes past the four pages, `unk_1802308D0`).
pub fn init_uploads(image: &PluginImage) -> Option<Vec<Upload>> {
    let mut block = Vec::with_capacity(24576);
    for (rva, len) in BLOCK_TEMPLATES {
        block.extend(image.read(rva, len)?);
    }
    let mut out = vec![upload(
        Rect {
            x: 768,
            y: 320,
            w: 64,
            h: 192,
        },
        &block,
    )];
    let pages = image.read(PAGE_TEMPLATE.0, PAGE_TEMPLATE.1)?;
    for i in 0..4i16 {
        let x = 32 * ((i & !1) + 24);
        let y = ((i & 1) + 2) << 7;
        let start = 0x4000 * i as usize;
        out.push(upload(
            Rect {
                x,
                y,
                w: 64,
                h: 128,
            },
            &pages[start..],
        ));
    }
    out.push(upload(
        Rect {
            x: 0,
            y: 240,
            w: 256,
            h: 2,
        },
        &pages[0x10000..],
    ));
    Some(out)
}

/// A record of an effect package: its type, flag byte and data.
#[derive(Debug, Clone, Copy)]
pub struct RecordData<'a> {
    pub record_type: u8,
    pub flag: u8,
    pub data: &'a [u8],
}

/// The loader's state that crosses records (`dword_180323234` page entry
/// cursor and `word_180323238` its use count, the type 0 record buffer).
#[derive(Debug, Default)]
pub struct Loader {
    /// The last type 0 record (the `unk_1802208D0` buffer) and the offset
    /// of its texture table.
    buffer: Vec<u8>,
    table: usize,
    /// Offset of the page pixels in `buffer` (`dword_18032323C`).
    pixels_at: usize,
    entry: usize,
    entry_uses: u16,
    /// `dword_18032318C` bit 0: a page went to x 768..896, y 256..512.
    pub touched_effect_area: bool,
}

fn i32_at(bytes: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

impl Loader {
    /// `SfxLoaderStepProcessRecord` for one loaded record: its uploads.
    pub fn record(&mut self, record: RecordData<'_>) -> Vec<Upload> {
        let mut out = Vec::new();
        match record.record_type {
            0 => self.images(record, &mut out),
            1 => {
                // Type 1 sectors land at the type 0 table (`dword_18032323C`)
                // and upload from there.
                let pixels = record.data.to_vec();
                let end = (self.pixels_at + pixels.len()).min(self.buffer.len());
                if self.pixels_at < end {
                    let len = end - self.pixels_at;
                    self.buffer[self.pixels_at..end].copy_from_slice(&pixels[..len]);
                }
                self.texture_pages(record.flag, &pixels, &mut out);
            }
            4 => model_textures(record.data, &mut out),
            9 => self.vram_pages(record, &mut out),
            _ => {}
        }
        out
    }

    /// `SfxLoaderStepImages`: `+4` offset of the 8-byte RECT headers, `+8`
    /// their count, `+0` offset of the texture table (whose first word is
    /// the page pixels' offset; 8-byte page entries follow at `+8`).
    fn images(&mut self, record: RecordData<'_>, out: &mut Vec<Upload>) {
        let buffer = record.data;
        let (Some(list), Some(count), Some(table)) =
            (i32_at(buffer, 4), i32_at(buffer, 8), i32_at(buffer, 0))
        else {
            return;
        };
        let mut at = list.max(0) as usize;
        for _ in 0..count.max(0) {
            let (Some(x), Some(y), Some(w), Some(h)) = (
                u16_at(buffer, at),
                u16_at(buffer, at + 2),
                u16_at(buffer, at + 4),
                u16_at(buffer, at + 6),
            ) else {
                return;
            };
            at += 8;
            let size = 2 * usize::from(w) * usize::from(h);
            if w != 0 && h != 0 {
                let rect = Rect {
                    x: x as i16,
                    y: y as i16,
                    w: w as i16,
                    h: h as i16,
                };
                out.push(upload(rect, buffer.get(at..).unwrap_or_default()));
            }
            at += size;
        }
        let table = table.max(0) as usize;
        let pixels_at = i32_at(buffer, table).unwrap_or(0).max(0) as usize;
        self.buffer = buffer.to_vec();
        self.table = table;
        self.pixels_at = pixels_at;
        self.entry = table + 8;
        self.entry_uses = 0;
        let pixels = buffer.get(pixels_at..).unwrap_or_default().to_vec();
        self.texture_pages(record.flag, &pixels, out);
    }

    /// `SfxUploadTexturePages`: `count` 64x128 pages from consecutive 16 KB
    /// blocks; each page entry gives x, y (`+0`, `+2`); an entry whose `+6`
    /// is 128 is used once, others twice (the second at y + 128).
    fn texture_pages(&mut self, count: u8, pixels: &[u8], out: &mut Vec<Upload>) {
        for page in 0..usize::from(count) {
            let x = u16_at(&self.buffer, self.entry).unwrap_or(0) as i16;
            let y = u16_at(&self.buffer, self.entry + 2).unwrap_or(0);
            let height = u16_at(&self.buffer, self.entry + 6).unwrap_or(0);
            self.entry_uses += 1;
            let first = self.entry_uses < 2;
            let rect_y = if first { y } else { y.wrapping_add(128) } as i16;
            if !first || height == 128 {
                self.entry += 8;
                self.entry_uses = 0;
            }
            let rect = Rect {
                x,
                y: rect_y,
                w: 64,
                h: 128,
            };
            self.note_area(rect);
            out.push(upload(
                rect,
                pixels.get(page * 0x4000..).unwrap_or_default(),
            ));
        }
    }

    fn note_area(&mut self, rect: Rect) {
        if rect.x >= 768 && rect.x + rect.w <= 896 && rect.y >= 256 && rect.y + rect.h <= 512 {
            self.touched_effect_area = true;
        }
    }

    /// `SfxLoaderStepVramPages`: up to eight 64x128 pages chosen by the flag
    /// bits (bit k covers pages 2k and 2k + 1 for the first four, then one
    /// bit per page).
    fn vram_pages(&mut self, record: RecordData<'_>, out: &mut Vec<Upload>) {
        let mut mask = 1u8;
        let mut pixels = 0usize;
        for i in 0..8i16 {
            if mask & record.flag != 0 {
                let y = ((i & 1) + 2) * 128;
                let x = if i < 4 {
                    ((i & !1) + 24) << 5
                } else {
                    (((i << 5) - 97) & !0x3F) + 320
                };
                let rect = Rect {
                    x,
                    y,
                    w: 64,
                    h: 128,
                };
                out.push(upload(rect, record.data.get(pixels..).unwrap_or_default()));
                self.note_area(rect);
                pixels += 0x4000;
            }
            if i >= 4 || i & 1 != 0 {
                mask = mask.wrapping_add(mask);
            }
        }
    }
}

/// `SfxLoaderStepModelTextures`: the model block's CLUT strip (256 wide,
/// `+6` rows, at `+0` + `+8`) to (256, 230), then `+4` pages of 64x128 from
/// `+0`, page i at x `(attr & 0xF) << 6`, y `((attr & 0x10) << 4) + y`
/// with `attr` the u16 at `+24 + 2i` and `y` the u16 at `+48 + 2i`.
fn model_textures(block: &[u8], out: &mut Vec<Upload>) {
    let pixels_at = u16_at(block, 0).unwrap_or(0) as i16 as isize;
    let count = u16_at(block, 4).unwrap_or(0) as i16;
    let h = u16_at(block, 6).unwrap_or(0) as i16;
    let clut = i32_at(block, 8).unwrap_or(0) as isize;
    let slice = |at: isize| {
        usize::try_from(at)
            .ok()
            .and_then(|at| block.get(at..))
            .unwrap_or_default()
    };
    if h != 0 {
        out.push(upload(
            Rect {
                x: 256,
                y: 230,
                w: 256,
                h,
            },
            slice(pixels_at + clut),
        ));
    }
    for i in 0..count.max(0) as usize {
        let attributes = u16_at(block, 24 + 2 * i).unwrap_or(0);
        let y = u16_at(block, 48 + 2 * i).unwrap_or(0);
        let rect = Rect {
            x: ((attributes & 0xF) << 6) as i16,
            y: (((attributes & 0x10) << 4) + y) as i16,
            w: 64,
            h: 128,
        };
        out.push(upload(rect, slice(pixels_at + 0x4000 * i as isize)));
    }
}

/// Applies `uploads` to `vram` in order.
pub fn apply(vram: &mut Vram, uploads: &[Upload]) {
    for upload in uploads {
        vram.load_image(upload.rect, &upload.pixels);
    }
}

/// The records `SfxLoaderStep` loads for `effect`, in order: the groups the
/// sequence's `LOAD_CHUNK` ops (0x05) name, each group's records in
/// directory order; group 0 when the sequence names none.
pub fn effect_records<'a>(effect: &EffectFile, bytes: &'a [u8]) -> Vec<RecordData<'a>> {
    let mut groups: Vec<usize> = effect
        .sequence
        .iter()
        .filter(|op| op.code == 0x05)
        .map(|op| usize::from(op.arg1))
        .collect();
    if groups.is_empty() {
        groups.push(0);
    }
    let mut out = Vec::new();
    for group in groups {
        for record in effect.records.iter().filter(|record| record.group == group) {
            let end = (record.offset + record.length).min(bytes.len());
            out.push(RecordData {
                record_type: record.record_type,
                flag: record.flag,
                data: bytes.get(record.offset..end).unwrap_or_default(),
            });
        }
    }
    out
}

/// The four pages of the plugin's own page buffer (`unk_18004F860`), as
/// `SfxVramPagesUploadOrRestore(1)` uploads them.
pub fn plugin_pages(image: &PluginImage) -> Option<Vec<Upload>> {
    let pages = image.read(PAGE_TEMPLATE.0, PAGE_TEMPLATE.1)?;
    Some(
        (0..4i16)
            .map(|i| {
                let rect = Rect {
                    x: 32 * ((i & !1) + 24),
                    y: ((i & 1) + 2) << 7,
                    w: 64,
                    h: 128,
                };
                upload(rect, &pages[0x4000 * i as usize..])
            })
            .collect(),
    )
}

/// Every upload `effect` makes as it loads, then, when one of them went to
/// the plugin's pages at x 768..896, y 256..512, the restore of those pages
/// once the sequence ends (`SeqStateUpdateEnd`, u0010).
pub fn effect_uploads(effect: &EffectFile, bytes: &[u8], image: &PluginImage) -> Vec<Upload> {
    let mut loader = Loader::default();
    let mut out: Vec<Upload> = effect_records(effect, bytes)
        .into_iter()
        .flat_map(|record| loader.record(record))
        .collect();
    if loader.touched_effect_area {
        out.extend(plugin_pages(image).unwrap_or_default());
    }
    out
}
