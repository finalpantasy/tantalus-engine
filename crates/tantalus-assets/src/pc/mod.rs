pub mod akb2;
pub mod asset_bundle;
pub mod audio_decode;
pub mod battle_scene;
pub mod battle_tables;
pub mod bgi;
pub mod bgs;
pub mod bundled_field;
pub mod dialog_render;
pub mod field;
pub mod field_composition;
pub mod field_models;
pub mod font_bundle;
pub mod map_config;
pub mod mbg;
pub mod mes;
pub mod model_assembly;
pub mod ogg;
pub mod resource_container;
pub mod serialized;
pub mod serialized_inventory;
pub mod text_asset;
pub mod texture;
pub mod ui_atlas;
pub mod unity_object;
pub mod unity_raw;
pub mod world_map;

use crate::install::ImportError;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;
use std::path::Path;

const UNITY_FS_MAGIC: &[u8; 8] = b"UnityFS\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PcContainerKind {
    UnityFs,
    Unrecognized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodecSupport {
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PcContainerReport {
    pub path: String,
    pub kind: PcContainerKind,
    pub codec_support: CodecSupport,
    pub detail: String,
}

pub fn inspect_pc_container(path: &Path) -> Result<PcContainerReport, ImportError> {
    let mut file =
        File::open(path).map_err(|source| ImportError::io("open PC container", path, source))?;
    let mut header = [0_u8; 8];
    let count = file
        .read(&mut header)
        .map_err(|source| ImportError::io("read PC container header", path, source))?;
    if count < header.len() {
        return Err(ImportError::invalid(
            path.display(),
            "truncated container header; expected at least 8 bytes",
        ));
    }

    let kind = if &header == UNITY_FS_MAGIC {
        PcContainerKind::UnityFs
    } else {
        PcContainerKind::Unrecognized
    };
    Ok(PcContainerReport {
        path: path.to_string_lossy().into_owned(),
        kind,
        codec_support: CodecSupport::Unsupported,
        detail: match kind {
            PcContainerKind::UnityFs => {
                "UnityFS container recognized; UnityFS object decoding is not implemented"
                    .to_owned()
            }
            PcContainerKind::Unrecognized => {
                "container signature is not recognized; no decoding was attempted".to_owned()
            }
        },
    })
}
pub mod managed;
pub mod managed_il;
