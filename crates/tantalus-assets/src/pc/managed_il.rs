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

    /// Every type definition's full name, in metadata order.
    pub fn type_names(&self) -> Result<Vec<String>> {
        (1..=self.rows(0x02))
            .map(|row| self.type_name_of_def(row))
            .collect()
    }

    /// Method names of a type definition, in metadata order.
    pub fn method_names(&self, type_row: u32) -> Result<Vec<String>> {
        let (first, end) = self.type_def_range(type_row)?;
        (first..end)
            .map(|method_row| self.string(self.row(0x06, method_row)?[3]))
            .collect()
    }

    /// Field names of a type definition, in metadata order.
    pub fn field_names(&self, type_row: u32) -> Result<Vec<String>> {
        let first = self.row(0x02, type_row)?[4];
        let end = if type_row < self.rows(0x02) {
            self.row(0x02, type_row + 1)?[4]
        } else {
            self.rows(0x04) + 1
        };
        (first..end)
            .map(|field_row| self.string(self.row(0x04, field_row)?[1]))
            .collect()
    }

    /// IL bytes of the `ordinal`-th method (0-based, metadata order) with the
    /// given name, for overloaded methods.
    pub fn method_body_nth(
        &self,
        type_row: u32,
        method: &str,
        ordinal: usize,
    ) -> Result<Option<Vec<u8>>> {
        let (first, end) = self.type_def_range(type_row)?;
        let mut seen = 0usize;
        for method_row in first..end {
            let data = self.row(0x06, method_row)?;
            if self.string(data[3])? != method || data[0] == 0 {
                continue;
            }
            if seen != ordinal {
                seen += 1;
                continue;
            }
            return self.method_body_at(data[0]).map(Some);
        }
        Ok(None)
    }

    fn method_body_at(&self, rva: u32) -> Result<Vec<u8>> {
        let at = self.rva_to_offset(rva)?;
        let header = *self
            .bytes
            .get(at)
            .ok_or_else(|| IlError::new(at, "method header past end"))?;
        let (code_start, code_size) = match header & 3 {
            2 => (at + 1, (header >> 2) as usize),
            3 => (at + 12, read_u32(self.bytes, at + 4)? as usize),
            _ => return Err(IlError::new(at, "unknown method header format")),
        };
        self.bytes
            .get(code_start..code_start + code_size)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| IlError::new(code_start, "method body past end"))
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
        self.static_initializer_seeded(type_name, &BTreeMap::new())
    }

    /// Like [`Self::static_initializer`], with constant statics of other
    /// classes (keyed by field name) available to `ldsfld`.
    pub fn static_initializer_seeded(
        &self,
        type_name: &str,
        seeds: &BTreeMap<String, i64>,
    ) -> Result<BTreeMap<String, Tree>> {
        let row = self
            .find_type(type_name)?
            .ok_or_else(|| IlError::new(0, format!("type `{type_name}` not found")))?;
        let code = self
            .method_body(row, ".cctor")?
            .ok_or_else(|| IlError::new(0, format!("type `{type_name}` has no .cctor")))?;
        evaluate_seeded(&code, self, seeds)
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
    /// Set when the owner is an array type (a `newobj` of `T[,]` and so on).
    pub array: Option<ArrayCtor>,
}

