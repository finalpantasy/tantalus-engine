//! Generic reader for Unity 5.x embedded type trees (serialized format 15).
//!
//! Each node is 24 bytes: `version: u16`, `depth: u8`, `type_flags: u8`
//! (bit 0 = array), `type_offset: i32`, `name_offset: i32`, `byte_size: i32`,
//! `index: i32`, `meta_flag: i32`. String offsets with the high bit set point
//! into Unity's built-in common string table; others into the local buffer.
//! After a node is read, `meta_flag & 0x4000` aligns the stream to 4 bytes,
//! measured from the start of the object. This layout was verified against
//! `p0data4.bin` and `p0data5.bin` (Unity 5.2.3p2) by exact consumption of
//! every object in both bundles.

use crate::install::ImportError;
use crate::pc::serialized::RawTypeTree;

const ALIGN_FLAG: i32 = 0x4000;
const ARRAY_FLAG: u8 = 0x01;

/// Unity's built-in common string table (5.x), in offset order.
const COMMON_STRINGS: &[&str] = &[
    "AABB",
    "AnimationClip",
    "AnimationCurve",
    "AnimationState",
    "Array",
    "Base",
    "BitField",
    "bitset",
    "bool",
    "char",
    "ColorRGBA",
    "Component",
    "data",
    "deque",
    "double",
    "dynamic_array",
    "FastPropertyName",
    "first",
    "float",
    "Font",
    "GameObject",
    "Generic Mono",
    "GradientNEW",
    "GUID",
    "GUIStyle",
    "int",
    "list",
    "long long",
    "map",
    "Matrix4x4f",
    "MdFour",
    "MonoBehaviour",
    "MonoScript",
    "m_ByteSize",
    "m_Curve",
    "m_EditorClassIdentifier",
    "m_EditorHideFlags",
    "m_Enabled",
    "m_ExtensionPtr",
    "m_GameObject",
    "m_Index",
    "m_IsArray",
    "m_IsStatic",
    "m_MetaFlag",
    "m_Name",
    "m_ObjectHideFlags",
    "m_PrefabInternal",
    "m_PrefabParentObject",
    "m_Script",
    "m_StaticEditorFlags",
    "m_Type",
    "m_Version",
    "Object",
    "pair",
    "PPtr<Component>",
    "PPtr<GameObject>",
    "PPtr<Material>",
    "PPtr<MonoBehaviour>",
    "PPtr<MonoScript>",
    "PPtr<Object>",
    "PPtr<Prefab>",
    "PPtr<Sprite>",
    "PPtr<TextAsset>",
    "PPtr<Texture>",
    "PPtr<Texture2D>",
    "PPtr<Transform>",
    "Prefab",
    "Quaternionf",
    "Rectf",
    "RectInt",
    "RectOffset",
    "second",
    "set",
    "short",
    "size",
    "SInt16",
    "SInt32",
    "SInt64",
    "SInt8",
    "staticvector",
    "string",
    "TextAsset",
    "TextMesh",
    "Texture",
    "Texture2D",
    "Transform",
    "TypelessData",
    "UInt16",
    "UInt32",
    "UInt64",
    "UInt8",
    "unsigned int",
    "unsigned long long",
    "unsigned short",
    "vector",
    "Vector2f",
    "Vector3f",
    "Vector4f",
    "m_ScriptingClassIdentifier",
    "Gradient",
    "Type*",
];

