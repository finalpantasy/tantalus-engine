//! .NET metadata reader and constant-only static-initializer evaluator.
//!
//! The installed `Assembly-CSharp.dll` keeps the vanilla battle, item and
//! level tables as C# static array initializers, not as embedded resources.
//! The C# compiler turns each one into straight-line IL in the type's
//! `.cctor`: `newarr`, `dup`, constants, `newobj` of a data-class constructor,
//! `stelem`, then `stsfld`. Primitive arrays use `RuntimeHelpers.InitializeArray`
//! with a `FieldRVA` blob.
//!
//! This module reads the ECMA-335 metadata tables it needs and runs that IL
//! on a symbolic stack. It supports only constants, arrays, dictionaries,
//! lists, constructor calls and static stores. Anything else (branches,
//! `ldsfld` of another static, calls with results, multi-dimensional arrays)
//! becomes [`Tree::Unsupported`] so callers can see exactly what was not
//! read. It never guesses a value.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IlError {
    pub offset: usize,
    pub context: String,
}

impl IlError {
    fn new(offset: usize, context: impl Into<String>) -> Self {
        Self {
            offset,
            context: context.into(),
        }
    }
}

impl std::fmt::Display for IlError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "managed IL at {:#x}: {}",
            self.offset, self.context
        )
    }
}

impl std::error::Error for IlError {}

type Result<T> = std::result::Result<T, IlError>;

/// A fully resolved static value.
#[derive(Debug, Clone, PartialEq)]
pub enum Tree {
    Null,
    /// Integers and booleans, normalized to the declared element or
    /// parameter type. `UInt64` values are stored as their `i64` bit pattern.
    Int(i64),
    Float(f64),
    Str(String),
    Array {
        element: String,
        items: Vec<Tree>,
    },
    /// A constructor call with the constructor's parameter names.
    Object {
        type_name: String,
        args: Vec<(String, Tree)>,
    },
    Map(Vec<(Tree, Tree)>),
    List(Vec<Tree>),
    /// Something the evaluator did not model, with a reason.
    Unsupported(String),
}

