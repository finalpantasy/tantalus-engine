//! Original PC base-save container tests. All fixtures are project-owned synthetic data.

use tantalus_legacy::*;

fn synthetic_metadata(data_size: i32) -> BaseSaveMetadata {
    BaseSaveMetadata {
        save_version: 1.0,
        data_size,
        latest_slot: 2,
        latest_save: 7,
        latest_timestamp: 1234.5,
        is_game_finish_flag: 0,
        selected_language: 1,
        is_auto_login: -1,
        system_achievement_status: 0x21,
        screen_rotation: 3,
    }
}

fn location(is_autosave: bool, slot_id: i32, save_id: i32) -> LegacySaveLocation {
    LegacySaveLocation {
        is_autosave,
        slot_id,
        save_id,
    }
}

/// Metadata at offset 0 plus one block at its source-defined offset; everything else zero.
fn synthetic_container(
    metadata: &BaseSaveMetadata,
    location: &LegacySaveLocation,
    block: &BaseSaveBlock,
) -> Vec<u8> {
    let mut container = vec![0_u8; BASE_SAVE_CONTAINER_BYTES];
    let encrypted = metadata.encode();
    container[..encrypted.len()].copy_from_slice(&encrypted);
    let offset = base_save_block_offset(location).unwrap();
    let encrypted = block.encode();
    container[offset..offset + encrypted.len()].copy_from_slice(&encrypted);
    container
}

fn block(state: BaseSaveBlockState, payload: Vec<u8>) -> BaseSaveBlock {
    BaseSaveBlock {
        offset: 0,
        state,
        payload,
    }
}

#[test]
fn cipher_sizes_always_add_padding_like_the_reference() {
    assert_eq!(LegacyAesCodec::cipher_size(0), 16);
    assert_eq!(LegacyAesCodec::cipher_size(15), 16);
    assert_eq!(LegacyAesCodec::cipher_size(16), 32);
    assert_eq!(LegacyAesCodec::cipher_size(288), 304);
    for len in [0, 1, 15, 16, 17, 288, 965] {
        let plain: Vec<u8> = (0..len).map(|index| index as u8).collect();
        let cipher = LegacyAesCodec::encrypt(&plain);
        assert_eq!(cipher.len(), LegacyAesCodec::cipher_size(len));
        assert_eq!(LegacyAesCodec::decrypt(&cipher).unwrap(), plain);
    }
}

#[test]
fn decrypt_rejects_malformed_ciphertext_without_panicking() {
    assert!(LegacyAesCodec::decrypt(&[]).is_err());
    assert!(LegacyAesCodec::decrypt(&[0; 15]).is_err());
    assert!(LegacyAesCodec::decrypt(&[0; 32]).is_err());
}

#[test]
fn container_layout_matches_the_source_constants() {
    assert_eq!(BASE_SAVE_BLOCK_OFFSET, 153_920);
    assert_eq!(
        BASE_SAVE_BLOCK_OFFSET + BASE_SAVE_BLOCK_BYTES * (1 + BASE_SAVE_TOTAL_COUNT),
        BASE_SAVE_CONTAINER_BYTES
    );
    assert_eq!(
        base_save_block_offset(&location(true, 9, 14)).unwrap(),
        BASE_SAVE_BLOCK_OFFSET
    );
    assert_eq!(
        base_save_block_offset(&location(false, 0, 0)).unwrap(),
        153_920 + 18_432
    );
    assert_eq!(
        base_save_block_offset(&location(false, 9, 14)).unwrap(),
        BASE_SAVE_CONTAINER_BYTES - 18_432
    );
    assert!(base_save_block_offset(&location(false, 10, 0)).is_err());
    assert!(base_save_block_offset(&location(false, 0, 15)).is_err());
    assert!(base_save_block_offset(&location(false, -1, 0)).is_err());
}

#[test]
fn metadata_plaintext_is_the_reference_288_bytes() {
    let plain = synthetic_metadata(64).encode_plain();
    assert_eq!(plain.len(), BASE_METADATA_PLAINTEXT_BYTES);
    assert_eq!(&plain[..4], b"SAVE");
    assert_eq!(plain[36], 0xFF); // is_auto_login: -1
    assert_eq!(plain[37], 0x21);
    assert_eq!(plain[38], 3);
    assert!(plain[39..].iter().all(|byte| *byte == 0));
}

