use std::error::Error;
use std::fmt::{Display, Formatter};

const MAX_DEPTH: usize = 128;
const MAX_NODES: usize = 100_000;

#[derive(Debug, Clone, PartialEq)]
pub enum SimpleJsonNode {
    Array(Vec<SimpleJsonNode>),
    Class(Vec<(String, SimpleJsonNode)>),
    String(String),
    Int(i32),
    Double(f64),
    Bool(bool),
    Float(f32),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SimpleJsonDocument {
    pub root: SimpleJsonNode,
    pub trailing_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacySaveError {
    pub offset: usize,
    pub context: String,
}

impl LegacySaveError {
    fn new(offset: usize, context: impl Into<String>) -> Self {
        Self {
            offset,
            context: context.into(),
        }
    }
}

impl Display for LegacySaveError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "legacy save at byte {}: {}",
            self.offset, self.context
        )
    }
}

impl Error for LegacySaveError {}

impl SimpleJsonDocument {
    pub fn decode(bytes: &[u8]) -> Result<Self, LegacySaveError> {
        let mut decoder = Decoder {
            bytes,
            position: 0,
            nodes: 0,
        };
        let root = decoder.node(0)?;
        Ok(Self {
            root,
            trailing_bytes: bytes.len() - decoder.position,
        })
    }

    pub fn encode(root: &SimpleJsonNode) -> Result<Vec<u8>, LegacySaveError> {
        let mut output = Vec::new();
        encode_node(root, &mut output, 0)?;
        Ok(output)
    }
}

struct Decoder<'a> {
    bytes: &'a [u8],
    position: usize,
    nodes: usize,
}

impl Decoder<'_> {
    fn node(&mut self, depth: usize) -> Result<SimpleJsonNode, LegacySaveError> {
        if depth > MAX_DEPTH {
            return Err(LegacySaveError::new(
                self.position,
                "nesting limit exceeded",
            ));
        }
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(LegacySaveError::new(self.position, "node limit exceeded"));
        }
        let tag_offset = self.position;
        match self.i32()? {
            1 => {
                let count = self.count()?;
                let mut values = Vec::with_capacity(count);
                for _ in 0..count {
                    values.push(self.node(depth + 1)?);
                }
                Ok(SimpleJsonNode::Array(values))
            }
            2 => {
                let count = self.count()?;
                let mut values = Vec::with_capacity(count);
                let mut keys = std::collections::BTreeSet::new();
                for _ in 0..count {
                    let key = self.string()?;
                    if !keys.insert(key.clone()) {
                        return Err(LegacySaveError::new(
                            self.position,
                            format!("duplicate class key `{key}`"),
                        ));
                    }
                    values.push((key, self.node(depth + 1)?));
                }
                Ok(SimpleJsonNode::Class(values))
            }
            3 => Ok(SimpleJsonNode::String(self.string()?)),
            4 => Ok(SimpleJsonNode::Int(self.i32()?)),
            5 => Ok(SimpleJsonNode::Double(self.f64()?)),
            6 => Ok(SimpleJsonNode::Bool(self.byte()? != 0)),
            7 => Ok(SimpleJsonNode::Float(self.f32()?)),
            tag => Err(LegacySaveError::new(
                tag_offset,
                format!("unsupported SimpleJSON binary tag {tag}"),
            )),
        }
    }

    fn count(&mut self) -> Result<usize, LegacySaveError> {
        let offset = self.position;
        let count = self.i32()?;
        let count = usize::try_from(count)
            .map_err(|_| LegacySaveError::new(offset, "negative collection length"))?;
        if count > MAX_NODES - self.nodes {
            return Err(LegacySaveError::new(
                offset,
                "collection exceeds node budget",
            ));
        }
        Ok(count)
    }

    fn string(&mut self) -> Result<String, LegacySaveError> {
        let length_offset = self.position;
        let mut length = 0_u32;
        for shift in (0..35).step_by(7) {
            let byte = self.byte()?;
            length |= u32::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                let length = usize::try_from(length).map_err(|_| {
                    LegacySaveError::new(length_offset, "string length exceeds this platform")
                })?;
                let end = self
                    .position
                    .checked_add(length)
                    .ok_or_else(|| LegacySaveError::new(length_offset, "string length overflow"))?;
                let bytes = self
                    .bytes
                    .get(self.position..end)
                    .ok_or_else(|| LegacySaveError::new(length_offset, "truncated UTF-8 string"))?;
                let value = std::str::from_utf8(bytes).map_err(|error| {
                    LegacySaveError::new(length_offset, format!("invalid UTF-8 string: {error}"))
                })?;
                self.position = end;
                return Ok(value.to_owned());
            }
        }
        Err(LegacySaveError::new(
            length_offset,
            "invalid .NET seven-bit string length",
        ))
    }

    fn byte(&mut self) -> Result<u8, LegacySaveError> {
        let byte = self.bytes.get(self.position).copied().ok_or_else(|| {
            LegacySaveError::new(self.position, "unexpected end of SimpleJSON data")
        })?;
        self.position += 1;
        Ok(byte)
    }

    fn i32(&mut self) -> Result<i32, LegacySaveError> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    fn f32(&mut self) -> Result<f32, LegacySaveError> {
        Ok(f32::from_le_bytes(self.array()?))
    }

    fn f64(&mut self) -> Result<f64, LegacySaveError> {
        Ok(f64::from_le_bytes(self.array()?))
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], LegacySaveError> {
        let start = self.position;
        let end = start
            .checked_add(N)
            .ok_or_else(|| LegacySaveError::new(start, "numeric value length overflow"))?;
        let bytes = self
            .bytes
            .get(start..end)
            .ok_or_else(|| LegacySaveError::new(start, "truncated numeric value"))?;
        self.position = end;
        Ok(bytes.try_into().expect("fixed slice length"))
    }
}

