use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tantalus_contracts::ContentId;

pub const PACK_SCHEMA_VERSION: u32 = 1;
pub const CONVERTER_VERSION: &str = "tantalus-assets/0.1.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Binary,
    Text,
    Texture,
    Model,
    Animation,
    Field,
    Event,
    Audio,
    Video,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetCodec {
    Raw,
    Utf8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetRequest {
    pub id: ContentId,
    pub source_path: String,
    pub kind: AssetKind,
    pub codec: AssetCodec,
    pub dependencies: Vec<ContentId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportLimits {
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    pub max_assets: usize,
}

impl Default for ImportLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 64 * 1024 * 1024,
            max_total_bytes: 512 * 1024 * 1024,
            max_assets: 4096,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportPlan {
    pub target_schema: u32,
    pub source_id: String,
    pub source_root: PathBuf,
    pub cache_root: PathBuf,
    pub requests: Vec<AssetRequest>,
    pub limits: ImportLimits,
}

impl ImportPlan {
    pub fn new(requests: Vec<AssetRequest>) -> Self {
        Self {
            target_schema: PACK_SCHEMA_VERSION,
            source_id: "synthetic".to_owned(),
            source_root: PathBuf::new(),
            cache_root: PathBuf::new(),
            requests,
            limits: ImportLimits::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackAsset {
    pub id: ContentId,
    pub kind: AssetKind,
    pub codec: AssetCodec,
    pub source_path: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub object: String,
    pub dependencies: Vec<ContentId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackManifest {
    pub schema_version: u32,
    pub converter_version: String,
    pub source_id: String,
    pub pack_id: String,
    pub assets: Vec<PackAsset>,
}