/// An array type's element type and rank, from a type signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrayCtor {
    pub element: String,
    pub rank: usize,
    /// True when the element is a value type, whose unset slots read as 0.
    pub value_type: bool,
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
                    array: None,
                })
            }
            0x0A => {
                let data = self.row(0x0A, row)?;
                let sig = parse_method_sig(self.blob(data[2])?)?;
                let owner = self.type_name_of_member_parent(data[0])?;
                let array = self.array_of_member_parent(data[0])?;
                Ok(MethodInfo {
                    owner,
                    name: self.string(data[1])?,
                    param_names: (0..sig.param_codes.len())
                        .map(|i| format!("arg{i}"))
                        .collect(),
                    param_codes: sig.param_codes,
                    has_this: sig.has_this,
                    returns_void: sig.returns_void,
                    array,
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
    /// Element type and rank when a member reference's parent is an array
    /// type signature (`ELEMENT_TYPE_ARRAY` or `SZARRAY`).
    fn array_of_member_parent(&self, value: u32) -> Result<Option<ArrayCtor>> {
        let Some((0x1B, row)) = self.decode_coded(value, &[0x02, 0x01, 0x1A, 0x06, 0x1B], 3) else {
            return Ok(None);
        };
        let signature = self.blob(self.row(0x1B, row)?[0])?;
        match signature.first() {
            Some(0x14) => {
                let mut at = 1;
                let value_type = matches!(signature.get(at), Some(0x02..=0x0D | 0x11));
                let element = self.type_sig_name(signature, &mut at)?;
                let (rank, _) = compressed(
                    signature
                        .get(at..)
                        .ok_or_else(|| IlError::new(at, "truncated array rank"))?,
                )?;
                Ok(Some(ArrayCtor {
                    element,
                    rank: rank as usize,
                    value_type,
                }))
            }
            Some(0x1D) => {
                let mut at = 1;
                let value_type = matches!(signature.get(at), Some(0x02..=0x0D | 0x11));
                let element = self.type_sig_name(signature, &mut at)?;
                Ok(Some(ArrayCtor {
                    element,
                    rank: 1,
                    value_type,
                }))
            }
            _ => Ok(None),
        }
    }

    /// A readable name for one type signature (primitives by their
    /// `System.*` name, classes and value types by definition name).
    fn type_sig_name(&self, sig: &[u8], at: &mut usize) -> Result<String> {
        let code = *sig
            .get(*at)
            .ok_or_else(|| IlError::new(*at, "truncated type signature"))?;
        match code {
            0x11 | 0x12 => {
                *at += 1;
                let (coded, used) = compressed(&sig[*at..])?;
                *at += used;
                self.type_name_of_coded(coded)
            }
            0x02..=0x0D => {
                *at += 1;
                Ok(match code {
                    0x02 => "System.Boolean",
                    0x03 => "System.Char",
                    0x04 => "System.SByte",
                    0x05 => "System.Byte",
                    0x06 => "System.Int16",
                    0x07 => "System.UInt16",
                    0x08 => "System.Int32",
                    0x09 => "System.UInt32",
                    0x0A => "System.Int64",
                    0x0B => "System.UInt64",
                    0x0C => "System.Single",
                    _ => "System.Double",
                }
                .to_owned())
            }
            0x0E => {
                *at += 1;
                Ok("System.String".to_owned())
            }
            other => Err(IlError::new(
                *at,
                format!("array element signature {other:#x}"),
            )),
        }
    }

    /// Integer `const` fields of a type (`Constant` table rows whose parent is
    /// one of the type's fields), by field name. Only integer element types
    /// are read; others are skipped.
    pub fn literal_constants(&self, type_name: &str) -> Result<BTreeMap<String, i64>> {
        let type_row = self
            .find_type(type_name)?
            .ok_or_else(|| IlError::new(0, format!("type `{type_name}` not found")))?;
        let first = self.row(0x02, type_row)?[4];
        let end = if type_row < self.rows(0x02) {
            self.row(0x02, type_row + 1)?[4]
        } else {
            self.rows(0x04) + 1
        };
        let mut out = BTreeMap::new();
        for row in 1..=self.rows(0x0B) {
            let data = self.row(0x0B, row)?;
            let Some((0x04, field)) = self.decode_coded(data[2], &[0x04, 0x08, 0x17], 2) else {
                continue;
            };
            if !(first..end).contains(&field) {
                continue;
            }
            let blob = self.blob(data[3])?;
            let value = match (data[0] as u8, blob.len()) {
                (0x02 | 0x05, 1) => i64::from(blob[0]),
                (0x04, 1) => i64::from(blob[0] as i8),
                (0x06, 2) => i64::from(i16::from_le_bytes([blob[0], blob[1]])),
                (0x03 | 0x07, 2) => i64::from(u16::from_le_bytes([blob[0], blob[1]])),
                (0x08, 4) => i64::from(i32::from_le_bytes([blob[0], blob[1], blob[2], blob[3]])),
                (0x09, 4) => i64::from(u32::from_le_bytes([blob[0], blob[1], blob[2], blob[3]])),
                _ => continue,
            };
            out.insert(self.string(self.row(0x04, field)?[1])?, value);
        }
        Ok(out)
    }

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
    /// A multi-dimensional array, row-major.
    Dimensional {
        element: String,
        dims: Vec<usize>,
        items: Vec<Value>,
    },
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
    calls: Vec<CallRecord>,
    stores: Vec<(String, Value)>,
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
                Node::Dimensional {
                    element,
                    dims,
                    items,
                } => self.resolve_dimensional(element, dims, items, depth),
            },
        }
    }
}

