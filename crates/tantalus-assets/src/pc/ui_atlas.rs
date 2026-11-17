//! NGUI UI data in the standalone Unity files: `UIAtlas` and `UIFont`
//! (with its `BMFont` glyph table) MonoBehaviours, their Materials and
//! Texture2D atlases.
//!
//! The standalone files (`sharedassetsN.assets`, `resources.assets`,
//! `mainData`, `levelN`) carry no embedded type trees, so MonoBehaviour
//! payloads are decoded with the fixed NGUI 3.x serialization layouts and
//! must consume their byte range exactly. The script class is identified by
//! the `m_Script` PPtr: a MonoScript object (usually in `sharedassets0.assets`,
//! reached through the file's external reference list) whose leading name and
//! `m_ClassName` give the class (`UIAtlas`, `UIFont`, ...).
//!
//! MonoBehaviour payload:
//! `m_GameObject` PPtr (i32 file, i64 path), `m_Enabled` u8 + pad to 4,
//! `m_Script` PPtr, `m_Name` string, then the class fields.

use super::serialized_inventory::{StandaloneSerialized, class_name};
use super::texture::Texture2D;
use crate::install::ImportError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const MONO_BEHAVIOUR_CLASS_ID: u16 = 114;
pub const MONO_SCRIPT_CLASS_ID: u16 = 115;
pub const MATERIAL_CLASS_ID: u16 = 21;
pub const TEXTURE_2D_CLASS_ID: u16 = 28;

/// Unity object reference (`file_id` 0 is the same file; otherwise a 1-based
/// index into the file's external reference list).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Ref {
    pub file_id: i32,
    pub path_id: i64,
}

impl Ref {
    pub fn is_null(&self) -> bool {
        self.path_id == 0
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    source: &'a str,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8], source: &'a str) -> Self {
        Self {
            bytes,
            pos: 0,
            source,
        }
    }

    fn err(&self, message: impl std::fmt::Display) -> ImportError {
        ImportError::invalid(self.source, format!("{message} at byte {}", self.pos))
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], ImportError> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| self.err("truncated payload"))?;
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn i32(&mut self) -> Result<i32, ImportError> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().expect("4")))
    }

    fn i64(&mut self) -> Result<i64, ImportError> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().expect("8")))
    }

    fn f32(&mut self) -> Result<f32, ImportError> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().expect("4")))
    }

    fn u8(&mut self) -> Result<u8, ImportError> {
        Ok(self.take(1)?[0])
    }

    fn align(&mut self) -> Result<(), ImportError> {
        let pad = (4 - self.pos % 4) % 4;
        self.take(pad).map(|_| ())
    }

    /// Element count for a vector; bounded by the bytes left so a wrong
    /// layout fails instead of allocating.
    fn count(&mut self, min_element: usize) -> Result<usize, ImportError> {
        let n = self.i32()?;
        let n = usize::try_from(n).map_err(|_| self.err("negative array length"))?;
        if n.saturating_mul(min_element) > self.bytes.len() - self.pos {
            return Err(self.err(format!("array length {n} exceeds payload")));
        }
        Ok(n)
    }

    fn string(&mut self) -> Result<String, ImportError> {
        let n = self.count(1)?;
        let bytes = self.take(n)?;
        let text = std::str::from_utf8(bytes).map_err(|_| self.err("string is not valid UTF-8"))?;
        let text = text.to_owned();
        self.align()?;
        Ok(text)
    }

    fn pptr(&mut self) -> Result<Ref, ImportError> {
        Ok(Ref {
            file_id: self.i32()?,
            path_id: self.i64()?,
        })
    }

    fn finish(&self) -> Result<(), ImportError> {
        if self.pos == self.bytes.len() {
            Ok(())
        } else {
            Err(self.err(format!(
                "{} unconsumed trailing bytes",
                self.bytes.len() - self.pos
            )))
        }
    }
}

/// Common MonoBehaviour prefix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonoHeader {
    pub game_object: Ref,
    pub enabled: bool,
    pub script: Ref,
    pub name: String,
}

fn read_header(r: &mut Reader<'_>) -> Result<MonoHeader, ImportError> {
    let game_object = r.pptr()?;
    let enabled = r.u8()? != 0;
    r.align()?;
    let script = r.pptr()?;
    let name = r.string()?;
    Ok(MonoHeader {
        game_object,
        enabled,
        script,
        name,
    })
}

/// Parses only the MonoBehaviour prefix.
pub fn parse_mono_header(bytes: &[u8], source: &str) -> Result<MonoHeader, ImportError> {
    read_header(&mut Reader::new(bytes, source))
}

/// One `UISpriteData` entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiSprite {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub border_left: i32,
    pub border_right: i32,
    pub border_top: i32,
    pub border_bottom: i32,
    pub padding_left: i32,
    pub padding_right: i32,
    pub padding_top: i32,
    pub padding_bottom: i32,
}