impl Tree {
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Tree::Int(value) => Some(*value),
            _ => None,
        }
    }

    pub fn contains_unsupported(&self) -> bool {
        match self {
            Tree::Unsupported(_) => true,
            Tree::Array { items, .. } | Tree::List(items) => {
                items.iter().any(Tree::contains_unsupported)
            }
            Tree::Object { args, .. } => args.iter().any(|(_, value)| value.contains_unsupported()),
            Tree::Map(entries) => entries
                .iter()
                .any(|(k, v)| k.contains_unsupported() || v.contains_unsupported()),
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Metadata
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Col {
    U8,
    U16,
    U32,
    Str,
    Guid,
    Blob,
    Idx(u8),
    Coded(&'static [u8], u32),
}

const NONE: u8 = 0xFF;
const TDOR: Col = Col::Coded(&[0x02, 0x01, 0x1B], 2);
const RES_SCOPE: Col = Col::Coded(&[0x00, 0x1A, 0x23, 0x01], 2);
const HAS_CONST: Col = Col::Coded(&[0x04, 0x08, 0x17], 2);
const HAS_CA: Col = Col::Coded(
    &[
        0x06, 0x04, 0x01, 0x02, 0x08, 0x09, 0x0A, 0x00, 0x0E, 0x17, 0x14, 0x11, 0x1A, 0x1B, 0x20,
        0x23, 0x26, 0x27, 0x28, 0x2A, 0x2C, 0x2B,
    ],
    5,
);
const CA_TYPE: Col = Col::Coded(&[NONE, NONE, 0x06, 0x0A, NONE], 3);
const HAS_FM: Col = Col::Coded(&[0x04, 0x08], 1);
const HAS_DS: Col = Col::Coded(&[0x02, 0x06, 0x20], 2);
const MEMBER_REF_PARENT: Col = Col::Coded(&[0x02, 0x01, 0x1A, 0x06, 0x1B], 3);
const HAS_SEM: Col = Col::Coded(&[0x14, 0x17], 1);
const MDOR: Col = Col::Coded(&[0x06, 0x0A], 1);
const MEMBER_FWD: Col = Col::Coded(&[0x04, 0x06], 1);
const IMPL: Col = Col::Coded(&[0x26, 0x23, 0x27], 2);
const TOMD: Col = Col::Coded(&[0x02, 0x06], 1);

fn schema(table: usize) -> Option<&'static [Col]> {
    use Col::*;
    Some(match table {
        0x00 => &[U16, Str, Guid, Guid, Guid],
        0x01 => &[RES_SCOPE, Str, Str],
        0x02 => &[U32, Str, Str, TDOR, Idx(0x04), Idx(0x06)],
        0x03 => &[Idx(0x04)],
        0x04 => &[U16, Str, Blob],
        0x05 => &[Idx(0x06)],
        0x06 => &[U32, U16, U16, Str, Blob, Idx(0x08)],
        0x07 => &[Idx(0x08)],
        0x08 => &[U16, U16, Str],
        0x09 => &[Idx(0x02), TDOR],
        0x0A => &[MEMBER_REF_PARENT, Str, Blob],
        0x0B => &[U8, U8, HAS_CONST, Blob],
        0x0C => &[HAS_CA, CA_TYPE, Blob],
        0x0D => &[HAS_FM, Blob],
        0x0E => &[U16, HAS_DS, Blob],
        0x0F => &[U16, U32, Idx(0x02)],
        0x10 => &[U32, Idx(0x04)],
        0x11 => &[Blob],
        0x12 => &[Idx(0x02), Idx(0x14)],
        0x13 => &[Idx(0x14)],
        0x14 => &[U16, Str, TDOR],
        0x15 => &[Idx(0x02), Idx(0x17)],
        0x16 => &[Idx(0x17)],
        0x17 => &[U16, Str, Blob],
        0x18 => &[U16, Idx(0x06), HAS_SEM],
        0x19 => &[Idx(0x02), MDOR, MDOR],
        0x1A => &[Str],
        0x1B => &[Blob],
        0x1C => &[U16, MEMBER_FWD, Str, Idx(0x1A)],
        0x1D => &[U32, Idx(0x04)],
        0x1E => &[U32, U32],
        0x1F => &[U32],
        0x20 => &[U32, U16, U16, U16, U16, U32, Blob, Str, Str],
        0x21 => &[U32],
        0x22 => &[U32, U32, U32],
        0x23 => &[U16, U16, U16, U16, U32, Blob, Str, Str, Blob],
        0x24 => &[U32, Idx(0x23)],
        0x25 => &[U32, U32, U32, Idx(0x23)],
        0x26 => &[U32, Str, Blob],
        0x27 => &[U32, U32, Str, Str, IMPL],
        0x28 => &[U32, U32, Str, IMPL],
        0x29 => &[Idx(0x02), Idx(0x02)],
        0x2A => &[U16, U16, TOMD, Str],
        0x2B => &[MDOR, Blob],
        0x2C => &[Idx(0x2A), TDOR],
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy)]
struct Section {
    virtual_address: u32,
    virtual_size: u32,
    raw_offset: u32,
    raw_size: u32,
}

#[derive(Clone, Copy, Default)]
struct TableInfo {
    rows: u32,
    start: usize,
    row_size: usize,
}

/// A parsed managed assembly image (metadata only).
pub struct Assembly<'a> {
    bytes: &'a [u8],
    sections: Vec<Section>,
    strings: (usize, usize),
    blob: (usize, usize),
    user_strings: (usize, usize),
    heap_sizes: u8,
    tables: [TableInfo; 64],
}

fn read_u16(bytes: &[u8], at: usize) -> Result<u16> {
    bytes
        .get(at..at + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or_else(|| IlError::new(at, "unexpected end of image"))
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32> {
    bytes
        .get(at..at + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| IlError::new(at, "unexpected end of image"))
}

fn read_u64(bytes: &[u8], at: usize) -> Result<u64> {
    bytes
        .get(at..at + 8)
        .map(|s| u64::from_le_bytes(s.try_into().unwrap()))
        .ok_or_else(|| IlError::new(at, "unexpected end of image"))
}

impl<'a> Assembly<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        if read_u16(bytes, 0)? != 0x5A4D {
            return Err(IlError::new(0, "missing MZ signature"));
        }
        let pe = read_u32(bytes, 0x3C)? as usize;
        if read_u32(bytes, pe)? != 0x0000_4550 {
            return Err(IlError::new(pe, "missing PE signature"));
        }
        let section_count = read_u16(bytes, pe + 6)? as usize;
        let optional_size = read_u16(bytes, pe + 20)? as usize;
        let optional = pe + 24;
        let plus = match read_u16(bytes, optional)? {
            0x10B => false,
            0x20B => true,
            other => {
                return Err(IlError::new(
                    optional,
                    format!("unknown optional header magic {other:#x}"),
                ));
            }
        };
        let cli_entry = optional + if plus { 112 } else { 96 } + 14 * 8;
        let cli_rva = read_u32(bytes, cli_entry)?;
        let mut sections = Vec::with_capacity(section_count);
        let mut at = optional + optional_size;
        for _ in 0..section_count {
            sections.push(Section {
                virtual_size: read_u32(bytes, at + 8)?,
                virtual_address: read_u32(bytes, at + 12)?,
                raw_size: read_u32(bytes, at + 16)?,
                raw_offset: read_u32(bytes, at + 20)?,
            });
            at += 40;
        }
        let mut assembly = Assembly {
            bytes,
            sections,
            strings: (0, 0),
            blob: (0, 0),
            user_strings: (0, 0),
            heap_sizes: 0,
            tables: [TableInfo::default(); 64],
        };
        if cli_rva == 0 {
            return Err(IlError::new(0, "no CLI header (not a .NET assembly)"));
        }
        let cli = assembly.rva_to_offset(cli_rva)?;
        let metadata_rva = read_u32(bytes, cli + 8)?;
        let metadata = assembly.rva_to_offset(metadata_rva)?;
        if read_u32(bytes, metadata)? != 0x424A_5342 {
            return Err(IlError::new(metadata, "missing BSJB metadata signature"));
        }
        let version_length = read_u32(bytes, metadata + 12)? as usize;
        let mut cursor = metadata + 16 + version_length;
        let stream_count = read_u16(bytes, cursor + 2)? as usize;
        cursor += 4;
        let mut tables_stream = None;
        for _ in 0..stream_count {
            let offset = read_u32(bytes, cursor)? as usize;
            let size = read_u32(bytes, cursor + 4)? as usize;
            let name_start = cursor + 8;
            let mut length = 0;
            while *bytes
                .get(name_start + length)
                .ok_or_else(|| IlError::new(name_start, "unterminated stream name"))?
                != 0
            {
                length += 1;
            }
            let name =
                String::from_utf8_lossy(&bytes[name_start..name_start + length]).into_owned();
            cursor = name_start + (length + 1).div_ceil(4) * 4;
            let range = (metadata + offset, size);
            match name.as_str() {
                "#~" | "#-" => tables_stream = Some(range),
                "#Strings" => assembly.strings = range,
                "#Blob" => assembly.blob = range,
                "#US" => assembly.user_strings = range,
                _ => {}
            }
        }
        let (tables_at, _) = tables_stream
            .ok_or_else(|| IlError::new(metadata, "metadata tables stream missing"))?;
        assembly.heap_sizes = *bytes
            .get(tables_at + 6)
            .ok_or_else(|| IlError::new(tables_at, "truncated tables header"))?;
        let valid = read_u64(bytes, tables_at + 8)?;
        let mut position = tables_at + 24;
        for table in 0..64usize {
            if valid & (1u64 << table) != 0 {
                assembly.tables[table].rows = read_u32(bytes, position)?;
                position += 4;
            }
        }
        for table in 0..64usize {
            if assembly.tables[table].rows == 0 {
                continue;
            }
            let columns = schema(table).ok_or_else(|| {
                IlError::new(table, format!("metadata table {table:#x} schema unknown"))
            })?;
            let size: usize = columns.iter().map(|c| assembly.col_size(*c)).sum();
            assembly.tables[table].row_size = size;
        }
        for table in 0..64usize {
            if assembly.tables[table].rows == 0 {
                continue;
            }
            assembly.tables[table].start = position;
            position += assembly.tables[table].rows as usize * assembly.tables[table].row_size;
        }
        Ok(assembly)
    }

    fn rows(&self, table: u8) -> u32 {
        self.tables.get(table as usize).map_or(0, |t| t.rows)
    }

    fn col_size(&self, col: Col) -> usize {
        let heap = |bit: u8| if self.heap_sizes & bit != 0 { 4 } else { 2 };
        match col {
            Col::U8 => 1,
            Col::U16 => 2,
            Col::U32 => 4,
            Col::Str => heap(1),
            Col::Guid => heap(2),
            Col::Blob => heap(4),
            Col::Idx(table) => {
                if self.rows(table) >= 1 << 16 {
                    4
                } else {
                    2
                }
            }
            Col::Coded(tables, bits) => {
                let max = tables.iter().map(|t| self.rows(*t)).max().unwrap_or(0);
                if max >= 1 << (16 - bits) { 4 } else { 2 }
            }
        }
    }

    fn rva_to_offset(&self, rva: u32) -> Result<usize> {
        for section in &self.sections {
            let span = section.virtual_size.max(section.raw_size);
            if rva >= section.virtual_address && rva < section.virtual_address.saturating_add(span)
            {
                return Ok((rva - section.virtual_address + section.raw_offset) as usize);
            }
        }
        Err(IlError::new(rva as usize, "RVA does not map to a section"))
    }

    /// One row (1-based) as raw column values; coded indexes are returned
    /// undecoded.
    fn row(&self, table: u8, index: u32) -> Result<Vec<u32>> {
        let info = self.tables[table as usize];
        if index == 0 || index > info.rows {
            return Err(IlError::new(
                index as usize,
                format!("row {index} out of range for table {table:#x}"),
            ));
        }
        let columns = schema(table as usize).expect("table has rows so schema exists");
        let mut at = info.start + (index as usize - 1) * info.row_size;
        let mut values = Vec::with_capacity(columns.len());
        for column in columns {
            let size = self.col_size(*column);
            let value = match size {
                1 => *self
                    .bytes
                    .get(at)
                    .ok_or_else(|| IlError::new(at, "truncated table"))?
                    as u32,
                2 => read_u16(self.bytes, at)? as u32,
                _ => read_u32(self.bytes, at)?,
            };
            values.push(value);
            at += size;
        }
        Ok(values)
    }

    fn string(&self, index: u32) -> Result<String> {
        let (start, size) = self.strings;
        let index = index as usize;
        if index >= size {
            return Err(IlError::new(start + index, "string index out of range"));
        }
        let tail = &self.bytes[start + index..start + size];
        let end = tail.iter().position(|b| *b == 0).unwrap_or(tail.len());
        Ok(String::from_utf8_lossy(&tail[..end]).into_owned())
    }

    fn blob(&self, index: u32) -> Result<&'a [u8]> {
        let (start, size) = self.blob;
        let index = index as usize;
        let head = self
            .bytes
            .get(start + index..start + size)
            .ok_or_else(|| IlError::new(start + index, "blob index out of range"))?;
        let (length, used) = compressed(head)?;
        head.get(used..used + length as usize)
            .ok_or_else(|| IlError::new(start + index, "blob exceeds heap"))
    }

    fn user_string(&self, index: u32) -> Result<String> {
        let (start, size) = self.user_strings;
        let index = index as usize;
        let head = self
            .bytes
            .get(start + index..start + size)
            .ok_or_else(|| IlError::new(start + index, "#US index out of range"))?;
        let (length, used) = compressed(head)?;
        let data = head
            .get(used..used + (length as usize).saturating_sub(1))
            .ok_or_else(|| IlError::new(start + index, "#US entry exceeds heap"))?;
        let units: Vec<u16> = data
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        Ok(String::from_utf16_lossy(&units))
    }

    fn decode_coded(&self, value: u32, tables: &[u8], bits: u32) -> Option<(u8, u32)> {
        let tag = (value & ((1 << bits) - 1)) as usize;
        let table = *tables.get(tag)?;
        (table != NONE).then_some((table, value >> bits))
    }

    fn type_def_range(&self, row: u32) -> Result<(u32, u32)> {
        let this = self.row(0x02, row)?;
        let method_end = if row < self.rows(0x02) {
            self.row(0x02, row + 1)?[5]
        } else {
            self.rows(0x06) + 1
        };
        Ok((this[5], method_end))
    }

    fn type_name_of_def(&self, row: u32) -> Result<String> {
        let data = self.row(0x02, row)?;
        let name = self.string(data[1])?;
        let namespace = self.string(data[2])?;
        Ok(if namespace.is_empty() {
            name
        } else {
            format!("{namespace}.{name}")
        })
    }

    fn type_name_of_ref(&self, row: u32) -> Result<String> {
        let data = self.row(0x01, row)?;
        let name = self.string(data[1])?;
        let namespace = self.string(data[2])?;
        Ok(if namespace.is_empty() {
            name
        } else {
            format!("{namespace}.{name}")
        })
    }

    fn type_name_of_coded(&self, value: u32) -> Result<String> {
        match self.decode_coded(value, &[0x02, 0x01, 0x1B], 2) {
            Some((0x02, row)) => self.type_name_of_def(row),
            Some((0x01, row)) => self.type_name_of_ref(row),
            Some((0x1B, row)) => {
                let signature = self.blob(self.row(0x1B, row)?[0])?;
                // GENERICINST (CLASS|VALUETYPE) TypeDefOrRef ...
                if signature.len() >= 3 && signature[0] == 0x15 {
                    let (token, _) = compressed(&signature[2..])?;
                    return self.type_name_of_coded(token);
                }
                Ok("<typespec>".to_owned())
            }
            _ => Ok("<unknown>".to_owned()),
        }
    }

    /// Find a type definition by `Name` or `Namespace.Name`.
    pub fn find_type(&self, wanted: &str) -> Result<Option<u32>> {
        for row in 1..=self.rows(0x02) {
            let full = self.type_name_of_def(row)?;
            let short = self.string(self.row(0x02, row)?[1])?;
            if full == wanted || short == wanted {
                return Ok(Some(row));
            }
        }
        Ok(None)
    }

    /// IL bytes of the named method on a type.
    pub fn method_body(&self, type_row: u32, method: &str) -> Result<Option<Vec<u8>>> {
        let (first, end) = self.type_def_range(type_row)?;
        for method_row in first..end {
            let data = self.row(0x06, method_row)?;
            if self.string(data[3])? != method || data[0] == 0 {
                continue;
            }
            let at = self.rva_to_offset(data[0])?;
            let header = *self
                .bytes
                .get(at)
                .ok_or_else(|| IlError::new(at, "method header past end"))?;
            let (code_start, code_size) = match header & 3 {
                2 => (at + 1, (header >> 2) as usize),
                3 => (at + 12, read_u32(self.bytes, at + 4)? as usize),
                _ => return Err(IlError::new(at, "unknown method header format")),
            };
            return self
                .bytes
                .get(code_start..code_start + code_size)
                .map(|code| Some(code.to_vec()))
                .ok_or_else(|| IlError::new(code_start, "method body past end"));
        }
        Ok(None)
    }

    /// Evaluate a type's `.cctor` and return its static stores by field name.
    pub fn static_initializer(&self, type_name: &str) -> Result<BTreeMap<String, Tree>> {
        let row = self
            .find_type(type_name)?
            .ok_or_else(|| IlError::new(0, format!("type `{type_name}` not found")))?;
        let code = self
            .method_body(row, ".cctor")?
            .ok_or_else(|| IlError::new(0, format!("type `{type_name}` has no .cctor")))?;
        evaluate(&code, self)
    }

    fn method_row_owner(&self, method_row: u32) -> Result<String> {
        for type_row in 1..=self.rows(0x02) {
            let (first, end) = self.type_def_range(type_row)?;
            if (first..end).contains(&method_row) {
                return self.type_name_of_def(type_row);
            }
        }
        Ok("<unknown>".to_owned())
    }
}