#[test]
fn synthetic_container_decodes_metadata_and_saved_block() {
    let metadata = synthetic_metadata(64);
    let at = location(false, 2, 7);
    let saved = block(BaseSaveBlockState::Saved, (0..64).collect());
    let container = synthetic_container(&metadata, &at, &saved);

    let decoded = LegacySaveImporter::decode_base(&container, &at).unwrap();
    assert_eq!(decoded.metadata, metadata);
    assert_eq!(decoded.block.state, BaseSaveBlockState::Saved);
    assert_eq!(decoded.block.payload, saved.payload);
    assert_eq!(decoded.block.offset, base_save_block_offset(&at).unwrap());

    // An unwritten slot is all zero bytes, which is not valid ciphertext padding.
    assert!(LegacySaveImporter::decode_base(&container, &location(false, 0, 0)).is_err());
}

#[test]
fn none_block_is_an_empty_slot_and_autosave_is_the_first_block() {
    let at = location(true, 0, 0);
    let empty = block(BaseSaveBlockState::Empty, vec![0; 8]);
    let container = synthetic_container(&synthetic_metadata(8), &at, &empty);
    let decoded = LegacySaveImporter::decode_base(&container, &at).unwrap();
    assert_eq!(decoded.block.state, BaseSaveBlockState::Empty);
    assert_eq!(decoded.block.offset, BASE_SAVE_BLOCK_OFFSET);
}

#[test]
fn corrupt_data_reports_errors_instead_of_guessing() {
    let at = location(true, 0, 0);
    let saved = block(BaseSaveBlockState::Saved, vec![1; 16]);

    // data_size disagrees with the block that was actually written.
    let container = synthetic_container(&synthetic_metadata(32), &at, &saved);
    assert!(LegacySaveImporter::decode_base(&container, &at).is_err());

    // Negative and oversized data sizes are refused before any block read.
    for size in [-1, 18_428, i32::MAX] {
        let container = synthetic_container(&synthetic_metadata(size), &at, &saved);
        let error = LegacySaveImporter::decode_base(&container, &at).unwrap_err();
        assert_eq!(error.offset, 8, "data size {size}");
    }

    // A validly encrypted block with the wrong magic.
    let mut container = synthetic_container(&synthetic_metadata(16), &at, &saved);
    let encrypted = LegacyAesCodec::encrypt(b"JUNK\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0");
    container[BASE_SAVE_BLOCK_OFFSET..BASE_SAVE_BLOCK_OFFSET + encrypted.len()]
        .copy_from_slice(&encrypted);
    let error = LegacySaveImporter::decode_base(&container, &at).unwrap_err();
    assert_eq!(error.offset, BASE_SAVE_BLOCK_OFFSET);
    assert!(error.context.contains("SAVE"));

    // Bad metadata magic and truncated metadata.
    let encrypted = LegacyAesCodec::encrypt(&[b'X'; 288]);
    let mut container = vec![0_u8; BASE_SAVE_CONTAINER_BYTES];
    container[..encrypted.len()].copy_from_slice(&encrypted);
    assert!(BaseSaveMetadata::decode(&container).is_err());
    assert!(BaseSaveMetadata::decode(&container[..100]).is_err());

    // Flipped ciphertext bytes and a container that ends inside the block.
    let mut container = synthetic_container(&synthetic_metadata(16), &at, &saved);
    container[BASE_SAVE_BLOCK_OFFSET + 16] ^= 0xFF;
    container[BASE_SAVE_BLOCK_OFFSET + 31] ^= 0xFF;
    assert!(LegacySaveImporter::decode_base(&container, &at).is_err());
    let container = synthetic_container(&synthetic_metadata(16), &at, &saved);
    assert!(
        LegacySaveImporter::decode_base(&container[..BASE_SAVE_BLOCK_OFFSET + 8], &at).is_err()
    );
}

#[test]
fn key_derivation_matches_an_independent_known_answer() {
    // Key/IV from Python hashlib.pbkdf2_hmac('sha1', password, salt, 1000, 48); ciphertext from
    // `openssl enc -aes-256-cbc` with that key/IV. Neither uses this crate.
    let plain = b"SAVE tantalus known answer";
    let expected = "80392231c8995753ca174f449fbcd06c2c03cf9a93e6f43edc05d242db88d3ca";
    let cipher = LegacyAesCodec::encrypt(plain);
    let hex: String = cipher.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(hex, expected);
    assert_eq!(LegacyAesCodec::decrypt(&cipher).unwrap(), plain);
}