/// Typed `UIAtlas` MonoBehaviour.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiAtlas {
    pub header: MonoHeader,
    pub material: Ref,
    pub sprites: Vec<UiSprite>,
    pub pixel_size: f32,
    pub replacement: Ref,
    /// 0 = Pixels, 1 = TexCoords.
    pub coordinates: i32,
    /// Entries in the obsolete UV-based `sprites` list (expected 0).
    pub legacy_sprite_count: usize,
}

impl UiAtlas {
    pub fn parse(bytes: &[u8], source: &str) -> Result<Self, ImportError> {
        let mut r = Reader::new(bytes, source);
        let header = read_header(&mut r)?;
        let material = r.pptr()?;
        let count = r.count(4 * 13)?;
        let mut sprites = Vec::with_capacity(count);
        for _ in 0..count {
            sprites.push(UiSprite {
                name: r.string()?,
                x: r.i32()?,
                y: r.i32()?,
                width: r.i32()?,
                height: r.i32()?,
                border_left: r.i32()?,
                border_right: r.i32()?,
                border_top: r.i32()?,
                border_bottom: r.i32()?,
                padding_left: r.i32()?,
                padding_right: r.i32()?,
                padding_top: r.i32()?,
                padding_bottom: r.i32()?,
            });
        }
        let pixel_size = r.f32()?;
        let replacement = r.pptr()?;
        let coordinates = r.i32()?;
        let legacy_sprite_count = r.count(1)?;
        if legacy_sprite_count != 0 {
            return Err(r.err(format!(
                "legacy UV sprite list has {legacy_sprite_count} entries; layout unknown"
            )));
        }
        r.finish()?;
        Ok(Self {
            header,
            material,
            sprites,
            pixel_size,
            replacement,
            coordinates,
            legacy_sprite_count,
        })
    }
}

/// One BMFont glyph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BmGlyph {
    pub id: i32,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub offset_x: i32,
    pub offset_y: i32,
    pub advance: i32,
    pub channel: i32,
    /// Flat `[previous_char, amount, ...]` pairs.
    pub kerning: Vec<i32>,
}

/// BMFont table embedded in a UIFont.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BmFont {
    pub char_size: i32,
    pub base_offset: i32,
    pub tex_width: i32,
    pub tex_height: i32,
    pub sprite_name: String,
    pub glyphs: Vec<BmGlyph>,
}

/// Typed `UIFont` MonoBehaviour.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiFont {
    pub header: MonoHeader,
    pub material: Ref,
    /// `[x, y, w, h]` UV rectangle.
    pub uv_rect: [f32; 4],
    pub bm_font: BmFont,
    pub atlas: Ref,
    pub replacement: Ref,
    /// `(sequence, sprite_name)` for each BMSymbol.
    pub symbols: Vec<(String, String)>,
    pub dynamic_font: Ref,
    pub dynamic_font_size: i32,
    pub dynamic_font_style: i32,
}

impl UiFont {
    pub fn parse(bytes: &[u8], source: &str) -> Result<Self, ImportError> {
        let mut r = Reader::new(bytes, source);
        let header = read_header(&mut r)?;
        let material = r.pptr()?;
        let uv_rect = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
        let char_size = r.i32()?;
        let base_offset = r.i32()?;
        let tex_width = r.i32()?;
        let tex_height = r.i32()?;
        let sprite_name = r.string()?;
        let glyph_count = r.count(10 * 4)?;
        let mut glyphs = Vec::with_capacity(glyph_count);
        for _ in 0..glyph_count {
            let mut glyph = BmGlyph {
                id: r.i32()?,
                x: r.i32()?,
                y: r.i32()?,
                width: r.i32()?,
                height: r.i32()?,
                offset_x: r.i32()?,
                offset_y: r.i32()?,
                advance: r.i32()?,
                channel: r.i32()?,
                kerning: Vec::new(),
            };
            let kerning = r.count(4)?;
            for _ in 0..kerning {
                glyph.kerning.push(r.i32()?);
            }
            glyphs.push(glyph);
        }
        let atlas = r.pptr()?;
        let replacement = r.pptr()?;
        let symbol_count = r.count(8)?;
        let mut symbols = Vec::with_capacity(symbol_count);
        for _ in 0..symbol_count {
            symbols.push((r.string()?, r.string()?));
        }
        let dynamic_font = r.pptr()?;
        let dynamic_font_size = r.i32()?;
        let dynamic_font_style = r.i32()?;
        r.finish()?;
        Ok(Self {
            header,
            material,
            uv_rect,
            bm_font: BmFont {
                char_size,
                base_offset,
                tex_width,
                tex_height,
                sprite_name,
                glyphs,
            },
            atlas,
            replacement,
            symbols,
            dynamic_font,
            dynamic_font_size,
            dynamic_font_style,
        })
    }
}

/// The parts of a Material needed to find its textures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialRefs {
    pub name: String,
    pub shader: Ref,
    /// `(property, texture)` for every texture slot.
    pub textures: Vec<(String, Ref)>,
}