impl Machine {
    /// A rank-N array becomes nested [`Tree::Array`]s, outermost dimension
    /// first; every level above the leaves names its element `T[,]` (one
    /// comma per remaining dimension beyond the first).
    fn resolve_dimensional(
        &self,
        element: &str,
        dims: &[usize],
        items: &[Value],
        depth: usize,
    ) -> Tree {
        let Some((first, rest)) = dims.split_first() else {
            return Tree::Unsupported("array with no dimensions".to_owned());
        };
        if rest.is_empty() {
            return Tree::Array {
                element: element.to_owned(),
                items: items.iter().map(|v| self.resolve(v, depth + 1)).collect(),
            };
        }
        let stride: usize = rest.iter().product();
        let name = format!("{element}[{}]", ",".repeat(dims.len() - 1));
        Tree::Array {
            element: name,
            items: (0..*first)
                .map(|row| {
                    self.resolve_dimensional(
                        element,
                        rest,
                        &items[row * stride..(row + 1) * stride],
                        depth + 1,
                    )
                })
                .collect(),
        }
    }
}

fn token_at(code: &[u8], pc: usize) -> Result<u32> {
    read_u32(code, pc)
}

/// A call seen while running a method body. The callee is never executed;
/// the record keeps the argument values the evaluator had at that point.
#[derive(Debug, Clone, PartialEq)]
pub struct CallRecord {
    pub offset: usize,
    pub owner: String,
    pub name: String,
    pub args: Vec<Tree>,
}

/// Why a method run ended before `ret`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stop {
    pub offset: usize,
    pub reason: String,
}

/// Starting state for [`run_method`].
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// IL offset to start at (0 for the method start).
    pub entry: usize,
    /// Integer values already on the evaluation stack (bottom first).
    pub stack: Vec<i64>,
    /// Integer locals set before the first instruction.
    pub locals: Vec<(u16, i64)>,
    /// Constant static fields read through `ldsfld` (keyed by field name).
    pub int_statics: BTreeMap<String, i64>,
    /// Static fields that read as `null` until stored.
    pub null_statics: Vec<String>,
    /// Integer arguments by index (`None` leaves the argument unsupported).
    /// An instance method's `this` is index 0.
    pub args: Vec<Option<i64>>,
    /// Instruction budget; 0 means 100000.
    pub step_limit: usize,
}

/// Outcome of [`run_method`].
#[derive(Debug, Clone, PartialEq)]
pub struct MethodRun {
    pub locals: BTreeMap<u16, Tree>,
    pub statics: BTreeMap<String, Tree>,
    /// Every `call`/`callvirt` reached, in order.
    pub calls: Vec<CallRecord>,
    /// `stfld` stores into objects the evaluator does not track, in order.
    pub stores: Vec<(String, Tree)>,
    /// `None` when the method reached `ret`.
    pub stop: Option<Stop>,
    /// The value on top of the stack at `ret` (`None` when the run stopped
    /// early or the stack was empty).
    pub returned: Option<Tree>,
    pub steps: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Static,
    Method,
}

struct Ctx<'a> {
    code: &'a [u8],
    resolver: &'a dyn Resolver,
    mode: Mode,
    int_statics: &'a BTreeMap<String, i64>,
    null_statics: &'a [String],
    args: &'a [Option<i64>],
}

enum Flow {
    Go(usize),
    Halt(String),
    Finish,
}

