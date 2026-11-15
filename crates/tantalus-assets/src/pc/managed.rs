//! Minimal .NET assembly manifest-resource extraction.
//!
//! Parses enough of the PE/CLI format (PE32 and PE32+, ECMA-335 metadata) to
//! locate the ManifestResource table and read one named embedded resource.
//! This is read-only format compatibility work; no IL is interpreted.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedResource {
    pub name: String,
    pub bytes: usize,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedResourceError {
    pub offset: usize,
    pub context: String,
}

impl ManagedResourceError {
    fn new(offset: usize, context: impl Into<String>) -> Self {
        Self {
            offset,
            context: context.into(),
        }
    }
}

impl std::fmt::Display for ManagedResourceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "managed resource at byte {}: {}",
            self.offset, self.context
        )
    }
}

impl std::error::Error for ManagedResourceError {}

type Result<T> = std::result::Result<T, ManagedResourceError>;

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn at(bytes: &'a [u8], position: usize) -> Self {
        Self { bytes, position }
    }

    fn u8(&mut self) -> Result<u8> {
        let byte = *self
            .bytes
            .get(self.position)
            .ok_or_else(|| ManagedResourceError::new(self.position, "unexpected end of file"))?;
        self.position += 1;
        Ok(byte)
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(length)
            .ok_or_else(|| ManagedResourceError::new(self.position, "range overflow"))?;
        let slice = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| ManagedResourceError::new(self.position, "unexpected end of file"))?;
        self.position = end;
        Ok(slice)
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn seek(&mut self, position: usize) -> Result<()> {
        if position > self.bytes.len() {
            return Err(ManagedResourceError::new(position, "seek past end of file"));
        }
        self.position = position;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct Section {
    virtual_address: u32,
    virtual_size: u32,
    raw_offset: u32,
    raw_size: u32,
}

fn rva_to_offset(sections: &[Section], rva: u32) -> Result<usize> {
    for section in sections {
        let span = section.virtual_size.max(section.raw_size);
        if rva >= section.virtual_address && rva < section.virtual_address.saturating_add(span) {
            return usize::try_from(rva - section.virtual_address + section.raw_offset)
                .map_err(|_| ManagedResourceError::new(rva as usize, "RVA conversion overflow"));
        }
    }
    Err(ManagedResourceError::new(
        rva as usize,
        "RVA does not map to any section",
    ))
}

const TABLE_COUNT: usize = 0x29;

/// Byte size of one table row for the schemas at or below ManifestResource.
fn row_size(table: usize, rows: &[u32; TABLE_COUNT], heap_sizes: u8) -> Result<usize> {
    let heap = |bit: u8| -> usize { if heap_sizes & bit != 0 { 4 } else { 2 } };
    let string = || heap(0x01);
    let guid = || heap(0x02);
    let blob = || heap(0x04);
    let simple = |t: usize| -> usize { if rows[t] >= 1 << 16 { 4 } else { 2 } };
    let coded = |tag_bits: u32, tables: &[usize]| -> usize {
        let max = tables.iter().map(|&t| rows[t]).max().unwrap_or(0);
        if max >= 1 << (16 - tag_bits) { 4 } else { 2 }
    };
    let type_def_or_ref = || coded(2, &[0x02, 0x01, 0x1B]);
    let has_constant = || coded(2, &[0x04, 0x08, 0x17]);
    let has_attribute = || {
        coded(
            5,
            &[
                0x06, 0x04, 0x01, 0x02, 0x08, 0x09, 0x0A, 0x00, 0x0E, 0x17, 0x14, 0x11, 0x1A, 0x1B,
                0x20, 0x23, 0x26, 0x27, 0x28, 0x2A,
            ],
        )
    };
    let attribute_type = || coded(3, &[0x06, 0x0A]);
    let has_field_marshal = || coded(1, &[0x04, 0x08]);
    let has_decl_security = || coded(2, &[0x02, 0x06, 0x20]);
    let member_ref_parent = || coded(3, &[0x02, 0x01, 0x1A, 0x06, 0x1B]);
    let has_semantics = || coded(1, &[0x14, 0x17]);
    let method_def_or_ref = || coded(1, &[0x06, 0x0A]);
    let member_forwarded = || coded(1, &[0x04, 0x06]);
    let implementation = || coded(2, &[0x26, 0x23, 0x27]);
    let resolution_scope = || coded(2, &[0x00, 0x1A, 0x23, 0x01]);

    Ok(match table {
        0x00 => 2 + string() + guid() * 3,
        0x01 => resolution_scope() + string() * 2,
        0x02 => 4 + string() * 2 + type_def_or_ref() + simple(0x04) + simple(0x06),
        0x03 => simple(0x04),
        0x04 => 2 + string() + blob(),
        0x05 => simple(0x06),
        0x06 => 4 + 2 + 2 + string() + blob() + simple(0x08),
        0x07 => simple(0x08),
        0x08 => 2 + 2 + string(),
        0x09 => simple(0x02) + type_def_or_ref(),
        0x0A => member_ref_parent() + string() + blob(),
        0x0B => 2 + has_constant() + blob(),
        0x0C => has_attribute() + attribute_type() + blob(),
        0x0D => has_field_marshal() + blob(),
        0x0E => 2 + has_decl_security() + blob(),
        0x0F => 2 + 4 + simple(0x02),
        0x10 => 4 + simple(0x04),
        0x11 => blob(),
        0x12 => simple(0x02) + simple(0x14),
        0x13 => simple(0x14),
        0x14 => 2 + string() + type_def_or_ref(),
        0x15 => simple(0x02) + simple(0x17),
        0x16 => simple(0x17),
        0x17 => 2 + string() + blob(),
        0x18 => 2 + simple(0x06) + has_semantics(),
        0x19 => simple(0x02) + method_def_or_ref() * 2,
        0x1A => string(),
        0x1B => blob(),
        0x1C => 2 + member_forwarded() + string() + simple(0x1A),
        0x1D => 4 + simple(0x04),
        0x1E => 8,
        0x1F => 4,
        0x20 => 4 + 2 + 2 + 2 + 2 + 4 + blob() + string() * 2,
        0x21 => 4,
        0x22 => 12,
        0x23 => 2 + 2 + 2 + 2 + 4 + blob() + string() * 2 + blob(),
        0x24 => 4 + simple(0x23),
        0x25 => 12 + simple(0x23),
        0x26 => 4 + string() + blob(),
        0x27 => 4 + 4 + string() * 2 + implementation(),
        0x28 => 4 + 4 + string() + implementation(),
        other => {
            return Err(ManagedResourceError::new(
                other,
                "table schema is not implemented",
            ));
        }
    })
}

/// Extract one embedded manifest resource by name from a .NET assembly image.
pub fn extract_resource(bytes: &[u8], name: &str) -> Result<Vec<u8>> {
    let mut cursor = Cursor::new(bytes);
    if cursor.u16()? != 0x5A4D {
        return Err(ManagedResourceError::new(0, "missing MZ signature"));
    }
    cursor.seek(0x3C)?;
    let pe_offset = cursor.u32()? as usize;
    cursor.seek(pe_offset)?;
    if cursor.u32()? != 0x0000_4550 {
        return Err(ManagedResourceError::new(pe_offset, "missing PE signature"));
    }
    let _machine = cursor.u16()?;
    let section_count = cursor.u16()? as usize;
    cursor.take(12)?;
    let optional_size = cursor.u16()? as usize;
    cursor.take(2)?;
    let optional_start = cursor.position;
    let magic = cursor.u16()?;
    let is_pe32_plus = match magic {
        0x10B => false,
        0x20B => true,
        other => {
            return Err(ManagedResourceError::new(
                optional_start,
                format!("unknown optional header magic {other:#x}"),
            ));
        }
    };
    cursor.seek(optional_start + (if is_pe32_plus { 112 } else { 96 }) + 14 * 8)?;
    let cli_rva = cursor.u32()?;
    let _cli_size = cursor.u32()?;
    let optional_end = optional_start + optional_size;
    if cursor.position > optional_end {
        return Err(ManagedResourceError::new(
            optional_start,
            "CLI directory lies outside the optional header",
        ));
    }
    cursor.seek(optional_end)?;
    let mut sections = Vec::with_capacity(section_count);
    for _ in 0..section_count {
        cursor.take(8)?;
        let virtual_size = cursor.u32()?;
        let virtual_address = cursor.u32()?;
        let raw_size = cursor.u32()?;
        let raw_offset = cursor.u32()?;
        cursor.take(16)?;
        sections.push(Section {
            virtual_address,
            virtual_size,
            raw_offset,
            raw_size,
        });
    }
    if cli_rva == 0 {
        return Err(ManagedResourceError::new(
            0,
            "no CLI header (not a .NET assembly)",
        ));
    }
    let cli_offset = rva_to_offset(&sections, cli_rva)?;
    let mut cli = Cursor::at(bytes, cli_offset);
    cli.take(8)?;
    let metadata_rva = cli.u32()?;
    let _metadata_size = cli.u32()?;
    let metadata_offset = rva_to_offset(&sections, metadata_rva)?;

    let mut meta = Cursor::at(bytes, metadata_offset);
    if meta.u32()? != 0x424A_5342 {
        return Err(ManagedResourceError::new(
            metadata_offset,
            "missing BSJB metadata signature",
        ));
    }
    meta.take(8)?;
    let version_length = meta.u32()? as usize;
    meta.take(version_length)?;
    meta.take(2)?;
    let stream_count = meta.u16()? as usize;
    let mut tables: Option<(usize, usize)> = None;
    let mut strings: Option<(usize, usize)> = None;
    let mut stream_end = 0usize;
    for _ in 0..stream_count {
        let stream_offset = meta.u32()? as usize;
        let stream_size = meta.u32()? as usize;
        let name_length = aligned_stream_name_len(&meta)?;
        let name_bytes = meta.take(name_length)?;
        let stream_name = String::from_utf8_lossy(name_bytes)
            .trim_end_matches(' ')
            .to_owned();
        stream_end = stream_end.max(stream_offset + stream_size);
        match stream_name.as_str() {
            "#~" | "#-" => tables = Some((stream_offset, stream_size)),
            "#Strings" => strings = Some((stream_offset, stream_size)),
            _ => {}
        }
    }
    let resources_base = (metadata_offset + stream_end + 3) & !3;
    let (tables_offset, _) = tables.ok_or_else(|| {
        ManagedResourceError::new(metadata_offset, "metadata tables stream is missing")
    })?;
    let (strings_offset, strings_size) = strings
        .ok_or_else(|| ManagedResourceError::new(metadata_offset, "#Strings stream is missing"))?;

    let tables_absolute = metadata_offset + tables_offset;
    let mut tables_cursor = Cursor::at(bytes, tables_absolute);
    let _reserved = tables_cursor.u32()?;
    let major = tables_cursor.u8()?;
    let minor = tables_cursor.u8()?;
    let _ = (major, minor);
    let heap_sizes = tables_cursor.u8()?;
    tables_cursor.take(1)?;
    let valid = tables_cursor.u64()?;
    let _sorted = tables_cursor.u64()?;
    let mut rows = [0u32; TABLE_COUNT];
    for (table, row_count) in rows.iter_mut().enumerate() {
        if valid & (1u64 << table) != 0 {
            *row_count = tables_cursor.u32()?;
        }
    }
    eprintln!(
        "DEBUG heap={heap_sizes:#x} valid={valid:#x} rows28={} tables_abs={tables_absolute:#x}",
        rows[0x28]
    );
    if valid & (1u64 << 0x28) == 0 || rows[0x28] == 0 {
        return Err(ManagedResourceError::new(
            tables_absolute,
            "assembly declares no manifest resources",
        ));
    }
    let mut table_rows_start = tables_cursor.position;
    for table in 0..0x28 {
        if valid & (1u64 << table) != 0 {
            let size = row_size(table, &rows, heap_sizes)?
                .checked_mul(rows[table] as usize)
                .ok_or_else(|| {
                    ManagedResourceError::new(table_rows_start, "table size overflow")
                })?;
            table_rows_start = table_rows_start.checked_add(size).ok_or_else(|| {
                ManagedResourceError::new(table_rows_start, "table size overflow")
            })?;
        }
    }

    let resource_name = |index: usize| -> Result<String> {
        if index >= strings_size {
            return Err(ManagedResourceError::new(
                strings_offset + index,
                "string heap index is out of range",
            ));
        }
        let base = strings_offset + index;
        let tail = &bytes[base..strings_offset + strings_size];
        let terminator = tail
            .iter()
            .position(|&byte| byte == 0)
            .ok_or_else(|| ManagedResourceError::new(base, "unterminated string heap entry"))?;
        Ok(String::from_utf8_lossy(&tail[..terminator]).into_owned())
    };

    let mut found = None;
    let mut rows_cursor = Cursor::at(bytes, table_rows_start);
    for _ in 0..rows[0x28] {
        let offset = rows_cursor.u32()? as usize;
        let _flags = rows_cursor.u32()?;
        let name_index = if heap_sizes & 0x01 != 0 {
            rows_cursor.u32()? as usize
        } else {
            rows_cursor.u16()? as usize
        };
        let implementation_size =
            if rows[0x26] >= 1 << 14 || rows[0x23] >= 1 << 14 || rows[0x27] >= 1 << 14 {
                4
            } else {
                2
            };
        rows_cursor.take(implementation_size)?;
        if resource_name(name_index)? == name {
            found = Some(offset);
        }
    }
    let offset = found.ok_or_else(|| {
        ManagedResourceError::new(table_rows_start, format!("resource `{name}` was not found"))
    })?;

    // Embedded resources live in the region that follows the metadata
    // streams; the ManifestResource offset is relative to that base.
    let mut resource = Cursor::at(bytes, resources_base + offset);
    let name_length = resource.u32()? as usize;
    if name_length & 0x8000_0000 != 0 {
        return Err(ManagedResourceError::new(
            resource.position - 4,
            "UTF-16 resource names are not supported",
        ));
    }
    resource.take(name_length)?;
    while !resource.position.is_multiple_of(4) {
        resource.u8()?;
    }
    let data_length = resource.u32()? as usize;
    Ok(resource.take(data_length)?.to_vec())
}

fn aligned_stream_name_len(cursor: &Cursor<'_>) -> Result<usize> {
    // Stream names are NUL-terminated and padded to a four-byte boundary
    // relative to the stream header start; we re-scan from the name field.
    let start = cursor.position;
    let mut length = 0usize;
    loop {
        let byte = *cursor
            .bytes
            .get(start + length)
            .ok_or_else(|| ManagedResourceError::new(start, "unterminated stream name"))?;
        length += 1;
        if byte == 0 {
            break;
        }
    }
    while !(start + length).is_multiple_of(4) {
        length += 1;
    }
    Ok(length)
}

#[cfg(test)]
mod tests;