impl MaterialRefs {
    /// Unity 5.2 Material: name, shader, keywords string, lightmap flags,
    /// custom render queue, string tag map, then saved properties
    /// (`m_TexEnvs`, `m_Floats`, `m_Colors`).
    pub fn parse(bytes: &[u8], source: &str) -> Result<Self, ImportError> {
        let mut r = Reader::new(bytes, source);
        let name = r.string()?;
        let shader = r.pptr()?;
        let _keywords = r.string()?;
        let _lightmap_flags = r.i32()?;
        let _render_queue = r.i32()?;
        let tags = r.count(8)?;
        for _ in 0..tags {
            r.string()?;
            r.string()?;
        }
        let tex_envs = r.count(4 + 12 + 16)?;
        let mut textures = Vec::with_capacity(tex_envs);
        for _ in 0..tex_envs {
            let property = r.string()?;
            let texture = r.pptr()?;
            r.take(16)?; // scale + offset
            textures.push((property, texture));
        }
        let floats = r.count(8)?;
        for _ in 0..floats {
            r.string()?;
            r.f32()?;
        }
        let colors = r.count(4 + 16)?;
        for _ in 0..colors {
            r.string()?;
            r.take(16)?;
        }
        r.finish()?;
        Ok(Self {
            name,
            shader,
            textures,
        })
    }

    /// `_MainTex` if present, else the first non-null texture.
    pub fn main_texture(&self) -> Option<Ref> {
        self.textures
            .iter()
            .find(|(property, texture)| property == "_MainTex" && !texture.is_null())
            .or_else(|| self.textures.iter().find(|(_, texture)| !texture.is_null()))
            .map(|(_, texture)| *texture)
    }
}

/// Class and name of a MonoScript (`m_Name`, then the fixed fields up to
/// `m_ClassName`).
pub fn parse_mono_script(bytes: &[u8], source: &str) -> Result<(String, String), ImportError> {
    let mut r = Reader::new(bytes, source);
    let name = r.string()?;
    r.i32()?; // m_ExecutionOrder
    r.take(16)?; // m_PropertiesHash
    let class = r.string()?;
    Ok((name, class))
}

/// Name of a GameObject payload (`m_Component` pairs, `m_Layer`, `m_Name`).
/// NGUI atlases and fonts take their name from their GameObject.
pub fn parse_game_object_name(bytes: &[u8], source: &str) -> Result<String, ImportError> {
    let mut r = Reader::new(bytes, source);
    let components = r.count(16)?;
    r.take(components * 16)?;
    r.i32()?; // m_Layer
    r.string()
}

/// Typed decode of one UI MonoBehaviour.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum UiObject {
    Atlas(UiAtlas),
    Font(UiFont),
}

// ---------------------------------------------------------------------------
// Scan over a standalone file

