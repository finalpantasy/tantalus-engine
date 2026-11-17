mod base;
mod container;
pub mod ff9_tree;
mod importer;
mod preview;
mod schema;
mod simple_json;
mod slot_file;
pub mod state;

pub use base::{
    BASE_METADATA_PLAINTEXT_BYTES, BASE_METADATA_RESERVED_BYTES, BASE_PREVIEW_RESERVED_BYTES,
    BASE_SAVE_BLOCK_BYTES, BASE_SAVE_BLOCK_OFFSET, BASE_SAVE_COUNT_PER_SLOT, BASE_SAVE_SLOT_COUNT,
    BASE_SAVE_TOTAL_COUNT, BaseSaveBlock, BaseSaveBlockState, BaseSaveMetadata, LegacyAesCodec,
    base_save_block_offset,
};
pub use container::{blank_container, new_metadata, write_save};
pub use importer::{
    BASE_SAVE_CONTAINER_BYTES, BaseSaveObservation, DecodedBaseSave, DecodedLegacySlot,
    LegacySaveImporter, LegacySaveLocation, LegacySaveReport, PairingStatus,
};
pub use preview::{
    BASE_SAVE_PREVIEW_PLAINTEXT_BYTES, BASE_SAVE_PREVIEW_REGION_OFFSET,
    BASE_SAVE_PREVIEW_RESERVED_BYTES, BaseSavePreview, BaseSavePreviewCharacter,
    base_save_preview_offset,
};
pub use schema::{
    BaseSaveDataNode, BaseSaveFieldType, BaseSavePayload, BaseSaveSchemaNode, BaseSaveValue,
};
pub use simple_json::{LegacySaveError, SimpleJsonDocument, SimpleJsonNode};
pub use slot_file::LegacySlotFileReport;