/// One decoded instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instruction {
    pub offset: usize,
    /// One-byte opcode, or `0xFE00 | second byte` for two-byte opcodes.
    pub opcode: u16,
    /// Raw operand for 4-byte operands (tokens, `ldc.i4`, branch offsets);
    /// the entry count for `switch`.
    pub operand: Option<u32>,
    pub len: usize,
}

fn operand_len(op: u8) -> usize {
    match op {
        0x0E..=0x13 | 0x1F | 0x2B..=0x37 | 0xDE => 1,
        0x20
        | 0x22
        | 0x27..=0x29
        | 0x38..=0x44
        | 0x6F
        | 0x70..=0x75
        | 0x79
        | 0x7B..=0x81
        | 0x8C
        | 0x8D
        | 0x8F
        | 0xA3..=0xA5
        | 0xC2
        | 0xC6
        | 0xD0
        | 0xDD => 4,
        0x21 | 0x23 => 8,
        _ => 0,
    }
}

/// Decode a method body into instructions (no execution).
pub fn instructions(code: &[u8]) -> Result<Vec<Instruction>> {
    let mut out = Vec::new();
    let mut pc = 0usize;
    while pc < code.len() {
        let offset = pc;
        let op = code[pc];
        pc += 1;
        let mut opcode = u16::from(op);
        let mut operand = None;
        if op == 0xFE {
            let sub = *code
                .get(pc)
                .ok_or_else(|| IlError::new(pc, "truncated two-byte opcode"))?;
            pc += 1;
            opcode = 0xFE00 | u16::from(sub);
            pc += match sub {
                0x06 | 0x07 | 0x15 | 0x16 | 0x1C => 4,
                0x09..=0x0E => 2,
                0x12 | 0x19 => 1,
                _ => 0,
            };
        } else if op == 0x45 {
            let count = read_u32(code, pc)? as usize;
            operand = Some(count as u32);
            pc += 4 + 4 * count;
        } else {
            let size = operand_len(op);
            if size == 4 {
                operand = Some(read_u32(code, pc)?);
            }
            pc += size;
        }
        if pc > code.len() {
            return Err(IlError::new(offset, "instruction runs past method end"));
        }
        out.push(Instruction {
            offset,
            opcode,
            operand,
            len: pc - offset,
        });
    }
    Ok(out)
}

/// Run straight-line IL (a `.cctor`) and return its static stores.
pub fn evaluate(code: &[u8], resolver: &dyn Resolver) -> Result<BTreeMap<String, Tree>> {
    evaluate_seeded(code, resolver, &BTreeMap::new())
}

/// Like [`evaluate`], with constant statics from other classes available to
/// `ldsfld` (keyed by field name).
pub fn evaluate_seeded(
    code: &[u8],
    resolver: &dyn Resolver,
    seeds: &BTreeMap<String, i64>,
) -> Result<BTreeMap<String, Tree>> {
    let mut machine = Machine::new();
    let ctx = Ctx {
        code,
        resolver,
        mode: Mode::Static,
        int_statics: seeds,
        null_statics: &[],
        args: &[],
    };
    let mut pc = 0usize;
    while pc < code.len() {
        match step(&mut machine, &ctx, pc)? {
            Flow::Go(next) => pc = next,
            Flow::Finish => break,
            Flow::Halt(reason) => return Err(IlError::new(pc, reason)),
        }
    }
    Ok(machine
        .statics
        .iter()
        .map(|(name, value)| (name.clone(), machine.resolve(value, 0)))
        .collect())
}