/// A texture an atlas or font resolves to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextureInfo {
    pub file: String,
    pub path_id: i64,
    pub name: String,
    pub width: i32,
    pub height: i32,
    pub format: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AtlasEntry {
    pub file: String,
    pub path_id: i64,
    pub name: String,
    pub sprite_count: usize,
    pub pixel_size: f32,
    pub coordinates: i32,
    pub material: Option<String>,
    pub texture: Option<TextureInfo>,
    /// Sprites whose rectangle leaves the texture.
    pub sprites_outside_texture: usize,
    pub shader: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontEntry {
    pub file: String,
    pub path_id: i64,
    pub name: String,
    pub glyph_count: usize,
    pub kerning_pairs: usize,
    pub char_size: i32,
    pub base_offset: i32,
    pub tex_width: i32,
    pub tex_height: i32,
    pub symbol_count: usize,
    pub sprite_name: String,
    /// Atlas MonoBehaviour name when the font draws from a UIAtlas.
    pub atlas: Option<String>,
    pub material: Option<String>,
    pub texture: Option<TextureInfo>,
    pub has_dynamic_font: bool,
    pub glyphs_outside_texture: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiFailure {
    pub file: String,
    pub path_id: i64,
    pub script: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UiScan {
    pub file: String,
    pub mono_behaviours: usize,
    /// MonoBehaviour count by script class name (`?` when unresolved).
    pub scripts: BTreeMap<String, usize>,
    pub atlases: Vec<AtlasEntry>,
    pub fonts: Vec<FontEntry>,
    pub failures: Vec<UiFailure>,
    /// `(object path id, script PPtr file id, script PPtr path id)` for
    /// MonoBehaviours whose script could not be resolved.
    pub unresolved_scripts: Vec<(i64, i32, i64)>,
}

/// Opens sibling serialized files on demand.
struct Siblings {
    dir: PathBuf,
    open: BTreeMap<String, Option<StandaloneSerialized>>,
}

impl Siblings {
    fn get(&mut self, name: &str) -> Option<&mut StandaloneSerialized> {
        let dir = self.dir.clone();
        self.open
            .entry(name.to_owned())
            .or_insert_with(|| StandaloneSerialized::open(&dir.join(name)).ok())
            .as_mut()
    }
}

/// A fully decoded target: which file holds it and its payload.
fn external_name(file: &StandaloneSerialized, file_id: i32) -> Option<String> {
    let index = usize::try_from(file_id.checked_sub(1)?).ok()?;
    let path = file.inventory().external_paths.get(index)?;
    Some(path.rsplit(['/', '\\']).next().unwrap_or(path).to_owned())
}

struct Resolver<'a> {
    main_name: String,
    main: &'a mut StandaloneSerialized,
    siblings: Siblings,
}

impl Resolver<'_> {
    /// Reads the payload of `reference` (resolved relative to the main file)
    /// together with the owning file's name.
    fn read(
        &mut self,
        reference: Ref,
        class_id: u16,
    ) -> Result<Option<(String, Vec<u8>)>, ImportError> {
        if reference.is_null() {
            return Ok(None);
        }
        if reference.file_id == 0 {
            return Self::read_in(self.main, &self.main_name, reference.path_id, class_id);
        }
        let Some(name) = external_name(self.main, reference.file_id) else {
            return Ok(None);
        };
        if name == self.main_name {
            return Self::read_in(self.main, &self.main_name, reference.path_id, class_id);
        }
        let Some(file) = self.siblings.get(&name) else {
            return Ok(None);
        };
        Self::read_in(file, &name, reference.path_id, class_id)
    }

    fn read_in(
        file: &mut StandaloneSerialized,
        name: &str,
        path_id: i64,
        class_id: u16,
    ) -> Result<Option<(String, Vec<u8>)>, ImportError> {
        let Some(object) = file
            .objects()
            .iter()
            .find(|o| o.path_id == path_id && o.class_id == class_id)
            .cloned()
        else {
            return Ok(None);
        };
        Ok(Some((name.to_owned(), file.read_object(&object)?)))
    }

    /// Resolves a material to its main texture payload. Returns
    /// `(material name, shader path id, (texture file, path id, payload))`.
    #[allow(clippy::type_complexity)]
    fn game_object_name(&mut self, game_object: Ref, fallback: &str) -> String {
        self.read(game_object, 1)
            .ok()
            .flatten()
            .and_then(|(file, data)| parse_game_object_name(&data, &file).ok())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| fallback.to_owned())
    }

    #[allow(clippy::type_complexity)]
    fn texture_payload(
        &mut self,
        material: Ref,
    ) -> Result<(Option<String>, Option<i64>, Option<(String, i64, Vec<u8>)>), ImportError> {
        let Some((material_file, bytes)) = self.read(material, MATERIAL_CLASS_ID)? else {
            return Ok((None, None, None));
        };
        let refs = MaterialRefs::parse(&bytes, &material_file)?;
        let shader = (!refs.shader.is_null()).then_some(refs.shader.path_id);
        let Some(texture_ref) = refs.main_texture() else {
            return Ok((Some(refs.name), shader, None));
        };
        // A texture PPtr is relative to the Material's own file.
        let target = if texture_ref.file_id == 0 {
            material_file.clone()
        } else if material_file == self.main_name {
            external_name(self.main, texture_ref.file_id).unwrap_or_default()
        } else {
            self.siblings
                .get(&material_file)
                .and_then(|f| external_name(f, texture_ref.file_id))
                .unwrap_or_default()
        };
        let Some(file_id) = self.main_index_of(&target) else {
            return Ok((Some(refs.name), shader, None));
        };
        let reference = Ref {
            file_id,
            path_id: texture_ref.path_id,
        };
        let payload = self
            .read(reference, TEXTURE_2D_CLASS_ID)?
            .map(|(file, data)| (file, texture_ref.path_id, data));
        Ok((Some(refs.name), shader, payload))
    }

    #[allow(clippy::type_complexity)]
    fn texture(
        &mut self,
        material: Ref,
    ) -> Result<(Option<String>, Option<TextureInfo>, Option<i64>), ImportError> {
        let (name, shader, payload) = self.texture_payload(material)?;
        let info = match payload {
            Some((file, path_id, data)) => {
                let header = TextureHeader::parse(&data, &file)?;
                Some(TextureInfo {
                    file,
                    path_id,
                    name: header.name,
                    width: header.width,
                    height: header.height,
                    format: header.format,
                })
            }
            None => None,
        };
        Ok((name, info, shader))
    }

    fn main_index_of(&self, name: &str) -> Option<i32> {
        if name == self.main_name {
            return Some(0);
        }
        self.main
            .inventory()
            .external_paths
            .iter()
            .position(|path| path.rsplit(['/', '\\']).next().unwrap_or(path) == name)
            .map(|index| index as i32 + 1)
    }
}

fn texture_fits(x: i32, y: i32, w: i32, h: i32, texture: &Option<TextureInfo>) -> bool {
    texture.as_ref().is_none_or(|t| {
        x >= 0
            && y >= 0
            && w >= 0
            && h >= 0
            && i64::from(x) + i64::from(w) <= i64::from(t.width)
            && i64::from(y) + i64::from(h) <= i64::from(t.height)
    })
}

