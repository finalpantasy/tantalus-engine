//! World-map data from `p0data3.bin` (milestone W7).
//!
//! The bundle holds two kinds of data:
//!
//! * Unity objects: per-block terrain/sea/object `Mesh` assets, one prefab per
//!   block (a `WMBlockPrefab` MonoBehaviour that points at the block's meshes),
//!   materials, textures and render textures. See [`unity`].
//! * Raw binary `TextAsset`s under `worldmap/wmap/`: the PSX-era disc image
//!   (`disc.img`/`discmr.img`: a 256-slot sector directory over packs of TIM
//!   textures and the "frame pack"), the standalone `.ntp` packs that are the same
//!   sub-files, `.hed`/`.nwp` model tables, `fx.tcb` and `map.bin`.
//!
//! Memoria reads only the frame pack (disc image slot 1, `ff9.w_worldSystemConstructor`):
//! encounter table (pack 3), special encounters (pack 4), texture animation
//! project (pack 6), effect data lists (packs 41..=52), `Eva` (pack 37) and the sea
//! block's `sNWBBlockHeader` (pack 66). Those are typed here. Everything else is
//! parsed structurally (sector directory, pack offset tables, TIM headers) and the
//! remaining bytes are kept as named raw regions, never silently dropped.

pub mod unity;

use crate::install::ImportError;
use serde::Serialize;

pub const SECTOR_BYTES: usize = 2048;
pub const IMAGE_SLOTS: usize = 256;
/// Image slot that holds the frame pack (`ff9.w_fileImageSectorInfo[1]`).
pub const FRAME_PACK_SLOT: usize = 1;
pub const ENCOUNTER_RECORDS: usize = 355;
pub const ENCOUNTER_RECORD_BYTES: usize = 10;
pub const SPECIAL_ENCOUNTER_GROUPS: usize = 9;
pub const SPECIAL_ENCOUNTER_AREAS: usize = 12;
pub const EFFECT_RECORD_BYTES: usize = 64;
pub const EFFECT_LIST_PACKS: std::ops::RangeInclusive<usize> = 41..=52;
pub const SPECIAL_AREA_UNUSED: u16 = 0xFFFF;
pub const ENCOUNTER_PACK: usize = 3;
pub const SPECIAL_ENCOUNTER_PACK: usize = 4;
pub const TEXTURE_PROJECT_PACK: usize = 6;
pub const EVA_PACK: usize = 37;
pub const SEA_BLOCK_PACK: usize = 66;
pub const NWB_CELL_HEADER_BYTES: usize = 36;
pub const NWB_SURFACE_BYTES: usize = 16;
pub const NWB_VERTEX_BYTES: usize = 8;
/// `map.bin`: one byte per world block, 24 columns by 20 rows.
pub const MAP_GRID_WIDTH: usize = 24;
pub const MAP_GRID_HEIGHT: usize = 20;

fn bad(source: &str, message: impl std::fmt::Display) -> ImportError {
    ImportError::invalid(source, message)
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
    source: &'a str,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8], source: &'a str) -> Self {
        Self {
            bytes,
            position: 0,
            source,
        }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], ImportError> {
        let end = self
            .position
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| {
                bad(
                    self.source,
                    format!(
                        "truncated: need {count} bytes at {} of {}",
                        self.position,
                        self.bytes.len()
                    ),
                )
            })?;
        let slice = &self.bytes[self.position..end];
        self.position = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, ImportError> {
        Ok(self.take(1)?[0])
    }

    fn i8(&mut self) -> Result<i8, ImportError> {
        Ok(self.u8()? as i8)
    }

    fn u16(&mut self) -> Result<u16, ImportError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().expect("2")))
    }

    fn i16(&mut self) -> Result<i16, ImportError> {
        Ok(self.u16()? as i16)
    }

    fn u32(&mut self) -> Result<u32, ImportError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("4")))
    }

    fn i32(&mut self) -> Result<i32, ImportError> {
        Ok(self.u32()? as i32)
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }
}

fn nonzero_len(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .rposition(|byte| *byte != 0)
        .map_or(0, |last| last + 1)
}

// ---------------------------------------------------------------------------
// Disc image: 256 x (start sector, sector count) directory in sector 0.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ImageEntry {
    pub slot: usize,
    pub start_sector: u32,
    pub sector_count: u32,
}

