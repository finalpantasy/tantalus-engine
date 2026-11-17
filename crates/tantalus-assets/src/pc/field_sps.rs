//! Field SPS sprite-strip effects: the `.sps` frame files and the `spt.tcb`
//! texture container of one field, from the field's asset bundle
//! (`assets/resources/fieldmaps/<field>/<id>.sps.bytes` and `spt.tcb.bytes`).
//!
//! Formats follow `SPSEffect.LoadSPS` / `_GenerateSPSPrims` (frame files) and
//! `PSXTextureMgr.LoadTCBInVram` / `PSXTexture.GenTexture` (the container is
//! blitted into a 1024x512 PS1 VRAM; the effect samples one texture page
//! through one CLUT).

use crate::install::ImportError;
use crate::pc::asset_bundle::AssetBundleMetadata;
use crate::pc::serialized::SerializedFile;
use crate::pc::text_asset::TextAsset;
use crate::pc::unity_raw::UnityRawBundle;
use std::collections::BTreeMap;
use std::path::Path;

/// One sprite of a frame: its center in effect units (`sbyte << 2`), its
/// texture corner in the page, and its color bytes (`works.rgb`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpsPrim {
    pub x: i32,
    pub y: i32,
    pub u: u8,
    pub v: u8,
    pub color: [u8; 3],
}

/// A parsed `.sps` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpsFile {
    /// Animation frames (`ushort & 0x7FFF`); the playback counter runs in 1/16ths.
    pub frame_count: u16,
    /// `works.tpage` before the blend bits are added.
    pub tpage: u16,
    pub clut: u16,
    /// `works.h` and `works.w`: the half height and width of every sprite in effect units.
    pub half_height: i32,
    pub half_width: i32,
    pub frames: Vec<Vec<SpsPrim>>,
}

impl SpsFile {
    /// `SPSEffect.LoadSPS` for every frame.
    pub fn parse(bytes: &[u8], source: &str) -> Result<Self, ImportError> {
        let short = |what: &str| ImportError::invalid(source, format!("SPS file is short: {what}"));
        let u16_at = |at: usize| -> Result<u16, ImportError> {
            let pair = bytes.get(at..at + 2).ok_or_else(|| short("u16"))?;
            Ok(u16::from_le_bytes([pair[0], pair[1]]))
        };
        let byte_at = |at: usize| -> Result<u8, ImportError> {
            bytes.get(at).copied().ok_or_else(|| short("byte"))
        };
        let frame_count = u16_at(0)? & 0x7FFF;
        if frame_count == 0 {
            return Err(ImportError::invalid(source, "SPS file has no frames"));
        }
        let tpage = u16_at(2)?;
        let clut = u16_at(4)?;
        let half_height = (i32::from(byte_at(6)?) - 1) * 2;
        let half_width = (i32::from(byte_at(7)?) - 1) * 2;
        let work = usize::from(frame_count) * 2 + 8;
        let rgb_offset = usize::from(u16_at(work)?);
        let pt = work + 2;
        let rgb = pt + rgb_offset * 2 + 2;
        let mut frames = Vec::with_capacity(usize::from(frame_count));
        for frame in 0..usize::from(frame_count) {
            let mut at = usize::from(u16_at(frame * 2 + 8)?);
            let count = usize::from(byte_at(at)?);
            at += 1;
            let mut prims = Vec::with_capacity(count);
            for _ in 0..count {
                let x = i32::from(byte_at(at)? as i8) << 2;
                let y = i32::from(byte_at(at + 1)? as i8) << 2;
                let texpos = byte_at(at + 2)?;
                at += 3;
                let uv = pt + (usize::from(texpos & 0xF) << 1);
                let color = rgb + (usize::from(texpos >> 4) << 2);
                prims.push(SpsPrim {
                    x,
                    y,
                    u: byte_at(uv)?,
                    v: byte_at(uv + 1)?,
                    color: [byte_at(color)?, byte_at(color + 1)?, byte_at(color + 2)?],
                });
            }
            frames.push(prims);
        }
        Ok(Self {
            frame_count,
            tpage,
            clut,
            half_height,
            half_width,
            frames,
        })
    }

