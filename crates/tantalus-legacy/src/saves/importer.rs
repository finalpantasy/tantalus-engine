use super::{LegacySaveError, SimpleJsonDocument};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

pub const BASE_SAVE_CONTAINER_BYTES: usize = 2_937_152;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseSaveObservation {
    pub bytes: usize,
    pub sha256: String,
    pub fixed_container_size_matches: bool,
    pub decoded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingStatus {
    NotChecked,
    Matched,
    Mismatched,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacySaveLocation {
    pub is_autosave: bool,
    pub slot_id: i32,
    pub save_id: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacySaveReport {
    pub base: BaseSaveObservation,
    pub extra: Option<SimpleJsonDocument>,
    pub pairing: PairingStatus,
}

pub struct LegacySaveImporter;

impl LegacySaveImporter {
    pub fn inspect(base: &[u8], extra: Option<&[u8]>) -> Result<LegacySaveReport, LegacySaveError> {
        Ok(LegacySaveReport {
            base: BaseSaveObservation {
                bytes: base.len(),
                sha256: format!("{:x}", Sha256::digest(base)),
                fixed_container_size_matches: base.len() == BASE_SAVE_CONTAINER_BYTES,
                decoded: false,
            },
            extra: extra.map(SimpleJsonDocument::decode).transpose()?,
            pairing: PairingStatus::NotChecked,
        })
    }

    pub fn validate_pairing(
        base_path: &Path,
        extra_path: &Path,
        location: &LegacySaveLocation,
    ) -> PairingStatus {
        let Some(base_name) = base_path.file_name().and_then(|name| name.to_str()) else {
            return PairingStatus::Mismatched;
        };
        if !base_name.ends_with(".dat") || location.slot_id < 0 || location.save_id < 0 {
            return PairingStatus::Mismatched;
        }
        let suffix = if location.is_autosave {
            "_Memoria_Autosave".to_owned()
        } else {
            format!("_Memoria_{}_{}", location.slot_id, location.save_id)
        };
        let expected = format!("{}{}.dat", &base_name[..base_name.len() - 4], suffix);
        if extra_path.file_name().and_then(|name| name.to_str()) == Some(expected.as_str()) {
            PairingStatus::Matched
        } else {
            PairingStatus::Mismatched
        }
    }
}