/// Run a method body as far as constants allow. Branches on integer
/// constants are followed, calls are recorded but never executed (a
/// non-void call yields an unsupported value), and values read from
/// objects or arguments are unsupported. The run stops, and says where and
/// why, at the first instruction whose effect would need a non-constant
/// value or an opcode outside the supported set; nothing past that point is
/// reported.
pub fn run_method(code: &[u8], resolver: &dyn Resolver, options: &RunOptions) -> Result<MethodRun> {
    let mut machine = Machine::new();
    for value in &options.stack {
        machine.stack.push(Value::Int(*value));
    }
    for (index, value) in &options.locals {
        machine.locals.insert(*index, Value::Int(*value));
    }
    let ctx = Ctx {
        code,
        resolver,
        mode: Mode::Method,
        int_statics: &options.int_statics,
        null_statics: &options.null_statics,
        args: &options.args,
    };
    let limit = if options.step_limit == 0 {
        100_000
    } else {
        options.step_limit
    };
    let mut pc = options.entry;
    let mut steps = 0usize;
    let mut stop = None;
    let mut finished = false;
    while pc < code.len() {
        if steps >= limit {
            stop = Some(Stop {
                offset: pc,
                reason: "step limit reached".to_owned(),
            });
            break;
        }
        steps += 1;
        match step(&mut machine, &ctx, pc)? {
            Flow::Go(next) => pc = next,
            Flow::Finish => {
                finished = true;
                break;
            }
            Flow::Halt(reason) => {
                stop = Some(Stop { offset: pc, reason });
                break;
            }
        }
    }
    let returned = if finished {
        machine.stack.last().map(|value| machine.resolve(value, 0))
    } else {
        None
    };
    Ok(MethodRun {
        returned,
        locals: machine
            .locals
            .iter()
            .map(|(index, value)| (*index, machine.resolve(value, 0)))
            .collect(),
        statics: machine
            .statics
            .iter()
            .map(|(name, value)| (name.clone(), machine.resolve(value, 0)))
            .collect(),
        calls: std::mem::take(&mut machine.calls),
        stores: machine
            .stores
            .iter()
            .map(|(name, value)| (name.clone(), machine.resolve(value, 0)))
            .collect(),
        stop,
        steps,
    })
}

impl Machine {
    fn new() -> Self {
        Self {
            heap: Vec::new(),
            stack: Vec::new(),
            locals: BTreeMap::new(),
            statics: BTreeMap::new(),
            calls: Vec::new(),
            stores: Vec::new(),
        }
    }
}

fn truthy(value: &Value) -> Option<bool> {
    match value {
        Value::Int(v) => Some(*v != 0),
        Value::Null => Some(false),
        Value::Ref(_) | Value::Str(_) => Some(true),
        _ => None,
    }
}

fn unsigned(value: i64) -> u64 {
    if value < 0 {
        u64::from(value as u32)
    } else {
        value as u64
    }
}

fn default_item(element: &str) -> Value {
    match element_code(element) {
        NONE => Value::Null,
        0x0C | 0x0D => Value::Float(0.0),
        _ => Value::Int(0),
    }
}

/// Returns (next instruction offset, jump destination) for a branch at `at`.
fn branch_target(code: &[u8], at: usize, short: bool) -> Result<(usize, usize)> {
    let operand_at = at + 1;
    let (next, delta) = if short {
        let byte = *code
            .get(operand_at)
            .ok_or_else(|| IlError::new(at, "truncated branch"))? as i8;
        (operand_at + 1, i64::from(byte))
    } else {
        (
            operand_at + 4,
            i64::from(read_u32(code, operand_at)? as i32),
        )
    };
    let target = i64::try_from(next)
        .ok()
        .and_then(|n| n.checked_add(delta))
        .and_then(|t| usize::try_from(t).ok())
        .filter(|t| *t <= code.len())
        .ok_or_else(|| IlError::new(at, "branch target outside the method"))?;
    Ok((next, target))
}

