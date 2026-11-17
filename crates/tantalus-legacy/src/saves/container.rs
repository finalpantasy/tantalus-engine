//! Writing the original PC base-save container (`SavedData_ww.dat`).
//!
//! Source: pinned Memoria `SharedDataBytesStorage.cs`: `CreateFileIfDoesNotExist` (a new file:
//! metadata, 150 empty previews, 151 empty blocks) and `Save` (latest slot and save, timestamp,
//! then the encrypted block at the slot's offset). The slot preview is written by
//! `SaveSlotPreview`.

use super::{
    BASE_METADATA_RESERVED_BYTES, BASE_PREVIEW_RESERVED_BYTES, BASE_SAVE_BLOCK_BYTES,
    BASE_SAVE_BLOCK_OFFSET, BASE_SAVE_TOTAL_COUNT, BaseSaveBlock, BaseSaveBlockState,
    BaseSaveMetadata, BaseSavePreview, LegacyAesCodec, LegacySaveError, LegacySaveLocation,
    base_save_block_offset, base_save_preview_offset,
};
use super::{BASE_SAVE_CONTAINER_BYTES, BASE_SAVE_PREVIEW_PLAINTEXT_BYTES};

/// The metadata of a file the game has just created (`MetaData` field initializers).
pub fn new_metadata(data_size: usize) -> BaseSaveMetadata {
    BaseSaveMetadata {
        save_version: 1.0,
        data_size: i32::try_from(data_size).unwrap_or(i32::MAX),
        latest_slot: -1,
        latest_save: -1,
        latest_timestamp: -1.0,
        is_game_finish_flag: 0,
        selected_language: -1,
        is_auto_login: 0,
        system_achievement_status: 0,
        screen_rotation: 0,
    }
}

/// A new container with every preview and save block empty
/// (`SharedDataBytesStorage.CreateFileIfDoesNotExist`).
pub fn blank_container(data_size: usize) -> Vec<u8> {
    let mut container = vec![0_u8; BASE_SAVE_CONTAINER_BYTES];
    let metadata = new_metadata(data_size).encode();
    container[..metadata.len()].copy_from_slice(&metadata);
    let mut preview = b"NONE".to_vec();
    preview.resize(BASE_SAVE_PREVIEW_PLAINTEXT_BYTES, 0);
    let preview = LegacyAesCodec::encrypt(&preview);
    for index in 0..BASE_SAVE_TOTAL_COUNT {
        let offset = BASE_METADATA_RESERVED_BYTES + index * BASE_PREVIEW_RESERVED_BYTES;
        container[offset..offset + preview.len()].copy_from_slice(&preview);
    }
    let empty = BaseSaveBlock {
        offset: 0,
        state: BaseSaveBlockState::Empty,
        payload: vec![0; data_size],
    }
    .encode();
    for index in 0..=BASE_SAVE_TOTAL_COUNT {
        let offset = BASE_SAVE_BLOCK_OFFSET + index * BASE_SAVE_BLOCK_BYTES;
        container[offset..offset + empty.len()].copy_from_slice(&empty);
    }
    container
}

/// Stores one save in `container`: the block, the slot's preview (not for the autosave, which
/// has none), and the latest slot, save and timestamp in the metadata (`Save`,
/// `WriteLatestSlotAndSaveAndLatestTimestamp`). `timestamp` is seconds since 1970.
pub fn write_save(
    container: &mut [u8],
    location: &LegacySaveLocation,
    payload: Vec<u8>,
    preview: Option<&BaseSavePreview>,
    timestamp: f64,
) -> Result<(), LegacySaveError> {
    if container.len() != BASE_SAVE_CONTAINER_BYTES {
        return Err(LegacySaveError::new(
            container.len(),
            format!("container is not {BASE_SAVE_CONTAINER_BYTES} bytes"),
        ));
    }
    let mut metadata = BaseSaveMetadata::decode(container)?;
    let data_size = usize::try_from(metadata.data_size).unwrap_or(usize::MAX);
    if payload.len() != data_size {
        return Err(LegacySaveError::new(
            0,
            format!(
                "save payload is {} bytes but the file's data size is {data_size}",
                payload.len()
            ),
        ));
    }
    let offset = base_save_block_offset(location)?;
    let block = BaseSaveBlock {
        offset,
        state: BaseSaveBlockState::Saved,
        payload,
    }
    .encode();
    container[offset..offset + block.len()].copy_from_slice(&block);
    if !location.is_autosave {
        if let Some(preview) = preview {
            let encoded = preview.encode()?;
            let at = base_save_preview_offset(location.slot_id, location.save_id)?;
            container[at..at + encoded.len()].copy_from_slice(&encoded);
        }
        metadata.latest_slot = location.slot_id;
        metadata.latest_save = location.save_id;
        metadata.latest_timestamp = timestamp;
        let encoded = metadata.encode();
        container[..encoded.len()].copy_from_slice(&encoded);
    }
    Ok(())
}
