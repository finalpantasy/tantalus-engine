//! Memoria-style embedded font bundles (`EmbeddedAsset/FA/p_fa.mpc`).
//!
//! Source of truth (pinned Memoria reference): `EncryptFontManager.LoadFont` loads
//! `EmbeddedAsset/FA/<platform prefix>_fa.mpc`, runs `ByteEncryption.Decryption` over the
//! bytes and hands the result to `AssetBundle.CreateFromMemoryImmediate`. The
//! decryption is a fixed block rotation with no key: the last 1024 bytes of the file
//! become the first 1024 bytes of the bundle, followed by file bytes `1024..len-1024`.
//!
//! The decrypted bytes are an ordinary Unity bundle. This module reads the in-memory
//! `UnityRaw` (format 3) container, enumerates the serialized file through the generic
//! type-tree reader, and extracts each `Font` object.

use super::serialized::SerializedFile;
use super::unity_object::type_tree::{TypeTree, Value};
use crate::install::ImportError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const BLOCK_SIZE: usize = 1024;
const FONT_CLASS_ID: u16 = 128;
const TEXTURE2D_CLASS_ID: u16 = 28;
const MATERIAL_CLASS_ID: u16 = 21;

/// Port of `ByteEncryption.Decryption(Byte[])`.
pub fn decrypt(bytes: &[u8]) -> Result<Vec<u8>, ImportError> {
    if bytes.len() <= BLOCK_SIZE {
        return Err(ImportError::invalid(
            "font bundle",
            format!(
                "encrypted data is {} bytes; need more than 1024",
                bytes.len()
            ),
        ));
    }
    let tail = bytes.len() - BLOCK_SIZE;
    let mut out = Vec::with_capacity(tail);
    out.extend_from_slice(&bytes[tail..]);
    if tail > BLOCK_SIZE {
        out.extend_from_slice(&bytes[BLOCK_SIZE..tail]);
    }
    out.truncate(tail);
    Ok(out)
}

/// Port of `ByteEncryption.Encryption(Byte[])` (used for round-trip tests only).
pub fn encrypt(bytes: &[u8]) -> Result<Vec<u8>, ImportError> {
    if bytes.len() < 2 * BLOCK_SIZE {
        return Err(ImportError::invalid(
            "font bundle",
            "encryption source needs at least 2048 bytes",
        ));
    }
    let mut out = Vec::with_capacity(bytes.len() + BLOCK_SIZE);
    out.extend_from_slice(&bytes[BLOCK_SIZE..2 * BLOCK_SIZE]);
    out.extend_from_slice(&bytes[BLOCK_SIZE..]);
    out.extend_from_slice(&bytes[..BLOCK_SIZE]);
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawBundleEntry {
    pub path: String,
    pub offset: u64,
    pub size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawBundleHeader {
    pub signature: String,
    pub format_version: u32,
    pub player_version: String,
    pub engine_version: String,
    pub header_size: u32,
    pub complete_file_size: u32,
    pub entries: Vec<RawBundleEntry>,
}

struct Be<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Be<'_> {
    fn u32(&mut self) -> Result<u32, ImportError> {
        let end = self.pos + 4;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| ImportError::invalid("font bundle", "truncated bundle header"))?;
        self.pos = end;
        Ok(u32::from_be_bytes(slice.try_into().expect("4 bytes")))
    }

    fn cstr(&mut self) -> Result<String, ImportError> {
        let tail = self
            .bytes
            .get(self.pos..)
            .ok_or_else(|| ImportError::invalid("font bundle", "truncated bundle header"))?;
        let len = tail
            .iter()
            .take(4096)
            .position(|b| *b == 0)
            .ok_or_else(|| ImportError::invalid("font bundle", "unterminated header string"))?;
        self.pos += len + 1;
        Ok(String::from_utf8_lossy(&tail[..len]).into_owned())
    }
}

