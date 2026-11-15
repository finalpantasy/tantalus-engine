//! Streaming, read-only inventory of standalone Unity serialized files
//! (`resources.assets`, `sharedassetsN.assets`, `mainData`, `levelN`).
//!
//! Only the metadata prefix (`data_offset` bytes) is loaded; object payloads are read
//! on demand, so a 590 MB file never has to sit in memory.

use super::serialized::{SerializedFile, SerializedObject};
use crate::install::ImportError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Class IDs whose first serialized field is the object name (Unity `NamedObject`).
const NAMED_CLASSES: &[u16] = &[
    21, 28, 43, 48, 49, 74, 83, 84, 89, 115, 128, 152, 156, 213, 272,
];
const MAX_NAME_BYTES: usize = 1024;
pub const TEXT_ASSET_CLASS_ID: u16 = 49;

pub fn class_name(class_id: u16) -> &'static str {
    match class_id {
        1 => "GameObject",
        4 => "Transform",
        20 => "Camera",
        21 => "Material",
        23 => "MeshRenderer",
        25 => "Renderer",
        28 => "Texture2D",
        33 => "MeshFilter",
        43 => "Mesh",
        48 => "Shader",
        49 => "TextAsset",
        74 => "AnimationClip",
        83 => "AudioClip",
        84 => "RenderTexture",
        89 => "CubeMap",
        114 => "MonoBehaviour",
        115 => "MonoScript",
        128 => "Font",
        152 => "MovieTexture",
        156 => "TerrainData",
        213 => "Sprite",
        272 => "AudioMixer",
        _ => "Other",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassInventory {
    pub class_id: u16,
    pub class_name: String,
    pub count: usize,
    /// Names, in object order, for classes whose first field is a name.
    pub names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SerializedInventory {
    pub file_name: String,
    pub file_size: u64,
    pub format_version: u32,
    pub unity_version: String,
    pub target_platform: i32,
    pub type_trees_embedded: bool,
    pub object_count: usize,
    pub external_paths: Vec<String>,
    pub classes: Vec<ClassInventory>,
}

/// A parsed standalone file: metadata plus the open handle for payload reads.
pub struct StandaloneSerialized {
    file: File,
    source: String,
    objects: Vec<SerializedObject>,
    meta: SerializedInventory,
}

impl StandaloneSerialized {
    pub fn open(path: &Path) -> Result<Self, ImportError> {
        let source = path.display().to_string();
        let mut file =
            File::open(path).map_err(|e| ImportError::io("open serialized file", path, e))?;
        let total = file
            .metadata()
            .map_err(|e| ImportError::io("stat serialized file", path, e))?
            .len();
        let mut head = [0_u8; 16];
        file.read_exact(&mut head)
            .map_err(|e| ImportError::io("read serialized header", path, e))?;
        let data_offset = u64::from(u32::from_be_bytes(head[12..16].try_into().expect("4")));
        let prefix_len = data_offset.min(total);
        if !(20..=64 * 1024 * 1024).contains(&prefix_len) {
            return Err(ImportError::invalid(
                &source,
                format!("implausible serialized metadata size {prefix_len}"),
            ));
        }
        file.seek(SeekFrom::Start(0))
            .map_err(|e| ImportError::io("seek serialized file", path, e))?;
        let mut prefix = vec![0_u8; prefix_len as usize];
        file.read_exact(&mut prefix)
            .map_err(|e| ImportError::io("read serialized metadata", path, e))?;
        let parsed = SerializedFile::parse_prefix(&prefix, total, &source)?;
        let report = parsed.report().clone();
        let mut me = Self {
            file,
            source,
            objects: report.objects.clone(),
            meta: SerializedInventory {
                file_name: path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                file_size: total,
                format_version: report.format_version,
                unity_version: report.unity_version,
                target_platform: report.target_platform,
                type_trees_embedded: report.type_trees_embedded,
                object_count: report.objects.len(),
                external_paths: report.external_paths,
                classes: Vec::new(),
            },
        };
        me.meta.classes = me.build_classes();
        Ok(me)
    }

    pub fn inventory(&self) -> &SerializedInventory {
        &self.meta
    }

    pub fn objects(&self) -> &[SerializedObject] {
        &self.objects
    }

    /// Reads the whole payload of one object.
    pub fn read_object(&mut self, object: &SerializedObject) -> Result<Vec<u8>, ImportError> {
        self.read_at(object.absolute_offset, object.size as usize)
    }

    fn read_at(&mut self, offset: u64, len: usize) -> Result<Vec<u8>, ImportError> {
        self.file
            .seek(SeekFrom::Start(offset))
            .map_err(|e| ImportError::invalid(&self.source, format!("seek failed: {e}")))?;
        let mut buf = vec![0_u8; len];
        self.file
            .read_exact(&mut buf)
            .map_err(|e| ImportError::invalid(&self.source, format!("read failed: {e}")))?;
        Ok(buf)
    }

    /// Reads the leading `name` string of a NamedObject payload.
    pub fn object_name(&mut self, object: &SerializedObject) -> Result<String, ImportError> {
        let want = (object.size as usize).min(4 + MAX_NAME_BYTES);
        let buf = self.read_at(object.absolute_offset, want)?;
        if buf.len() < 4 {
            return Err(ImportError::invalid(
                &self.source,
                "object too small for a name",
            ));
        }
        let len = u32::from_le_bytes(buf[..4].try_into().expect("4")) as usize;
        let bytes = buf.get(4..4 + len).ok_or_else(|| {
            ImportError::invalid(
                &self.source,
                format!(
                    "object {} name length {len} exceeds payload",
                    object.path_id
                ),
            )
        })?;
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }

    fn build_classes(&mut self) -> Vec<ClassInventory> {
        let mut map: BTreeMap<u16, ClassInventory> = BTreeMap::new();
        let objects = self.objects.clone();
        for object in &objects {
            let named = NAMED_CLASSES.contains(&object.class_id);
            let name = if named {
                Some(self.object_name(object).unwrap_or_default())
            } else {
                None
            };
            let entry = map
                .entry(object.class_id)
                .or_insert_with(|| ClassInventory {
                    class_id: object.class_id,
                    class_name: class_name(object.class_id).to_owned(),
                    count: 0,
                    names: Vec::new(),
                });
            entry.count += 1;
            if let Some(name) = name {
                entry.names.push(name);
            }
        }
        map.into_values().collect()
    }

    /// All TextAssets as (name, object).
    pub fn text_assets(&mut self) -> Result<Vec<(String, SerializedObject)>, ImportError> {
        let objects: Vec<_> = self
            .objects
            .iter()
            .filter(|o| o.class_id == TEXT_ASSET_CLASS_ID)
            .cloned()
            .collect();
        let mut out = Vec::new();
        for object in objects {
            let name = self.object_name(&object)?;
            out.push((name, object));
        }
        Ok(out)
    }

    /// Reads and decodes a TextAsset payload.
    pub fn read_text_asset(
        &mut self,
        object: &SerializedObject,
    ) -> Result<super::text_asset::TextAsset, ImportError> {
        let bytes = self.read_object(object)?;
        super::text_asset::TextAsset::parse(&bytes, &self.source)
    }
}

pub fn inspect_serialized_file(path: &Path) -> Result<SerializedInventory, ImportError> {
    Ok(StandaloneSerialized::open(path)?.meta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_install_resources_assets_inventory() {
        let path = Path::new(
            "E:/SteamLibrary/steamapps/common/FINAL FANTASY IX/x64/FF9_Data/resources.assets",
        );
        if !path.is_file() {
            eprintln!("skipping: FFIX install not present");
            return;
        }
        let inventory = inspect_serialized_file(path).expect("inventory");
        assert_eq!(inventory.format_version, 15);
        assert!(inventory.unity_version.starts_with("5."));
        let text = inventory
            .classes
            .iter()
            .find(|c| c.class_id == TEXT_ASSET_CLASS_ID)
            .expect("TextAssets");
        assert_eq!(text.count, text.names.len());
        assert!(text.names.iter().any(|n| n.ends_with(".mes")));
    }

    #[test]
    fn missing_file_is_an_error() {
        assert!(StandaloneSerialized::open(Path::new("does/not/exist.assets")).is_err());
    }
}