    /// `CommonSPSSystem.GetSpsFrameCount` for the playback counter: frames times 16.
    pub fn counter_limit(&self) -> i32 {
        i32::from(self.frame_count) << 4
    }

    /// The sprite's texel size, `(w >> 1, h >> 1)` of `SPSEffect.works`.
    pub fn texel_size(&self) -> (i32, i32) {
        (self.half_width >> 1, self.half_height >> 1)
    }

    /// The texture page's 256x256 RGBA image this effect samples
    /// (`PSXTextureMgr.GetTexture(TP, TY, TX, ClutY, ClutX)`).
    pub fn page_rgba(&self, vram: &Vram) -> Vec<u8> {
        vram.page_rgba(
            i32::from(self.tpage >> 7 & 3),
            i32::from(self.tpage >> 4 & 1),
            i32::from(self.tpage & 0xF),
            i32::from(self.clut >> 6 & 0x1FF),
            i32::from(self.clut & 0x3F),
        )
    }
}

pub const VRAM_WIDTH: usize = 1024;
pub const VRAM_HEIGHT: usize = 512;

/// The PS1 video memory as `PSXTextureMgr.originalVram`: 1024x512 words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vram {
    words: Vec<u16>,
}

impl Default for Vram {
    fn default() -> Self {
        Self {
            words: vec![0; VRAM_WIDTH * VRAM_HEIGHT],
        }
    }
}

impl Vram {
    fn word(&self, x: usize, y: usize) -> u16 {
        // `(y + i) * 1024 + x` over the whole array, as the reference indexes it.
        self.words
            .get(y * VRAM_WIDTH + x)
            .copied()
            .unwrap_or_default()
    }

    /// `PSXTextureMgr.LoadTCBInVram`.
    pub fn load_tcb(bytes: &[u8], source: &str) -> Result<Self, ImportError> {
        let short = || ImportError::invalid(source, "TCB container is short");
        let u32_at = |at: usize| -> Result<u32, ImportError> {
            let four = bytes.get(at..at + 4).ok_or_else(short)?;
            Ok(u32::from_le_bytes([four[0], four[1], four[2], four[3]]))
        };
        let i16_at = |at: usize| -> Result<i16, ImportError> {
            let two = bytes.get(at..at + 2).ok_or_else(short)?;
            Ok(i16::from_le_bytes([two[0], two[1]]))
        };
        let mut vram = Self::default();
        let blit = |vram: &mut Self, x: i16, y: i16, w: i16, h: i16, mut at: usize| {
            let (x, y, w, h) = (x as usize, y as usize, w as usize, h as usize);
            for row in 0..h {
                for column in 0..w {
                    let pair = bytes.get(at..at + 2).ok_or_else(short)?;
                    at += 2;
                    if let Some(word) = vram.words.get_mut((y + row) * VRAM_WIDTH + x + column) {
                        *word = u16::from_le_bytes([pair[0], pair[1]]);
                    }
                }
            }
            Ok::<usize, ImportError>(at)
        };
        let second = u32_at(0)? as usize;
        let mut first = u32_at(4)? as usize;
        let first_count = u32_at(8)? as i32;
        for _ in 0..first_count.max(0) {
            let (x, y, w, h) = (
                i16_at(first)?,
                i16_at(first + 2)?,
                i16_at(first + 4)?,
                i16_at(first + 6)?,
            );
            blit(&mut vram, x, y, w, h, first + 8)?;
            first += (w as usize) * (h as usize) * 2 + 8;
        }
        let mut image = u32_at(second)? as usize;
        let second_count = u32_at(second + 4)? as i32;
        let mut header = second + 8;
        for _ in 0..second_count.max(0) {
            let (x, y, w, h) = (
                i16_at(header)?,
                i16_at(header + 2)?,
                i16_at(header + 4)?,
                i16_at(header + 6)?,
            );
            image = blit(&mut vram, x, y, w, h, image)?;
            header += 8;
        }
        Ok(vram)
    }

