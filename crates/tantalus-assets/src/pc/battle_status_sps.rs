//! The battle status effects drawn as SPS sprite strips (Poison, Venom, Sleep,
//! Heat, Freeze, Reflect, Blind, Berserk): `BattleSPSSystem` with the
//! prototypes of Memoria's `Data/SpecialEffects/Common/SPS.csv`.
//!
//! Both halves are in the shipped game data: the frame files are TextAssets of
//! the battle bundle (`p0data2.bin`, `assets/resources/battlemap/battlesps/st_*.sps.bytes`,
//! `CommonSPSSystem.LoadSPSBin`), the textures are Texture2Ds of `resources.assets`
//! reached through the `ResourceManager` container in `mainData`
//! (`EmbeddedAsset/BattleMap/Status/<name>`, `CommonSPSSystem.SetupSPSTexture`).

use crate::install::ImportError;
use crate::pc::asset_bundle::AssetBundleMetadata;
use crate::pc::field_sps::SpsFile;
use crate::pc::resource_container::{RESOURCE_MANAGER_CLASS_ID, parse_resource_container};
use crate::pc::serialized::SerializedFile;
use crate::pc::serialized_inventory::StandaloneSerialized;
use crate::pc::text_asset::TextAsset;
use crate::pc::texture::Texture2D;
use crate::pc::unity_raw::UnityRawBundle;
use std::path::Path;

/// One `SPS.csv` prototype (`SPSPrototype`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StatusSpsPrototype {
    /// The frame file under `BattleMap/BattleSPS/`.
    pub sps: &'static str,
    /// The texture under `EmbeddedAsset/BattleMap/Status/`.
    pub texture: &'static str,
    /// `ShaderIndex`: the `SPSConst.ABR_*` blend (255 is none).
    pub abr: u8,
    /// `BattleScale` and `BattleDistance` (`SPSEffect.battleScaleFactor`, `battleDistanceFactor`).
    pub battle_scale: f32,
    pub battle_distance: f32,
}

/// `SPS.csv` ids 0 to 7.
pub const PROTOTYPES: [StatusSpsPrototype; 8] = [
    proto("st_doku", "poison", 1, 6.0, 4.0),
    proto("st_mdoku", "venom", 1, 2.0, 1.5),
    proto("st_nemu", "sleep", 255, 2.5, 4.5),
    proto("st_heat", "heat", 1, 4.0, 5.0),
    proto("st_friz", "freeze", 1, 4.0, 5.0),
    proto("st_rif", "reflect", 1, 3.0, 3.0),
    proto("st_moum", "blind", 2, 5.0, 5.5),
    proto("st_basak", "berserk", 1, 3.0, 2.0),
];

const fn proto(
    sps: &'static str,
    texture: &'static str,
    abr: u8,
    battle_scale: f32,
    battle_distance: f32,
) -> StatusSpsPrototype {
    StatusSpsPrototype {
        sps,
        texture,
        abr,
        battle_scale,
        battle_distance,
    }
}

/// `StatusData.csv`'s `SPSEffect` column: status bit (`BattleStatusId`), SPS
/// prototype and the icon position it attaches to (`btl2d.ICON_POS_*`).
pub const STATUS_SPS: [(u32, usize, usize); 8] = [
    (1, 1, 0),  // Venom
    (4, 6, 3),  // Blind, at the eyes
    (11, 7, 4), // Berserk, at the forehead
    (16, 0, 0), // Poison
    (17, 2, 0), // Sleep
    (24, 3, 1), // Heat, at the head
    (25, 4, 1), // Freeze, at the head
    (29, 5, 1), // Reflect, at the head
];

/// Where a unit's status icons sit (`btl2d.GetIconPosition`): per icon position
/// (`ICON_POS_DEFAULT` to `ICON_POS_NUMBER`), the bone, and the offsets up and forward in
/// 16-unit steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StatusIcons {
    pub bone: [u8; 6],
    pub y: [i8; 6],
    pub z: [i8; 6],
}

const fn icons(bone: [u8; 6], y: [i8; 6], z: [i8; 6]) -> StatusIcons {
    StatusIcons { bone, y, z }
}