fn encode_node(
    node: &SimpleJsonNode,
    output: &mut Vec<u8>,
    depth: usize,
) -> Result<(), LegacySaveError> {
    if depth > MAX_DEPTH {
        return Err(LegacySaveError::new(output.len(), "nesting limit exceeded"));
    }
    match node {
        SimpleJsonNode::Array(values) => {
            output.extend_from_slice(&1_i32.to_le_bytes());
            encode_count(values.len(), output)?;
            for value in values {
                encode_node(value, output, depth + 1)?;
            }
        }
        SimpleJsonNode::Class(values) => {
            output.extend_from_slice(&2_i32.to_le_bytes());
            encode_count(values.len(), output)?;
            let mut keys = std::collections::BTreeSet::new();
            for (key, value) in values {
                if !keys.insert(key) {
                    return Err(LegacySaveError::new(
                        output.len(),
                        format!("duplicate class key `{key}`"),
                    ));
                }
                encode_string(key, output)?;
                encode_node(value, output, depth + 1)?;
            }
        }
        SimpleJsonNode::String(value) => {
            output.extend_from_slice(&3_i32.to_le_bytes());
            encode_string(value, output)?;
        }
        SimpleJsonNode::Int(value) => {
            output.extend_from_slice(&4_i32.to_le_bytes());
            output.extend_from_slice(&value.to_le_bytes());
        }
        SimpleJsonNode::Double(value) => {
            output.extend_from_slice(&5_i32.to_le_bytes());
            output.extend_from_slice(&value.to_le_bytes());
        }
        SimpleJsonNode::Bool(value) => {
            output.extend_from_slice(&6_i32.to_le_bytes());
            output.push(u8::from(*value));
        }
        SimpleJsonNode::Float(value) => {
            output.extend_from_slice(&7_i32.to_le_bytes());
            output.extend_from_slice(&value.to_le_bytes());
        }
    }
    Ok(())
}

fn encode_count(count: usize, output: &mut Vec<u8>) -> Result<(), LegacySaveError> {
    let count = i32::try_from(count)
        .map_err(|_| LegacySaveError::new(output.len(), "collection exceeds Int32 range"))?;
    output.extend_from_slice(&count.to_le_bytes());
    Ok(())
}

fn encode_string(value: &str, output: &mut Vec<u8>) -> Result<(), LegacySaveError> {
    let mut length = value.len();
    while length >= 0x80 {
        output.push((length as u8 & 0x7f) | 0x80);
        length >>= 7;
    }
    output.push(length as u8);
    output.extend_from_slice(value.as_bytes());
    Ok(())
}