/// Parses an in-memory `UnityRaw` v3 container. `UnityWeb`/`UnityFS` (compressed)
/// containers are rejected explicitly; the Memoria font bundle does not use them.
pub fn parse_unity_raw(bytes: &[u8]) -> Result<RawBundleHeader, ImportError> {
    let mut r = Be { bytes, pos: 0 };
    let signature = r.cstr()?;
    if signature != "UnityRaw" {
        return Err(ImportError::unsupported(
            "font bundle",
            format!("container signature `{signature}`; only UnityRaw is implemented"),
        ));
    }
    let format_version = r.u32()?;
    if format_version != 3 {
        return Err(ImportError::unsupported(
            "font bundle",
            format!("UnityRaw format {format_version}"),
        ));
    }
    let player_version = r.cstr()?;
    let engine_version = r.cstr()?;
    let _minimum_streamed_bytes = r.u32()?;
    let header_size = r.u32()?;
    let _levels_before_streaming = r.u32()?;
    let level_count = r.u32()?;
    if level_count == 0 || level_count > 1024 {
        return Err(ImportError::invalid("font bundle", "bad level count"));
    }
    r.pos += (level_count as usize - 1) * 8;
    let streamed_content_size = r.u32()?;
    let _uncompressed = r.u32()?;
    let complete_file_size = r.u32()?;
    let _file_info_header_size = r.u32()?;
    if complete_file_size as usize != bytes.len() {
        return Err(ImportError::invalid(
            "font bundle",
            format!(
                "header declares {complete_file_size} bytes, have {}",
                bytes.len()
            ),
        ));
    }
    let header = header_size as usize;
    if header >= bytes.len()
        || header as u64 + u64::from(streamed_content_size) > bytes.len() as u64
    {
        return Err(ImportError::invalid(
            "font bundle",
            "directory out of range",
        ));
    }
    let mut r = Be { bytes, pos: header };
    let count = r.u32()?;
    if count > 1024 {
        return Err(ImportError::invalid("font bundle", "too many entries"));
    }
    let mut entries = Vec::new();
    for _ in 0..count {
        let path = r.cstr()?;
        let offset = r.u32()?;
        let size = r.u32()?;
        let absolute = header as u64 + u64::from(offset);
        if absolute + u64::from(size) > bytes.len() as u64 {
            return Err(ImportError::invalid(
                "font bundle",
                format!("entry `{path}` lies outside the bundle"),
            ));
        }
        entries.push(RawBundleEntry {
            path,
            offset: absolute,
            size,
        });
    }
    Ok(RawBundleHeader {
        signature,
        format_version,
        player_version,
        engine_version,
        header_size,
        complete_file_size,
        entries,
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlyphRect {
    /// Unicode code point (`CharacterInfo.index`).
    pub index: u32,
    /// Unity `Rectf` as `[x, y, width, height]`.
    pub uv: [f32; 4],
    pub vert: [f32; 4],
    pub width: f32,
    pub flipped: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontTexture {
    pub name: String,
    pub path_id: i64,
    pub width: i32,
    pub height: i32,
    /// Raw `m_TextureFormat` value (Unity `TextureFormat` enum).
    pub format: i32,
    pub data_bytes: usize,
    pub png_file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontEntry {
    pub path_id: i64,
    pub name: String,
    pub line_spacing: Option<f64>,
    pub ascent: Option<f64>,
    pub descent: Option<f64>,
    pub character_count: usize,
    pub characters: Vec<GlyphRect>,
    pub material_path_id: Option<i64>,
    pub texture: Option<FontTexture>,
    pub font_data_bytes: usize,
    /// Header facts of the embedded `m_FontData` blob when it is an sfnt (TrueType) file.
    pub sfnt: Option<SfntInfo>,
    /// Every other top-level field, kept raw (long arrays summarized by length).
    pub raw_fields: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontInventory {
    pub bundle: RawBundleHeader,
    pub serialized_entry: String,
    pub unity_version: String,
    pub object_classes: BTreeMap<u16, usize>,
    pub fonts: Vec<FontEntry>,
    pub notes: Vec<String>,
}

/// Decodes a Unity `Value` to JSON; arrays longer than `limit` are summarized.
pub fn value_to_json(value: &Value<'_>, limit: usize) -> serde_json::Value {
    use serde_json::{Value as J, json};
    let summary = |n: usize| json!({ "array_len": n });
    match value {
        Value::Bool(v) => J::Bool(*v),
        Value::Int(v) => json!(v),
        Value::UInt(v) => json!(v),
        Value::Float(v) => json!(v),
        Value::String(v) => J::String(v.clone()),
        Value::Bytes(v) => summary(v.len()),
        Value::I32s(v) if v.len() <= limit => json!(v),
        Value::U32s(v) if v.len() <= limit => json!(v),
        Value::F32s(v) if v.len() <= limit => json!(v),
        Value::I32s(v) => summary(v.len()),
        Value::U32s(v) => summary(v.len()),
        Value::F32s(v) => summary(v.len()),
        Value::Array(v) if v.len() <= limit => {
            J::Array(v.iter().map(|x| value_to_json(x, limit)).collect())
        }
        Value::Array(v) => summary(v.len()),
        Value::Struct(fields) => J::Object(
            fields
                .iter()
                .map(|(k, v)| ((*k).to_owned(), value_to_json(v, limit)))
                .collect(),
        ),
    }
}

fn rect(value: &Value<'_>) -> Option<[f32; 4]> {
    Some([
        value.field("x")?.as_f32()?,
        value.field("y")?.as_f32()?,
        value.field("width")?.as_f32()?,
        value.field("height")?.as_f32()?,
    ])
}

fn pptr_id(value: &Value<'_>) -> Option<i64> {
    value.field("m_PathID")?.as_i64()
}

/// Binary payloads extracted next to a [`FontEntry`] (same order).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontBlobs {
    /// Top-down RGBA of the font's own texture; empty when absent or undecodable.
    pub rgba: Vec<u8>,
    /// Raw `m_FontData` (the embedded font file).
    pub font_data: Vec<u8>,
}

/// Reads every `Font` in a decrypted font bundle.
pub fn inventory_decrypted(
    decrypted: &[u8],
) -> Result<(FontInventory, Vec<FontBlobs>), ImportError> {
    let bundle = parse_unity_raw(decrypted)?;
    let entry = bundle
        .entries
        .iter()
        .find(|e| !e.path.ends_with(".resS") && !e.path.ends_with(".resource"))
        .ok_or_else(|| ImportError::invalid("font bundle", "no serialized entry"))?
        .clone();
    let start = entry.offset as usize;
    let data = &decrypted[start..start + entry.size as usize];
    let file = SerializedFile::parse(data, &entry.path)?;
    let report = file.report();
    if !report.type_trees_embedded {
        return Err(ImportError::unsupported(
            &entry.path,
            "font bundle without embedded type trees",
        ));
    }
    let mut object_classes = BTreeMap::new();
    for object in &report.objects {
        *object_classes.entry(object.class_id).or_insert(0) += 1;
    }
    let mut notes = Vec::new();
    let mut trees: BTreeMap<i32, TypeTree> = BTreeMap::new();
    for t in &report.types {
        if let Some(raw) = file.type_tree(t.class_id) {
            trees.insert(t.class_id, TypeTree::parse(&raw, &entry.path)?);
        }
    }
    let read = |index: usize| -> Result<Value<'_>, ImportError> {
        let object = &report.objects[index];
        let tree = trees.get(&object.type_id).ok_or_else(|| {
            ImportError::invalid(&entry.path, format!("no type tree for {}", object.type_id))
        })?;
        tree.read(file.object_data(index)?, &entry.path)
    };
    let by_path: BTreeMap<i64, usize> = report
        .objects
        .iter()
        .enumerate()
        .map(|(i, o)| (o.path_id, i))
        .collect();
    let mut fonts = Vec::new();
    let mut pixels = Vec::new();
    for (index, object) in report.objects.iter().enumerate() {
        if object.class_id != FONT_CLASS_ID {
            continue;
        }
        let value = read(index)?;
        let name = value
            .field("m_Name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let mut characters = Vec::new();
        if let Some(rects) = value.field("m_CharacterRects").and_then(Value::as_array) {
            for item in rects {
                let (Some(i), Some(uv), Some(vert)) = (
                    item.field("index").and_then(Value::as_u64),
                    item.field("uv").and_then(rect),
                    item.field("vert").and_then(rect),
                ) else {
                    return Err(ImportError::invalid(
                        &entry.path,
                        "unexpected m_CharacterRects element layout",
                    ));
                };
                characters.push(GlyphRect {
                    index: i as u32,
                    uv,
                    vert,
                    width: item.field("width").and_then(Value::as_f32).unwrap_or(0.0),
                    flipped: item
                        .field("flipped")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                });
            }
        }
        let float = |n: &str| value.field(n).and_then(|v| v.as_f32().map(f64::from));
        let material_path_id = value.field("m_DefaultMaterial").and_then(pptr_id);
        let mut texture = None;
        let mut rgba = Vec::new();
        let mat_index = material_path_id.and_then(|id| by_path.get(&id)).copied();
        if let Some(mat_index) =
            mat_index.filter(|i| report.objects[*i].class_id == MATERIAL_CLASS_ID)
        {
            let material = read(mat_index)?;
            let tex_id = material
                .field("m_SavedProperties")
                .and_then(|p| p.field("m_TexEnvs"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find_map(|pair| {
                    let key = pair.field("first")?.field("name")?.as_str()?;
                    if key == "_MainTex" {
                        pptr_id(pair.field("second")?.field("m_Texture")?)
                    } else {
                        None
                    }
                });
            let tex_index = tex_id.and_then(|id| by_path.get(&id)).copied();
            if let Some(tex_index) =
                tex_index.filter(|i| report.objects[*i].class_id == TEXTURE2D_CLASS_ID)
            {
                let tex = read(tex_index)?;
                let int = |n: &str| tex.field(n).and_then(Value::as_i64).unwrap_or(-1) as i32;
                let (width, height, format) =
                    (int("m_Width"), int("m_Height"), int("m_TextureFormat"));
                let data = tex
                    .field("image data")
                    .and_then(Value::as_bytes)
                    .unwrap_or_default();
                if width == 0 || height == 0 || data.is_empty() {
                    notes.push(format!(
                        "{name}: dynamic Unity font; texture is empty ({width}x{height}), glyphs come from the embedded font data"
                    ));
                } else if let Some(decoded) = texture_rgba(width, height, format, data) {
                    rgba = decoded;
                } else {
                    notes.push(format!(
                        "{name}: texture format {format} {width}x{height} ({} bytes) has no decoder",
                        data.len()
                    ));
                }
                texture = Some(FontTexture {
                    name: tex
                        .field("m_Name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    path_id: report.objects[tex_index].path_id,
                    width,
                    height,
                    format,
                    data_bytes: data.len(),
                    png_file: None,
                });
            }
        }
        let font_data = value
            .field("m_FontData")
            .and_then(Value::as_bytes)
            .unwrap_or_default()
            .to_vec();
        let sfnt = parse_sfnt(&font_data);
        let font_data_bytes = font_data.len();
        pixels.push(FontBlobs { rgba, font_data });
        let mut raw_fields = BTreeMap::new();
        if let Value::Struct(fields) = &value {
            for (key, v) in fields {
                if *key != "m_CharacterRects" {
                    raw_fields.insert((*key).to_owned(), value_to_json(v, 16));
                }
            }
        }
        fonts.push(FontEntry {
            path_id: object.path_id,
            name,
            line_spacing: float("m_LineSpacing"),
            ascent: float("m_Ascent"),
            descent: float("m_Descent"),
            character_count: characters.len(),
            characters,
            material_path_id,
            texture,
            font_data_bytes,
            sfnt,
            raw_fields,
        });
    }
    Ok((
        FontInventory {
            bundle,
            serialized_entry: entry.path,
            unity_version: report.unity_version.clone(),
            object_classes,
            fonts,
            notes,
        },
        pixels,
    ))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SfntInfo {
    /// First four bytes as hex (`00010000` TrueType, `4f54544f` OpenType/CFF).
    pub magic: String,
    pub table_tags: Vec<String>,
    pub units_per_em: Option<u16>,
    pub glyph_count: Option<u16>,
    pub hhea_ascender: Option<i16>,
    pub hhea_descender: Option<i16>,
    pub hhea_line_gap: Option<i16>,
    /// `name` table IDs 1 (family) and 4 (full name), when decodable.
    pub family_name: Option<String>,
    pub full_name: Option<String>,
    /// Best Unicode `cmap` subtable format (4 or 12 only) and its code-point count.
    pub cmap_format: Option<u16>,
    pub cmap_code_points: Option<u32>,
}

fn be16(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(at..at + 2)?.try_into().ok()?))
}

fn be32(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

/// Minimal sfnt table-directory reader (header facts only; no rasterization).
pub fn parse_sfnt(d: &[u8]) -> Option<SfntInfo> {
    let magic = be32(d, 0)?;
    if !matches!(magic, 0x0001_0000 | 0x4f54_544f | 0x7472_7565) {
        return None;
    }
    let count = usize::from(be16(d, 4)?);
    let mut tables = Vec::new();
    for i in 0..count {
        let at = 12 + i * 16;
        let tag = String::from_utf8_lossy(d.get(at..at + 4)?).into_owned();
        let offset = be32(d, at + 8)? as usize;
        let length = be32(d, at + 12)? as usize;
        tables.push((tag, offset, length));
    }
    let table = |tag: &str| {
        tables
            .iter()
            .find(|t| t.0 == tag)
            .and_then(|(_, o, l)| d.get(*o..o.checked_add(*l)?))
    };
    let units_per_em = table("head").and_then(|t| be16(t, 18));
    let glyph_count = table("maxp").and_then(|t| be16(t, 4));
    let hhea = table("hhea");
    let signed = |t: Option<&[u8]>, at| t.and_then(|t| be16(t, at)).map(|v| v as i16);
    let (family_name, full_name) =
        table("name").map_or((None, None), |t| (sfnt_name(t, 1), sfnt_name(t, 4)));
    let (cmap_format, cmap_code_points) = table("cmap").map_or((None, None), sfnt_cmap);
    Some(SfntInfo {
        magic: format!("{magic:08x}"),
        table_tags: tables.iter().map(|t| t.0.clone()).collect(),
        units_per_em,
        glyph_count,
        hhea_ascender: signed(hhea, 4),
        hhea_descender: signed(hhea, 6),
        hhea_line_gap: signed(hhea, 8),
        family_name,
        full_name,
        cmap_format,
        cmap_code_points,
    })
}

fn sfnt_name(t: &[u8], wanted: u16) -> Option<String> {
    let count = usize::from(be16(t, 2)?);
    let strings = usize::from(be16(t, 4)?);
    let mut fallback = None;
    for i in 0..count {
        let at = 6 + i * 12;
        if be16(t, at + 6)? != wanted {
            continue;
        }
        let (platform, length, offset) = (be16(t, at)?, be16(t, at + 8)?, be16(t, at + 10)?);
        let start = strings + usize::from(offset);
        let raw = t.get(start..start + usize::from(length))?;
        if platform == 3 || platform == 0 {
            let units: Vec<u16> = raw
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            return Some(String::from_utf16_lossy(&units));
        }
        fallback.get_or_insert_with(|| String::from_utf8_lossy(raw).into_owned());
    }
    fallback
}

fn sfnt_cmap(t: &[u8]) -> (Option<u16>, Option<u32>) {
    let mut best: Option<(u16, u32)> = None;
    let count = usize::from(be16(t, 2).unwrap_or(0));
    for i in 0..count {
        let at = 4 + i * 8;
        let (Some(platform), Some(encoding), Some(offset)) =
            (be16(t, at), be16(t, at + 2), be32(t, at + 4))
        else {
            continue;
        };
        let unicode = platform == 0 || (platform == 3 && (encoding == 1 || encoding == 10));
        let Some(sub) = t.get(offset as usize..).filter(|_| unicode) else {
            continue;
        };
        let Some(format) = be16(sub, 0) else { continue };
        let points = match format {
            4 => sfnt_cmap4_count(sub),
            12 => sfnt_cmap12_count(sub),
            _ => None,
        };
        if let Some(points) = points.filter(|p| best.is_none_or(|(_, n)| *p > n)) {
            best = Some((format, points));
        }
    }
    (best.map(|b| b.0), best.map(|b| b.1))
}

fn sfnt_cmap4_count(sub: &[u8]) -> Option<u32> {
    let segs = usize::from(be16(sub, 6)?) / 2;
    let mut total = 0_u32;
    for i in 0..segs {
        let end = u32::from(be16(sub, 14 + i * 2)?);
        let start = u32::from(be16(sub, 16 + segs * 2 + i * 2)?);
        if end >= start && start != 0xffff {
            total += end - start + 1;
        }
    }
    Some(total)
}

fn sfnt_cmap12_count(sub: &[u8]) -> Option<u32> {
    let groups = be32(sub, 12)? as usize;
    let mut total = 0_u32;
    for i in 0..groups {
        let start = be32(sub, 16 + i * 12)?;
        let end = be32(sub, 20 + i * 12)?;
        total = total.checked_add(end.checked_sub(start)? + 1)?;
    }
    Some(total)
}

/// Unity `TextureFormat` values handled here: 1 Alpha8, 3 RGB24, 4 RGBA32, 5 ARGB32.
/// Output is top-down RGBA (Unity stores rows bottom-up).
fn texture_rgba(width: i32, height: i32, format: i32, data: &[u8]) -> Option<Vec<u8>> {
    let (w, h) = (usize::try_from(width).ok()?, usize::try_from(height).ok()?);
    let n = w.checked_mul(h)?;
    let mut rgba = Vec::with_capacity(n * 4);
    match format {
        1 if data.len() >= n => data[..n]
            .iter()
            .for_each(|a| rgba.extend([255, 255, 255, *a])),
        3 if data.len() >= n * 3 => data[..n * 3]
            .chunks_exact(3)
            .for_each(|p| rgba.extend([p[0], p[1], p[2], 255])),
        4 if data.len() >= n * 4 => rgba.extend_from_slice(&data[..n * 4]),
        5 if data.len() >= n * 4 => data[..n * 4]
            .chunks_exact(4)
            .for_each(|p| rgba.extend([p[1], p[2], p[3], p[0]])),
        _ => return None,
    }
    let stride = w * 4;
    let mut flipped = vec![0; rgba.len()];
    for row in 0..h {
        flipped[(h - 1 - row) * stride..(h - row) * stride]
            .copy_from_slice(&rgba[row * stride..(row + 1) * stride]);
    }
    Some(flipped)
}

/// Full pipeline from the encrypted `.mpc` bytes.
pub fn inventory_encrypted(bytes: &[u8]) -> Result<(FontInventory, Vec<FontBlobs>), ImportError> {
    inventory_decrypted(&decrypt(bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decrypt_round_trips_synthetic_data() {
        let plain: Vec<u8> = (0..5000_u32).map(|i| (i * 7 + i / 256) as u8).collect();
        let encrypted = encrypt(&plain).expect("encrypt");
        assert_eq!(encrypted.len(), plain.len() + BLOCK_SIZE);
        assert_ne!(encrypted[..BLOCK_SIZE], plain[..BLOCK_SIZE]);
        assert_eq!(decrypt(&encrypted).expect("decrypt"), plain);
    }

    #[test]
    fn decrypt_moves_tail_block_to_front() {
        let mut data = vec![1_u8; 3000];
        data[3000 - BLOCK_SIZE..].fill(9);
        let out = decrypt(&data).expect("decrypt");
        assert_eq!(out.len(), 3000 - BLOCK_SIZE);
        assert!(out[..BLOCK_SIZE].iter().all(|b| *b == 9));
        assert!(out[BLOCK_SIZE..].iter().all(|b| *b == 1));
    }

    #[test]
    fn short_input_is_rejected() {
        assert!(decrypt(&[0; 1024]).is_err());
    }

    #[test]
    fn compressed_containers_are_rejected_not_guessed() {
        let mut bytes = b"UnityWeb\0".to_vec();
        bytes.resize(64, 0);
        assert!(parse_unity_raw(&bytes).is_err());
    }

    #[test]
    fn sfnt_header_facts_from_synthetic_font() {
        let mut head = vec![0_u8; 54];
        head[18..20].copy_from_slice(&1000_u16.to_be_bytes());
        let mut hhea = vec![0_u8; 36];
        hhea[4..6].copy_from_slice(&800_i16.to_be_bytes());
        hhea[6..8].copy_from_slice(&(-200_i16).to_be_bytes());
        hhea[8..10].copy_from_slice(&90_i16.to_be_bytes());
        let mut maxp = vec![0_u8; 6];
        maxp[4..6].copy_from_slice(&7_u16.to_be_bytes());
        let tables: [(&[u8; 4], Vec<u8>); 3] = [(b"head", head), (b"hhea", hhea), (b"maxp", maxp)];
        let mut d = vec![0, 1, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0];
        let mut offset = 12 + 16 * 3;
        for (tag, body) in &tables {
            d.extend_from_slice(*tag);
            d.extend_from_slice(&[0; 4]);
            d.extend_from_slice(&(offset as u32).to_be_bytes());
            d.extend_from_slice(&(body.len() as u32).to_be_bytes());
            offset += body.len();
        }
        for (_, body) in &tables {
            d.extend_from_slice(body);
        }
        let info = parse_sfnt(&d).expect("sfnt");
        assert_eq!(info.units_per_em, Some(1000));
        assert_eq!(info.glyph_count, Some(7));
        assert_eq!(info.hhea_descender, Some(-200));
        assert_eq!(info.hhea_line_gap, Some(90));
        assert!(parse_sfnt(b"not a font").is_none());
    }

    #[test]
    fn alpha8_texture_flips_rows() {
        let rgba = texture_rgba(1, 2, 1, &[10, 20]).expect("decode");
        assert_eq!(rgba, vec![255, 255, 255, 20, 255, 255, 255, 10]);
        assert!(texture_rgba(1, 1, 12, &[0; 16]).is_none());
    }
}
