//! On-disk, read-only legacy slot inspection. Mirrors Memoria's
//! `SharedDataBytesStorage.MetaData.GetMemoriaExtraSaveFilePath` naming so a
//! user-supplied base container can be inspected together with its optional
//! Memoria sidecar without writing anything.

use super::{
    BASE_SAVE_CONTAINER_BYTES, BaseSaveObservation, DecodedLegacySlot, LegacySaveError,
    LegacySaveImporter, LegacySaveLocation, PairingStatus,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// One legacy slot inspected straight from on-disk files (read-only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LegacySlotFileReport {
    pub base_path: PathBuf,
    pub base: BaseSaveObservation,
    pub sidecar_path: Option<PathBuf>,
    pub sidecar_found: bool,
    pub pairing: PairingStatus,
    pub slot: DecodedLegacySlot,
}

impl LegacySaveImporter {
    /// Memoria's sidecar naming for one slot: the base name without `.dat`
    /// plus `_Memoria_Autosave` or `_Memoria_<slot>_<save>`, then `.dat`.
    /// Returns `None` when the base path does not end in `.dat`, matching the
    /// reference's `String.Empty` result.
    pub fn memoria_sidecar_path(
        base_path: &Path,
        location: &LegacySaveLocation,
    ) -> Option<PathBuf> {
        let name = base_path.file_name()?.to_str()?;
        let stem = name.strip_suffix(".dat")?;
        let suffix = if location.is_autosave {
            "_Memoria_Autosave".to_owned()
        } else {
            format!("_Memoria_{}_{}", location.slot_id, location.save_id)
        };
        Some(base_path.with_file_name(format!("{stem}{suffix}.dat")))
    }

    /// Reads the base container and its Memoria sidecar (when present) from
    /// disk and decodes the requested slot. Never writes. A missing base file
    /// is an error naming the path; a missing sidecar yields
    /// `sidecar_found == false` and `PairingStatus::NotChecked`.
    pub fn inspect_slot_file(
        base_path: &Path,
        location: &LegacySaveLocation,
    ) -> Result<LegacySlotFileReport, LegacySaveError> {
        let base = fs::read(base_path).map_err(|error| {
            LegacySaveError::new(
                0,
                format!("cannot read base save {}: {error}", base_path.display()),
            )
        })?;
        let (sidecar_path, extra) = match Self::memoria_sidecar_path(base_path, location) {
            Some(path) if path.is_file() => {
                let bytes = fs::read(&path).map_err(|error| {
                    LegacySaveError::new(
                        0,
                        format!("cannot read Memoria sidecar {}: {error}", path.display()),
                    )
                })?;
                (Some(path), Some(bytes))
            }
            _ => (None, None),
        };
        let pairing = match &sidecar_path {
            Some(path) => Self::validate_pairing(base_path, path, location),
            None => PairingStatus::NotChecked,
        };
        let observation = BaseSaveObservation {
            bytes: base.len(),
            sha256: format!("{:x}", Sha256::digest(&base)),
            fixed_container_size_matches: base.len() == BASE_SAVE_CONTAINER_BYTES,
            decoded: true,
        };
        let slot = Self::decode_slot(&base, extra.as_deref(), location)?;
        Ok(LegacySlotFileReport {
            base_path: base_path.to_path_buf(),
            base: observation,
            sidecar_found: sidecar_path.is_some(),
            sidecar_path,
            pairing,
            slot,
        })
    }
}