/// Execute one instruction. In [`Mode::Static`] anything unsupported is an
/// error; in [`Mode::Method`] it halts the run.
fn step(machine: &mut Machine, ctx: &Ctx<'_>, pc: usize) -> Result<Flow> {
    let code = ctx.code;
    let resolver = ctx.resolver;
    let method = ctx.mode == Mode::Method;
    let at = pc;
    let op = code[pc];
    let mut pc = pc + 1;
    let halt = |reason: &str| -> Result<Flow> {
        if method {
            Ok(Flow::Halt(reason.to_owned()))
        } else {
            Err(IlError::new(at, reason))
        }
    };
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
            let value = if let Some(array) = &info.array {
                if array.rank < 2 || args.len() != array.rank {
                    return halt("newobj of an array type with unsupported dimensions");
                }
                let mut dims = Vec::with_capacity(args.len());
                for arg in &args {
                    match arg {
                        Value::Int(n) if (0..=1 << 24).contains(n) => dims.push(*n as usize),
                        _ => return halt("array dimension is not a constant"),
                    }
                }
                let total = dims.iter().try_fold(1usize, |acc, d| acc.checked_mul(*d));
                let Some(total) = total.filter(|t| *t <= 1 << 24) else {
                    return halt("array larger than 2^24 elements");
                };
                machine.alloc(Node::Dimensional {
                    element: array.element.clone(),
                    dims,
                    items: vec![
                        if array.value_type && element_code(&array.element) == NONE {
                            Value::Int(0)
                        } else {
                            default_item(&array.element)
                        };
                        total
                    ],
                })
            } else if owner.starts_with("System.Collections.Generic.Dictionary`") {
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
            let pure_helper =
                info.name == "InitializeArray" && info.owner.ends_with("RuntimeHelpers");
            if method && !pure_helper {
                let recorded = args.iter().map(|v| machine.resolve(v, 0)).collect();
                machine.calls.push(CallRecord {
                    offset: at,
                    owner: info.owner.clone(),
                    name: info.name.clone(),
                    args: recorded,
                });
            }
            if info.name == "InitializeArray" && info.owner.ends_with("RuntimeHelpers") {
                let (Some(Value::Ref(index)), Some(Value::FieldHandle(field))) =
                    (args.first().cloned(), args.get(1).cloned())
                else {
                    if matches!(args.first(), Some(Value::Unsupported(_))) {
                        // An array this evaluator did not build.
                        return Ok(Flow::Go(pc));
                    }
                    return halt("InitializeArray operands");
                };
                let (element, count) = match &machine.heap[index] {
                    Node::Array { element, items } => (element.clone(), items.len()),
                    Node::Dimensional { element, items, .. } => (element.clone(), items.len()),
                    _ => return halt("InitializeArray target is not an array"),
                };
                let code_type = element_code(&element);
                let Some(size) = element_size(code_type) else {
                    return halt(&format!("InitializeArray element `{element}`"));
                };
                let data = resolver.field_data(field, size * count)?;
                let (Node::Array { items, .. } | Node::Dimensional { items, .. }) =
                    &mut machine.heap[index]
                else {
                    return halt("InitializeArray target is not an array");
                };
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
            } else if info.name == "Set" && info.has_this && args.len() >= 3 {
                let Some(Value::Ref(index)) = target else {
                    return halt("Set on a value that is not a tracked array");
                };
                let Node::Dimensional {
                    element,
                    dims,
                    items,
                } = &mut machine.heap[index]
                else {
                    return halt("Set target is not a multi-dimensional array");
                };
                if args.len() != dims.len() + 1 {
                    return halt("Set argument count does not match the array rank");
                }
                let mut slot = 0usize;
                for (arg, dim) in args.iter().zip(dims.iter()) {
                    let Value::Int(i) = arg else {
                        return halt("Set index is not a constant");
                    };
                    let Some(i) = usize::try_from(*i).ok().filter(|i| i < dim) else {
                        return halt("Set index out of range");
                    };
                    slot = slot * dim + i;
                }
                let code_type = element_code(element);
                let value = args.last().cloned().unwrap_or(Value::Null);
                items[slot] = match value {
                    Value::Int(v) if code_type != NONE => Value::Int(normalize(code_type, v)),
                    other => other,
                };
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
            let value = if let Some(value) = machine.statics.get(&name) {
                value.clone()
            } else if let Some(value) = ctx.int_statics.get(&name) {
                Value::Int(*value)
            } else if ctx.null_statics.contains(&name) {
                Value::Null
            } else {
                Value::Unsupported(format!("ldsfld {name}"))
            };
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
                _ => return halt("newarr with non-constant length"),
            };
            let default = default_item(&element);
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
                return halt("ldtoken of a non-field token");
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
            let Value::Ref(heap_index) = array else {
                if method {
                    // A store into storage the evaluator does not track.
                    return Ok(Flow::Go(pc));
                }
                return halt("stelem operands are not array and index");
            };
            let Value::Int(index) = index else {
                return halt("stelem index is not a constant");
            };
            let Node::Array { element, items } = &mut machine.heap[heap_index] else {
                return halt("stelem target is not an array");
            };
            let Some(slot) = usize::try_from(index).ok().filter(|i| *i < items.len()) else {
                return halt("stelem index out of range");
            };
            let code_type = element_code(element);
            items[slot] = match value {
                Value::Int(v) if code_type != NONE => Value::Int(normalize(code_type, v)),
                other => other,
            };
        }
        0x2A => return Ok(Flow::Finish),
        _ if method => return method_step(machine, ctx, at),
        other => {
            return Err(IlError::new(at, format!("unsupported opcode {other:#04x}")));
        }
    }
    Ok(Flow::Go(pc))
}