    /// `PSXTexture.GenTexture` for a 256x256 page: `tp` 0 is 4-bit, 1 is 8-bit and 2 is 15-bit
    /// direct color; pixel value 0 is transparent. `tx` is the page column (x 64 words), `ty`
    /// the page row (x 256 lines), and the CLUT sits at `(clut_x * 16, clut_y)`.
    pub fn page_rgba(&self, tp: i32, ty: i32, tx: i32, clut_y: i32, clut_x: i32) -> Vec<u8> {
        let (page_x, page_y) = (tx as usize * 64, ty as usize * 256);
        let (clut_x, clut_y) = (clut_x as usize * 16, clut_y as usize);
        let mut out = Vec::with_capacity(256 * 256 * 4);
        let mut push = |value: u16| {
            out.push(((value & 0x1F) << 3) as u8);
            out.push(((value & 0x3E0) >> 2) as u8);
            out.push(((value & 0x7C00) >> 7) as u8);
            out.push(if value == 0 { 0 } else { 255 });
        };
        for y in 0..256usize {
            match tp {
                0 => {
                    for x in 0..64usize {
                        let word = self.word(page_x + x, page_y + y);
                        for shift in [0, 4, 8, 12] {
                            push(self.word(clut_x + usize::from(word >> shift & 0xF), clut_y));
                        }
                    }
                }
                1 => {
                    for x in 0..128usize {
                        let word = self.word(page_x + x, page_y + y);
                        for shift in [0, 8] {
                            push(self.word(clut_x + usize::from(word >> shift & 0xFF), clut_y));
                        }
                    }
                }
                _ => {
                    for x in 0..256usize {
                        push(self.word(page_x + x, page_y + y));
                    }
                }
            }
        }
        out
    }
}

/// Everything one field's bundle holds for SPS: the container and every frame file by id.
#[derive(Debug, Clone)]
pub struct FieldSps {
    pub vram: Vram,
    pub files: BTreeMap<i32, SpsFile>,
}

/// Loads `spt.tcb` and every `<id>.sps` of `field_name` from a field bundle
/// (`p0data11.bin`-`p0data19.bin`); `Ok(None)` when this bundle does not hold the field.
pub fn load_field_sps(
    bundle_path: &Path,
    field_name: &str,
) -> Result<Option<FieldSps>, ImportError> {
    let source = bundle_path.display().to_string();
    let bundle = UnityRawBundle::open(bundle_path)?;
    let bytes = bundle.map_entry(0, 300 * 1024 * 1024)?;
    let serialized = SerializedFile::parse(&bytes, &source)?;
    let Some(metadata_index) = serialized
        .report()
        .objects
        .iter()
        .position(|object| object.class_id == 142)
    else {
        return Ok(None);
    };
    let metadata = AssetBundleMetadata::parse(
        serialized.object_data(metadata_index)?,
        &format!("{source}:AssetBundle"),
    )?;
    if metadata.atlas_for(field_name).is_none() {
        return Ok(None);
    }
    let prefix = format!("assets/resources/fieldmaps/{}/", field_name.to_lowercase());
    let text_asset = |path_id: i64| -> Result<TextAsset, ImportError> {
        let index = serialized
            .report()
            .objects
            .iter()
            .position(|object| object.class_id == 49 && object.path_id == path_id)
            .ok_or_else(|| ImportError::invalid(&source, "SPS TextAsset is not in the bundle"))?;
        TextAsset::parse(serialized.object_data(index)?, &source)
    };
    let mut vram = Vram::default();
    let mut files = BTreeMap::new();
    for (path, info) in &metadata.container {
        let Some(name) = path.strip_prefix(&prefix) else {
            continue;
        };
        if name == "spt.tcb.bytes" {
            vram = Vram::load_tcb(
                &text_asset(info.path_id)?.script,
                &format!("{source}:{path}"),
            )?;
        } else if let Some(id) = name
            .strip_suffix(".sps.bytes")
            .and_then(|id| id.parse::<i32>().ok())
        {
            let asset = text_asset(info.path_id)?;
            files.insert(
                id,
                SpsFile::parse(&asset.script, &format!("{source}:{path}"))?,
            );
        }
    }
    Ok(Some(FieldSps { vram, files }))
}