/// Decodes every UIAtlas and UIFont MonoBehaviour in `path`; other script
/// classes are only counted.
pub fn scan_ui(path: &Path) -> Result<UiScan, ImportError> {
    let mut main = StandaloneSerialized::open(path)?;
    let file = main.inventory().file_name.clone();
    let dir = path.parent().unwrap_or_else(|| Path::new(".")).to_owned();
    let mut scan = UiScan {
        file: file.clone(),
        ..UiScan::default()
    };
    let behaviours: Vec<_> = main
        .objects()
        .iter()
        .filter(|o| o.class_id == MONO_BEHAVIOUR_CLASS_ID)
        .cloned()
        .collect();
    scan.mono_behaviours = behaviours.len();
    let mut resolver = Resolver {
        main_name: file.clone(),
        main: &mut main,
        siblings: Siblings {
            dir,
            open: BTreeMap::new(),
        },
    };
    let mut script_names: BTreeMap<Ref, String> = BTreeMap::new();
    for object in behaviours {
        let bytes = resolver.main.read_object(&object)?;
        let header = match parse_mono_header(&bytes, &file) {
            Ok(header) => header,
            Err(error) => {
                *scan.scripts.entry("?".to_owned()).or_insert(0) += 1;
                scan.failures.push(UiFailure {
                    file: file.clone(),
                    path_id: object.path_id,
                    script: "?".to_owned(),
                    message: error.to_string(),
                });
                continue;
            }
        };
        let script = match script_names.get(&header.script) {
            Some(name) => name.clone(),
            None => {
                let name = match resolver.read(header.script, MONO_SCRIPT_CLASS_ID) {
                    Ok(Some((script_file, data))) => parse_mono_script(&data, &script_file)
                        .map(|(_, class)| class)
                        .unwrap_or_else(|_| "?".to_owned()),
                    _ => "?".to_owned(),
                };
                script_names.insert(header.script, name.clone());
                name
            }
        };
        *scan.scripts.entry(script.clone()).or_insert(0) += 1;
        if script == "?" {
            scan.unresolved_scripts.push((
                object.path_id,
                header.script.file_id,
                header.script.path_id,
            ));
        }
        let mut fail = |message: String| {
            scan.failures.push(UiFailure {
                file: file.clone(),
                path_id: object.path_id,
                script: script.clone(),
                message,
            });
        };
        match script.as_str() {
            "UIAtlas" => match UiAtlas::parse(&bytes, &file) {
                Ok(atlas) => match resolver.texture(atlas.material) {
                    Ok((material, texture, shader)) => {
                        let atlas_name =
                            resolver.game_object_name(atlas.header.game_object, &atlas.header.name);
                        let info = texture;
                        let outside = atlas
                            .sprites
                            .iter()
                            .filter(|s| !texture_fits(s.x, s.y, s.width, s.height, &info))
                            .count();
                        scan.atlases.push(AtlasEntry {
                            file: file.clone(),
                            path_id: object.path_id,
                            name: atlas_name.clone(),
                            sprite_count: atlas.sprites.len(),
                            pixel_size: atlas.pixel_size,
                            coordinates: atlas.coordinates,
                            material,
                            texture: info,
                            sprites_outside_texture: outside,
                            shader,
                        });
                    }
                    Err(error) => fail(format!("material/texture: {error}")),
                },
                Err(error) => fail(error.to_string()),
            },
            "UIFont" => match UiFont::parse(&bytes, &file) {
                Ok(font) => {
                    let font_name =
                        resolver.game_object_name(font.header.game_object, &font.header.name);
                    let atlas_name = resolver
                        .read(font.atlas, MONO_BEHAVIOUR_CLASS_ID)
                        .ok()
                        .flatten()
                        .and_then(|(f, data)| parse_mono_header(&data, &f).ok())
                        .map(|h| h.name);
                    match resolver.texture(font.material) {
                        Ok((material, texture, _)) => {
                            let info = texture;
                            let outside = font
                                .bm_font
                                .glyphs
                                .iter()
                                .filter(|g| !texture_fits(g.x, g.y, g.width, g.height, &info))
                                .count();
                            scan.fonts.push(FontEntry {
                                file: file.clone(),
                                path_id: object.path_id,
                                name: font_name,
                                glyph_count: font.bm_font.glyphs.len(),
                                kerning_pairs: font
                                    .bm_font
                                    .glyphs
                                    .iter()
                                    .map(|g| g.kerning.len() / 2)
                                    .sum(),
                                char_size: font.bm_font.char_size,
                                base_offset: font.bm_font.base_offset,
                                tex_width: font.bm_font.tex_width,
                                tex_height: font.bm_font.tex_height,
                                symbol_count: font.symbols.len(),
                                sprite_name: font.bm_font.sprite_name.clone(),
                                atlas: atlas_name,
                                material,
                                texture: info,
                                has_dynamic_font: !font.dynamic_font.is_null(),
                                glyphs_outside_texture: outside,
                            });
                        }
                        Err(error) => fail(format!("material/texture: {error}")),
                    }
                }
                Err(error) => fail(error.to_string()),
            },
            _ => {}
        }
    }
    Ok(scan)
}