fn common_string(offset: u32) -> Option<&'static str> {
    let mut position = 0_u32;
    for value in COMMON_STRINGS {
        if position == offset {
            return Some(value);
        }
        position += value.len() as u32 + 1;
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Primitive {
    Bool,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
}

impl Primitive {
    fn from_type(name: &str) -> Option<Self> {
        Some(match name {
            "bool" => Self::Bool,
            "SInt8" => Self::I8,
            "UInt8" | "char" => Self::U8,
            "SInt16" | "short" => Self::I16,
            "UInt16" | "unsigned short" => Self::U16,
            "SInt32" | "int" => Self::I32,
            "UInt32" | "unsigned int" | "Type*" => Self::U32,
            "SInt64" | "long long" => Self::I64,
            "UInt64" | "unsigned long long" | "FileSize" => Self::U64,
            "float" => Self::F32,
            "double" => Self::F64,
            _ => return None,
        })
    }

    fn size(self) -> usize {
        match self {
            Self::Bool | Self::I8 | Self::U8 => 1,
            Self::I16 | Self::U16 => 2,
            Self::I32 | Self::U32 | Self::F32 => 4,
            Self::I64 | Self::U64 | Self::F64 => 8,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeTreeNode {
    pub version: u16,
    pub depth: u8,
    pub type_flags: u8,
    pub type_name: String,
    pub name: String,
    pub byte_size: i32,
    pub index: i32,
    pub meta_flag: i32,
    primitive: Option<Primitive>,
    end: usize,
}

impl TypeTreeNode {
    pub fn is_array(&self) -> bool {
        self.type_flags & ARRAY_FLAG != 0
    }

    pub fn aligns(&self) -> bool {
        self.meta_flag & ALIGN_FLAG != 0
    }
}

/// Parsed type tree; node 0 is the object root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeTree {
    pub nodes: Vec<TypeTreeNode>,
}

impl TypeTree {
    pub fn parse(raw: &RawTypeTree<'_>, source: &str) -> Result<Self, ImportError> {
        let count = raw.node_count as usize;
        if raw.nodes.len() != count * 24 {
            return Err(invalid(source, "type-tree node table has the wrong size"));
        }
        if count == 0 {
            return Err(invalid(source, "type tree has no nodes"));
        }
        let string_at = |offset: i32| -> Result<String, ImportError> {
            let offset = offset as u32;
            if offset & 0x8000_0000 != 0 {
                return common_string(offset & 0x7fff_ffff)
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        invalid(
                            source,
                            format!("unknown common type-tree string {}", offset & 0x7fff_ffff),
                        )
                    });
            }
            let start = offset as usize;
            let tail = raw
                .strings
                .get(start..)
                .ok_or_else(|| invalid(source, "type-tree string offset out of range"))?;
            let length = tail
                .iter()
                .position(|byte| *byte == 0)
                .ok_or_else(|| invalid(source, "unterminated type-tree string"))?;
            String::from_utf8(tail[..length].to_vec())
                .map_err(|_| invalid(source, "non-UTF-8 type-tree string"))
        };
        let mut nodes = Vec::with_capacity(count);
        for chunk in raw.nodes.chunks_exact(24) {
            let word = |at: usize| i32::from_le_bytes(chunk[at..at + 4].try_into().unwrap());
            let type_name = string_at(word(4))?;
            let primitive = Primitive::from_type(&type_name);
            nodes.push(TypeTreeNode {
                version: u16::from_le_bytes([chunk[0], chunk[1]]),
                depth: chunk[2],
                type_flags: chunk[3],
                name: string_at(word(8))?,
                type_name,
                byte_size: word(12),
                index: word(16),
                meta_flag: word(20),
                primitive,
                end: 0,
            });
        }
        if nodes[0].depth != 0 {
            return Err(invalid(source, "type-tree root must have depth 0"));
        }
        for index in 0..count {
            let depth = nodes[index].depth;
            let mut end = index + 1;
            while end < count && nodes[end].depth > depth {
                if nodes[end].depth > nodes[end - 1].depth + 1 {
                    return Err(invalid(source, "type-tree depth skips a level"));
                }
                end += 1;
            }
            nodes[index].end = end;
        }
        if nodes[0].end != count {
            return Err(invalid(source, "type tree has more than one root"));
        }
        Ok(Self { nodes })
    }

    pub fn root_type(&self) -> &str {
        &self.nodes[0].type_name
    }

    fn children(&self, index: usize) -> Children<'_> {
        Children {
            tree: self,
            next: index + 1,
            end: self.nodes[index].end,
        }
    }

    /// Reads one object and requires its byte range to be consumed exactly.
    pub fn read<'t>(&'t self, bytes: &[u8], source: &str) -> Result<Value<'t>, ImportError> {
        let mut reader = Reader {
            tree: self,
            bytes,
            position: 0,
            source,
        };
        let value = reader.node(0)?;
        if reader.position != bytes.len() {
            return Err(invalid(
                source,
                format!(
                    "type tree consumed {} of {} object bytes",
                    reader.position,
                    bytes.len()
                ),
            ));
        }
        Ok(value)
    }
}