impl ImageEntry {
    pub fn byte_range(&self) -> std::ops::Range<usize> {
        let start = self.start_sector as usize * SECTOR_BYTES;
        start..start + self.sector_count as usize * SECTOR_BYTES
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiscImage {
    pub total_sectors: u32,
    pub entries: Vec<ImageEntry>,
    /// Sectors after sector 0 that no entry covers.
    pub uncovered_sectors: u32,
}

impl DiscImage {
    pub fn parse(bytes: &[u8], source: &str) -> Result<Self, ImportError> {
        if bytes.len() < SECTOR_BYTES || !bytes.len().is_multiple_of(SECTOR_BYTES) {
            return Err(bad(
                source,
                format!(
                    "disc image length {} is not a whole number of sectors",
                    bytes.len()
                ),
            ));
        }
        let total_sectors = (bytes.len() / SECTOR_BYTES) as u32;
        let mut reader = Reader::new(&bytes[..SECTOR_BYTES], source);
        let mut entries: Vec<ImageEntry> = Vec::new();
        let mut covered = 0_u32;
        for slot in 0..IMAGE_SLOTS {
            let start = reader.i32()?;
            let count = reader.i32()?;
            if count == 0 {
                if start != 0 {
                    return Err(bad(source, format!("empty slot {slot} has start {start}")));
                }
                continue;
            }
            if start < 1 || count < 0 || (start as i64 + count as i64) > i64::from(total_sectors) {
                return Err(bad(
                    source,
                    format!("slot {slot} range {start}+{count} exceeds {total_sectors} sectors"),
                ));
            }
            if entries.last().is_some_and(|previous| {
                (start as u32) < previous.start_sector + previous.sector_count
            }) {
                return Err(bad(
                    source,
                    format!("slot {slot} overlaps the previous slot"),
                ));
            }
            covered += count as u32;
            entries.push(ImageEntry {
                slot,
                start_sector: start as u32,
                sector_count: count as u32,
            });
        }
        Ok(Self {
            total_sectors,
            entries,
            uncovered_sectors: total_sectors - 1 - covered,
        })
    }

    pub fn entry<'a>(&self, bytes: &'a [u8], slot: usize) -> Option<&'a [u8]> {
        let entry = self.entries.iter().find(|entry| entry.slot == slot)?;
        bytes.get(entry.byte_range())
    }
}

// ---------------------------------------------------------------------------
// Pack: i32 count, `count` ascending offsets, segment data (last segment runs to
// the end of the buffer, which is sector padded inside a disc image).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pack<'a> {
    pub bytes: &'a [u8],
    pub offsets: Vec<u32>,
}

impl<'a> Pack<'a> {
    pub fn parse(bytes: &'a [u8], source: &str) -> Result<Self, ImportError> {
        let mut reader = Reader::new(bytes, source);
        let count = reader.i32()?;
        if !(1..=4096).contains(&count) {
            return Err(bad(source, format!("implausible pack count {count}")));
        }
        let count = count as usize;
        let mut offsets = Vec::with_capacity(count);
        for _ in 0..count {
            offsets.push(reader.u32()?);
        }
        let table_end = 4 + 4 * count as u32;
        if offsets[0] != table_end {
            return Err(bad(
                source,
                format!(
                    "first pack offset {} is not the table end {table_end}",
                    offsets[0]
                ),
            ));
        }
        for pair in offsets.windows(2) {
            if pair[0] > pair[1] {
                return Err(bad(source, "pack offsets are not ascending"));
            }
        }
        if offsets[count - 1] as usize > bytes.len() {
            return Err(bad(source, "last pack offset is beyond the buffer"));
        }
        Ok(Self { bytes, offsets })
    }

    pub fn count(&self) -> usize {
        self.offsets.len()
    }

    pub fn segment(&self, index: usize) -> Option<&'a [u8]> {
        let start = *self.offsets.get(index)? as usize;
        let end = self
            .offsets
            .get(index + 1)
            .map_or(self.bytes.len(), |end| *end as usize);
        self.bytes.get(start..end)
    }
}

// ---------------------------------------------------------------------------
// PSX TIM image inside pack segments.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Tim {
    /// 4, 8, 16 or 24.
    pub bits_per_pixel: u8,
    /// CLUT block rectangle `[x, y, width, height]` in 16-bit words.
    pub clut: Option<[u16; 4]>,
    /// Image block rectangle `[x, y, width, height]`; width counts 16-bit words.
    pub image: [u16; 4],
    pub consumed_bytes: usize,
}