/// The field bundles' SPS data by field name, read on first use and kept.
#[derive(Debug)]
pub struct FieldSpsLibrary {
    bundles: Vec<std::path::PathBuf>,
    fields: std::sync::Mutex<std::collections::HashMap<String, Option<std::sync::Arc<FieldSps>>>>,
}

impl FieldSpsLibrary {
    /// `bundles` are the field bundles (`p0data11.bin`-`p0data19.bin`).
    pub fn new(bundles: Vec<std::path::PathBuf>) -> Self {
        Self {
            bundles,
            fields: Default::default(),
        }
    }

    /// The SPS files and texture container of `field_name` (`FBG_...`), or `None` when no
    /// bundle holds the field or its data does not parse.
    pub fn field(&self, field_name: &str) -> Option<std::sync::Arc<FieldSps>> {
        let mut fields = self.fields.lock().ok()?;
        if let Some(known) = fields.get(field_name) {
            return known.clone();
        }
        let found = self
            .bundles
            .iter()
            .find_map(|bundle| load_field_sps(bundle, field_name).ok().flatten())
            .map(std::sync::Arc::new);
        fields.insert(field_name.to_owned(), found.clone());
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-frame file with one sprite at (-4, 8), texture corner (16, 32), color (1, 2, 3).
    fn sample() -> Vec<u8> {
        let mut file = vec![1, 0, 0x12, 0x00, 0x34, 0x01, 5, 9];
        file.extend([24, 0]); // frame 0 at offset 24
        file.extend([1, 0]); // work area at 10: one uv entry before the colors
        file.extend([16, 32]); // uv table at 12: entry 0
        file.extend([0, 0]); // padding word; colors start at 12 + 1 * 2 + 2 = 16
        file.extend([0, 0, 0, 0]); // color 0
        file.extend([1, 2, 3, 0]); // color 1
        file.push(1); // offset 24: one sprite
        file.extend([(-1i8) as u8, 2, 0x10]); // x, y, texpos (color 1, uv 0)
        file
    }

    #[test]
    fn parses_a_hand_built_file() {
        let sps = SpsFile::parse(&sample(), "test").unwrap();
        assert_eq!(sps.frame_count, 1);
        assert_eq!((sps.half_height, sps.half_width), (8, 16));
        assert_eq!(sps.texel_size(), (8, 4));
        assert_eq!(sps.counter_limit(), 16);
        assert_eq!(
            sps.frames[0][0],
            SpsPrim {
                x: -4,
                y: 8,
                u: 16,
                v: 32,
                color: [1, 2, 3]
            }
        );
    }

    #[test]
    fn rejects_short_files() {
        assert!(SpsFile::parse(&sample()[..12], "test").is_err());
        assert!(SpsFile::parse(&[0, 0], "test").is_err());
    }

    #[test]
    fn four_bit_pages_use_the_clut() {
        let mut vram = Vram::default();
        // Page (0, 0): the first word holds indices 1, 0, 0, 0; CLUT at (16, 480).
        vram.words[0] = 1;
        vram.words[480 * VRAM_WIDTH + 17] = 0x7C1F;
        let page = vram.page_rgba(0, 0, 0, 480, 1);
        assert_eq!(&page[..4], &[0xF8, 0x00, 0xF8, 255]);
        assert_eq!(page[7], 0, "index 0 is transparent");
    }

    #[test]
    #[ignore = "needs the game data"]
    fn field_50_has_its_effects() {
        let streaming = std::path::Path::new(
            "E:/SteamLibrary/steamapps/common/FINAL FANTASY IX/StreamingAssets",
        );
        let found = (11..=19)
            .find_map(|n| {
                load_field_sps(
                    &streaming.join(format!("p0data{n}.bin")),
                    "FBG_N00_TSHP_MAP002_TH_CGR_1",
                )
                .unwrap()
            })
            .expect("field 50 is in a field bundle");
        assert!(found.files.contains_key(&42) && found.files.contains_key(&43));
        for sps in found.files.values() {
            let page = sps.page_rgba(&found.vram);
            assert_eq!(page.len(), 256 * 256 * 4);
            assert!(
                page.chunks(4).any(|pixel| pixel[3] != 0),
                "the page has texels"
            );
            assert!(sps.frames.iter().all(|frame| !frame.is_empty()));
        }
    }
}