struct Children<'t> {
    tree: &'t TypeTree,
    next: usize,
    end: usize,
}

impl Iterator for Children<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        if self.next >= self.end {
            return None;
        }
        let current = self.next;
        self.next = self.tree.nodes[current].end;
        Some(current)
    }
}

/// Decoded object value. Arrays of one-byte and four-byte primitives are kept
/// packed so large vertex and index buffers stay cheap.
#[derive(Debug, Clone, PartialEq)]
pub enum Value<'t> {
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    I32s(Vec<i32>),
    U32s(Vec<u32>),
    F32s(Vec<f32>),
    Array(Vec<Value<'t>>),
    Struct(Vec<(&'t str, Value<'t>)>),
}

impl<'t> Value<'t> {
    pub fn field(&self, name: &str) -> Option<&Value<'t>> {
        match self {
            Self::Struct(fields) => fields
                .iter()
                .find(|(candidate, _)| *candidate == name)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::UInt(value) => Some(*value),
            Self::Int(value) => u64::try_from(*value).ok(),
            Self::Bool(value) => Some(u64::from(*value)),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(value) => Some(*value),
            Self::UInt(value) => i64::try_from(*value).ok(),
            Self::Bool(value) => Some(i64::from(*value)),
            _ => None,
        }
    }

    pub fn as_f32(&self) -> Option<f32> {
        match self {
            Self::Float(value) => Some(*value as f32),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            Self::UInt(value) => Some(*value != 0),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Bytes(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value<'t>]> {
        match self {
            Self::Array(value) => Some(value),
            _ => None,
        }
    }

    /// Element count for any array representation.
    pub fn array_len(&self) -> Option<usize> {
        Some(match self {
            Self::Bytes(value) => value.len(),
            Self::I32s(value) => value.len(),
            Self::U32s(value) => value.len(),
            Self::F32s(value) => value.len(),
            Self::Array(value) => value.len(),
            _ => return None,
        })
    }
}

struct Reader<'t, 'b> {
    tree: &'t TypeTree,
    bytes: &'b [u8],
    position: usize,
    source: &'b str,
}

impl<'t> Reader<'t, '_> {
    fn take(&mut self, count: usize) -> Result<&[u8], ImportError> {
        let end = self
            .position
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| {
                invalid(
                    self.source,
                    format!(
                        "object data ends at {} but {count} more bytes were needed at {}",
                        self.bytes.len(),
                        self.position
                    ),
                )
            })?;
        let slice = &self.bytes[self.position..end];
        self.position = end;
        Ok(slice)
    }

    fn align(&mut self) -> Result<(), ImportError> {
        let padding = (4 - self.position % 4) % 4;
        self.take(padding).map(|_| ())
    }

    fn primitive(&mut self, kind: Primitive) -> Result<Value<'t>, ImportError> {
        let bytes = self.take(kind.size())?;
        Ok(match kind {
            Primitive::Bool => Value::Bool(bytes[0] != 0),
            Primitive::I8 => Value::Int(i64::from(bytes[0] as i8)),
            Primitive::U8 => Value::UInt(u64::from(bytes[0])),
            Primitive::I16 => Value::Int(i64::from(i16::from_le_bytes([bytes[0], bytes[1]]))),
            Primitive::U16 => Value::UInt(u64::from(u16::from_le_bytes([bytes[0], bytes[1]]))),
            Primitive::I32 => Value::Int(i64::from(i32::from_le_bytes(
                bytes.try_into().expect("four bytes"),
            ))),
            Primitive::U32 => Value::UInt(u64::from(u32::from_le_bytes(
                bytes.try_into().expect("four bytes"),
            ))),
            Primitive::I64 => Value::Int(i64::from_le_bytes(bytes.try_into().expect("eight"))),
            Primitive::U64 => Value::UInt(u64::from_le_bytes(bytes.try_into().expect("eight"))),
            Primitive::F32 => Value::Float(f64::from(f32::from_le_bytes(
                bytes.try_into().expect("four bytes"),
            ))),
            Primitive::F64 => Value::Float(f64::from_le_bytes(bytes.try_into().expect("eight"))),
        })
    }

    fn node(&mut self, index: usize) -> Result<Value<'t>, ImportError> {
        let tree = self.tree;
        let node = &tree.nodes[index];
        let value = if node.is_array() {
            self.array(index)?
        } else if let Some(kind) = node.primitive.filter(|_| node.end == index + 1) {
            if node.byte_size != kind.size() as i32 {
                return Err(invalid(
                    self.source,
                    format!(
                        "primitive `{} {}` declares {} bytes",
                        node.type_name, node.name, node.byte_size
                    ),
                ));
            }
            self.primitive(kind)?
        } else {
            let mut children = tree.children(index);
            let first = children.next();
            match first {
                Some(child) if tree.nodes[child].is_array() && children.next().is_none() => {
                    let array = self.node(child)?;
                    if node.type_name == "string" {
                        // Unity strings are byte arrays; binary TextAsset
                        // payloads are not UTF-8 and stay as bytes.
                        match array {
                            Value::Bytes(bytes) => match String::from_utf8(bytes) {
                                Ok(text) => Value::String(text),
                                Err(error) => Value::Bytes(error.into_bytes()),
                            },
                            _ => return Err(invalid(self.source, "string payload is not bytes")),
                        }
                    } else {
                        array
                    }
                }
                None if node.byte_size == 0 => Value::Struct(Vec::new()),
                None => {
                    return Err(invalid(
                        self.source,
                        format!("unsupported leaf type `{}`", node.type_name),
                    ));
                }
                Some(_) => {
                    let mut fields = Vec::new();
                    for child in tree.children(index) {
                        let value = self.node(child)?;
                        fields.push((tree.nodes[child].name.as_str(), value));
                    }
                    Value::Struct(fields)
                }
            }
        };
        if node.aligns() {
            self.align()?;
        }
        Ok(value)
    }

    fn array(&mut self, index: usize) -> Result<Value<'t>, ImportError> {
        let tree = self.tree;
        let mut children = tree.children(index);
        let (Some(size_node), Some(data_node), None) =
            (children.next(), children.next(), children.next())
        else {
            return Err(invalid(
                self.source,
                "array node must have size and data children",
            ));
        };
        if tree.nodes[size_node].primitive.map(Primitive::size) != Some(4) {
            return Err(invalid(self.source, "array size must be a 32-bit integer"));
        }
        let count = i32::from_le_bytes(self.take(4)?.try_into().expect("four bytes"));
        let count = usize::try_from(count)
            .map_err(|_| invalid(self.source, format!("negative array count {count}")))?;
        let remaining = self.bytes.len() - self.position;
        let data = &tree.nodes[data_node];
        let minimum = usize::try_from(data.byte_size).unwrap_or(1).max(1);
        if count.saturating_mul(minimum) > remaining {
            return Err(invalid(
                self.source,
                format!(
                    "array of {count} `{}` exceeds the {remaining} remaining bytes",
                    data.type_name
                ),
            ));
        }
        let packed = data
            .primitive
            .filter(|_| data.end == data_node + 1 && !data.aligns());
        Ok(match packed {
            Some(Primitive::U8 | Primitive::I8 | Primitive::Bool) => {
                Value::Bytes(self.take(count)?.to_vec())
            }
            Some(Primitive::F32) => Value::F32s(
                self.take(count * 4)?
                    .chunks_exact(4)
                    .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
                    .collect(),
            ),
            Some(Primitive::I32) => Value::I32s(
                self.take(count * 4)?
                    .chunks_exact(4)
                    .map(|chunk| i32::from_le_bytes(chunk.try_into().unwrap()))
                    .collect(),
            ),
            Some(Primitive::U32) => Value::U32s(
                self.take(count * 4)?
                    .chunks_exact(4)
                    .map(|chunk| u32::from_le_bytes(chunk.try_into().unwrap()))
                    .collect(),
            ),
            _ => {
                let mut values = Vec::with_capacity(count);
                for _ in 0..count {
                    values.push(self.node(data_node)?);
                }
                Value::Array(values)
            }
        })
    }
}

fn invalid(source: &str, message: impl Into<String>) -> ImportError {
    ImportError::invalid(source, message.into())
}
