//! Byte-addressed event variable memory.
//!
//! Port of Memoria `EBin.GetVariableValueInternal` and
//! `EBin.SetVariableValueInternal`. The reference stores Global variables in
//! one 2048-byte `gEventGlobal` and Map variables in one 80-byte
//! `EventContext.mapvar`, both shared by every entry. Typed accesses overlap:
//! a bit index addresses `index >> 3` / `index & 7`, every other type
//! addresses the byte at `index` and the bytes after it, little-endian.

use serde::{Deserialize, Serialize};

use super::VariableType;

/// Size of `FF9StateSystem.EventState.gEventGlobal` (`EventEngine.Initialize.cs:54`).
pub const GLOBAL_VARIABLE_BYTES: usize = 2048;
/// Size of `EventContext.mapvar` (`EventContext.cs:9`).
pub const MAP_VARIABLE_BYTES: usize = 80;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariableMemory {
    bytes: Vec<u8>,
}

impl VariableMemory {
    pub fn zeroed(len: usize) -> Self {
        Self {
            bytes: vec![0; len],
        }
    }

    pub fn global() -> Self {
        Self::zeroed(GLOBAL_VARIABLE_BYTES)
    }

    pub fn map() -> Self {
        Self::zeroed(MAP_VARIABLE_BYTES)
    }

    /// A memory holding `bytes` (a save file's `gEventGlobal`); `None` when the length is not
    /// `len`.
    pub fn from_bytes(bytes: &[u8], len: usize) -> Option<Self> {
        (bytes.len() == len).then(|| Self {
            bytes: bytes.to_vec(),
        })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn clear(&mut self) {
        self.bytes.fill(0);
    }

    fn range(&self, index: u16, value_type: VariableType) -> Option<std::ops::Range<usize>> {
        let index = usize::from(index);
        let (start, width) = match value_type {
            VariableType::SignedBit | VariableType::Bit => (index >> 3, 1),
            VariableType::Int24 | VariableType::UInt24 => (index, 3),
            VariableType::SignedByte | VariableType::Byte => (index, 1),
            VariableType::Int16 | VariableType::UInt16 => (index, 2),
        };
        let end = start.checked_add(width)?;
        (end <= self.bytes.len()).then_some(start..end)
    }

    /// `GetVariableValueInternal`. `None` when the access runs past the
    /// buffer (an `IndexOutOfRangeException` in the reference).
    pub fn get(&self, index: u16, value_type: VariableType) -> Option<i32> {
        let range = self.range(index, value_type)?;
        let b = &self.bytes[range];
        Some(match value_type {
            VariableType::SignedBit | VariableType::Bit => i32::from((b[0] >> (index & 7)) & 1),
            // Both 24-bit types sign-extend the high byte (`(SByte)buffer[ofs + 2] << 16`).
            VariableType::Int24 | VariableType::UInt24 => {
                i32::from(b[0]) | (i32::from(b[1]) << 8) | (i32::from(b[2] as i8) << 16)
            }
            VariableType::SignedByte => i32::from(b[0] as i8),
            VariableType::Byte => i32::from(b[0]),
            VariableType::Int16 => i32::from(b[0]) | (i32::from(b[1] as i8) << 8),
            VariableType::UInt16 => i32::from(b[0]) | (i32::from(b[1]) << 8),
        })
    }

    /// `SetVariableValueInternal` (without the rope-jumping reward hack,
    /// which is a Memoria configuration option). Returns `None` when the
    /// access runs past the buffer.
    pub fn set(&mut self, index: u16, value_type: VariableType, value: i32) -> Option<()> {
        let range = self.range(index, value_type)?;
        let b = &mut self.bytes[range];
        match value_type {
            VariableType::SignedBit | VariableType::Bit => {
                let mask = 1u8 << (index & 7);
                if value == 0 {
                    b[0] &= !mask;
                } else {
                    b[0] |= mask;
                }
            }
            VariableType::Int24 | VariableType::UInt24 => {
                b[0] = value as u8;
                b[1] = (value >> 8) as u8;
                b[2] = (value >> 16) as u8;
            }
            VariableType::SignedByte | VariableType::Byte => b[0] = value as u8,
            VariableType::Int16 | VariableType::UInt16 => {
                b[0] = value as u8;
                b[1] = (value >> 8) as u8;
            }
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_address_byte_and_bit_and_overlap_bytes() {
        let mut memory = VariableMemory::map();
        memory.set(167, VariableType::Bit, 1).unwrap();
        // 167 >> 3 = 20, bit 7.
        assert_eq!(memory.get(20, VariableType::Byte), Some(0x80));
        assert_eq!(memory.get(167, VariableType::Bit), Some(1));
        assert_eq!(memory.get(166, VariableType::Bit), Some(0));
        memory.set(167, VariableType::Bit, 0).unwrap();
        assert_eq!(memory.get(20, VariableType::Byte), Some(0));
        memory.set(5, VariableType::SignedBit, 7).unwrap();
        assert_eq!(memory.get(5, VariableType::SignedBit), Some(1));
    }

    #[test]
    fn multi_byte_types_are_little_endian_and_overlap() {
        let mut memory = VariableMemory::global();
        memory.set(2, VariableType::Int16, -2).unwrap();
        assert_eq!(memory.get(2, VariableType::Int16), Some(-2));
        assert_eq!(memory.get(2, VariableType::UInt16), Some(0xFFFE));
        assert_eq!(memory.get(2, VariableType::Byte), Some(0xFE));
        assert_eq!(memory.get(3, VariableType::SignedByte), Some(-1));
        memory.set(10, VariableType::UInt24, 0x00AB_CDEF).unwrap();
        // Memoria sign-extends UInt24 reads too.
        assert_eq!(
            memory.get(10, VariableType::UInt24),
            Some(0xFFAB_CDEFu32 as i32)
        );
        assert_eq!(
            memory.get(10, VariableType::Int24),
            Some(0xFFAB_CDEFu32 as i32)
        );
        memory.set(20, VariableType::Byte, 300).unwrap();
        assert_eq!(memory.get(20, VariableType::Byte), Some(44));
    }

    #[test]
    fn out_of_range_accesses_fail() {
        let mut memory = VariableMemory::map();
        assert_eq!(memory.get(79, VariableType::Int16), None);
        assert_eq!(memory.get(79, VariableType::Byte), Some(0));
        assert_eq!(memory.get(640, VariableType::Bit), None);
        assert_eq!(memory.get(639, VariableType::Bit), Some(0));
        assert!(memory.set(80, VariableType::Byte, 1).is_none());
    }
}