impl Tim {
    pub fn parse(bytes: &[u8], source: &str) -> Result<Self, ImportError> {
        let mut reader = Reader::new(bytes, source);
        let magic = reader.u32()?;
        if magic != 0x10 {
            return Err(bad(source, format!("TIM magic {magic:#x}")));
        }
        let flag = reader.u32()?;
        if flag & !0b1011 != 0 {
            return Err(bad(source, format!("TIM flag {flag:#x}")));
        }
        let bits_per_pixel = match flag & 3 {
            0 => 4,
            1 => 8,
            2 => 16,
            _ => 24,
        };
        let block = |reader: &mut Reader<'_>, what: &str| -> Result<[u16; 4], ImportError> {
            let size = reader.u32()? as usize;
            let rect = [reader.u16()?, reader.u16()?, reader.u16()?, reader.u16()?];
            let data = usize::from(rect[2]) * usize::from(rect[3]) * 2;
            if size != 12 + data {
                return Err(bad(
                    source,
                    format!("TIM {what} block size {size} does not match {rect:?}"),
                ));
            }
            reader.take(data)?;
            Ok(rect)
        };
        let clut = if flag & 8 != 0 {
            Some(block(&mut reader, "CLUT")?)
        } else {
            None
        };
        let image = block(&mut reader, "image")?;
        Ok(Self {
            bits_per_pixel,
            clut,
            image,
            consumed_bytes: reader.position,
        })
    }
}

// ---------------------------------------------------------------------------
// Frame pack contents defined by Memoria (`WMBinarayReaderExtension`).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EncounterRecord {
    pub scene: [u16; 4],
    pub pattern: u8,
    pub pad: u8,
}

/// 355 records of 10 bytes. Returns the records and the unread tail length.
pub fn parse_encounters(
    bytes: &[u8],
    source: &str,
) -> Result<(Vec<EncounterRecord>, usize), ImportError> {
    let mut reader = Reader::new(bytes, source);
    let mut records = Vec::with_capacity(ENCOUNTER_RECORDS);
    for _ in 0..ENCOUNTER_RECORDS {
        records.push(EncounterRecord {
            scene: [reader.u16()?, reader.u16()?, reader.u16()?, reader.u16()?],
            pattern: reader.u8()?,
            pad: reader.u8()?,
        });
    }
    Ok((records, reader.remaining()))
}

/// 9 groups of 12 one-based encounter areas. `0xFFFF` marks an unused slot (Memoria's
/// `area - 1` is then out of range and skipped); every other area must index
/// [`ENCOUNTER_RECORDS`].
pub fn parse_special_encounters(
    bytes: &[u8],
    source: &str,
) -> Result<(Vec<[u16; SPECIAL_ENCOUNTER_AREAS]>, usize), ImportError> {
    let mut reader = Reader::new(bytes, source);
    let mut groups = Vec::with_capacity(SPECIAL_ENCOUNTER_GROUPS);
    for _ in 0..SPECIAL_ENCOUNTER_GROUPS {
        let mut area = [0_u16; SPECIAL_ENCOUNTER_AREAS];
        for slot in &mut area {
            *slot = reader.u16()?;
            if *slot != SPECIAL_AREA_UNUSED && usize::from(*slot) > ENCOUNTER_RECORDS {
                return Err(bad(
                    source,
                    format!("special encounter area {slot} exceeds {ENCOUNTER_RECORDS}"),
                ));
            }
        }
        groups.push(area);
    }
    Ok((groups, reader.remaining()))
}

/// `stexturePixelScroll`/`Anime`/`PaletScroll`: five `u16` each.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextureProject {
    pub pixel_scroll: Vec<[u16; 5]>,
    pub pixel_anime: Vec<[u16; 5]>,
    pub palette_scroll: Vec<[u16; 5]>,
}