/// Scans every standalone serialized file directly inside `dir`
/// (`*.assets`, `mainData`, `levelN`); unreadable files are listed apart.
#[allow(clippy::type_complexity)]
pub fn scan_ui_dir(dir: &Path) -> Result<(Vec<UiScan>, Vec<(String, String)>), ImportError> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map_err(|source| ImportError::io("read FF9_Data directory", dir, source))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| {
            name.ends_with(".assets")
                || name == "mainData"
                || name
                    .strip_prefix("level")
                    .is_some_and(|n| n.parse::<u32>().is_ok())
        })
        .collect();
    names.sort();
    let mut scans = Vec::new();
    let mut unreadable = Vec::new();
    for name in names {
        match scan_ui(&dir.join(&name)) {
            Ok(scan) => scans.push(scan),
            Err(error) => unreadable.push((name, error.to_string())),
        }
    }
    Ok((scans, unreadable))
}

/// Decodes one named atlas texture of `path` to RGBA8 (top row first) and
/// returns `(width, height, rgba)`.
pub fn atlas_texture_rgba(
    path: &Path,
    atlas_name: &str,
) -> Result<(u32, u32, Vec<u8>), ImportError> {
    let (_, width, height, rgba) = load_atlas(path, atlas_name)?;
    Ok((width, height, rgba))
}

/// Decodes one named atlas of `path`: its sprite table plus the texture as RGBA8
/// (top row first), returned as `(atlas, width, height, rgba)`.
pub fn load_atlas(
    path: &Path,
    atlas_name: &str,
) -> Result<(UiAtlas, u32, u32, Vec<u8>), ImportError> {
    let mut main = StandaloneSerialized::open(path)?;
    let file = main.inventory().file_name.clone();
    let behaviours: Vec<_> = main
        .objects()
        .iter()
        .filter(|o| o.class_id == MONO_BEHAVIOUR_CLASS_ID)
        .cloned()
        .collect();
    let mut found = None;
    for object in behaviours {
        let bytes = main.read_object(&object)?;
        if let Ok(atlas) = UiAtlas::parse(&bytes, &file) {
            let go = main
                .objects()
                .iter()
                .find(|o| o.path_id == atlas.header.game_object.path_id && o.class_id == 1)
                .cloned();
            let name = match go {
                Some(go) if atlas.header.game_object.file_id == 0 => {
                    let data = main.read_object(&go)?;
                    parse_game_object_name(&data, &file).unwrap_or_default()
                }
                _ => String::new(),
            };
            if name == atlas_name || atlas.header.name == atlas_name {
                found = Some(atlas);
                break;
            }
        }
    }
    let atlas = found.ok_or_else(|| {
        ImportError::invalid(path.display(), format!("no UIAtlas named `{atlas_name}`"))
    })?;
    let mut resolver = Resolver {
        main_name: file,
        main: &mut main,
        siblings: Siblings {
            dir: path.parent().unwrap_or_else(|| Path::new(".")).to_owned(),
            open: BTreeMap::new(),
        },
    };
    let (_, _, payload) = resolver.texture_payload(atlas.material)?;
    let (file, _, data) = payload
        .ok_or_else(|| ImportError::invalid(path.display(), "atlas texture is unresolved"))?;
    let header = TextureHeader::parse(&data, &file)?;
    let rgba = decode_texture_rgba(&data, &file)?;
    Ok((atlas, header.width as u32, header.height as u32, rgba))
}

/// One atlas sprite cut out of its texture: RGBA8, top row first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpriteImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Cuts the sprites `names` out of the atlas `atlas_name` of `path`
/// (`UISpriteData` rectangles index the decoded texture top-first). A name the atlas
/// lacks is an error.
pub fn load_sprites(
    path: &Path,
    atlas_name: &str,
    names: &[&str],
) -> Result<BTreeMap<String, SpriteImage>, ImportError> {
    let (atlas, width, height, rgba) = load_atlas(path, atlas_name)?;
    let mut sprites = BTreeMap::new();
    for name in names {
        let sprite = atlas
            .sprites
            .iter()
            .find(|sprite| sprite.name == *name)
            .ok_or_else(|| {
                ImportError::invalid(
                    path.display(),
                    format!("{atlas_name} has no sprite `{name}`"),
                )
            })?;
        let (w, h) = (sprite.width.max(0) as usize, sprite.height.max(0) as usize);
        let mut cut = Vec::with_capacity(w * h * 4);
        for row in 0..h {
            for column in 0..w {
                let x = (sprite.x as usize + column).min(width as usize - 1);
                let y = (sprite.y as usize + row).min(height as usize - 1);
                let at = (y * width as usize + x) * 4;
                cut.extend_from_slice(&rgba[at..at + 4]);
            }
        }
        sprites.insert(
            (*name).to_owned(),
            SpriteImage {
                width: w as u32,
                height: h as u32,
                rgba: cut,
            },
        );
    }
    Ok(sprites)
}

/// The leading Texture2D fields (enough to size and classify it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureHeader {
    pub name: String,
    pub width: i32,
    pub height: i32,
    pub complete_image_size: i32,
    pub format: i32,
}

