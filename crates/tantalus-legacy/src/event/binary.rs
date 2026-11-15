use super::{EventProgram, VmError, VmErrorKind, decode_program};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventEntry {
    pub index: usize,
    pub offset: u16,
    pub size: u16,
    pub variable_count: u8,
    pub flags: u8,
    pub absolute_offset: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventFunction {
    pub tag: u16,
    pub offset: u32,
    pub program: EventProgram,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventEntryProgram {
    pub entry_type: u8,
    pub functions: Vec<EventFunction>,
}

#[derive(Debug, Clone)]
pub struct EventBinary {
    pub version: u8,
    pub mode: u8,
    pub entries: Vec<EventEntry>,
    bytes: Vec<u8>,
}

impl EventBinary {
    pub fn parse(bytes: &[u8]) -> Result<Self, VmError> {
        if bytes.get(..2) != Some(b"EV") {
            return Err(error(0, "event binary does not start with EV"));
        }
        let version = *bytes
            .get(2)
            .ok_or_else(|| error(2, "missing event version"))?;
        if version != 2 {
            return Err(VmError::new(
                VmErrorKind::UnsupportedOpcode,
                2,
                Some(u32::from(version)),
                format!("unsupported event binary version {version}; expected vanilla 2"),
            ));
        }
        let mode = 0;
        let count = *bytes
            .get(3)
            .ok_or_else(|| error(3, "missing event entry count"))? as usize;
        if bytes.len() < 128 {
            return Err(error(bytes.len() as u32, "truncated event binary header"));
        }
        let table_end = 128_usize
            .checked_add(
                count
                    .checked_mul(8)
                    .ok_or_else(|| error(128, "event entry table size overflow"))?,
            )
            .ok_or_else(|| error(128, "event entry table overflow"))?;
        if table_end > bytes.len() {
            return Err(error(128, "truncated event entry table"));
        }

        let mut entries = Vec::with_capacity(count);
        for index in 0..count {
            let position = 128 + index * 8;
            let offset = u16::from_le_bytes([bytes[position], bytes[position + 1]]);
            let size = u16::from_le_bytes([bytes[position + 2], bytes[position + 3]]);
            let absolute_offset = 128_usize + offset as usize;
            let end = absolute_offset
                .checked_add(size as usize)
                .ok_or_else(|| error(position as u32, "event entry range overflow"))?;
            if end > bytes.len() {
                return Err(VmError::new(
                    VmErrorKind::Decode,
                    position as u32,
                    None,
                    format!("event entry {index} lies outside the binary"),
                ));
            }
            entries.push(EventEntry {
                index,
                offset,
                size,
                variable_count: bytes[position + 4],
                flags: bytes[position + 5],
                absolute_offset,
            });
        }
        Ok(Self {
            version,
            mode,
            entries,
            bytes: bytes.to_vec(),
        })
    }

    pub fn entry_bytes(&self, index: usize) -> Option<&[u8]> {
        let entry = self.entries.get(index)?;
        self.bytes
            .get(entry.absolute_offset..entry.absolute_offset + entry.size as usize)
    }

    pub fn decode_entry(&self, index: usize) -> Result<EventEntryProgram, VmError> {
        let bytes = self.entry_bytes(index).ok_or_else(|| {
            VmError::new(
                VmErrorKind::Decode,
                0,
                None,
                format!("event entry {index} does not exist"),
            )
        })?;
        if bytes.is_empty() {
            return Ok(EventEntryProgram {
                entry_type: 0xff,
                functions: Vec::new(),
            });
        }

        let entry_type = bytes[0];
        let function_count = *bytes.get(1).ok_or_else(|| {
            error(
                1,
                format!("event entry {index} is missing its function count"),
            )
        })? as usize;
        let table_end = 2 + function_count * 4;
        if table_end > bytes.len() {
            return Err(error(
                2,
                format!("event entry {index} has a truncated function table"),
            ));
        }

        let mut functions = Vec::with_capacity(function_count);
        for function_index in 0..function_count {
            let table_position = 2 + function_index * 4;
            let tag = u16::from_le_bytes([bytes[table_position], bytes[table_position + 1]]);
            let start =
                2 + u16::from_le_bytes([bytes[table_position + 2], bytes[table_position + 3]])
                    as usize;
            let end = if function_index + 1 < function_count {
                2 + u16::from_le_bytes([bytes[table_position + 6], bytes[table_position + 7]])
                    as usize
            } else {
                bytes.len()
            };
            if start < table_end || end > bytes.len() || end < start {
                return Err(error(
                    table_position as u32,
                    format!(
                        "event entry {index} function {function_index} (tag {tag}) has an invalid range {start}..{end}"
                    ),
                ));
            }
            let program = decode_program(&bytes[start..end]).map_err(|mut error| {
                error.offset += start as u32;
                error.message = format!(
                    "event entry {index} function {function_index} (tag {tag}): {}",
                    error.message
                );
                error
            })?;
            functions.push(EventFunction {
                tag,
                offset: start as u32,
                program,
            });
        }
        Ok(EventEntryProgram {
            entry_type,
            functions,
        })
    }
}

fn error(offset: u32, message: impl Into<String>) -> VmError {
    VmError::new(VmErrorKind::Decode, offset, None, message)
}
