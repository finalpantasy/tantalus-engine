//! Original PC base-save container: AES-256-CBC blocks, metadata, and save-block framing.
//!
//! Source: pinned Memoria `AESCryptography.cs`, `SharedDataAesEncryption.cs`, and
//! `SharedDataBytesStorage.cs` (`Load`, `MetaData`). This module stops at the decrypted
//! `SAVE`/`NONE` block; the schema-driven field mapping inside a `SAVE` payload is a
//! separate, not yet implemented step.

use super::{LegacySaveError, LegacySaveLocation};
use aes::Aes256;
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use pbkdf2::pbkdf2_hmac;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use std::sync::OnceLock;

pub const BASE_SAVE_SLOT_COUNT: usize = 10;
pub const BASE_SAVE_COUNT_PER_SLOT: usize = 15;
pub const BASE_SAVE_TOTAL_COUNT: usize = BASE_SAVE_SLOT_COUNT * BASE_SAVE_COUNT_PER_SLOT;
pub const BASE_METADATA_RESERVED_BYTES: usize = 320;
pub const BASE_METADATA_PLAINTEXT_BYTES: usize = 288;
pub const BASE_PREVIEW_RESERVED_BYTES: usize = 1024;
pub const BASE_SAVE_BLOCK_BYTES: usize = 18_432;
pub const BASE_SAVE_BLOCK_OFFSET: usize =
    BASE_METADATA_RESERVED_BYTES + BASE_SAVE_TOTAL_COUNT * BASE_PREVIEW_RESERVED_BYTES;

const AES_BLOCK_BYTES: usize = 16;
const PBKDF2_ROUNDS: u32 = 1_000;
const PBKDF2_PASSWORD: &[u8] =
    b"67434cd0-1ca3-11e5-9a21-1697f925ec7b7a5313a0-1ca3-11e5-b939-0800200c9a66";
const PBKDF2_SALT: [u8; 8] = [3, 3, 1, 4, 7, 0, 9, 7];
const METADATA_FIXED_BYTES: usize = 39;

type Aes256CbcDec = cbc::Decryptor<Aes256>;
type Aes256CbcEnc = cbc::Encryptor<Aes256>;

/// The key/IV format of the original container. This is file-format compatibility with the
/// fixed constants in the pinned reference, not protection for user data.
pub struct LegacyAesCodec;

impl LegacyAesCodec {
    /// Matches `SharedDataAesEncryption.GetCipherSize`: PKCS7 always adds 1 to 16 bytes.
    pub fn cipher_size(plain_len: usize) -> usize {
        plain_len + AES_BLOCK_BYTES - plain_len % AES_BLOCK_BYTES
    }

    pub fn encrypt(plain: &[u8]) -> Vec<u8> {
        let (key, iv) = key_and_iv();
        let mut buffer = vec![0_u8; Self::cipher_size(plain.len())];
        buffer[..plain.len()].copy_from_slice(plain);
        let length = Aes256CbcEnc::new(key.into(), iv.into())
            .encrypt_padded_mut::<Pkcs7>(&mut buffer, plain.len())
            .expect("buffer is sized for PKCS7 padding")
            .len();
        buffer.truncate(length);
        buffer
    }

    pub fn decrypt(cipher: &[u8]) -> Result<Vec<u8>, LegacySaveError> {
        if cipher.is_empty() || !cipher.len().is_multiple_of(AES_BLOCK_BYTES) {
            return Err(LegacySaveError::new(
                0,
                format!(
                    "ciphertext length {} is not a positive multiple of 16",
                    cipher.len()
                ),
            ));
        }
        let (key, iv) = key_and_iv();
        let mut buffer = cipher.to_vec();
        let length = Aes256CbcDec::new(key.into(), iv.into())
            .decrypt_padded_mut::<Pkcs7>(&mut buffer)
            .map_err(|_| LegacySaveError::new(0, "invalid PKCS7 padding after AES decryption"))?
            .len();
        buffer.truncate(length);
        Ok(buffer)
    }
}