impl TextureHeader {
    pub fn parse(bytes: &[u8], source: &str) -> Result<Self, ImportError> {
        let mut r = Reader::new(bytes, source);
        let name = r.string()?;
        let width = r.i32()?;
        let height = r.i32()?;
        let complete_image_size = r.i32()?;
        let format = r.i32()?;
        if !(1..=16_384).contains(&width) || !(1..=16_384).contains(&height) {
            return Err(r.err("implausible texture dimensions"));
        }
        Ok(Self {
            name,
            width,
            height,
            complete_image_size,
            format,
        })
    }
}

/// Decodes a Texture2D payload to RGBA8, top row first. DXT1/DXT5 use the
/// shared decoder; ARGB32 (5), RGBA32 (4), BGRA32 (14), RGB24 (3) and
/// Alpha8 (1) are expanded here.
pub fn decode_texture_rgba(bytes: &[u8], source: &str) -> Result<Vec<u8>, ImportError> {
    let header = TextureHeader::parse(bytes, source)?;
    if header.format == 10 || header.format == 12 {
        return Texture2D::parse(bytes, source)?.rgba_bytes();
    }
    let mut r = Reader::new(bytes, source);
    r.string()?;
    r.take(16)?;
    r.i32()?; // mip count
    r.take(2)?;
    r.align()?;
    r.take(4 * 8)?; // image count, dimension, settings, lightmap, color space
    let size = r.count(1)?;
    let data = r.take(size)?;
    let (width, height) = (header.width as usize, header.height as usize);
    let per_pixel = match header.format {
        1 => 1,
        3 => 3,
        4 | 5 | 14 => 4,
        other => {
            return Err(ImportError::unsupported(
                source,
                format!("Texture2D format {other}"),
            ));
        }
    };
    // The first mip level is stored first.
    let level = width * height * per_pixel;
    let data = data
        .get(..level)
        .ok_or_else(|| r.err("pixel data is shorter than the base level"))?;
    let mut rgba = Vec::with_capacity(width * height * 4);
    for row in (0..height).rev() {
        for px in
            data[row * width * per_pixel..(row + 1) * width * per_pixel].chunks_exact(per_pixel)
        {
            match header.format {
                1 => rgba.extend_from_slice(&[255, 255, 255, px[0]]),
                3 => rgba.extend_from_slice(&[px[0], px[1], px[2], 255]),
                4 => rgba.extend_from_slice(px),
                5 => rgba.extend_from_slice(&[px[1], px[2], px[3], px[0]]),
                _ => rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]),
            }
        }
    }
    Ok(rgba)
}