/// The party's icon positions by serial number: `BattleParameters.csv`'s `StatusBone`,
/// `StatusOffsetY` and `StatusOffsetZ` (`CharacterBattleParameter`).
pub const PLAYER_ICONS: [StatusIcons; 19] = [
    // Zidane (Dagger)
    icons(
        [8, 8, 8, 8, 8, 1],
        [-10, 0, -7, -1, -7, -18],
        [-1, -2, -9, -10, -6, 0],
    ),
    // Zidane (Thief sword)
    icons(
        [8, 8, 8, 8, 8, 1],
        [-10, 0, -7, -1, -7, -18],
        [-1, -2, -9, -10, -6, 0],
    ),
    // Vivi
    icons(
        [7, 7, 7, 7, 7, 1],
        [-11, 0, -8, -3, -8, -18],
        [0, -1, -7, -8, -3, 0],
    ),
    // Garnet (Long hair - Rod)
    icons(
        [19, 19, 19, 19, 19, 1],
        [-10, 0, -7, -1, -7, -18],
        [-1, -2, -9, -8, -5, 0],
    ),
    // Garnet (Long hair - Racket)
    icons(
        [19, 19, 19, 19, 19, 1],
        [-10, 0, -7, -1, -7, -18],
        [-1, -2, -9, -8, -5, 0],
    ),
    // Garnet (Short hair - Rod)
    icons(
        [19, 19, 19, 19, 19, 1],
        [-10, 0, -7, -1, -7, -18],
        [-1, -2, -9, -8, -5, 0],
    ),
    // Garnet (Short hair - Racket)
    icons(
        [19, 19, 19, 19, 19, 1],
        [-10, 0, -7, -1, -7, -18],
        [-1, -2, -9, -8, -5, 0],
    ),
    // Steiner (Outdoor)
    icons(
        [20, 20, 20, 20, 20, 1],
        [-11, 0, -6, -1, -6, -22],
        [0, -3, -8, -9, -5, 0],
    ),
    // Steiner (Indoor)
    icons(
        [20, 20, 20, 20, 20, 1],
        [-11, 0, -6, -1, -6, -22],
        [0, -3, -8, -9, -5, 0],
    ),
    // Quina
    icons(
        [6, 6, 6, 6, 6, 1],
        [-12, -2, -9, -1, -9, -23],
        [0, -1, -10, -9, -5, 0],
    ),
    // Eiko (flute)
    icons(
        [19, 19, 19, 19, 19, 1],
        [-10, 0, -7, -1, -7, -17],
        [-1, 0, -7, -7, -5, 0],
    ),
    // Eiko (Racket)
    icons(
        [19, 19, 19, 19, 19, 1],
        [-10, 0, -7, -1, -7, -17],
        [-1, 0, -7, -7, -5, 0],
    ),
    // Freya
    icons(
        [8, 8, 8, 8, 8, 1],
        [-13, 0, -7, -2, -7, -23],
        [0, -2, -8, -8, -4, 0],
    ),
    // Amarant
    icons(
        [18, 18, 18, 18, 18, 1],
        [-13, -2, -7, 1, -7, -21],
        [0, 0, -8, -11, -6, 0],
    ),
    // Cinna
    icons(
        [12, 12, 12, 12, 12, 1],
        [-11, 0, -8, -2, -8, -19],
        [-2, -2, -9, -9, -6, 0],
    ),
    // Marcus
    icons(
        [8, 8, 8, 8, 8, 1],
        [-9, 0, -6, -1, -6, -21],
        [-1, -1, -9, -9, -5, 0],
    ),
    // Blank
    icons(
        [3, 3, 3, 3, 3, 1],
        [-12, 0, -8, -2, -8, -19],
        [-1, -1, -8, -8, -5, 0],
    ),
    // Blank (Pluto armor)
    icons(
        [3, 3, 3, 3, 3, 1],
        [-10, 0, -8, -2, -8, -18],
        [-1, -1, -8, -8, -5, 0],
    ),
    // Beatrix
    icons(
        [18, 18, 18, 18, 18, 1],
        [-10, 0, -6, -1, -6, -18],
        [-1, -2, -7, -8, -5, 0],
    ),
];

/// A status SPS with its texture.
#[derive(Debug, Clone)]
pub struct StatusSps {
    pub prototype: StatusSpsPrototype,
    pub file: SpsFile,
    pub width: usize,
    pub height: usize,
    /// RGBA, rows top-down.
    pub rgba: Vec<u8>,
}

impl StatusSps {
    /// `SPSEffect.GetRelevantPartOfTCB`: the texel rectangle every frame's sprites read, which
    /// a texture smaller than a whole 256x256 page holds; `(min, size)`.
    pub fn texel_area(&self) -> ([f32; 2], [f32; 2]) {
        if self.width == 256 && self.height == 256 {
            // `_GenerateSPSMesh`: a 256x256 texture is taken for a VRAM page.
            return ([0.0, 0.0], [256.0, 256.0]);
        }
        let (w, h) = self.file.texel_size();
        let mut min = [f32::MAX; 2];
        let mut max = [f32::MIN; 2];
        for prim in self.file.frames.iter().flatten() {
            min[0] = min[0].min(f32::from(prim.u));
            min[1] = min[1].min(f32::from(prim.v));
            max[0] = max[0].max(f32::from(prim.u) + w as f32);
            max[1] = max[1].max(f32::from(prim.v) + h as f32);
        }
        if min[0] > max[0] {
            return ([0.0, 0.0], [1.0, 1.0]);
        }
        (min, [max[0] - min[0], max[1] - min[1]])
    }
}