fn compressed(bytes: &[u8]) -> Result<(u32, usize)> {
    let first = *bytes
        .first()
        .ok_or_else(|| IlError::new(0, "empty compressed integer"))?;
    let take = |n: usize| {
        bytes
            .get(..n)
            .ok_or_else(|| IlError::new(0, "truncated compressed integer"))
    };
    if first & 0x80 == 0 {
        Ok((first as u32, 1))
    } else if first & 0xC0 == 0x80 {
        let b = take(2)?;
        Ok(((((b[0] & 0x3F) as u32) << 8) | b[1] as u32, 2))
    } else {
        let b = take(4)?;
        Ok((
            (((b[0] & 0x1F) as u32) << 24)
                | ((b[1] as u32) << 16)
                | ((b[2] as u32) << 8)
                | b[3] as u32,
            4,
        ))
    }
}

/// Consume one type signature; return its primitive ELEMENT_TYPE (or `NONE`).
fn skip_type(sig: &[u8], at: &mut usize) -> Result<u8> {
    let code = *sig
        .get(*at)
        .ok_or_else(|| IlError::new(*at, "truncated type signature"))?;
    *at += 1;
    match code {
        0x01..=0x0E | 0x16 | 0x18 | 0x19 | 0x1C => Ok(if code <= 0x0D { code } else { NONE }),
        0x0F | 0x10 | 0x1D => {
            skip_type(sig, at)?;
            Ok(NONE)
        }
        0x11 | 0x12 => {
            let (_, used) = compressed(&sig[*at..])?;
            *at += used;
            Ok(NONE)
        }
        0x13 | 0x1E => {
            let (_, used) = compressed(&sig[*at..])?;
            *at += used;
            Ok(NONE)
        }
        0x15 => {
            skip_type(sig, at)?;
            let (count, used) = compressed(&sig[*at..])?;
            *at += used;
            for _ in 0..count {
                skip_type(sig, at)?;
            }
            Ok(NONE)
        }
        0x14 => {
            skip_type(sig, at)?;
            let (_rank, used) = compressed(&sig[*at..])?;
            *at += used;
            for _ in 0..2 {
                let (count, used) = compressed(&sig[*at..])?;
                *at += used;
                for _ in 0..count {
                    let (_, used) = compressed(&sig[*at..])?;
                    *at += used;
                }
            }
            Ok(NONE)
        }
        0x1F | 0x20 => {
            let (_, used) = compressed(&sig[*at..])?;
            *at += used;
            skip_type(sig, at)
        }
        other => Err(IlError::new(
            *at,
            format!("unsupported signature element {other:#x}"),
        )),
    }
}

