use crate::install::ImportError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SerializedTypeReport {
    pub class_id: i32,
    pub tree_nodes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SerializedObject {
    pub path_id: i64,
    pub relative_offset: u32,
    pub size: u32,
    pub absolute_offset: u64,
    pub type_id: i32,
    pub class_id: u16,
    pub script_type_index: i16,
    pub is_stripped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SerializedFileReport {
    pub metadata_size: u32,
    pub file_size: u32,
    pub format_version: u32,
    pub data_offset: u32,
    pub unity_version: String,
    pub target_platform: i32,
    pub type_trees_embedded: bool,
    pub types: Vec<SerializedTypeReport>,
    pub objects: Vec<SerializedObject>,
    pub external_paths: Vec<String>,
}

pub struct SerializedFile<'a> {
    bytes: &'a [u8],
    report: SerializedFileReport,
    type_trees: Vec<(i32, Option<RawTypeTree<'a>>)>,
}

/// Raw embedded type-tree blob for one serialized type: `node_count` 24-byte
/// nodes followed by the local string buffer (Unity 5.x format 15 layout).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawTypeTree<'a> {
    pub node_count: u32,
    pub nodes: &'a [u8],
    pub strings: &'a [u8],
}

impl<'a> SerializedFile<'a> {
    pub fn parse(bytes: &'a [u8], source: &str) -> Result<Self, ImportError> {
        Self::parse_prefix(bytes, bytes.len() as u64, source)
    }

    /// Parses the metadata from a prefix of a serialized file whose full length is
    /// `total_len`. The prefix must cover the metadata block (`data_offset` bytes);
    /// object payloads are not required, so large standalone files can be streamed.
    pub fn parse_prefix(
        bytes: &'a [u8],
        total_len: u64,
        source: &str,
    ) -> Result<Self, ImportError> {
        let mut cursor = Cursor { bytes, position: 0 };
        let metadata_size = cursor.be_u32(source)?;
        let file_size = cursor.be_u32(source)?;
        let format_version = cursor.be_u32(source)?;
        let data_offset = cursor.be_u32(source)?;
        if format_version != 15 {
            return Err(invalid(
                source,
                format!("only serialized format 15 is supported, found {format_version}"),
            ));
        }
        if u64::from(file_size) != total_len {
            return Err(invalid(
                source,
                format!("header declares {file_size} bytes, input has {total_len}"),
            ));
        }
        let little_endian = match cursor.byte(source)? {
            0 => true,
            1 => false,
            _ => return Err(invalid(source, "invalid serialized endian marker")),
        };
        if !little_endian {
            return Err(ImportError::unsupported(
                source,
                "big-endian serialized metadata",
            ));
        }
        cursor.skip(3, source)?;
        let unity_version = cursor.c_string(source)?;
        let target_platform = cursor.i32(source)?;
        let type_trees_embedded = cursor.byte(source)? != 0;
        let type_count = cursor.count(source, 4_096, "type")?;
        let mut types = Vec::with_capacity(type_count);
        let mut type_trees = Vec::with_capacity(type_count);
        for _ in 0..type_count {
            let class_id = cursor.i32(source)?;
            if class_id < 0 {
                cursor.skip(16, source)?;
            }
            cursor.skip(16, source)?;
            let mut tree_nodes = 0;
            let mut raw_tree = None;
            if type_trees_embedded {
                tree_nodes = cursor.count(source, 100_000, "type-tree node")? as u32;
                let string_bytes = cursor.count(source, 16 * 1024 * 1024, "type-tree string")?;
                let node_bytes = (tree_nodes as usize)
                    .checked_mul(24)
                    .ok_or_else(|| invalid(source, "type-tree size overflow"))?;
                let node_start = cursor.position;
                cursor.skip(node_bytes, source)?;
                cursor.skip(string_bytes, source)?;
                raw_tree = Some(RawTypeTree {
                    node_count: tree_nodes,
                    nodes: &bytes[node_start..node_start + node_bytes],
                    strings: &bytes[node_start + node_bytes..cursor.position],
                });
            }
            type_trees.push((class_id, raw_tree));
            types.push(SerializedTypeReport {
                class_id,
                tree_nodes,
            });
        }
        let object_count = cursor.count(source, 1_000_000, "object")?;
        let mut objects = Vec::with_capacity(object_count);
        let mut path_ids = std::collections::BTreeSet::new();
        for _ in 0..object_count {
            cursor.align(4, source)?;
            let path_id = cursor.i64(source)?;
            if !path_ids.insert(path_id) {
                return Err(invalid(
                    source,
                    format!("duplicate object path ID {path_id}"),
                ));
            }
            let relative_offset = cursor.u32(source)?;
            let size = cursor.u32(source)?;
            let type_id = cursor.i32(source)?;
            let class_id = cursor.u16(source)?;
            let script_type_index = cursor.i16(source)?;
            let is_stripped = cursor.byte(source)? != 0;
            let absolute_offset = u64::from(data_offset) + u64::from(relative_offset);
            let end = absolute_offset
                .checked_add(u64::from(size))
                .ok_or_else(|| invalid(source, "object range overflow"))?;
            if end > total_len {
                return Err(invalid(
                    source,
                    format!("object {path_id} lies outside the serialized file"),
                ));
            }
            if !types.iter().any(|kind| kind.class_id == type_id) {
                return Err(invalid(
                    source,
                    format!("object {path_id} references unknown type {type_id}"),
                ));
            }
            objects.push(SerializedObject {
                path_id,
                relative_offset,
                size,
                absolute_offset,
                type_id,
                class_id,
                script_type_index,
                is_stripped,
            });
        }
        let script_count = cursor.count(source, 100_000, "script")?;
        for _ in 0..script_count {
            cursor.i32(source)?;
            cursor.align(4, source)?;
            cursor.i64(source)?;
        }
        let external_count = cursor.count(source, 4_096, "external reference")?;
        let mut external_paths = Vec::with_capacity(external_count);
        for _ in 0..external_count {
            cursor.c_string(source)?;
            cursor.skip(16, source)?;
            cursor.i32(source)?;
            external_paths.push(cursor.c_string(source)?);
        }
        if cursor.position < bytes.len() {
            cursor.c_string(source)?;
        }
        Ok(Self {
            bytes,
            type_trees,
            report: SerializedFileReport {
                metadata_size,
                file_size,
                format_version,
                data_offset,
                unity_version,
                target_platform,
                type_trees_embedded,
                types,
                objects,
                external_paths,
            },
        })
    }

    pub fn report(&self) -> &SerializedFileReport {
        &self.report
    }

    /// Embedded type tree for a serialized type ID (the object's `type_id`).
    pub fn type_tree(&self, type_id: i32) -> Option<RawTypeTree<'a>> {
        self.type_trees
            .iter()
            .find(|(candidate, _)| *candidate == type_id)
            .and_then(|(_, tree)| *tree)
    }

    pub fn object_data(&self, index: usize) -> Result<&'a [u8], ImportError> {
        let object =
            self.report.objects.get(index).ok_or_else(|| {
                invalid("serialized-file", format!("unknown object index {index}"))
            })?;
        let start = object.absolute_offset as usize;
        let end = start + object.size as usize;
        self.bytes
            .get(start..end)
            .ok_or_else(|| invalid("serialized-file", "object range is unavailable"))
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl Cursor<'_> {
    fn be_u32(&mut self, source: &str) -> Result<u32, ImportError> {
        Ok(u32::from_be_bytes(self.array(source)?))
    }

    fn u32(&mut self, source: &str) -> Result<u32, ImportError> {
        Ok(u32::from_le_bytes(self.array(source)?))
    }

    fn i32(&mut self, source: &str) -> Result<i32, ImportError> {
        Ok(i32::from_le_bytes(self.array(source)?))
    }

    fn i64(&mut self, source: &str) -> Result<i64, ImportError> {
        Ok(i64::from_le_bytes(self.array(source)?))
    }

    fn u16(&mut self, source: &str) -> Result<u16, ImportError> {
        Ok(u16::from_le_bytes(self.array(source)?))
    }

    fn i16(&mut self, source: &str) -> Result<i16, ImportError> {
        Ok(i16::from_le_bytes(self.array(source)?))
    }

    fn byte(&mut self, source: &str) -> Result<u8, ImportError> {
        Ok(self.array::<1>(source)?[0])
    }

    fn align(&mut self, alignment: usize, source: &str) -> Result<(), ImportError> {
        let padding = (alignment - self.position % alignment) % alignment;
        self.skip(padding, source)
    }

    fn array<const N: usize>(&mut self, source: &str) -> Result<[u8; N], ImportError> {
        let end = self
            .position
            .checked_add(N)
            .ok_or_else(|| invalid(source, "serialized cursor overflow"))?;
        let slice = self.bytes.get(self.position..end).ok_or_else(|| {
            invalid(
                source,
                format!("truncated serialized data at byte {}", self.position),
            )
        })?;
        self.position = end;
        Ok(slice.try_into().expect("fixed serialized slice"))
    }

    fn skip(&mut self, count: usize, source: &str) -> Result<(), ImportError> {
        let end = self
            .position
            .checked_add(count)
            .ok_or_else(|| invalid(source, "serialized skip overflow"))?;
        if end > self.bytes.len() {
            return Err(invalid(
                source,
                format!("truncated serialized data at byte {}", self.position),
            ));
        }
        self.position = end;
        Ok(())
    }

    fn count(&mut self, source: &str, maximum: usize, kind: &str) -> Result<usize, ImportError> {
        let value = self.i32(source)?;
        let value = usize::try_from(value)
            .map_err(|_| invalid(source, format!("negative {kind} count")))?;
        if value > maximum {
            return Err(invalid(source, format!("{kind} count exceeds {maximum}")));
        }
        Ok(value)
    }

    fn c_string(&mut self, source: &str) -> Result<String, ImportError> {
        let remaining = &self.bytes[self.position..];
        let length = remaining
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| invalid(source, "unterminated serialized string"))?;
        let value = String::from_utf8(remaining[..length].to_vec())
            .map_err(|error| invalid(source, format!("non-UTF-8 serialized string: {error}")))?;
        self.position += length + 1;
        Ok(value)
    }
}

fn invalid(source: &str, message: impl Into<String>) -> ImportError {
    ImportError::invalid(source, message.into())
}
