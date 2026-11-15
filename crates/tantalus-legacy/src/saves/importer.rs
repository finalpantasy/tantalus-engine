use super::{
    BaseSaveBlock, BaseSaveMetadata, BaseSavePreview, LegacySaveError, SimpleJsonDocument,
};
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

/// Decrypted base metadata and one decrypted `SAVE`/`NONE` block. Field mapping is not decoded.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedBaseSave {
    pub metadata: BaseSaveMetadata,
    pub block: BaseSaveBlock,
}

/// One decoded legacy slot: base metadata/block, its preview when the location has one, and the
/// optional Memoria sidecar. This is an inspection aggregate, not a native save import.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedLegacySlot {
    pub metadata: BaseSaveMetadata,
    pub block: BaseSaveBlock,
    pub preview: Option<BaseSavePreview>,
    pub extra: Option<SimpleJsonDocument>,
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

    /// Decrypts the metadata and one block of an original PC base container (read-only).
    pub fn decode_base(
        base: &[u8],
        location: &LegacySaveLocation,
    ) -> Result<DecodedBaseSave, LegacySaveError> {
        let metadata = BaseSaveMetadata::decode(base)?;
        let block = BaseSaveBlock::decode(base, &metadata, location)?;
        Ok(DecodedBaseSave { metadata, block })
    }

    /// Decrypts and decodes one slot-preview reservation (read-only).
    pub fn decode_preview(
        base: &[u8],
        slot_id: i32,
        save_id: i32,
    ) -> Result<Option<BaseSavePreview>, LegacySaveError> {
        BaseSavePreview::decode(base, slot_id, save_id)
    }

    /// Decodes all available synthetic-verifiable layers for one legacy slot (read-only).
    /// Autosave has no preview reservation, so its preview is `None`.
    pub fn decode_slot(
        base: &[u8],
        extra: Option<&[u8]>,
        location: &LegacySaveLocation,
    ) -> Result<DecodedLegacySlot, LegacySaveError> {
        let metadata = BaseSaveMetadata::decode(base)?;
        let block = BaseSaveBlock::decode(base, &metadata, location)?;
        let preview = if location.is_autosave {
            None
        } else {
            BaseSavePreview::decode(base, location.slot_id, location.save_id)?
        };
        Ok(DecodedLegacySlot {
            metadata,
            block,
            preview,
            extra: extra.map(SimpleJsonDocument::decode).transpose()?,
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