struct MethodSig {
    has_this: bool,
    returns_void: bool,
    param_codes: Vec<u8>,
}

fn parse_method_sig(sig: &[u8]) -> Result<MethodSig> {
    let convention = *sig
        .first()
        .ok_or_else(|| IlError::new(0, "empty method signature"))?;
    let mut at = 1;
    if convention & 0x10 != 0 {
        let (_, used) = compressed(&sig[at..])?;
        at += used;
    }
    let (count, used) = compressed(&sig[at..])?;
    at += used;
    let returns_void = sig.get(at) == Some(&0x01);
    skip_type(sig, &mut at)?;
    let mut param_codes = Vec::with_capacity(count as usize);
    for _ in 0..count {
        param_codes.push(skip_type(sig, &mut at)?);
    }
    Ok(MethodSig {
        has_this: convention & 0x20 != 0,
        returns_void,
        param_codes,
    })
}

// ---------------------------------------------------------------------------
// Evaluator
// ---------------------------------------------------------------------------

/// What the evaluator needs to know about a call target.
pub struct MethodInfo {
    pub owner: String,
    pub name: String,
    pub param_names: Vec<String>,
    pub param_codes: Vec<u8>,
    pub has_this: bool,
    pub returns_void: bool,
}

/// Token resolution, separated from IL execution so tests can fake it.
pub trait Resolver {
    fn field_name(&self, token: u32) -> Result<String>;
    fn type_name(&self, token: u32) -> Result<String>;
    fn method(&self, token: u32) -> Result<MethodInfo>;
    fn user_string(&self, token: u32) -> Result<String>;
    /// Raw `FieldRVA` bytes for a field token.
    fn field_data(&self, token: u32, length: usize) -> Result<Vec<u8>>;
}