fn arg_value(ctx: &Ctx<'_>, index: usize) -> Value {
    match ctx.args.get(index) {
        Some(Some(value)) => Value::Int(*value),
        _ => Value::Unsupported(format!("ldarg {index}")),
    }
}

/// Opcodes only [`run_method`] accepts: arguments, branches on constants,
/// small integer arithmetic, field and element reads of untracked values.
fn method_step(machine: &mut Machine, ctx: &Ctx<'_>, at: usize) -> Result<Flow> {
    let code = ctx.code;
    let op = code[at];
    let mut pc = at + 1;
    match op {
        0x02..=0x05 => {
            let index = usize::from(op - 0x02);
            machine.stack.push(arg_value(ctx, index));
        }
        0x0E => {
            let index = usize::from(
                *code
                    .get(pc)
                    .ok_or_else(|| IlError::new(pc, "truncated ldarg.s"))?,
            );
            pc += 1;
            machine.stack.push(arg_value(ctx, index));
        }
        0x10 => {
            pc += 1;
            machine.pop(at)?;
        }
        0x12 => {
            pc += 1;
            machine
                .stack
                .push(Value::Unsupported("address of local".to_owned()));
        }
        0x2B | 0x38 => {
            let (_, target) = branch_target(code, at, op == 0x2B)?;
            return Ok(Flow::Go(target));
        }
        0x2C | 0x2D | 0x39 | 0x3A => {
            let short = op < 0x38;
            let (next, target) = branch_target(code, at, short)?;
            let value = machine.pop(at)?;
            let Some(flag) = truthy(&value) else {
                return Ok(Flow::Halt(
                    "branch on a value that is not constant".to_owned(),
                ));
            };
            let want = matches!(op, 0x2D | 0x3A);
            return Ok(Flow::Go(if flag == want { target } else { next }));
        }
        0x2E..=0x37 | 0x3B..=0x44 => {
            let short = op < 0x38;
            let (next, target) = branch_target(code, at, short)?;
            let rhs = machine.pop(at)?;
            let lhs = machine.pop(at)?;
            let (Value::Int(a), Value::Int(b)) = (lhs, rhs) else {
                return Ok(Flow::Halt(
                    "compare-and-branch on values that are not constants".to_owned(),
                ));
            };
            let kind = if short { op - 0x2E } else { op - 0x3B };
            let taken = match kind {
                0 => a == b,
                1 => a >= b,
                2 => a > b,
                3 => a <= b,
                4 => a < b,
                5 => a != b,
                6 => unsigned(a) >= unsigned(b),
                7 => unsigned(a) > unsigned(b),
                8 => unsigned(a) <= unsigned(b),
                _ => unsigned(a) < unsigned(b),
            };
            return Ok(Flow::Go(if taken { target } else { next }));
        }
        0x45 => {
            let count = read_u32(code, pc)? as usize;
            let base = pc + 4 + 4 * count;
            let Value::Int(index) = machine.pop(at)? else {
                return Ok(Flow::Halt(
                    "switch on a value that is not constant".to_owned(),
                ));
            };
            if let Some(slot) = usize::try_from(index).ok().filter(|i| *i < count) {
                let delta = i64::from(read_u32(code, pc + 4 + 4 * slot)? as i32);
                let target = i64::try_from(base)
                    .ok()
                    .and_then(|b| b.checked_add(delta))
                    .and_then(|t| usize::try_from(t).ok())
                    .filter(|t| *t <= code.len())
                    .ok_or_else(|| IlError::new(at, "switch target outside the method"))?;
                return Ok(Flow::Go(target));
            }
            return Ok(Flow::Go(base));
        }
        0x58 | 0x59 | 0x5A | 0x5F | 0x60 => {
            let rhs = machine.pop(at)?;
            let lhs = machine.pop(at)?;
            machine.stack.push(match (lhs, rhs) {
                (Value::Int(a), Value::Int(b)) => Value::Int(match op {
                    0x58 => a.wrapping_add(b),
                    0x59 => a.wrapping_sub(b),
                    0x5A => a.wrapping_mul(b),
                    0x5F => a & b,
                    _ => a | b,
                }),
                _ => Value::Unsupported("arithmetic on a value that is not constant".to_owned()),
            });
        }
        0x7B => {
            let name = ctx.resolver.field_name(read_u32(code, pc)?)?;
            pc += 4;
            match machine.pop(at)? {
                Value::Ref(_) => {
                    return Ok(Flow::Halt("ldfld on a tracked object".to_owned()));
                }
                _ => machine
                    .stack
                    .push(Value::Unsupported(format!("ldfld {name}"))),
            }
        }
        0x7D => {
            let name = ctx.resolver.field_name(read_u32(code, pc)?)?;
            pc += 4;
            let value = machine.pop(at)?;
            if matches!(machine.pop(at)?, Value::Ref(_)) {
                return Ok(Flow::Halt("stfld on a tracked object".to_owned()));
            }
            machine.stores.push((name, value));
        }
        0x8E => {
            let length = match machine.pop(at)? {
                Value::Ref(index) => match &machine.heap[index] {
                    Node::Array { items, .. } => Value::Int(items.len() as i64),
                    _ => Value::Unsupported("ldlen on a non-vector".to_owned()),
                },
                _ => Value::Unsupported("ldlen on an untracked value".to_owned()),
            };
            machine.stack.push(length);
        }
        0x90..=0x9A => {
            let index = machine.pop(at)?;
            let array = machine.pop(at)?;
            let value = match (&array, &index) {
                (Value::Ref(heap_index), Value::Int(i)) => match &machine.heap[*heap_index] {
                    Node::Array { items, .. } => {
                        match usize::try_from(*i).ok().and_then(|i| items.get(i)) {
                            Some(item) => item.clone(),
                            None => {
                                return Ok(Flow::Halt("ldelem index out of range".to_owned()));
                            }
                        }
                    }
                    _ => return Ok(Flow::Halt("ldelem on a non-vector".to_owned())),
                },
                _ => Value::Unsupported("ldelem on untracked storage or index".to_owned()),
            };
            machine.stack.push(value);
        }
        0xFE => {
            let sub = *code
                .get(pc)
                .ok_or_else(|| IlError::new(pc, "truncated two-byte opcode"))?;
            pc += 1;
            if !(0x01..=0x05).contains(&sub) {
                return Ok(Flow::Halt(format!("unsupported opcode 0xfe {sub:#04x}")));
            }
            let rhs = machine.pop(at)?;
            let lhs = machine.pop(at)?;
            machine.stack.push(match (lhs, rhs) {
                (Value::Int(a), Value::Int(b)) => Value::Int(i64::from(match sub {
                    0x01 => a == b,
                    0x02 => a > b,
                    0x03 => unsigned(a) > unsigned(b),
                    0x04 => a < b,
                    _ => unsigned(a) < unsigned(b),
                })),
                _ => Value::Unsupported("compare on a value that is not constant".to_owned()),
            });
        }
        other => {
            return Ok(Flow::Halt(format!("unsupported opcode {other:#04x}")));
        }
    }
    Ok(Flow::Go(pc))
}

#[cfg(test)]
mod method_tests;
#[cfg(test)]
mod tests;
