//! Container of the `SpecialEffects/ef<id:D3>` TextAssets that `SFX.SetEffectArgs` hands to the
//! native `FF9SpecialEffectPlugin.dll` (`SFX_Play`). Typed, bounds-checked; regions whose meaning
//! needs the plugin's machine code stay raw and named.
//!
//! Sources (pinned Memoria, read-only):
//! - `FF9SpecialEffectPlugin/Rewritten/u0068_SfxIndexChunkTable.cs`: the chunk directory at file
//!   offset 0 (`i16` group count; per group an `i16` kind and an `i16` entry count; per entry
//!   `{u8 type, u8 flag, i16 sectors}`; type 2 with a nonzero flag is followed by one more `i16`
//!   sector count). Data starts at sector 1 (`0x800`), one sector is `0x800` bytes, entries are
//!   laid out in directory order. Type 3 records a script: first `u32` is a PSX address,
//!   rebased with `0x801E7700` (`+0x5000` for odd groups), which locates a table of 16 `u32`
//!   addresses (`+8` bytes past that offset).
//! - `u0070_SfxLoaderStep.cs`: what each record type is for (`SfxLoaderStepLoadRecord`,
//!   `SfxLoaderStepProcessRecord`, `SfxLoaderStepImages`: `u32` texture table offset at `+0`,
//!   image list offset at `+4`, image count at `+8`, each image a `u16` RECT `x y w h` followed by
//!   `w*h` `u16` pixels; `SfxUploadTexturePages` in `u0067`: 8-byte page entries, count = the
//!   record's flag byte).
//! - `Assembly-CSharp/Memoria/Battle/SFX/SFXBinaryFile.cs`: the 3-byte sequence stream at file
//!   offset `0x400` (ends at op 0), the type 2 "small file" table (offset/flags pairs, same
//!   offset = same file, flags `0xFFFF` = external) and which small files are cameras and sounds.
//! - `SFXDataCamera.cs`: the camera small-file layout.
//!
//! Not decoded (no source): the script bodies (MIPS code run by the plugin), image and model
//! pixel meaning beyond the RECTs, sound data, and what the sequence ops without a name in
//! `SFXBinaryFile.SequenceCode.CODE` do.

use crate::install::ImportError;
use serde::Serialize;
use std::collections::BTreeMap;

pub const SECTOR: usize = 0x800;
pub const SEQUENCE_OFFSET: usize = 0x400;
const PSX_MIRROR_BASE: u32 = 0x801E_7700;
const PSX_ODD_GROUP_SHIFT: u32 = 0x5000;
const PSX_ADDRESS_MASK: u32 = 0x0FFF_FFFF;
const SCRIPT_OFFSET_SLOTS: usize = 16;