impl Resolver for Assembly<'_> {
    fn field_name(&self, token: u32) -> Result<String> {
        let row = token & 0x00FF_FFFF;
        match token >> 24 {
            0x04 => self.string(self.row(0x04, row)?[1]),
            0x0A => {
                let data = self.row(0x0A, row)?;
                let owner = match self.decode_coded(data[0], &[0x02, 0x01, 0x1A, 0x06, 0x1B], 3) {
                    Some((0x02, r)) => self.type_name_of_def(r)?,
                    Some((0x01, r)) => self.type_name_of_ref(r)?,
                    _ => "<unknown>".to_owned(),
                };
                Ok(format!("{owner}.{}", self.string(data[1])?))
            }
            other => Err(IlError::new(
                token as usize,
                format!("field token table {other:#x}"),
            )),
        }
    }

    fn type_name(&self, token: u32) -> Result<String> {
        let row = token & 0x00FF_FFFF;
        match token >> 24 {
            0x02 => self.type_name_of_def(row),
            0x01 => self.type_name_of_ref(row),
            0x1B => self.type_name_of_coded((row << 2) | 2),
            other => Err(IlError::new(
                token as usize,
                format!("type token table {other:#x}"),
            )),
        }
    }

    fn method(&self, token: u32) -> Result<MethodInfo> {
        let row = token & 0x00FF_FFFF;
        match token >> 24 {
            0x06 => {
                let data = self.row(0x06, row)?;
                let sig = parse_method_sig(self.blob(data[4])?)?;
                let param_end = if row < self.rows(0x06) {
                    self.row(0x06, row + 1)?[5]
                } else {
                    self.rows(0x08) + 1
                };
                let mut names = vec![String::new(); sig.param_codes.len()];
                for param_row in data[5]..param_end {
                    let param = self.row(0x08, param_row)?;
                    let sequence = param[1] as usize;
                    if sequence >= 1 && sequence <= names.len() {
                        names[sequence - 1] = self.string(param[2])?;
                    }
                }
                for (index, name) in names.iter_mut().enumerate() {
                    if name.is_empty() {
                        *name = format!("arg{index}");
                    }
                }
                Ok(MethodInfo {
                    owner: self.method_row_owner(row)?,
                    name: self.string(data[3])?,
                    param_names: names,
                    param_codes: sig.param_codes,
                    has_this: sig.has_this,
                    returns_void: sig.returns_void,
                })
            }
            0x0A => {
                let data = self.row(0x0A, row)?;
                let sig = parse_method_sig(self.blob(data[2])?)?;
                let owner = self.type_name_of_member_parent(data[0])?;
                Ok(MethodInfo {
                    owner,
                    name: self.string(data[1])?,
                    param_names: (0..sig.param_codes.len())
                        .map(|i| format!("arg{i}"))
                        .collect(),
                    param_codes: sig.param_codes,
                    has_this: sig.has_this,
                    returns_void: sig.returns_void,
                })
            }
            other => Err(IlError::new(
                token as usize,
                format!("method token table {other:#x}"),
            )),
        }
    }

    fn user_string(&self, token: u32) -> Result<String> {
        Assembly::user_string(self, token & 0x00FF_FFFF)
    }

    fn field_data(&self, token: u32, length: usize) -> Result<Vec<u8>> {
        let field = token & 0x00FF_FFFF;
        for row in 1..=self.rows(0x1D) {
            let data = self.row(0x1D, row)?;
            if data[1] == field {
                let at = self.rva_to_offset(data[0])?;
                return self
                    .bytes
                    .get(at..at + length)
                    .map(<[u8]>::to_vec)
                    .ok_or_else(|| IlError::new(at, "FieldRVA data past end"));
            }
        }
        Err(IlError::new(token as usize, "field has no FieldRVA data"))
    }
}