fn key_and_iv() -> (&'static [u8; 32], &'static [u8; 16]) {
    static MATERIAL: OnceLock<([u8; 32], [u8; 16])> = OnceLock::new();
    let material = MATERIAL.get_or_init(|| {
        // `Rfc2898DeriveBytes.GetBytes(32)` then `GetBytes(16)` continues one PBKDF2-HMAC-SHA1 stream.
        let mut stream = [0_u8; 48];
        pbkdf2_hmac::<Sha1>(PBKDF2_PASSWORD, &PBKDF2_SALT, PBKDF2_ROUNDS, &mut stream);
        let mut key = [0_u8; 32];
        let mut iv = [0_u8; 16];
        key.copy_from_slice(&stream[..32]);
        iv.copy_from_slice(&stream[32..]);
        (key, iv)
    });
    (&material.0, &material.1)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaseSaveMetadata {
    pub save_version: f32,
    pub data_size: i32,
    pub latest_slot: i32,
    pub latest_save: i32,
    pub latest_timestamp: f64,
    pub is_game_finish_flag: i32,
    pub selected_language: i32,
    pub is_auto_login: i8,
    pub system_achievement_status: u8,
    pub screen_rotation: u8,
}

impl BaseSaveMetadata {
    /// Reads the encrypted metadata at the start of the container (`MetaData.Read`).
    pub fn decode(container: &[u8]) -> Result<Self, LegacySaveError> {
        let cipher_len = LegacyAesCodec::cipher_size(BASE_METADATA_PLAINTEXT_BYTES);
        let cipher = container.get(..cipher_len).ok_or_else(|| {
            LegacySaveError::new(
                container.len(),
                format!("container ends before the {cipher_len}-byte encrypted metadata"),
            )
        })?;
        let plain = LegacyAesCodec::decrypt(cipher)
            .map_err(|error| LegacySaveError::new(0, format!("metadata: {}", error.context)))?;
        if plain.len() < METADATA_FIXED_BYTES {
            return Err(LegacySaveError::new(
                plain.len(),
                format!("decrypted metadata is shorter than {METADATA_FIXED_BYTES} bytes"),
            ));
        }
        if &plain[..4] != b"SAVE" {
            return Err(LegacySaveError::new(
                0,
                format!(
                    "metadata starts with {:02x?} instead of `SAVE`",
                    &plain[..4]
                ),
            ));
        }
        let metadata = Self {
            save_version: f32::from_le_bytes(plain[4..8].try_into().unwrap()),
            data_size: i32::from_le_bytes(plain[8..12].try_into().unwrap()),
            latest_slot: i32::from_le_bytes(plain[12..16].try_into().unwrap()),
            latest_save: i32::from_le_bytes(plain[16..20].try_into().unwrap()),
            latest_timestamp: f64::from_le_bytes(plain[20..28].try_into().unwrap()),
            is_game_finish_flag: i32::from_le_bytes(plain[28..32].try_into().unwrap()),
            selected_language: i32::from_le_bytes(plain[32..36].try_into().unwrap()),
            is_auto_login: plain[36] as i8,
            system_achievement_status: plain[37],
            screen_rotation: plain[38],
        };
        // Memoria would read past one block for these; Tantalus refuses instead of guessing.
        let Ok(data_size) = usize::try_from(metadata.data_size) else {
            return Err(LegacySaveError::new(
                8,
                format!("negative data size {}", metadata.data_size),
            ));
        };
        if LegacyAesCodec::cipher_size(data_size + 4) > BASE_SAVE_BLOCK_BYTES {
            return Err(LegacySaveError::new(
                8,
                format!(
                    "data size {data_size} does not fit one {BASE_SAVE_BLOCK_BYTES}-byte block"
                ),
            ));
        }
        Ok(metadata)
    }

    /// Plaintext metadata as written by `MetaData.WriteAllFields` (288 bytes before encryption).
    pub fn encode_plain(&self) -> Vec<u8> {
        let mut plain = Vec::with_capacity(BASE_METADATA_PLAINTEXT_BYTES);
        plain.extend_from_slice(b"SAVE");
        plain.extend_from_slice(&self.save_version.to_le_bytes());
        plain.extend_from_slice(&self.data_size.to_le_bytes());
        plain.extend_from_slice(&self.latest_slot.to_le_bytes());
        plain.extend_from_slice(&self.latest_save.to_le_bytes());
        plain.extend_from_slice(&self.latest_timestamp.to_le_bytes());
        plain.extend_from_slice(&self.is_game_finish_flag.to_le_bytes());
        plain.extend_from_slice(&self.selected_language.to_le_bytes());
        plain.push(self.is_auto_login as u8);
        plain.push(self.system_achievement_status);
        plain.push(self.screen_rotation);
        plain.resize(BASE_METADATA_PLAINTEXT_BYTES, 0);
        plain
    }

    pub fn encode(&self) -> Vec<u8> {
        LegacyAesCodec::encrypt(&self.encode_plain())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BaseSaveBlockState {
    /// Block begins with `SAVE`; the payload is the schema-driven state, not yet mapped.
    Saved,
    /// Block begins with `NONE`: an empty slot.
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseSaveBlock {
    pub offset: usize,
    pub state: BaseSaveBlockState,
    /// Decrypted bytes after the four-byte magic.
    pub payload: Vec<u8>,
}

/// Byte offset of a save block (`SharedDataBytesStorage.Load`): autosave is the first block.
pub fn base_save_block_offset(location: &LegacySaveLocation) -> Result<usize, LegacySaveError> {
    if location.is_autosave {
        return Ok(BASE_SAVE_BLOCK_OFFSET);
    }
    let slot = usize::try_from(location.slot_id)
        .ok()
        .filter(|slot| *slot < BASE_SAVE_SLOT_COUNT);
    let save = usize::try_from(location.save_id)
        .ok()
        .filter(|save| *save < BASE_SAVE_COUNT_PER_SLOT);
    match (slot, save) {
        (Some(slot), Some(save)) => Ok(BASE_SAVE_BLOCK_OFFSET
            + BASE_SAVE_BLOCK_BYTES * (1 + slot * BASE_SAVE_COUNT_PER_SLOT + save)),
        _ => Err(LegacySaveError::new(
            0,
            format!(
                "slot {} save {} is outside {BASE_SAVE_SLOT_COUNT} slots of {BASE_SAVE_COUNT_PER_SLOT} saves",
                location.slot_id, location.save_id
            ),
        )),
    }
}

impl BaseSaveBlock {
    pub fn decode(
        container: &[u8],
        metadata: &BaseSaveMetadata,
        location: &LegacySaveLocation,
    ) -> Result<Self, LegacySaveError> {
        let offset = base_save_block_offset(location)?;
        let plain_len = usize::try_from(metadata.data_size).unwrap_or(usize::MAX) + 4;
        let cipher_len = LegacyAesCodec::cipher_size(plain_len);
        let cipher = offset
            .checked_add(cipher_len)
            .and_then(|end| container.get(offset..end))
            .ok_or_else(|| {
                LegacySaveError::new(
                    container.len(),
                    format!("container ends inside the save block at {offset}"),
                )
            })?;
        let plain = LegacyAesCodec::decrypt(cipher)
            .map_err(|error| LegacySaveError::new(offset, error.context))?;
        if plain.len() != plain_len {
            return Err(LegacySaveError::new(
                offset,
                format!(
                    "decrypted block is {} bytes but metadata data size implies {plain_len}",
                    plain.len()
                ),
            ));
        }
        let state = match &plain[..4] {
            b"SAVE" => BaseSaveBlockState::Saved,
            b"NONE" => BaseSaveBlockState::Empty,
            magic => {
                return Err(LegacySaveError::new(
                    offset,
                    format!("block starts with {magic:02x?} instead of `SAVE` or `NONE`"),
                ));
            }
        };
        Ok(Self {
            offset,
            state,
            payload: plain[4..].to_vec(),
        })
    }

    /// Plaintext block as `Load` expects it: magic followed by `data_size` bytes.
    pub fn encode(&self) -> Vec<u8> {
        let magic: &[u8; 4] = match self.state {
            BaseSaveBlockState::Saved => b"SAVE",
            BaseSaveBlockState::Empty => b"NONE",
        };
        let mut plain = magic.to_vec();
        plain.extend_from_slice(&self.payload);
        LegacyAesCodec::encrypt(&plain)
    }
}