pub fn parse_texture_project(
    bytes: &[u8],
    source: &str,
) -> Result<(TextureProject, usize), ImportError> {
    let mut reader = Reader::new(bytes, source);
    let mut read = |count: usize| -> Result<Vec<[u16; 5]>, ImportError> {
        (0..count)
            .map(|_| {
                Ok([
                    reader.u16()?,
                    reader.u16()?,
                    reader.u16()?,
                    reader.u16()?,
                    reader.u16()?,
                ])
            })
            .collect()
    };
    let project = TextureProject {
        pixel_scroll: read(8)?,
        pixel_anime: read(4)?,
        palette_scroll: read(18)?,
    };
    Ok((project, reader.remaining()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EffectData {
    pub position: [i32; 3],
    pub rotation: [i32; 3],
    pub velocity: [i16; 3],
    pub acceleration: [i16; 3],
    /// `SPSConst.WorldSPSEffect`; -1 terminates a list.
    pub effect: i16,
    pub size: i16,
    pub random: i16,
    pub temp: i16,
    pub pad: [i32; 5],
}

fn read_effect(reader: &mut Reader<'_>) -> Result<EffectData, ImportError> {
    Ok(EffectData {
        position: [reader.i32()?, reader.i32()?, reader.i32()?],
        rotation: [reader.i32()?, reader.i32()?, reader.i32()?],
        velocity: [reader.i16()?, reader.i16()?, reader.i16()?],
        acceleration: [reader.i16()?, reader.i16()?, reader.i16()?],
        effect: reader.i16()?,
        size: reader.i16()?,
        random: reader.i16()?,
        temp: reader.i16()?,
        pad: [
            reader.i32()?,
            reader.i32()?,
            reader.i32()?,
            reader.i32()?,
            reader.i32()?,
        ],
    })
}

/// Records until the terminator (`effect == -1`, the terminator is consumed).
pub fn parse_effect_list(
    bytes: &[u8],
    source: &str,
) -> Result<(Vec<EffectData>, usize), ImportError> {
    let mut reader = Reader::new(bytes, source);
    let mut list = Vec::new();
    loop {
        let record = read_effect(&mut reader)?;
        if record.effect == -1 {
            break;
        }
        list.push(record);
    }
    Ok((list, reader.remaining()))
}

/// One 16-byte `sNWBBlockHeader` surface (PSX `POLY_GT3`-shaped; bytes after the
/// vertex indices are kept as raw named fields).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct NwbSurface {
    pub vertex: [u8; 3],
    pub flags: u8,
    pub uv: [[u8; 2]; 3],
    /// Raw `u16` after each UV pair (CLUT, texture page, pad on a GT3).
    pub tail: [u16; 3],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NwbCell {
    pub vertex_count: u8,
    pub area_id: u8,
    pub surface_count: u16,
    pub vertex_offset: u32,
    pub surface_offset: u32,
    pub checkpoint: [u8; 20],
    pub check_figure: u8,
    pub special: [u8; 3],
    /// `SVECTOR` x, y, z, pad.
    pub vertices: Vec<[i16; 4]>,
    pub surfaces: Vec<NwbSurface>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NwbBlock {
    pub cell_count: i8,
    pub texture_set: i8,
    pub area_id: i8,
    pub special: u8,
    pub texture_offset: u32,
    pub cell_offsets: Vec<u32>,
    pub cells: Vec<NwbCell>,
    /// Bytes covered by the header, cell headers, vertices and surfaces.
    pub structured_bytes: usize,
    /// Bytes between the last structure and `texture_offset`, and how many of them
    /// are non-zero.
    pub gap_bytes: usize,
    pub gap_nonzero: usize,
    /// Bytes from `texture_offset` to the end of the buffer (raw texture region).
    pub texture_region_bytes: usize,
    /// Non-zero length of the texture region (0 = all zero).
    pub texture_region_nonzero: usize,
}

impl NwbBlock {
    /// All offsets are relative to the start of `bytes` (the block). Surface vertex
    /// indices must index the cell's vertices and the cell ranges must not overlap.
    pub fn parse(bytes: &[u8], source: &str) -> Result<Self, ImportError> {
        let mut reader = Reader::new(bytes, source);
        let cell_count = reader.i8()?;
        let texture_set = reader.i8()?;
        let area_id = reader.i8()?;
        let special = reader.u8()?;
        let texture_offset = reader.u32()?;
        if cell_count < 0 {
            return Err(bad(source, format!("negative cell count {cell_count}")));
        }
        let mut cell_offsets = Vec::new();
        for _ in 0..cell_count {
            cell_offsets.push(reader.u32()?);
        }
        let header_end = reader.position;
        if texture_offset as usize > bytes.len() || (texture_offset as usize) < header_end {
            return Err(bad(
                source,
                format!("texture offset {texture_offset} out of range"),
            ));
        }
        let mut ranges: Vec<(usize, usize)> = vec![(0, header_end)];
        let mut cells = Vec::new();
        for (number, offset) in cell_offsets.iter().enumerate() {
            let at = *offset as usize;
            let head = bytes.get(at..at + NWB_CELL_HEADER_BYTES).ok_or_else(|| {
                bad(
                    source,
                    format!("cell {number} header at {at} is out of range"),
                )
            })?;
            let mut cell_reader = Reader::new(head, source);
            let vertex_count = cell_reader.u8()?;
            let cell_area = cell_reader.u8()?;
            let surface_count = cell_reader.u16()?;
            let vertex_offset = cell_reader.u32()?;
            let surface_offset = cell_reader.u32()?;
            let checkpoint: [u8; 20] = cell_reader.take(20)?.try_into().expect("20");
            let check_figure = cell_reader.u8()?;
            let cell_special: [u8; 3] = cell_reader.take(3)?.try_into().expect("3");

            let vertex_bytes = usize::from(vertex_count) * NWB_VERTEX_BYTES;
            let surface_bytes = usize::from(surface_count) * NWB_SURFACE_BYTES;
            let vertex_data = bytes
                .get(vertex_offset as usize..vertex_offset as usize + vertex_bytes)
                .ok_or_else(|| bad(source, format!("cell {number} vertices out of range")))?;
            let surface_data = bytes
                .get(surface_offset as usize..surface_offset as usize + surface_bytes)
                .ok_or_else(|| bad(source, format!("cell {number} surfaces out of range")))?;
            let mut vertices = Vec::with_capacity(usize::from(vertex_count));
            let mut vertex_reader = Reader::new(vertex_data, source);
            for _ in 0..vertex_count {
                vertices.push([
                    vertex_reader.i16()?,
                    vertex_reader.i16()?,
                    vertex_reader.i16()?,
                    vertex_reader.i16()?,
                ]);
            }
            let mut surfaces = Vec::with_capacity(usize::from(surface_count));
            let mut surface_reader = Reader::new(surface_data, source);
            for surface in 0..surface_count {
                let vertex = [
                    surface_reader.u8()?,
                    surface_reader.u8()?,
                    surface_reader.u8()?,
                ];
                if let Some(index) = vertex.iter().find(|index| **index >= vertex_count) {
                    return Err(bad(
                        source,
                        format!("cell {number} surface {surface} vertex {index} >= {vertex_count}"),
                    ));
                }
                let flags = surface_reader.u8()?;
                let uv0 = [surface_reader.u8()?, surface_reader.u8()?];
                let tail0 = surface_reader.u16()?;
                let uv1 = [surface_reader.u8()?, surface_reader.u8()?];
                let tail1 = surface_reader.u16()?;
                let uv2 = [surface_reader.u8()?, surface_reader.u8()?];
                let tail2 = surface_reader.u16()?;
                surfaces.push(NwbSurface {
                    vertex,
                    flags,
                    uv: [uv0, uv1, uv2],
                    tail: [tail0, tail1, tail2],
                });
            }
            ranges.push((at, at + NWB_CELL_HEADER_BYTES));
            ranges.push((
                vertex_offset as usize,
                vertex_offset as usize + vertex_bytes,
            ));
            ranges.push((
                surface_offset as usize,
                surface_offset as usize + surface_bytes,
            ));
            cells.push(NwbCell {
                vertex_count,
                area_id: cell_area,
                surface_count,
                vertex_offset,
                surface_offset,
                checkpoint,
                check_figure,
                special: cell_special,
                vertices,
                surfaces,
            });
        }
        ranges.sort_unstable();
        let mut structured_bytes = 0;
        let mut end = 0;
        let mut gap_bytes = 0;
        let mut gap_nonzero = 0;
        for (start, stop) in ranges {
            if start < end {
                return Err(bad(source, "block structures overlap"));
            }
            gap_bytes += start - end;
            gap_nonzero += bytes[end..start].iter().filter(|byte| **byte != 0).count();
            structured_bytes += stop - start;
            end = stop;
        }
        gap_bytes += texture_offset as usize - end.min(texture_offset as usize);
        if end > texture_offset as usize {
            return Err(bad(source, "block structures run into the texture region"));
        }
        let texture_region = &bytes[texture_offset as usize..];
        Ok(Self {
            cell_count,
            texture_set,
            area_id,
            special,
            texture_offset,
            cell_offsets,
            cells,
            structured_bytes,
            gap_bytes,
            gap_nonzero,
            texture_region_bytes: texture_region.len(),
            texture_region_nonzero: nonzero_len(texture_region),
        })
    }
}

// ---------------------------------------------------------------------------
// Whole-asset analysis of the `wmap` TextAssets.
// ---------------------------------------------------------------------------

/// Byte accounting: every byte of an asset is parsed, zero padding, a named raw
/// region, or unexplained (non-zero bytes no parser accounted for).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ByteAccount {
    pub parsed: usize,
    pub zero_padding: usize,
    pub raw_named: usize,
    pub unexplained: usize,
}

impl ByteAccount {
    pub fn total(&self) -> usize {
        self.parsed + self.zero_padding + self.raw_named + self.unexplained
    }

    fn add(&mut self, other: &ByteAccount) {
        self.parsed += other.parsed;
        self.zero_padding += other.zero_padding;
        self.raw_named += other.raw_named;
        self.unexplained += other.unexplained;
    }

    /// Accounts a tail after a parser stopped: zeros are padding, anything else is
    /// unexplained.
    fn tail(&mut self, tail: &[u8]) {
        let nonzero = nonzero_len(tail);
        self.zero_padding += tail.len() - nonzero;
        self.unexplained += nonzero;
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct WmapAssetReport {
    pub path: String,
    pub bytes: usize,
    /// `disc_image`, `tim_pack`, `frame_pack`, `pack`, `nwp_model_table`, `tcb`,
    /// `grid_map` or `raw`.
    pub kind: String,
    pub parsed_ok: bool,
    pub account: ByteAccount,
    /// Counts of parsed things (images, records, cells, ...), by name.
    pub counts: std::collections::BTreeMap<String, u64>,
    pub notes: Vec<String>,
    pub failures: Vec<String>,
}

fn count(report: &mut WmapAssetReport, name: &str, value: usize) {
    *report.counts.entry(name.to_owned()).or_insert(0) += value as u64;
}

/// Analyzes a pack of TIM images or generic segments.
fn analyze_pack(bytes: &[u8], source: &str, report: &mut WmapAssetReport) -> ByteAccount {
    let mut account = ByteAccount::default();
    let pack = match Pack::parse(bytes, source) {
        Ok(pack) => pack,
        Err(error) => {
            report.failures.push(error.to_string());
            account.raw_named = bytes.len();
            return account;
        }
    };
    account.parsed += pack.offsets[0] as usize;
    count(report, "pack_segments", pack.count());
    let mut tims = 0;
    for index in 0..pack.count() {
        let segment = pack.segment(index).expect("validated offsets");
        match Tim::parse(segment, source) {
            Ok(tim) => {
                tims += 1;
                account.parsed += tim.consumed_bytes;
                account.tail(&segment[tim.consumed_bytes..]);
            }
            Err(_) => account.raw_named += segment.len(),
        }
    }
    count(report, "tim_images", tims);
    if tims == pack.count() {
        report.kind = "tim_pack".to_owned();
    } else if tims == 0 {
        report.kind = "pack".to_owned();
        report
            .notes
            .push("segments are not TIM images; format not defined by Memoria".to_owned());
    } else {
        report.kind = "pack".to_owned();
        report.failures.push(format!(
            "{tims} of {} segments are TIM images",
            pack.count()
        ));
    }
    account
}

/// Result of one typed segment parse: bytes parsed, bytes kept as a named raw region,
/// the unread tail, and bytes no structure accounts for.
struct SegmentParse<'a> {
    parsed: usize,
    raw: usize,
    tail: &'a [u8],
    /// Non-zero bytes between structures that nothing accounts for.
    gap: usize,
    /// Zero bytes between structures (padding).
    gap_zero: usize,
}

/// Typed parse of the frame pack (`fx.ntp` = disc image slot 1).
fn analyze_frame_pack(bytes: &[u8], source: &str, report: &mut WmapAssetReport) -> ByteAccount {
    let mut account = ByteAccount::default();
    let pack = match Pack::parse(bytes, source) {
        Ok(pack) => pack,
        Err(error) => {
            report.failures.push(error.to_string());
            account.raw_named = bytes.len();
            return account;
        }
    };
    report.kind = "frame_pack".to_owned();
    account.parsed += pack.offsets[0] as usize;
    count(report, "pack_segments", pack.count());
    for index in 0..pack.count() {
        let segment = pack.segment(index).expect("validated offsets");
        let label = format!("{source} pack {index}");
        let tail_of = |left: usize| &segment[segment.len() - left..];
        let typed: Result<SegmentParse<'_>, ImportError> = match index {
            ENCOUNTER_PACK => parse_encounters(segment, &label).map(|(records, left)| {
                count(report, "encounter_records", records.len());
                SegmentParse {
                    parsed: segment.len() - left,
                    raw: 0,
                    tail: tail_of(left),
                    gap: 0,
                    gap_zero: 0,
                }
            }),
            SPECIAL_ENCOUNTER_PACK => {
                parse_special_encounters(segment, &label).map(|(groups, left)| {
                    count(report, "special_encounter_groups", groups.len());
                    SegmentParse {
                        parsed: segment.len() - left,
                        raw: 0,
                        tail: tail_of(left),
                        gap: 0,
                        gap_zero: 0,
                    }
                })
            }
            TEXTURE_PROJECT_PACK => {
                parse_texture_project(segment, &label).map(|(project, left)| {
                    count(
                        report,
                        "texture_project_entries",
                        project.pixel_scroll.len()
                            + project.pixel_anime.len()
                            + project.palette_scroll.len(),
                    );
                    SegmentParse {
                        parsed: segment.len() - left,
                        raw: 0,
                        tail: tail_of(left),
                        gap: 0,
                        gap_zero: 0,
                    }
                })
            }
            i if EFFECT_LIST_PACKS.contains(&i) => {
                parse_effect_list(segment, &label).map(|(list, left)| {
                    count(report, "effect_lists", 1);
                    count(report, "effect_records", list.len());
                    SegmentParse {
                        parsed: segment.len() - left,
                        raw: 0,
                        tail: tail_of(left),
                        gap: 0,
                        gap_zero: 0,
                    }
                })
            }
            SEA_BLOCK_PACK => NwbBlock::parse(segment, &label).map(|block| {
                count(report, "nwb_blocks", 1);
                count(report, "nwb_cells", block.cells.len());
                count(
                    report,
                    "nwb_vertices",
                    block.cells.iter().map(|cell| cell.vertices.len()).sum(),
                );
                count(
                    report,
                    "nwb_surfaces",
                    block.cells.iter().map(|cell| cell.surfaces.len()).sum(),
                );
                count(
                    report,
                    "nwb_texture_region_nonzero_bytes",
                    block.texture_region_nonzero,
                );
                count(
                    report,
                    "nwb_gap_zero_bytes",
                    block.gap_bytes - block.gap_nonzero,
                );
                SegmentParse {
                    parsed: block.structured_bytes,
                    raw: block.texture_region_bytes,
                    tail: &[],
                    gap: block.gap_nonzero,
                    gap_zero: block.gap_bytes - block.gap_nonzero,
                }
            }),
            EVA_PACK => {
                count(report, "eva_raw_bytes", segment.len());
                Ok(SegmentParse {
                    parsed: 0,
                    raw: segment.len(),
                    tail: &[],
                    gap: 0,
                    gap_zero: 0,
                })
            }
            _ => Ok(SegmentParse {
                parsed: 0,
                raw: segment.len(),
                tail: &[],
                gap: 0,
                gap_zero: 0,
            }),
        };
        match typed {
            Ok(parsed) => {
                account.parsed += parsed.parsed;
                account.raw_named += parsed.raw;
                account.unexplained += parsed.gap;
                account.zero_padding += parsed.gap_zero;
                account.tail(parsed.tail);
            }
            Err(error) => {
                report.failures.push(error.to_string());
                account.raw_named += segment.len();
            }
        }
    }
    account
}

fn analyze_standalone(path: &str, bytes: &[u8]) -> WmapAssetReport {
    let name = path.rsplit('/').next().unwrap_or(path);
    let mut report = WmapAssetReport {
        path: path.to_owned(),
        bytes: bytes.len(),
        kind: "raw".to_owned(),
        ..WmapAssetReport::default()
    };
    let account = if name.starts_with("fx.ntp") {
        analyze_frame_pack(bytes, path, &mut report)
    } else if name.ends_with(".ntp.bytes") {
        analyze_pack(bytes, path, &mut report)
    } else if name.ends_with(".img.bytes") {
        analyze_image(bytes, path, &mut report)
    } else if name.ends_with(".hed.bytes") {
        report.kind = "nwp_model_table".to_owned();
        report.notes.push(
            "PSX model table (also the first 4096 bytes of the .nwp); not read by Memoria"
                .to_owned(),
        );
        let used = nonzero_len(bytes);
        ByteAccount {
            raw_named: used,
            zero_padding: bytes.len() - used,
            ..ByteAccount::default()
        }
    } else if name.ends_with(".nwp.bytes") {
        report.kind = "nwp_model_table".to_owned();
        report.notes.push(
            "PSX model data: 4096-byte table then sector-aligned blocks; not read by Memoria"
                .to_owned(),
        );
        count(&mut report, "sectors", bytes.len() / SECTOR_BYTES);
        if !bytes.len().is_multiple_of(SECTOR_BYTES) {
            report
                .failures
                .push("length is not sector aligned".to_owned());
        }
        ByteAccount {
            raw_named: bytes.len(),
            ..ByteAccount::default()
        }
    } else if name.ends_with(".tcb.bytes") {
        report.kind = "tcb".to_owned();
        report
            .notes
            .push("SPS texture container; format read by Memoria's SPS code, not here".to_owned());
        ByteAccount {
            raw_named: bytes.len(),
            ..ByteAccount::default()
        }
    } else if name == "map.bin.bytes" {
        analyze_map_bin(bytes, &mut report)
    } else {
        report
            .failures
            .push("no parser for this asset name".to_owned());
        ByteAccount {
            unexplained: bytes.len(),
            ..ByteAccount::default()
        }
    };
    report.account = account;
    report.parsed_ok = report.failures.is_empty() && report.account.unexplained == 0;
    report
}

fn analyze_map_bin(bytes: &[u8], report: &mut WmapAssetReport) -> ByteAccount {
    report.kind = "grid_map".to_owned();
    if bytes.len() != MAP_GRID_WIDTH * MAP_GRID_HEIGHT {
        report.failures.push(format!(
            "expected {} bytes",
            MAP_GRID_WIDTH * MAP_GRID_HEIGHT
        ));
    }
    let max = bytes.iter().copied().max().unwrap_or(0);
    count(report, "grid_cells", bytes.len());
    count(report, "max_value", usize::from(max));
    report.notes.push(
        "one byte per world block (24 columns x 20 rows, matching the disc-1 block prefab grid); meaning of the value is not defined by Memoria".to_owned(),
    );
    ByteAccount {
        parsed: bytes.len(),
        ..ByteAccount::default()
    }
}

fn analyze_image(bytes: &[u8], source: &str, report: &mut WmapAssetReport) -> ByteAccount {
    let mut account = ByteAccount::default();
    report.kind = "disc_image".to_owned();
    let image = match DiscImage::parse(bytes, source) {
        Ok(image) => image,
        Err(error) => {
            report.failures.push(error.to_string());
            account.raw_named = bytes.len();
            return account;
        }
    };
    account.parsed += SECTOR_BYTES;
    count(report, "image_entries", image.entries.len());
    count(
        report,
        "uncovered_sectors",
        image.uncovered_sectors as usize,
    );
    account.unexplained += image.uncovered_sectors as usize * SECTOR_BYTES;
    for entry in &image.entries {
        let data = &bytes[entry.byte_range()];
        let label = format!("{source} slot {}", entry.slot);
        let mut sub = WmapAssetReport::default();
        let sub_account = if entry.slot == FRAME_PACK_SLOT {
            analyze_frame_pack(data, &label, &mut sub)
        } else if data.len() >= 4 && Pack::parse(data, &label).is_ok() {
            analyze_pack(data, &label, &mut sub)
        } else {
            sub.kind = "raw".to_owned();
            ByteAccount {
                raw_named: nonzero_len(data),
                zero_padding: data.len() - nonzero_len(data),
                ..ByteAccount::default()
            }
        };
        report.notes.push(format!(
            "slot {}: {} ({} bytes)",
            entry.slot,
            sub.kind,
            data.len()
        ));
        for (key, value) in &sub.counts {
            *report.counts.entry(key.clone()).or_insert(0) += value;
        }
        for failure in sub.failures {
            report
                .failures
                .push(format!("slot {}: {failure}", entry.slot));
        }
        account.add(&sub_account);
    }
    account
}

pub fn analyze_wmap_asset(path: &str, bytes: &[u8]) -> WmapAssetReport {
    analyze_standalone(path, bytes)
}

#[cfg(test)]
mod tests;