/// Loads the eight status SPS: frame files from the battle bundle `battle_bundle`
/// (`p0data2.bin`), textures through `game_data` (`x64/FF9_Data`). A prototype whose data
/// is missing is `None`.
pub fn load_status_sps(
    battle_bundle: &Path,
    game_data: &Path,
) -> Result<Vec<Option<StatusSps>>, ImportError> {
    let files = load_frame_files(battle_bundle)?;
    let textures = load_textures(game_data)?;
    Ok(PROTOTYPES
        .iter()
        .zip(files.into_iter().zip(textures))
        .map(|(prototype, (file, texture))| {
            let file = file?;
            let (width, height, rgba) = texture?;
            Some(StatusSps {
                prototype: *prototype,
                file,
                width,
                height,
                rgba,
            })
        })
        .collect())
}

fn load_frame_files(bundle_path: &Path) -> Result<Vec<Option<SpsFile>>, ImportError> {
    let source = bundle_path.display().to_string();
    let bundle = UnityRawBundle::open(bundle_path)?;
    let bytes = bundle.map_entry(0, 300 * 1024 * 1024)?;
    let serialized = SerializedFile::parse(&bytes, &source)?;
    let metadata_index = serialized
        .report()
        .objects
        .iter()
        .position(|object| object.class_id == 142)
        .ok_or_else(|| ImportError::invalid(&source, "battle bundle has no AssetBundle"))?;
    let metadata = AssetBundleMetadata::parse(
        serialized.object_data(metadata_index)?,
        &format!("{source}:AssetBundle"),
    )?;
    Ok(PROTOTYPES
        .iter()
        .map(|prototype| {
            let path = format!(
                "assets/resources/battlemap/battlesps/{}.sps.bytes",
                prototype.sps
            );
            let (_, info) = metadata.container.iter().find(|(name, _)| *name == path)?;
            let index = serialized
                .report()
                .objects
                .iter()
                .position(|object| object.class_id == 49 && object.path_id == info.path_id)?;
            let asset = TextAsset::parse(serialized.object_data(index).ok()?, &source).ok()?;
            SpsFile::parse(&asset.script, &format!("{source}:{path}")).ok()
        })
        .collect())
}

type Rgba = (usize, usize, Vec<u8>);

fn load_textures(data_dir: &Path) -> Result<Vec<Option<Rgba>>, ImportError> {
    let main = data_dir.join("mainData");
    let mut file = StandaloneSerialized::open(&main)?;
    let manager = file
        .objects()
        .iter()
        .find(|object| object.class_id == RESOURCE_MANAGER_CLASS_ID)
        .cloned()
        .ok_or_else(|| ImportError::invalid(main.display().to_string(), "no ResourceManager"))?;
    let bytes = file.read_object(&manager)?;
    let externals = file.inventory().external_paths.clone();
    let entries = parse_resource_container(&bytes, &main.display().to_string())?;
    let mut opened: Vec<(String, StandaloneSerialized)> = Vec::new();
    let mut textures = Vec::new();
    for prototype in &PROTOTYPES {
        let key = format!("embeddedasset/battlemap/status/{}", prototype.texture);
        let Some(entry) = entries
            .iter()
            .find(|entry| entry.path.to_lowercase() == key)
        else {
            textures.push(None);
            continue;
        };
        let file_name = match usize::try_from(entry.file_id) {
            Ok(0) => "mainData".to_owned(),
            Ok(index) => match externals.get(index - 1) {
                Some(path) => path.rsplit(['/', '\\']).next().unwrap_or(path).to_owned(),
                None => {
                    textures.push(None);
                    continue;
                }
            },
            Err(_) => {
                textures.push(None);
                continue;
            }
        };
        if !opened.iter().any(|(name, _)| *name == file_name) {
            let serialized = StandaloneSerialized::open(&data_dir.join(&file_name))?;
            opened.push((file_name.clone(), serialized));
        }
        let (_, serialized) = opened
            .iter_mut()
            .find(|(name, _)| *name == file_name)
            .expect("opened above");
        let texture = serialized
            .objects()
            .iter()
            .find(|object| object.path_id == entry.path_id && object.class_id == 28)
            .cloned()
            .and_then(|object| serialized.read_object(&object).ok())
            .and_then(|bytes| Texture2D::parse(&bytes, &key).ok())
            .and_then(|texture| {
                let rgba = texture.rgba_bytes().ok()?;
                Some((texture.width as usize, texture.height as usize, rgba))
            });
        textures.push(texture);
    }
    Ok(textures)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real-install check; skips when the install is absent.
    #[test]
    fn real_install_status_sps_load() {
        let root = Path::new("E:/SteamLibrary/steamapps/common/FINAL FANTASY IX");
        if !root.join("StreamingAssets/p0data2.bin").is_file() {
            eprintln!("skipping: FFIX install not present");
            return;
        }
        let loaded = load_status_sps(
            &root.join("StreamingAssets/p0data2.bin"),
            &root.join("x64/FF9_Data"),
        )
        .unwrap();
        for sps in &loaded {
            let sps = sps.as_ref().expect("every status SPS loads");
            println!(
                "{} {}x{} frames {} half {}x{} area {:?}",
                sps.prototype.sps,
                sps.width,
                sps.height,
                sps.file.frame_count,
                sps.file.half_width,
                sps.file.half_height,
                sps.texel_area()
            );
        }
    }
}