impl Assembly<'_> {
    fn type_name_of_member_parent(&self, value: u32) -> Result<String> {
        match self.decode_coded(value, &[0x02, 0x01, 0x1A, 0x06, 0x1B], 3) {
            Some((0x02, row)) => self.type_name_of_def(row),
            Some((0x01, row)) => self.type_name_of_ref(row),
            Some((0x1B, row)) => {
                let signature = self.blob(self.row(0x1B, row)?[0])?;
                if signature.len() >= 3 && signature[0] == 0x15 {
                    let (token, _) = compressed(&signature[2..])?;
                    return self.type_name_of_coded(token);
                }
                Ok("<typespec>".to_owned())
            }
            _ => Ok("<unknown>".to_owned()),
        }
    }
}

#[derive(Clone, Debug)]
enum Value {
    Null,
    Int(i64),
    Float(f64),
    Str(String),
    Ref(usize),
    FieldHandle(u32),
    Unsupported(String),
}

enum Node {
    Array {
        element: String,
        items: Vec<Value>,
    },
    Object {
        type_name: String,
        args: Vec<(String, Value)>,
    },
    Map(Vec<(Value, Value)>),
    List(Vec<Value>),
}

fn element_code(name: &str) -> u8 {
    match name {
        "System.Boolean" => 0x02,
        "System.Char" => 0x03,
        "System.SByte" => 0x04,
        "System.Byte" => 0x05,
        "System.Int16" => 0x06,
        "System.UInt16" => 0x07,
        "System.Int32" => 0x08,
        "System.UInt32" => 0x09,
        "System.Int64" => 0x0A,
        "System.UInt64" => 0x0B,
        "System.Single" => 0x0C,
        "System.Double" => 0x0D,
        _ => NONE,
    }
}

fn element_size(code: u8) -> Option<usize> {
    match code {
        0x02 | 0x04 | 0x05 => Some(1),
        0x03 | 0x06 | 0x07 => Some(2),
        0x08 | 0x09 | 0x0C => Some(4),
        0x0A | 0x0B | 0x0D => Some(8),
        _ => None,
    }
}

fn normalize(code: u8, value: i64) -> i64 {
    match code {
        0x02 => i64::from(value != 0),
        0x03 | 0x07 => value as u16 as i64,
        0x04 => value as i8 as i64,
        0x05 => value as u8 as i64,
        0x06 => value as i16 as i64,
        0x08 => value as i32 as i64,
        0x09 => value as u32 as i64,
        _ => value,
    }
}

struct Machine {
    heap: Vec<Node>,
    stack: Vec<Value>,
    locals: BTreeMap<u16, Value>,
    statics: BTreeMap<String, Value>,
}

impl Machine {
    fn pop(&mut self, pc: usize) -> Result<Value> {
        self.stack
            .pop()
            .ok_or_else(|| IlError::new(pc, "evaluation stack underflow"))
    }

    fn alloc(&mut self, node: Node) -> Value {
        self.heap.push(node);
        Value::Ref(self.heap.len() - 1)
    }

    fn resolve(&self, value: &Value, depth: usize) -> Tree {
        if depth > 16 {
            return Tree::Unsupported("nesting deeper than 16".to_owned());
        }
        match value {
            Value::Null => Tree::Null,
            Value::Int(v) => Tree::Int(*v),
            Value::Float(v) => Tree::Float(*v),
            Value::Str(v) => Tree::Str(v.clone()),
            Value::FieldHandle(_) => Tree::Unsupported("field handle".to_owned()),
            Value::Unsupported(why) => Tree::Unsupported(why.clone()),
            Value::Ref(index) => match &self.heap[*index] {
                Node::Array { element, items } => Tree::Array {
                    element: element.clone(),
                    items: items.iter().map(|v| self.resolve(v, depth + 1)).collect(),
                },
                Node::Object { type_name, args } => Tree::Object {
                    type_name: type_name.clone(),
                    args: args
                        .iter()
                        .map(|(n, v)| (n.clone(), self.resolve(v, depth + 1)))
                        .collect(),
                },
                Node::Map(entries) => Tree::Map(
                    entries
                        .iter()
                        .map(|(k, v)| (self.resolve(k, depth + 1), self.resolve(v, depth + 1)))
                        .collect(),
                ),
                Node::List(items) => {
                    Tree::List(items.iter().map(|v| self.resolve(v, depth + 1)).collect())
                }
            },
        }
    }
}

