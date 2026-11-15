//! Ordered schema decoding for an original PC `SAVE` payload.
//!
//! Source: pinned Memoria `SharedDataBytesStorage.ReadDataFromStream` and
//! `WriteDataToStream`. This is the primitive value layer only; constructing the complete
//! FF9 state schema remains separate work.

use super::LegacySaveError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BaseSaveFieldType {
    Byte,
    SByte,
    Int16,
    UInt16,
    Int32,
    UInt32,
    Int64,
    UInt64,
    Single,
    Double,
    Boolean,
    String128,
    String4K,
}

impl BaseSaveFieldType {
    pub fn from_legacy_type_name(name: &str) -> Option<Self> {
        match name {
            "System.Byte" => Some(Self::Byte),
            "System.SByte" => Some(Self::SByte),
            "System.Int16" => Some(Self::Int16),
            "System.UInt16" => Some(Self::UInt16),
            "System.Int32" => Some(Self::Int32),
            "System.UInt32" => Some(Self::UInt32),
            "System.Int64" => Some(Self::Int64),
            "System.UInt64" => Some(Self::UInt64),
            "System.Single" => Some(Self::Single),
            "System.Double" => Some(Self::Double),
            "System.Boolean" => Some(Self::Boolean),
            "System.String" => Some(Self::String128),
            "String4K" => Some(Self::String4K),
            _ => None,
        }
    }