/// Minimal PNG writer (RGBA8, stored deflate blocks) for debug exports.
pub fn encode_png_rgba(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut crc = !0_u32;
        for byte in data {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
    }
    let stride = width as usize * 4;
    let mut raw = Vec::with_capacity((stride + 1) * height as usize);
    for row in rgba.chunks(stride.max(1)).take(height as usize) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65_535).peekable();
    if blocks.peek().is_none() {
        z.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    while let Some(block) = blocks.next() {
        z.push(u8::from(blocks.peek().is_none()));
        z.extend_from_slice(&(block.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        z.extend_from_slice(block);
    }
    let (mut a, mut b) = (1_u32, 0_u32);
    for byte in &raw {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Human-readable class name for a class id (re-exported for the CLI).
pub fn class_label(class_id: u16) -> &'static str {
    class_name(class_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_str(out: &mut Vec<u8>, text: &str) {
        out.extend_from_slice(&(text.len() as u32).to_le_bytes());
        out.extend_from_slice(text.as_bytes());
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
    }

    fn put_i32(out: &mut Vec<u8>, v: i32) {
        out.extend_from_slice(&v.to_le_bytes());
    }

    fn put_pptr(out: &mut Vec<u8>, file: i32, path: i64) {
        put_i32(out, file);
        out.extend_from_slice(&path.to_le_bytes());
    }

    fn header(name: &str) -> Vec<u8> {
        let mut out = Vec::new();
        put_pptr(&mut out, 0, 0);
        out.extend_from_slice(&[1, 0, 0, 0]);
        put_pptr(&mut out, 1, 77);
        put_str(&mut out, name);
        out
    }

    fn atlas_bytes() -> Vec<u8> {
        let mut out = header("Gray Atlas");
        put_pptr(&mut out, 0, 5);
        put_i32(&mut out, 2);
        for (name, x) in [("a", 0), ("bc", 16)] {
            put_str(&mut out, name);
            for v in [x, 2, 8, 9, 1, 2, 3, 4, 5, 6, 7, 8] {
                put_i32(&mut out, v);
            }
        }
        out.extend_from_slice(&1.0_f32.to_le_bytes());
        put_pptr(&mut out, 0, 0);
        put_i32(&mut out, 0);
        put_i32(&mut out, 0);
        out
    }

    #[test]
    fn parses_atlas_and_requires_exact_consumption() {
        let bytes = atlas_bytes();
        let atlas = UiAtlas::parse(&bytes, "t").unwrap();
        assert_eq!(atlas.header.name, "Gray Atlas");
        assert_eq!(
            atlas.header.script,
            Ref {
                file_id: 1,
                path_id: 77
            }
        );
        assert_eq!(
            atlas.material,
            Ref {
                file_id: 0,
                path_id: 5
            }
        );
        assert_eq!(atlas.sprites.len(), 2);
        assert_eq!(atlas.sprites[1].name, "bc");
        assert_eq!(atlas.sprites[1].x, 16);
        assert_eq!(atlas.sprites[1].border_left, 1);
        assert_eq!(atlas.sprites[1].padding_bottom, 8);
        let mut extra = bytes.clone();
        extra.extend_from_slice(&[0; 4]);
        assert!(UiAtlas::parse(&extra, "t").is_err());
        assert!(UiAtlas::parse(&bytes[..bytes.len() - 4], "t").is_err());
    }

    #[test]
    fn parses_font_with_kerning_and_symbols() {
        let mut out = header("Font");
        put_pptr(&mut out, 0, 9);
        for v in [0.0_f32, 0.0, 1.0, 1.0] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for v in [24, 20, 256, 128] {
            put_i32(&mut out, v);
        }
        put_str(&mut out, "glyphs");
        put_i32(&mut out, 1);
        for v in [65, 1, 2, 10, 12, 0, 3, 11, 0] {
            put_i32(&mut out, v);
        }
        put_i32(&mut out, 2);
        put_i32(&mut out, 86);
        put_i32(&mut out, -2);
        put_pptr(&mut out, 0, 0);
        put_pptr(&mut out, 0, 0);
        put_i32(&mut out, 1);
        put_str(&mut out, "[a]");
        put_str(&mut out, "icon");
        put_pptr(&mut out, 0, 0);
        put_i32(&mut out, 16);
        put_i32(&mut out, 0);
        let font = UiFont::parse(&out, "t").unwrap();
        assert_eq!(font.bm_font.glyphs.len(), 1);
        assert_eq!(font.bm_font.glyphs[0].id, 65);
        assert_eq!(font.bm_font.glyphs[0].kerning, vec![86, -2]);
        assert_eq!(font.symbols, vec![("[a]".to_owned(), "icon".to_owned())]);
        assert!(UiFont::parse(&out[..out.len() - 1], "t").is_err());
    }

    #[test]
    fn parses_material_texture_slots() {
        let mut out = Vec::new();
        put_str(&mut out, "Mat");
        put_pptr(&mut out, 1, 3);
        put_str(&mut out, "");
        put_i32(&mut out, 0);
        put_i32(&mut out, -1);
        put_i32(&mut out, 0);
        put_i32(&mut out, 2);
        for (prop, path) in [("_Other", 0_i64), ("_MainTex", 42)] {
            put_str(&mut out, prop);
            put_pptr(&mut out, 0, path);
            out.extend_from_slice(&[0; 16]);
        }
        put_i32(&mut out, 1);
        put_str(&mut out, "_Cutoff");
        out.extend_from_slice(&0.5_f32.to_le_bytes());
        put_i32(&mut out, 1);
        put_str(&mut out, "_Color");
        out.extend_from_slice(&[0; 16]);
        let material = MaterialRefs::parse(&out, "t").unwrap();
        assert_eq!(
            material.main_texture(),
            Some(Ref {
                file_id: 0,
                path_id: 42
            })
        );
        assert_eq!(material.shader.path_id, 3);
    }

    #[test]
    fn script_and_header_parse() {
        let mut out = Vec::new();
        put_str(&mut out, "UIAtlas");
        put_i32(&mut out, 0);
        out.extend_from_slice(&[0; 16]);
        put_str(&mut out, "UIAtlas");
        let (name, class) = parse_mono_script(&out, "t").unwrap();
        assert_eq!((name.as_str(), class.as_str()), ("UIAtlas", "UIAtlas"));
        let header = parse_mono_header(&header("X"), "t").unwrap();
        assert!(header.enabled);
        assert_eq!(header.name, "X");
    }

    #[test]
    fn rect_bounds_check() {
        let texture = Some(TextureInfo {
            file: String::new(),
            path_id: 0,
            name: String::new(),
            width: 64,
            height: 32,
            format: 12,
        });
        assert!(texture_fits(0, 0, 64, 32, &texture));
        assert!(!texture_fits(1, 0, 64, 32, &texture));
        assert!(!texture_fits(-1, 0, 4, 4, &texture));
    }

    #[test]
    fn png_encoder_has_valid_structure() {
        let png = encode_png_rgba(2, 1, &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
    }

    #[test]
    fn real_install_ui_scan() {
        let dir = Path::new("E:/SteamLibrary/steamapps/common/FINAL FANTASY IX/x64/FF9_Data");
        let path = dir.join("sharedassets2.assets");
        if !path.is_file() {
            eprintln!("skipping: FFIX install not present");
            return;
        }
        let scan = scan_ui(&path).expect("scan");
        assert!(scan.failures.is_empty(), "{:?}", scan.failures);
        assert!(!scan.atlases.is_empty());
        for atlas in &scan.atlases {
            assert_eq!(atlas.sprites_outside_texture, 0, "{}", atlas.name);
            assert!(atlas.texture.is_some(), "{}", atlas.name);
        }
    }
}