fn token_at(code: &[u8], pc: usize) -> Result<u32> {
    read_u32(code, pc)
}

/// Run straight-line IL (a `.cctor`) and return its static stores.
pub fn evaluate(code: &[u8], resolver: &dyn Resolver) -> Result<BTreeMap<String, Tree>> {
    let mut machine = Machine {
        heap: Vec::new(),
        stack: Vec::new(),
        locals: BTreeMap::new(),
        statics: BTreeMap::new(),
    };
    let mut pc = 0usize;
    while pc < code.len() {
        let at = pc;
        let op = code[pc];
        pc += 1;
        match op {
            0x00 => {}
            0x14 => machine.stack.push(Value::Null),
            0x15 => machine.stack.push(Value::Int(-1)),
            0x16..=0x1E => machine.stack.push(Value::Int(i64::from(op - 0x16))),
            0x1F => {
                let value = *code
                    .get(pc)
                    .ok_or_else(|| IlError::new(pc, "truncated ldc.i4.s"))?
                    as i8;
                pc += 1;
                machine.stack.push(Value::Int(i64::from(value)));
            }
            0x20 => {
                machine
                    .stack
                    .push(Value::Int(i64::from(read_u32(code, pc)? as i32)));
                pc += 4;
            }
            0x21 => {
                machine.stack.push(Value::Int(read_u64(code, pc)? as i64));
                pc += 8;
            }
            0x22 => {
                machine
                    .stack
                    .push(Value::Float(f64::from(f32::from_bits(read_u32(code, pc)?))));
                pc += 4;
            }
            0x23 => {
                machine
                    .stack
                    .push(Value::Float(f64::from_bits(read_u64(code, pc)?)));
                pc += 8;
            }
            0x25 => {
                let top = machine
                    .stack
                    .last()
                    .cloned()
                    .ok_or_else(|| IlError::new(at, "dup on empty stack"))?;
                machine.stack.push(top);
            }
            0x26 => {
                machine.pop(at)?;
            }
            0x06..=0x09 => {
                let value = machine
                    .locals
                    .get(&u16::from(op - 0x06))
                    .cloned()
                    .unwrap_or(Value::Unsupported("uninitialized local".to_owned()));
                machine.stack.push(value);
            }
            0x0A..=0x0D => {
                let value = machine.pop(at)?;
                machine.locals.insert(u16::from(op - 0x0A), value);
            }
            0x11 | 0x13 => {
                let index = u16::from(
                    *code
                        .get(pc)
                        .ok_or_else(|| IlError::new(pc, "truncated local index"))?,
                );
                pc += 1;
                if op == 0x11 {
                    let value = machine
                        .locals
                        .get(&index)
                        .cloned()
                        .unwrap_or(Value::Unsupported("uninitialized local".to_owned()));
                    machine.stack.push(value);
                } else {
                    let value = machine.pop(at)?;
                    machine.locals.insert(index, value);
                }
            }
            0x67..=0x6E | 0xD1 | 0xD2 | 0xD3 | 0xE0 => {
                let value = machine.pop(at)?;
                let converted = match (value, op) {
                    (Value::Int(v), 0x67) => Value::Int(v as i8 as i64),
                    (Value::Int(v), 0x68) => Value::Int(v as i16 as i64),
                    (Value::Int(v), 0x69) => Value::Int(v as i32 as i64),
                    (Value::Int(v), 0x6A) => Value::Int(v),
                    (Value::Int(v), 0x6B | 0x6C) => Value::Float(v as f64),
                    (Value::Int(v), 0x6D) => Value::Int(v as u32 as i64),
                    (Value::Int(v), 0x6E) => Value::Int(if (i64::from(i32::MIN)..0).contains(&v) {
                        v as u32 as i64
                    } else {
                        v
                    }),
                    (Value::Int(v), 0xD1) => Value::Int(v as u16 as i64),
                    (Value::Int(v), 0xD2) => Value::Int(v as u8 as i64),
                    (other, _) => other,
                };
                machine.stack.push(converted);
            }
            0x72 => {
                let text = resolver.user_string(token_at(code, pc)?)?;
                pc += 4;
                machine.stack.push(Value::Str(text));
            }
            0x73 => {
                let info = resolver.method(token_at(code, pc)?)?;
                pc += 4;
                let mut args = Vec::with_capacity(info.param_codes.len());
                for _ in 0..info.param_codes.len() {
                    args.push(machine.pop(at)?);
                }
                args.reverse();
                let owner = info.owner.as_str();
                let value = if owner.starts_with("System.Collections.Generic.Dictionary`") {
                    machine.alloc(Node::Map(Vec::new()))
                } else if owner.starts_with("System.Collections.Generic.List`") {
                    machine.alloc(Node::List(Vec::new()))
                } else if owner.starts_with("System.") || owner == "<typespec>" {
                    Value::Unsupported(format!("newobj {owner}::{}", info.name))
                } else {
                    let named = info
                        .param_names
                        .iter()
                        .zip(info.param_codes.iter())
                        .zip(args)
                        .map(|((name, code), value)| {
                            let value = match value {
                                Value::Int(v) => Value::Int(normalize(*code, v)),
                                other => other,
                            };
                            (name.clone(), value)
                        })
                        .collect();
                    machine.alloc(Node::Object {
                        type_name: info.owner.clone(),
                        args: named,
                    })
                };
                machine.stack.push(value);
            }
            0x28 | 0x6F => {
                let info = resolver.method(token_at(code, pc)?)?;
                pc += 4;
                let mut args = Vec::with_capacity(info.param_codes.len());
                for _ in 0..info.param_codes.len() {
                    args.push(machine.pop(at)?);
                }
                args.reverse();
                let target = if info.has_this {
                    Some(machine.pop(at)?)
                } else {
                    None
                };
                if info.name == "InitializeArray"
                    && matches!(args.first(), Some(Value::Unsupported(_)))
                {
                    // A multi-dimensional array this evaluator did not build.
                } else if info.name == "InitializeArray" {
                    let (Some(Value::Ref(index)), Some(Value::FieldHandle(field))) =
                        (args.first().cloned(), args.get(1).cloned())
                    else {
                        return Err(IlError::new(at, "InitializeArray operands"));
                    };
                    let Node::Array { element, items } = &mut machine.heap[index] else {
                        return Err(IlError::new(at, "InitializeArray target is not an array"));
                    };
                    let code_type = element_code(element);
                    let size = element_size(code_type).ok_or_else(|| {
                        IlError::new(at, format!("InitializeArray element `{element}`"))
                    })?;
                    let data = resolver.field_data(field, size * items.len())?;
                    for (item, chunk) in items.iter_mut().zip(data.chunks_exact(size)) {
                        let mut raw = [0u8; 8];
                        raw[..size].copy_from_slice(chunk);
                        let bits = u64::from_le_bytes(raw);
                        *item = match code_type {
                            0x0C => Value::Float(f64::from(f32::from_bits(bits as u32))),
                            0x0D => Value::Float(f64::from_bits(bits)),
                            _ => Value::Int(normalize(code_type, bits as i64)),
                        };
                    }
                } else if info.name == "Add" && info.has_this {
                    if let Some(Value::Ref(index)) = target {
                        match &mut machine.heap[index] {
                            Node::Map(entries) if args.len() == 2 => {
                                entries.push((args[0].clone(), args[1].clone()));
                            }
                            Node::List(items) if args.len() == 1 => items.push(args[0].clone()),
                            _ => {}
                        }
                    }
                    if !info.returns_void {
                        machine
                            .stack
                            .push(Value::Unsupported("Add result".to_owned()));
                    }
                } else if !info.returns_void {
                    machine.stack.push(Value::Unsupported(format!(
                        "call {}::{}",
                        info.owner, info.name
                    )));
                }
            }
            0x7E => {
                let name = resolver.field_name(token_at(code, pc)?)?;
                pc += 4;
                let value = machine
                    .statics
                    .get(&name)
                    .cloned()
                    .unwrap_or_else(|| Value::Unsupported(format!("ldsfld {name}")));
                machine.stack.push(value);
            }
            0x80 => {
                let name = resolver.field_name(token_at(code, pc)?)?;
                pc += 4;
                let value = machine.pop(at)?;
                machine.statics.insert(name, value);
            }
            0x8D => {
                let element = resolver.type_name(token_at(code, pc)?)?;
                pc += 4;
                let length = match machine.pop(at)? {
                    Value::Int(n) if (0..=1 << 24).contains(&n) => n as usize,
                    _ => return Err(IlError::new(at, "newarr with non-constant length")),
                };
                let default = match element_code(&element) {
                    NONE => Value::Null,
                    0x0C | 0x0D => Value::Float(0.0),
                    _ => Value::Int(0),
                };
                let value = machine.alloc(Node::Array {
                    element,
                    items: vec![default; length],
                });
                machine.stack.push(value);
            }
            0xD0 => {
                let token = token_at(code, pc)?;
                pc += 4;
                if token >> 24 != 0x04 {
                    return Err(IlError::new(at, "ldtoken of a non-field token"));
                }
                machine.stack.push(Value::FieldHandle(token));
            }
            0x9B..=0xA2 | 0xA4 => {
                if op == 0xA4 {
                    pc += 4;
                }
                let value = machine.pop(at)?;
                let index = machine.pop(at)?;
                let array = machine.pop(at)?;
                let (Value::Int(index), Value::Ref(heap_index)) = (index, array) else {
                    return Err(IlError::new(at, "stelem operands are not array and index"));
                };
                let Node::Array { element, items } = &mut machine.heap[heap_index] else {
                    return Err(IlError::new(at, "stelem target is not an array"));
                };
                let slot = usize::try_from(index)
                    .ok()
                    .filter(|i| *i < items.len())
                    .ok_or_else(|| IlError::new(at, "stelem index out of range"))?;
                let code_type = element_code(element);
                items[slot] = match value {
                    Value::Int(v) if code_type != NONE => Value::Int(normalize(code_type, v)),
                    other => other,
                };
            }
            0x2A => break,
            other => {
                return Err(IlError::new(at, format!("unsupported opcode {other:#04x}")));
            }
        }
    }
    Ok(machine
        .statics
        .iter()
        .map(|(name, value)| (name.clone(), machine.resolve(value, 0)))
        .collect())
}

#[cfg(test)]
mod tests;
