//! Original PC save-slot preview records.
//!
//! Source: pinned Memoria `SlotPreviewReadWriterUtil.cs` and `SharedDataPreviewSlot.cs`.
//! Preview records are separate from base `SAVE` blocks and live in the 1,024-byte preview
//! reservations immediately after the 320-byte metadata reservation.

use super::{BASE_SAVE_COUNT_PER_SLOT, BASE_SAVE_SLOT_COUNT, LegacyAesCodec, LegacySaveError};
use serde::{Deserialize, Serialize};

pub const BASE_SAVE_PREVIEW_REGION_OFFSET: usize = 320;
pub const BASE_SAVE_PREVIEW_PLAINTEXT_BYTES: usize = 965;
pub const BASE_SAVE_PREVIEW_RESERVED_BYTES: usize = 1024;

const PREVIEW_MAGIC: &[u8; 4] = b"PREV";
const EMPTY_MAGIC: &[u8; 4] = b"NONE";
const LOCATION_BYTES: usize = 128;
const CHARACTER_NAME_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseSavePreviewCharacter {
    pub serial_id: i32,
    pub level: i32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaseSavePreview {
    pub has_data: bool,
    pub gil: i64,
    pub play_duration: u64,
    pub win_type: u64,
    pub location: String,
    pub timestamp: f64,
    pub characters: [Option<BaseSavePreviewCharacter>; 4],
}

pub fn base_save_preview_offset(slot_id: i32, save_id: i32) -> Result<usize, LegacySaveError> {
    let slot = usize::try_from(slot_id)
        .ok()
        .filter(|slot| *slot < BASE_SAVE_SLOT_COUNT);
    let save = usize::try_from(save_id)
        .ok()
        .filter(|save| *save < BASE_SAVE_COUNT_PER_SLOT);
    match (slot, save) {
        (Some(slot), Some(save)) => {
            Ok(BASE_SAVE_PREVIEW_REGION_OFFSET
                + BASE_SAVE_PREVIEW_RESERVED_BYTES * (slot * 15 + save))
        }
        _ => Err(LegacySaveError::new(
            0,
            format!(
                "preview slot {slot_id} save {save_id} is outside {BASE_SAVE_SLOT_COUNT} slots of {BASE_SAVE_COUNT_PER_SLOT} saves"
            ),
        )),
    }
}

impl BaseSavePreview {
    /// Decrypts and decodes one preview reservation. `Ok(None)` means an empty or data-less slot.
    pub fn decode(
        container: &[u8],
        slot_id: i32,
        save_id: i32,
    ) -> Result<Option<Self>, LegacySaveError> {
        let offset = base_save_preview_offset(slot_id, save_id)?;
        let cipher_len = LegacyAesCodec::cipher_size(BASE_SAVE_PREVIEW_PLAINTEXT_BYTES);
        let cipher = container
            .get(offset..offset.saturating_add(cipher_len))
            .ok_or_else(|| {
                LegacySaveError::new(
                    container.len(),
                    format!("container ends inside the preview record at {offset}"),
                )
            })?;
        let plain = LegacyAesCodec::decrypt(cipher)
            .map_err(|error| LegacySaveError::new(offset, error.context))?;
        Self::decode_plain(&plain).map_err(|mut error| {
            error.offset += offset;
            error
        })
    }

    pub fn decode_plain(plain: &[u8]) -> Result<Option<Self>, LegacySaveError> {
        if plain.len() != BASE_SAVE_PREVIEW_PLAINTEXT_BYTES {
            return Err(LegacySaveError::new(
                plain.len(),
                format!(
                    "preview plaintext is {} bytes, expected {BASE_SAVE_PREVIEW_PLAINTEXT_BYTES}",
                    plain.len()
                ),
            ));
        }
        if &plain[..4] == EMPTY_MAGIC {
            return Ok(None);
        }
        if &plain[..4] != PREVIEW_MAGIC {
            return Err(LegacySaveError::new(
                0,
                format!(
                    "preview starts with {:02x?} instead of `PREV` or `NONE`",
                    &plain[..4]
                ),
            ));
        }
        let mut cursor = Cursor {
            bytes: plain,
            position: 4,
        };
        let has_data = cursor.boolean()?;
        let gil = cursor.i64()?;
        let play_duration = cursor.u64()?;
        let win_type = cursor.u64()?;
        let location = cursor.string(LOCATION_BYTES)?;
        let mut characters = Vec::with_capacity(4);
        for _ in 0..4 {
            let serial_id = cursor.i32()?;
            let level = cursor.i32()?;
            let name = cursor.string(CHARACTER_NAME_BYTES)?;
            characters.push(if serial_id == -1 {
                None
            } else {
                Some(BaseSavePreviewCharacter {
                    serial_id,
                    level,
                    name,
                })
            });
        }
        let timestamp = cursor.f64()?;
        if plain[cursor.position..].iter().any(|byte| *byte != 0) {
            return Err(LegacySaveError::new(
                cursor.position,
                "preview reserved tail contains nonzero bytes",
            ));
        }
        if !has_data {
            return Ok(None);
        }
        Ok(Some(Self {
            has_data,
            gil,
            play_duration,
            win_type,
            location,
            timestamp,
            characters: characters.try_into().expect("four preview characters"),
        }))
    }

    pub fn encode_plain(&self) -> Result<Vec<u8>, LegacySaveError> {
        let mut output = Vec::with_capacity(BASE_SAVE_PREVIEW_PLAINTEXT_BYTES);
        output.extend_from_slice(PREVIEW_MAGIC);
        output.push(u8::from(self.has_data));
        output.extend_from_slice(&self.gil.to_le_bytes());
        output.extend_from_slice(&self.play_duration.to_le_bytes());
        output.extend_from_slice(&self.win_type.to_le_bytes());
        encode_fixed_string(&self.location, LOCATION_BYTES, &mut output)?;
        if self.characters.len() != 4 {
            return Err(LegacySaveError::new(
                output.len(),
                "preview requires four character entries",
            ));
        }
        for character in &self.characters {
            match character {
                Some(character) => {
                    if character.serial_id == -1 {
                        return Err(LegacySaveError::new(
                            output.len(),
                            "a real preview character cannot use the reserved serial ID -1",
                        ));
                    }
                    output.extend_from_slice(&character.serial_id.to_le_bytes());
                    output.extend_from_slice(&character.level.to_le_bytes());
                    encode_fixed_string(&character.name, CHARACTER_NAME_BYTES, &mut output)?;
                }
                None => {
                    output.extend_from_slice(&(-1_i32).to_le_bytes());
                    output.extend_from_slice(&(-1_i32).to_le_bytes());
                    output.resize(output.len() + CHARACTER_NAME_BYTES, 0);
                }
            }
        }
        output.extend_from_slice(&self.timestamp.to_le_bytes());
        output.resize(BASE_SAVE_PREVIEW_PLAINTEXT_BYTES, 0);
        Ok(output)
    }

    pub fn encode(&self) -> Result<Vec<u8>, LegacySaveError> {
        Ok(LegacyAesCodec::encrypt(&self.encode_plain()?))
    }
}

impl BaseSavePreviewCharacter {
    pub fn new(serial_id: i32, level: i32, name: impl Into<String>) -> Self {
        Self {
            serial_id,
            level,
            name: name.into(),
        }
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl Cursor<'_> {
    fn boolean(&mut self) -> Result<bool, LegacySaveError> {
        Ok(self.array::<1>()?[0] != 0)
    }

    fn i32(&mut self) -> Result<i32, LegacySaveError> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    fn i64(&mut self) -> Result<i64, LegacySaveError> {
        Ok(i64::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, LegacySaveError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn f64(&mut self) -> Result<f64, LegacySaveError> {
        Ok(f64::from_le_bytes(self.array()?))
    }

    fn string(&mut self, capacity: usize) -> Result<String, LegacySaveError> {
        let start = self.position;
        let bytes = self
            .bytes
            .get(start..start.saturating_add(capacity))
            .ok_or_else(|| {
                LegacySaveError::new(
                    start,
                    format!("truncated fixed {capacity}-byte preview string"),
                )
            })?;
        self.position += capacity;
        let value = std::str::from_utf8(bytes).map_err(|error| {
            LegacySaveError::new(start, format!("invalid UTF-8 preview string: {error}"))
        })?;
        Ok(value.trim_matches('\0').to_owned())
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], LegacySaveError> {
        let start = self.position;
        let end = start
            .checked_add(N)
            .ok_or_else(|| LegacySaveError::new(start, "preview numeric value length overflow"))?;
        let bytes = self.bytes.get(start..end).ok_or_else(|| {
            LegacySaveError::new(start, format!("truncated {}-byte preview value", N))
        })?;
        self.position = end;
        Ok(bytes.try_into().expect("fixed slice length"))
    }
}

fn encode_fixed_string(
    value: &str,
    capacity: usize,
    output: &mut Vec<u8>,
) -> Result<(), LegacySaveError> {
    if value.len() > capacity {
        return Err(LegacySaveError::new(
            output.len(),
            format!(
                "UTF-8 preview string is {} bytes and exceeds the {capacity}-byte fixed field",
                value.len()
            ),
        ));
    }
    output.extend_from_slice(value.as_bytes());
    output.resize(output.len() + (capacity - value.len()), 0);
    Ok(())
}