fn invalid(name: &str, message: impl std::fmt::Display) -> ImportError {
    ImportError::invalid(name, message)
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
    name: &'a str,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8], pos: usize, name: &'a str) -> Self {
        Self { bytes, pos, name }
    }

    fn take(&mut self, count: usize, what: &str) -> Result<&'a [u8], ImportError> {
        let end = self
            .pos
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| {
                invalid(
                    self.name,
                    format_args!(
                        "{what}: {count} bytes at {:#x} run past the end ({:#x})",
                        self.pos,
                        self.bytes.len()
                    ),
                )
            })?;
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self, what: &str) -> Result<u8, ImportError> {
        Ok(self.take(1, what)?[0])
    }

    fn u16(&mut self, what: &str) -> Result<u16, ImportError> {
        let b = self.take(2, what)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    let b = bytes.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DirectoryEntry {
    pub record_type: u8,
    pub record_type_name: &'static str,
    pub flag: u8,
    pub sectors: u16,
    /// Only type 2 with a nonzero flag.
    pub extra_sectors: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Group {
    /// The `i16` the plugin skips; `SFXBinaryFile` calls it `chunkIndex`. The loader treats kind 1
    /// as a special buffer choice.
    pub kind: u16,
    pub entries: Vec<DirectoryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SequenceOp {
    pub offset: usize,
    pub code: u8,
    pub name: Option<&'static str>,
    pub arg1: u8,
    pub arg2: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Region {
    pub name: String,
    pub offset: usize,
    pub length: usize,
    /// `parsed` (the container structure is read; bytes inside such a region beyond the listed fields stay opaque, e.g. script bodies and pixels), `raw` (kept as bytes).
    pub kind: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImageRect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    /// Offset of the first pixel inside the record.
    pub pixel_offset: usize,
}

/// 8-byte entry of the texture page table (`SfxUploadTexturePages` reads `x`, `y`, word 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PageEntry {
    pub x: u16,
    pub y: u16,
    pub word2: u16,
    pub word3: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImageList {
    pub texture_table_offset: u32,
    pub image_list_offset: u32,
    pub image_count: u32,
    pub images: Vec<ImageRect>,
    /// Bytes inside the record after the last image.
    pub bytes_after_images: usize,
    /// Pixel-data offset word at the table (`+0`) and the second word (`+4`), kept raw.
    pub table_word0: Option<u32>,
    pub table_word1: Option<u32>,
    /// `flag` entries of 8 bytes at `table offset + 8`.
    pub pages: Vec<PageEntry>,
    pub pages_in_bounds: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScriptHeader {
    pub psx_base: u32,
    pub head_word: u32,
    /// `(head_word & 0x0FFFFFFF) - rebase`, as `SfxIndexChunkTable` computes it.
    pub first_offset: u32,
    /// The 16 `u32` found at `first_offset + 8`, rebased when nonzero. `None` when out of range.
    pub offsets: Option<Vec<u32>>,
    pub offsets_in_record: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SmallFile {
    pub index: usize,
    pub offset: usize,
    pub size: usize,
    pub flags: u16,
    /// Table slots (`file number`, the second index of `cameraFileList`) that map to this file.
    pub slots: Vec<usize>,
    pub role: &'static str,
    pub camera: Option<CameraFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SmallFileTable {
    pub base_offset: usize,
    pub slot_count: usize,
    /// External slots as `offset | flags << 16`, relative to the table base.
    pub external: Vec<u32>,
    pub files: Vec<SmallFile>,
    /// Slot number to unique file index, `-1` for external.
    pub slot_to_file: Vec<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CameraCode {
    pub offset: usize,
    pub frame: u16,
    pub flags: u16,
    pub length: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CameraSequence {
    pub slot: u8,
    pub offset: usize,
    /// Left out of the JSON reports; `code_count` and `last_frame` summarize it.
    #[serde(skip)]
    pub codes: Vec<CameraCode>,
    pub code_count: usize,
    pub terminated: bool,
    /// Stopped at a `HAS_UNKNOWN_1/2` flag whose block size the source does not know.
    pub aborted_unknown_block: bool,
    pub last_frame: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CameraFile {
    pub flags: u16,
    pub offsets: Vec<u16>,
    pub sequences: Vec<CameraSequence>,
    pub unknown_block: Option<[u8; 4]>,
    pub custom_position: Option<[i16; 3]>,
    /// Bytes of the file not covered by the header, offset table and sequences.
    pub uncovered_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum RecordDetail {
    Images(ImageList),
    Script(ScriptHeader),
    SmallFiles(SmallFileTable),
    /// Kept as bytes.
    Raw,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Record {
    pub group: usize,
    pub index: usize,
    pub record_type: u8,
    pub record_type_name: &'static str,
    pub flag: u8,
    /// File offset of the first byte this record owns (after the extra sectors of a type 2 record
    /// when `extra_offset` is set).
    pub offset: usize,
    pub length: usize,
    pub extra_offset: Option<usize>,
    pub extra_length: usize,
    pub detail: RecordDetail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SequenceRef {
    pub op_offset: usize,
    pub group: usize,
    pub slot: u8,
    /// Resolved small file index, `None` when out of range or external.
    pub file: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SequenceSummary {
    pub end_offset: usize,
    pub op_count: usize,
    pub histogram: BTreeMap<String, usize>,
    pub chunks_loaded: Vec<u8>,
    pub camera_refs: Vec<SequenceRef>,
    pub sound_refs: Vec<SequenceRef>,
    /// Ops `CHANGE_MODEL` and `PLAY_MODEL_ON_TARGET_V1..V4` with raw arguments (meaning not in the source).
    pub model_ops: Vec<SequenceOp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EffectFile {
    pub size: usize,
    pub whole_sectors: usize,
    pub tail_bytes: usize,
    pub groups: Vec<Group>,
    pub directory_bytes: usize,
    /// Every op; left out of the JSON reports (the summary keeps the counts and references).
    #[serde(skip)]
    pub sequence: Vec<SequenceOp>,
    pub sequence_summary: SequenceSummary,
    pub records: Vec<Record>,
    pub regions: Vec<Region>,
    pub raw_bytes: usize,
    pub parsed_bytes: usize,
}

pub fn record_type_name(record_type: u8) -> &'static str {
    match record_type {
        0 => "images",
        1 => "texture_pages",
        2 => "small_files",
        3 => "script",
        4 => "model_textures",
        5 => "summon_model",
        6 => "summon_model_area",
        7 => "type7_buffer",
        8 => "type8_buffer",
        9 => "vram_pages",
        10 => "stream_buffer",
        _ => "unknown",
    }
}

pub fn sequence_op_name(code: u8) -> Option<&'static str> {
    Some(match code {
        0x00 => "END",
        0x01 => "WAIT",
        0x02 => "PLAY_CASTER_ANIMATION",
        0x05 => "LOAD_CHUNK",
        0x0D => "CHANGE_MODEL",
        0x23 => "SETUP_CAMERA",
        0x24 => "SHOW_HIDE_CHARACTERS",
        0x25 => "EFFECT_POINT",
        0x27 => "CHANNEL_TOWARD_TARGET",
        0x28 => "PUT_BACK_IN_SLEEP_MODE",
        0x29 => "PLAY_CAMERA",
        0x2A => "SET_BATTLE_SCENE_TRANSPARENCY",
        0x2B => "WAIT_FOR_ANIMATION",
        0x2D => "PLAY_SOUND",
        0x51 => "SETUP_DESTINATION",
        0x55 => "PLAY_MISS_SOUND",
        0x5A => "PLAY_CHARACTER_ANIMATION_LOOPING",
        0x5B => "PLAY_CHARACTER_ANIMATION",
        0x62 => "FACE_BACK_TO_NORMAL",
        0x63 => "PLAY_RAW_ANIMATION",
        0x64 => "PLAY_FREYA_CASTING_ANIM",
        0x69 => "MOVE_BEFORE",
        0x6F => "MOVE_CASTER",
        0x80 => "PLAY_MODEL_ON_TARGET_V1",
        0x81 => "PLAY_MODEL_ON_TARGET_V2",
        0x82 => "PLAY_MODEL_ON_TARGET_V3",
        0x83 => "PLAY_MODEL_ON_TARGET_V4",
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

impl EffectFile {
    pub fn parse(bytes: &[u8], name: &str) -> Result<Self, ImportError> {
        if bytes.len() < SECTOR {
            return Err(invalid(
                name,
                format_args!("{} bytes is shorter than the directory sector", bytes.len()),
            ));
        }
        let mut cur = Cursor::new(bytes, 0, name);
        let group_count = cur.u16("group count")?;
        let mut groups = Vec::new();
        for g in 0..group_count {
            let kind = cur.u16("group kind")?;
            let count = cur.u16("group entry count")?;
            let mut entries = Vec::new();
            for e in 0..count {
                let record_type = cur.u8("entry type")?;
                let flag = cur.u8("entry flag")?;
                let sectors = cur.u16("entry sectors")?;
                let extra_sectors = if record_type == 2 && flag != 0 {
                    Some(cur.u16("entry extra sectors")?)
                } else {
                    None
                };
                if cur.pos > SEQUENCE_OFFSET {
                    return Err(invalid(
                        name,
                        format_args!("directory entry {g}.{e} runs into the sequence at 0x400"),
                    ));
                }
                entries.push(DirectoryEntry {
                    record_type,
                    record_type_name: record_type_name(record_type),
                    flag,
                    sectors,
                    extra_sectors,
                });
            }
            groups.push(Group { kind, entries });
        }
        let directory_bytes = cur.pos;
        if directory_bytes > SEQUENCE_OFFSET {
            return Err(invalid(name, "directory runs into the sequence at 0x400"));
        }

        // Sequence stream: 3-byte ops until op 0.
        let mut cur = Cursor::new(bytes, SEQUENCE_OFFSET, name);
        let mut sequence = Vec::new();
        loop {
            let offset = cur.pos;
            let code = cur.u8("sequence op")?;
            let arg1 = cur.u8("sequence arg1")?;
            let arg2 = cur.u8("sequence arg2")?;
            sequence.push(SequenceOp {
                offset,
                code,
                name: sequence_op_name(code),
                arg1,
                arg2,
            });
            if code == 0 {
                break;
            }
        }
        let sequence_end = cur.pos;

        // Records.
        let mut records = Vec::new();
        let mut position = SECTOR;
        for (g, group) in groups.iter().enumerate() {
            for (i, entry) in group.entries.iter().enumerate() {
                let (extra_offset, extra_length) = match entry.extra_sectors {
                    Some(extra) => (Some(position), usize::from(extra) * SECTOR),
                    None => (None, 0),
                };
                position = position
                    .checked_add(extra_length)
                    .ok_or_else(|| invalid(name, "record offset overflow"))?;
                let length = usize::from(entry.sectors) * SECTOR;
                let end = position
                    .checked_add(length)
                    .filter(|end| *end <= bytes.len())
                    .ok_or_else(|| {
                        invalid(
                            name,
                            format_args!(
                                "record {g}.{i} (type {}) needs {length} bytes at {position:#x} but the file ends at {:#x}",
                                entry.record_type,
                                bytes.len()
                            ),
                        )
                    })?;
                let body = &bytes[position..end];
                let detail = match entry.record_type {
                    0 => parse_images(body, entry.flag)
                        .map_or(RecordDetail::Raw, RecordDetail::Images),
                    2 => RecordDetail::SmallFiles(parse_small_files(body, position, name)?),
                    3 => parse_script(body, g).map_or(RecordDetail::Raw, RecordDetail::Script),
                    _ => RecordDetail::Raw,
                };
                records.push(Record {
                    group: g,
                    index: i,
                    record_type: entry.record_type,
                    record_type_name: entry.record_type_name,
                    flag: entry.flag,
                    offset: position,
                    length,
                    extra_offset,
                    extra_length,
                    detail,
                });
                position = end;
            }
        }
        let records_end = position;

        let sequence_summary = summarize_sequence(&sequence, sequence_end, &records);

        // Region table.
        let mut regions = vec![Region {
            name: "directory".into(),
            offset: 0,
            length: directory_bytes,
            kind: "parsed",
        }];
        if directory_bytes < SEQUENCE_OFFSET {
            regions.push(Region {
                name: "directory_padding".into(),
                offset: directory_bytes,
                length: SEQUENCE_OFFSET - directory_bytes,
                kind: "raw",
            });
        }
        regions.push(Region {
            name: "sequence".into(),
            offset: SEQUENCE_OFFSET,
            length: sequence_end - SEQUENCE_OFFSET,
            kind: "parsed",
        });
        let sequence_limit = SECTOR.max(sequence_end);
        if sequence_end < SECTOR {
            regions.push(Region {
                name: "sector0_tail".into(),
                offset: sequence_end,
                length: SECTOR - sequence_end,
                kind: "raw",
            });
        }
        for record in &records {
            if let Some(extra_offset) = record.extra_offset {
                regions.push(Region {
                    name: format!("record_{}_{}_extra", record.group, record.index),
                    offset: extra_offset,
                    length: record.extra_length,
                    kind: "raw",
                });
            }
            let kind = if matches!(record.detail, RecordDetail::Raw) {
                "raw"
            } else {
                "parsed"
            };
            regions.push(Region {
                name: format!(
                    "record_{}_{}_{}",
                    record.group, record.index, record.record_type_name
                ),
                offset: record.offset,
                length: record.length,
                kind,
            });
        }
        if records_end < bytes.len() {
            regions.push(Region {
                name: "trailing".into(),
                offset: records_end,
                length: bytes.len() - records_end,
                kind: "raw",
            });
        }
        // The sequence may in principle run past sector 0 into the first record.
        if sequence_limit > SECTOR {
            return Err(invalid(name, "sequence stream runs past sector 0"));
        }
        let parsed_bytes = regions
            .iter()
            .filter(|r| r.kind == "parsed")
            .map(|r| r.length)
            .sum();
        let raw_bytes = regions
            .iter()
            .filter(|r| r.kind == "raw")
            .map(|r| r.length)
            .sum();

        Ok(Self {
            size: bytes.len(),
            whole_sectors: bytes.len() / SECTOR,
            tail_bytes: bytes.len() % SECTOR,
            groups,
            directory_bytes,
            sequence,
            sequence_summary,
            records,
            regions,
            raw_bytes,
            parsed_bytes,
        })
    }
}

fn parse_images(body: &[u8], flag: u8) -> Option<ImageList> {
    let table_offset = u32_at(body, 0)?;
    let list_offset = u32_at(body, 4)?;
    let count = u32_at(body, 8)?;
    let mut pos = usize::try_from(list_offset).ok()?;
    let mut images = Vec::new();
    for _ in 0..count {
        let x = u16_at(body, pos)?;
        let y = u16_at(body, pos + 2)?;
        let w = u16_at(body, pos + 4)?;
        let h = u16_at(body, pos + 6)?;
        pos += 8;
        let pixels = usize::from(w) * usize::from(h) * 2;
        let end = pos.checked_add(pixels)?;
        if end > body.len() {
            return None;
        }
        images.push(ImageRect {
            x,
            y,
            w,
            h,
            pixel_offset: pos,
        });
        pos = end;
    }
    let table = usize::try_from(table_offset).ok()?;
    let table_word0 = u32_at(body, table);
    let table_word1 = u32_at(body, table + 4);
    let mut pages = Vec::new();
    let mut pages_in_bounds = true;
    for i in 0..usize::from(flag) {
        let at = table + 8 + i * 8;
        match (
            u16_at(body, at),
            u16_at(body, at + 2),
            u16_at(body, at + 4),
            u16_at(body, at + 6),
        ) {
            (Some(x), Some(y), Some(word2), Some(word3)) => {
                pages.push(PageEntry { x, y, word2, word3 })
            }
            _ => {
                pages_in_bounds = false;
                break;
            }
        }
    }
    Some(ImageList {
        texture_table_offset: table_offset,
        image_list_offset: list_offset,
        image_count: count,
        images,
        bytes_after_images: body.len() - pos,
        table_word0,
        table_word1,
        pages,
        pages_in_bounds,
    })
}

fn parse_script(body: &[u8], group: usize) -> Option<ScriptHeader> {
    let psx_base = PSX_MIRROR_BASE
        + if group & 1 == 1 {
            PSX_ODD_GROUP_SHIFT
        } else {
            0
        };
    let rebase = psx_base & PSX_ADDRESS_MASK;
    let head_word = u32_at(body, 0)?;
    let first_offset = (head_word & PSX_ADDRESS_MASK).wrapping_sub(rebase);
    let start = usize::try_from(first_offset).ok()?.checked_add(8)?;
    let mut offsets = Vec::new();
    for slot in 0..SCRIPT_OFFSET_SLOTS {
        match u32_at(body, start.checked_add(slot * 4)?) {
            Some(value) => offsets.push(if value == 0 {
                0
            } else {
                (value & PSX_ADDRESS_MASK).wrapping_sub(rebase)
            }),
            None => {
                return Some(ScriptHeader {
                    psx_base,
                    head_word,
                    first_offset,
                    offsets: None,
                    offsets_in_record: None,
                });
            }
        }
    }
    let in_record = offsets
        .iter()
        .filter(|offset| usize::try_from(**offset).is_ok_and(|o| o < body.len()))
        .count();
    Some(ScriptHeader {
        psx_base,
        head_word,
        first_offset,
        offsets: Some(offsets),
        offsets_in_record: Some(in_record),
    })
}

fn parse_small_files(
    body: &[u8],
    base_offset: usize,
    name: &str,
) -> Result<SmallFileTable, ImportError> {
    let mut cur = Cursor::new(body, 0, name);
    let first_offset = cur.u16("small file offset 0")?;
    let first_flags = cur.u16("small file flags 0")?;
    let slot_count = usize::from(first_offset) / 4;
    if slot_count == 0 || usize::from(first_offset) > body.len() {
        return Err(invalid(name, "small file table size is out of range"));
    }
    let mut offsets = vec![first_offset];
    let mut flags = vec![first_flags];
    let mut slot_to_file: Vec<i32> = vec![0];
    let mut external = Vec::new();
    for slot in 1..slot_count {
        let offset = cur.u16("small file offset")?;
        let flag = cur.u16("small file flags")?;
        let previous = slot_to_file[slot - 1];
        if flag == 0xFFFF || previous < 0 {
            slot_to_file.push(-1);
            external.push(u32::from(offset) | (u32::from(flag) << 16));
        } else if Some(&offset) == offsets.last() {
            slot_to_file.push(previous);
        } else {
            slot_to_file.push(previous + 1);
            offsets.push(offset);
            flags.push(flag);
        }
    }
    let mut files = Vec::new();
    for (index, (&offset, &flag)) in offsets.iter().zip(&flags).enumerate() {
        let end = offsets
            .get(index + 1)
            .map_or(body.len(), |next| usize::from(*next));
        let start = usize::from(offset);
        if end < start || end > body.len() {
            return Err(invalid(
                name,
                format_args!("small file {index}: {start:#x}..{end:#x} is outside the record"),
            ));
        }
        let slots = slot_to_file
            .iter()
            .enumerate()
            .filter(|(_, file)| usize::try_from(**file) == Ok(index))
            .map(|(slot, _)| slot)
            .collect();
        files.push(SmallFile {
            index,
            offset: start,
            size: end - start,
            flags: flag,
            slots,
            role: "unreferenced",
            camera: None,
        });
    }
    Ok(SmallFileTable {
        base_offset,
        slot_count,
        external,
        files,
        slot_to_file,
    })
}

fn summarize_sequence(
    ops: &[SequenceOp],
    end_offset: usize,
    records: &[Record],
) -> SequenceSummary {
    let mut histogram: BTreeMap<String, usize> = BTreeMap::new();
    let mut chunks_loaded = Vec::new();
    let mut camera_refs = Vec::new();
    let mut sound_refs = Vec::new();
    let mut model_ops = Vec::new();
    let mut current_chunk = 0usize;
    let resolve = |group: usize, slot: u8| -> Option<usize> {
        let table = records
            .iter()
            .filter(|r| r.group == group)
            .find_map(|r| match &r.detail {
                RecordDetail::SmallFiles(table) => Some(table),
                _ => None,
            })?;
        usize::try_from(*table.slot_to_file.get(usize::from(slot))?).ok()
    };
    for op in ops {
        let key = op
            .name
            .map_or_else(|| format!("0x{:02X}", op.code), str::to_owned);
        *histogram.entry(key).or_default() += 1;
        match op.code {
            0x05 => {
                chunks_loaded.push(op.arg1);
                current_chunk = usize::from(op.arg1);
            }
            0x23 if op.arg1 != 0xFF => camera_refs.push(SequenceRef {
                op_offset: op.offset,
                group: current_chunk,
                slot: op.arg1,
                file: resolve(current_chunk, op.arg1),
            }),
            0x29 if op.arg2 == 0 => camera_refs.push(SequenceRef {
                op_offset: op.offset,
                group: current_chunk,
                slot: op.arg1,
                file: resolve(current_chunk, op.arg1),
            }),
            0x2D => sound_refs.push(SequenceRef {
                op_offset: op.offset,
                group: current_chunk,
                slot: op.arg1,
                file: resolve(current_chunk, op.arg1),
            }),
            0x0D | 0x80..=0x83 => model_ops.push(op.clone()),
            _ => {}
        }
    }
    SequenceSummary {
        end_offset,
        op_count: ops.len(),
        histogram,
        chunks_loaded,
        camera_refs,
        sound_refs,
        model_ops,
    }
}

impl EffectFile {
    /// Parses every small file referenced as a camera by the sequence and fills `role` for
    /// cameras and sounds. `bytes` must be the same slice given to [`EffectFile::parse`].
    pub fn resolve_small_files(&mut self, bytes: &[u8], name: &str) -> Result<(), ImportError> {
        let cameras = self.sequence_summary.camera_refs.clone();
        let sounds = self.sequence_summary.sound_refs.clone();
        for record in &mut self.records {
            let RecordDetail::SmallFiles(table) = &mut record.detail else {
                continue;
            };
            let base = table.base_offset;
            let group = record.group;
            for reference in &cameras {
                if reference.group != group {
                    continue;
                }
                let Some(file) = reference.file else { continue };
                let entry = &mut table.files[file];
                if entry.camera.is_some() {
                    continue;
                }
                let slice = bytes
                    .get(base + entry.offset..base + entry.offset + entry.size)
                    .ok_or_else(|| invalid(name, "camera file outside the container"))?;
                entry.role = "camera";
                entry.camera = parse_camera(slice, name).ok();
            }
            for reference in &sounds {
                if reference.group != group {
                    continue;
                }
                if let Some(file) = reference.file {
                    let entry = &mut table.files[file];
                    if entry.role == "unreferenced" {
                        entry.role = "sound";
                    }
                }
            }
        }
        Ok(())
    }
}

/// `SFXDataCamera.Load` over one small file. Offsets are file-relative; reads past the file are errors.
pub fn parse_camera(bytes: &[u8], name: &str) -> Result<CameraFile, ImportError> {
    const SEQ0: u16 = 1;
    const SEQ1: u16 = 2;
    const SEQ2: u16 = 4;
    const UNKNOWN: u16 = 8;
    const CUSTOM: u16 = 0xF0;
    let mut cur = Cursor::new(bytes, 0, name);
    let flags = cur.u16("camera flags")?;
    let mut offsets = Vec::new();
    let mut sequences = Vec::new();
    let mut covered = vec![false; bytes.len()];
    covered[0] = true;
    if bytes.len() > 1 {
        covered[1] = true;
    }
    let mut unknown_block = None;
    let mut custom_position = None;
    fn mark(from: usize, to: usize, covered: &mut [bool]) {
        for slot in covered.iter_mut().take(to).skip(from) {
            *slot = true;
        }
    }
    let mut offset_at = 2usize;
    for (bit, slot) in [(SEQ0, 0u8), (SEQ1, 1), (SEQ2, 2)] {
        if flags & bit == 0 {
            continue;
        }
        let mut c = Cursor::new(bytes, offset_at, name);
        let seq_offset = c.u16("camera sequence offset")?;
        mark(offset_at, offset_at + 2, &mut covered);
        offsets.push(seq_offset);
        offset_at += 2;
        let (sequence, end) = parse_camera_sequence(bytes, usize::from(seq_offset), slot, name)?;
        mark(usize::from(seq_offset), end, &mut covered);
        sequences.push(sequence);
    }
    if flags & UNKNOWN != 0 {
        let mut c = Cursor::new(bytes, offset_at, name);
        let at = c.u16("camera unknown offset")?;
        mark(offset_at, offset_at + 2, &mut covered);
        offsets.push(at);
        offset_at += 2;
        let mut c = Cursor::new(bytes, usize::from(at), name);
        let block = c.take(4, "camera unknown block")?;
        unknown_block = Some([block[0], block[1], block[2], block[3]]);
        mark(usize::from(at), usize::from(at) + 4, &mut covered);
    }
    if flags & CUSTOM != 0 {
        let mut c = Cursor::new(bytes, offset_at, name);
        let at = c.u16("camera position offset")?;
        mark(offset_at, offset_at + 2, &mut covered);
        offsets.push(at);
        let mut c = Cursor::new(bytes, usize::from(at), name);
        let x = c.u16("camera x")? as i16;
        let y = c.u16("camera y")? as i16;
        let z = c.u16("camera z")? as i16;
        custom_position = Some([x, y, z]);
        mark(usize::from(at), usize::from(at) + 6, &mut covered);
    }
    let uncovered_bytes = covered.iter().filter(|c| !**c).count();
    Ok(CameraFile {
        flags,
        offsets,
        sequences,
        unknown_block,
        custom_position,
        uncovered_bytes,
    })
}

fn parse_camera_sequence(
    bytes: &[u8],
    start: usize,
    slot: u8,
    name: &str,
) -> Result<(CameraSequence, usize), ImportError> {
    let mut cur = Cursor::new(bytes, start, name);
    let mut codes = Vec::new();
    let mut terminated = false;
    let mut aborted = false;
    let mut last_frame = 0u16;
    loop {
        let offset = cur.pos;
        let frame = cur.u16("camera code frame")?;
        if frame == 0 {
            codes.push(CameraCode {
                offset,
                frame,
                flags: 0,
                length: 2,
            });
            terminated = true;
            break;
        }
        let flags = cur.u16("camera code flags")?;
        // Sizes of the optional blocks in the order `Sequence.Read` reads them.
        let mut size = 0usize;
        if flags & 0x03 != 0 {
            size += 6;
        }
        if flags & 0x02 != 0 {
            size += 4;
        }
        if flags & 0x04 != 0 {
            aborted = true;
            // Size of the block is unknown; the code stops here as in the source.
            codes.push(CameraCode {
                offset,
                frame,
                flags,
                length: cur.pos - offset + size,
            });
            cur.take(size, "camera code blocks")?;
            break;
        }
        if flags & 0x18 != 0 {
            size += 6;
        }
        if flags & 0x10 != 0 {
            size += 4;
        }
        if flags & 0x20 != 0 {
            aborted = true;
            codes.push(CameraCode {
                offset,
                frame,
                flags,
                length: cur.pos - offset + size,
            });
            cur.take(size, "camera code blocks")?;
            break;
        }
        if flags & 0x40 != 0 {
            size += 2;
        }
        if flags & 0x200 != 0 {
            size += 2;
        }
        if flags & 0x400 != 0 {
            size += 2;
        }
        if flags & 0x800 != 0 {
            size += 4;
        }
        if flags & 0x1000 != 0 {
            size += 4;
        }
        if flags & 0x4000 != 0 {
            size += 2;
        }
        if flags & 0x8000 != 0 {
            size += 4;
        }
        cur.take(size, "camera code blocks")?;
        last_frame = last_frame.max(frame);
        codes.push(CameraCode {
            offset,
            frame,
            flags,
            length: 4 + size,
        });
    }
    let end = cur.pos;
    Ok((
        CameraSequence {
            slot,
            offset: start,
            code_count: codes.len(),
            codes,
            terminated,
            aborted_unknown_block: aborted,
            last_frame,
        },
        end,
    ))
}
/// RVA of the plugin's per-effect flag words (`unk_18004F020`), which
/// `SfxVramInitUpload` copies to `0x1802208D0` and uploads to VRAM at
/// (768, 320); `SeqStateUpdateStart` reads entry `effect` back as the
/// play's flags (`dword_18032318C`).
const PLUGIN_EFFECT_FLAGS_RVA: u32 = 0x4F020;
/// Entries in that table (2048 bytes).
const PLUGIN_EFFECT_FLAGS: usize = 1024;

/// The per-effect flag words of `FF9SpecialEffectPlugin.dll` (its bytes),
/// by effect id. Bit `0x8` forces the effect's cameras on
/// (`RecomputeGate323268`); with bit `0x8000` `SeqStateUpdateStart`
/// takes its other path (no gate, no state reset).
pub fn plugin_effect_flags(dll: &[u8]) -> Option<Vec<u16>> {
    let u16_at = |at: usize| Some(u16::from_le_bytes(dll.get(at..at + 2)?.try_into().ok()?));
    let u32_at = |at: usize| Some(u32::from_le_bytes(dll.get(at..at + 4)?.try_into().ok()?));
    let pe = usize::try_from(u32_at(0x3C)?).ok()?;
    if dll.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let sections = usize::from(u16_at(pe + 6)?);
    let optional = usize::from(u16_at(pe + 20)?);
    let table = pe + 24 + optional;
    let rva = PLUGIN_EFFECT_FLAGS_RVA;
    let offset = (0..sections).find_map(|index| {
        let at = table + index * 40;
        let virtual_size = u32_at(at + 8)?;
        let virtual_address = u32_at(at + 12)?;
        let raw_size = u32_at(at + 16)?;
        let raw_offset = u32_at(at + 20)?;
        let inside = rva.checked_sub(virtual_address)?;
        (inside < virtual_size.max(raw_size)).then_some(raw_offset + inside)
    })?;
    let start = usize::try_from(offset).ok()?;
    (0..PLUGIN_EFFECT_FLAGS)
        .map(|index| u16_at(start + index * 2))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a container: groups of `(kind, [(type, flag, sectors, extra)])`, records filled with
    /// the closure's bytes (sized to the sector count), a sequence at 0x400.
    type TestEntry = (u8, u8, u16, Option<u16>);
    type TestGroup = (u16, Vec<TestEntry>);

    fn build(groups: &[TestGroup], sequence: &[[u8; 3]], bodies: &[Vec<u8>]) -> Vec<u8> {
        let mut out = vec![0u8; SECTOR];
        let mut pos = 0;
        let put16 = |out: &mut Vec<u8>, pos: &mut usize, v: u16| {
            out[*pos..*pos + 2].copy_from_slice(&v.to_le_bytes());
            *pos += 2;
        };
        put16(&mut out, &mut pos, groups.len() as u16);
        for (kind, entries) in groups {
            put16(&mut out, &mut pos, *kind);
            put16(&mut out, &mut pos, entries.len() as u16);
            for (t, f, s, extra) in entries {
                out[pos] = *t;
                out[pos + 1] = *f;
                pos += 2;
                put16(&mut out, &mut pos, *s);
                if let Some(extra) = extra {
                    put16(&mut out, &mut pos, *extra);
                }
            }
        }
        for (i, op) in sequence.iter().enumerate() {
            out[SEQUENCE_OFFSET + i * 3..SEQUENCE_OFFSET + i * 3 + 3].copy_from_slice(op);
        }
        for body in bodies {
            out.extend_from_slice(body);
        }
        out
    }

    fn sectors(count: usize, fill: u8) -> Vec<u8> {
        vec![fill; count * SECTOR]
    }

    #[test]
    fn parses_directory_sequence_and_raw_records() {
        let bytes = build(
            &[(0, vec![(9, 1, 1, None), (7, 0, 2, None)])],
            &[[5, 0, 0], [1, 0, 4], [0, 0, 0]],
            &[sectors(1, 0xAA), sectors(2, 0xBB)],
        );
        let file = EffectFile::parse(&bytes, "t").unwrap();
        assert_eq!(file.groups.len(), 1);
        assert_eq!(file.directory_bytes, 2 + 4 + 8);
        assert_eq!(file.records[0].offset, SECTOR);
        assert_eq!(file.records[1].offset, 2 * SECTOR);
        assert_eq!(file.records[0].record_type_name, "vram_pages");
        assert_eq!(file.sequence.len(), 3);
        assert_eq!(file.sequence[1].name, Some("WAIT"));
        assert_eq!(file.sequence_summary.chunks_loaded, vec![0]);
        assert_eq!(file.raw_bytes + file.parsed_bytes, file.size);
    }

    #[test]
    fn rejects_record_past_end_and_missing_sequence_end() {
        let bytes = build(
            &[(0, vec![(9, 0, 3, None)])],
            &[[0, 0, 0]],
            &[sectors(1, 0)],
        );
        assert!(EffectFile::parse(&bytes, "t").is_err());
        let mut no_end = build(&[(0, vec![])], &[], &[]);
        for byte in &mut no_end[SEQUENCE_OFFSET..SECTOR] {
            *byte = 1;
        }
        assert!(EffectFile::parse(&no_end, "t").is_err());
        assert!(EffectFile::parse(&[0u8; 10], "t").is_err());
    }

    #[test]
    fn extended_type_two_entry_has_extra_sectors_first() {
        let mut small = vec![0u8; 3 * SECTOR];
        // Table: two slots, both at offset 8 (the same file), then 8 bytes of file.
        small[0..2].copy_from_slice(&8u16.to_le_bytes());
        small[4..6].copy_from_slice(&8u16.to_le_bytes());
        let mut body = sectors(1, 0x11);
        body.extend_from_slice(&small[..2 * SECTOR]);
        let bytes = build(&[(0, vec![(2, 1, 2, Some(1))])], &[[0, 0, 0]], &[body]);
        let file = EffectFile::parse(&bytes, "t").unwrap();
        let record = &file.records[0];
        assert_eq!(record.extra_offset, Some(SECTOR));
        assert_eq!(record.offset, 2 * SECTOR);
        let RecordDetail::SmallFiles(table) = &record.detail else {
            panic!("small files expected");
        };
        assert_eq!(table.slot_count, 2);
        assert_eq!(table.files.len(), 1);
        assert_eq!(table.files[0].slots, vec![0, 1]);
        assert_eq!(table.files[0].size, 2 * SECTOR - 8);
    }

    #[test]
    fn small_file_table_marks_external_slots() {
        let mut body = vec![0u8; SECTOR];
        let entries: [(u16, u16); 4] = [(16, 1), (100, 2), (200, 0xFFFF), (300, 3)];
        for (i, (o, f)) in entries.iter().enumerate() {
            body[i * 4..i * 4 + 2].copy_from_slice(&o.to_le_bytes());
            body[i * 4 + 2..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
        }
        let table = parse_small_files(&body, 0x800, "t").unwrap();
        assert_eq!(table.slot_to_file, vec![0, 1, -1, -1]);
        assert_eq!(table.external, vec![200 | (0xFFFF << 16), 300 | (3 << 16)]);
        assert_eq!(table.files.len(), 2);
        assert_eq!(table.files[0].size, 84);
        assert_eq!(table.files[1].size, SECTOR - 100);
    }

    #[test]
    fn small_file_table_rejects_bad_offsets() {
        let mut body = vec![0u8; 64];
        body[0..2].copy_from_slice(&8u16.to_le_bytes());
        body[4..6].copy_from_slice(&200u16.to_le_bytes());
        body[6..8].copy_from_slice(&1u16.to_le_bytes());
        assert!(parse_small_files(&body, 0, "t").is_err());
    }

    #[test]
    fn parses_image_list_and_pages() {
        let mut body = vec![0u8; SECTOR];
        body[0..4].copy_from_slice(&0x100u32.to_le_bytes()); // table
        body[4..8].copy_from_slice(&0x10u32.to_le_bytes()); // list
        body[8..12].copy_from_slice(&1u32.to_le_bytes());
        // image 640,256 2x2 at 0x10
        for (i, v) in [640u16, 256, 2, 2].iter().enumerate() {
            body[0x10 + i * 2..0x12 + i * 2].copy_from_slice(&v.to_le_bytes());
        }
        // page entry at table + 8
        for (i, v) in [768u16, 256, 0, 128].iter().enumerate() {
            body[0x108 + i * 2..0x10A + i * 2].copy_from_slice(&v.to_le_bytes());
        }
        let list = parse_images(&body, 1).unwrap();
        assert_eq!(list.images.len(), 1);
        assert_eq!(list.images[0].pixel_offset, 0x18);
        assert_eq!(list.pages[0].x, 768);
        assert!(list.pages_in_bounds);
        // Image pixel data past the record is rejected.
        body[0x14..0x16].copy_from_slice(&0x4000u16.to_le_bytes());
        assert!(parse_images(&body, 1).is_none());
    }

    #[test]
    fn script_header_rebases_psx_addresses() {
        let mut body = vec![0u8; SECTOR];
        body[0..4].copy_from_slice(&(0x801E_7700u32 + 0x20).to_le_bytes());
        body[0x28..0x2C].copy_from_slice(&(0x801E_7700u32 + 0x40).to_le_bytes());
        let header = parse_script(&body, 0).unwrap();
        assert_eq!(header.first_offset, 0x20);
        let offsets = header.offsets.unwrap();
        assert_eq!(offsets[0], 0x40);
        assert_eq!(offsets[1], 0);
        let odd = parse_script(&body, 1).unwrap();
        assert_eq!(odd.psx_base, 0x801E_C700);
        // Too short for the 16 slots.
        let short = parse_script(&body[..0x30], 0).unwrap();
        assert!(short.offsets.is_none());
    }

    #[test]
    fn camera_file_flags_offsets_and_sequences() {
        // flags: sequence 0 + custom position (bit 0x10)
        let mut cam = vec![0u8; 0x20];
        cam[0..2].copy_from_slice(&0x11u16.to_le_bytes());
        cam[2..4].copy_from_slice(&0x08u16.to_le_bytes()); // seq 0 at 8
        cam[4..6].copy_from_slice(&0x16u16.to_le_bytes()); // position at 0x16
        // code at 8: frame 5, flags 0x40 (signing, 2 bytes), then frame 0
        cam[8..10].copy_from_slice(&5u16.to_le_bytes());
        cam[10..12].copy_from_slice(&0x40u16.to_le_bytes());
        cam[16..18].copy_from_slice(&0u16.to_le_bytes());
        cam[0x16..0x18].copy_from_slice(&(-3i16).to_le_bytes());
        let file = parse_camera(&cam, "t").unwrap();
        assert_eq!(file.sequences.len(), 1);
        assert!(file.sequences[0].terminated);
        assert_eq!(file.sequences[0].last_frame, 5);
        assert_eq!(file.sequences[0].codes[0].length, 6);
        assert_eq!(file.custom_position, Some([-3, 0, 0]));
        // Truncated sequence is an error.
        assert!(parse_camera(&cam[..12], "t").is_err());
    }

    #[test]
    fn sequence_references_resolve_cameras_through_loaded_chunk() {
        // One group with one small-file record: 2 slots -> files 0 and 1; file 0 is a camera.
        let mut body = vec![0u8; SECTOR];
        body[0..2].copy_from_slice(&8u16.to_le_bytes());
        body[4..6].copy_from_slice(&0x10u16.to_le_bytes());
        body[6..8].copy_from_slice(&1u16.to_le_bytes());
        // camera at 8..0x10: flags 0 (no sequences)
        let bytes = build(
            &[(0, vec![(2, 0, 1, None)])],
            &[[0x23, 0, 0], [0x2D, 1, 0], [0, 0, 0]],
            &[body],
        );
        let mut file = EffectFile::parse(&bytes, "t").unwrap();
        assert_eq!(file.sequence_summary.camera_refs[0].file, Some(0));
        assert_eq!(file.sequence_summary.sound_refs[0].file, Some(1));
        file.resolve_small_files(&bytes, "t").unwrap();
        let RecordDetail::SmallFiles(table) = &file.records[0].detail else {
            panic!();
        };
        assert_eq!(table.files[0].role, "camera");
        assert!(table.files[0].camera.is_some());
        assert_eq!(table.files[1].role, "sound");
    }
}