    pub fn size(self) -> usize {
        match self {
            Self::Byte | Self::SByte | Self::Boolean => 1,
            Self::Int16 | Self::UInt16 => 2,
            Self::Int32 | Self::UInt32 | Self::Single => 4,
            Self::Int64 | Self::UInt64 | Self::Double => 8,
            Self::String128 => 128,
            Self::String4K => 4096,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BaseSaveValue {
    U8(u8),
    I8(i8),
    I16(i16),
    U16(u16),
    I32(i32),
    U32(u32),
    I64(i64),
    U64(u64),
    F32(f32),
    F64(f64),
    Bool(bool),
    String(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaseSavePayload {
    pub values: Vec<BaseSaveValue>,
}

impl BaseSavePayload {
    pub fn decode(payload: &[u8], schema: &[BaseSaveFieldType]) -> Result<Self, LegacySaveError> {
        let mut cursor = Cursor {
            bytes: payload,
            position: 0,
        };
        let mut values = Vec::with_capacity(schema.len());
        for field_type in schema {
            values.push(cursor.value(*field_type)?);
        }
        if cursor.position != payload.len() {
            return Err(LegacySaveError::new(
                cursor.position,
                format!(
                    "schema consumed {} of {} payload bytes",
                    cursor.position,
                    payload.len()
                ),
            ));
        }
        Ok(Self { values })
    }

    pub fn encode(&self, schema: &[BaseSaveFieldType]) -> Result<Vec<u8>, LegacySaveError> {
        if self.values.len() != schema.len() {
            return Err(LegacySaveError::new(
                0,
                format!(
                    "payload has {} values for a {}-field schema",
                    self.values.len(),
                    schema.len()
                ),
            ));
        }
        let size = schema.iter().map(|field| field.size()).sum();
        let mut output = Vec::with_capacity(size);
        for (index, (value, field_type)) in self.values.iter().zip(schema).enumerate() {
            encode_value(value, *field_type, &mut output)
                .map_err(|context| LegacySaveError::new(index, context))?;
        }
        Ok(output)
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl Cursor<'_> {
    fn value(&mut self, field_type: BaseSaveFieldType) -> Result<BaseSaveValue, LegacySaveError> {
        match field_type {
            BaseSaveFieldType::Byte => Ok(BaseSaveValue::U8(self.array::<1>()?[0])),
            BaseSaveFieldType::SByte => Ok(BaseSaveValue::I8(self.array::<1>()?[0] as i8)),
            BaseSaveFieldType::Int16 => Ok(BaseSaveValue::I16(i16::from_le_bytes(self.array()?))),
            BaseSaveFieldType::UInt16 => Ok(BaseSaveValue::U16(u16::from_le_bytes(self.array()?))),
            BaseSaveFieldType::Int32 => Ok(BaseSaveValue::I32(i32::from_le_bytes(self.array()?))),
            BaseSaveFieldType::UInt32 => Ok(BaseSaveValue::U32(u32::from_le_bytes(self.array()?))),
            BaseSaveFieldType::Int64 => Ok(BaseSaveValue::I64(i64::from_le_bytes(self.array()?))),
            BaseSaveFieldType::UInt64 => Ok(BaseSaveValue::U64(u64::from_le_bytes(self.array()?))),
            BaseSaveFieldType::Single => Ok(BaseSaveValue::F32(f32::from_le_bytes(self.array()?))),
            BaseSaveFieldType::Double => Ok(BaseSaveValue::F64(f64::from_le_bytes(self.array()?))),
            BaseSaveFieldType::Boolean => Ok(BaseSaveValue::Bool(self.array::<1>()?[0] != 0)),
            BaseSaveFieldType::String128 => self.string(128),
            BaseSaveFieldType::String4K => self.string(4096),
        }
    }

    fn string(&mut self, capacity: usize) -> Result<BaseSaveValue, LegacySaveError> {
        let start = self.position;
        let bytes = self
            .bytes
            .get(start..start.saturating_add(capacity))
            .ok_or_else(|| {
                LegacySaveError::new(start, format!("truncated fixed {capacity}-byte string"))
            })?;
        self.position += capacity;
        let value = std::str::from_utf8(bytes).map_err(|error| {
            LegacySaveError::new(start, format!("invalid UTF-8 fixed string: {error}"))
        })?;
        Ok(BaseSaveValue::String(value.trim_matches('\0').to_owned()))
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], LegacySaveError> {
        let start = self.position;
        let end = start
            .checked_add(N)
            .ok_or_else(|| LegacySaveError::new(start, "numeric value length overflow"))?;
        let bytes = self
            .bytes
            .get(start..end)
            .ok_or_else(|| LegacySaveError::new(start, format!("truncated {}-byte value", N)))?;
        self.position = end;
        Ok(bytes.try_into().expect("fixed slice length"))
    }
}

fn encode_value(
    value: &BaseSaveValue,
    field_type: BaseSaveFieldType,
    output: &mut Vec<u8>,
) -> Result<(), String> {
    match (value, field_type) {
        (BaseSaveValue::U8(value), BaseSaveFieldType::Byte) => output.push(*value),
        (BaseSaveValue::I8(value), BaseSaveFieldType::SByte) => output.push(*value as u8),
        (BaseSaveValue::I16(value), BaseSaveFieldType::Int16) => output.extend(value.to_le_bytes()),
        (BaseSaveValue::U16(value), BaseSaveFieldType::UInt16) => {
            output.extend(value.to_le_bytes())
        }
        (BaseSaveValue::I32(value), BaseSaveFieldType::Int32) => output.extend(value.to_le_bytes()),
        (BaseSaveValue::U32(value), BaseSaveFieldType::UInt32) => {
            output.extend(value.to_le_bytes())
        }
        (BaseSaveValue::I64(value), BaseSaveFieldType::Int64) => output.extend(value.to_le_bytes()),
        (BaseSaveValue::U64(value), BaseSaveFieldType::UInt64) => {
            output.extend(value.to_le_bytes())
        }
        (BaseSaveValue::F32(value), BaseSaveFieldType::Single) => {
            output.extend(value.to_le_bytes())
        }
        (BaseSaveValue::F64(value), BaseSaveFieldType::Double) => {
            output.extend(value.to_le_bytes())
        }
        (BaseSaveValue::Bool(value), BaseSaveFieldType::Boolean) => output.push(u8::from(*value)),
        (BaseSaveValue::String(value), BaseSaveFieldType::String128) => {
            encode_fixed_string(value, 128, output)?;
        }
        (BaseSaveValue::String(value), BaseSaveFieldType::String4K) => {
            encode_fixed_string(value, 4096, output)?;
        }
        _ => {
            return Err(format!(
                "value {value:?} does not match schema field {field_type:?}"
            ));
        }
    }
    Ok(())
}

fn encode_fixed_string(value: &str, capacity: usize, output: &mut Vec<u8>) -> Result<(), String> {
    if value.len() > capacity {
        return Err(format!(
            "UTF-8 string is {} bytes and exceeds the {capacity}-byte fixed field",
            value.len()
        ));
    }
    output.extend_from_slice(value.as_bytes());
    output.resize(output.len() + (capacity - value.len()), 0);
    Ok(())
}
