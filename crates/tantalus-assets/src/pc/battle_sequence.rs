//! Battle action sequences (`.raw17`): file layout, typed opcode decoder and a pure
//! frame-by-frame interpreter that emits ordered markers. Nothing here renders.
//!
//! Sources (pinned Memoria C#, `Assembly-CSharp/Global/btlseq.cs` unless noted):
//! - `ReadBattleSequence`: header (`seqBlockOffset`, `camOffset`, `seqCount`, `animCount`,
//!   `seqOffset[i16]`, `animList[i32]`, `seqBaseAnim[u8]`). A sequence starts at file offset
//!   `seqOffset[i] + 4` and an offset of 0 means "no sequence" (`StartBtlSeq`, `RunSequence`).
//! - `gSeqProg` (34 entries, codes 0 to 0x21), the `SeqInit*`/`SeqExec*` functions and
//!   `btlseqinstance.AdvanceSeqCode` (opcode lengths). A sequence is a linear stream: no
//!   opcode jumps, so there is no loop opcode; it ends at code 0 (`SeqExecEnd`) or 0x18
//!   (`SeqExecFastEnd`).
//! - `Sequencer`: per-frame counters and the post-opcode timers (turn, SFX, special VFX, fade).
//! - `Memoria/Battle/SFX/SFXDataCamera.cs` `LoadFromBSC`: the camera block at `camOffset`
//!   starts with a table of `u16` offsets (count = first offset / 2). Only that table and
//!   each camera's flags word are read here; the camera sequences stay raw.
//!
//! Frame model: frame 0 is the first `Sequencer()` call after `EntrySequence`. Opcodes that
//! wait on the engine (animation end, effect plugin state, `SeqExecEnd` conditions) cannot be
//! derived from the file; the interpreter lets them pass at the first frame the sequence data
//! alone allows and says so (`ExternalWait` markers, [`Trace::exact`]).

use crate::install::ImportError;
use crate::pc::battle_scene::SequenceHeader;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// `gSeqProg.Length`: codes 0 to 0x21 exist; 0x22 indexes past the table in the source and
/// larger codes are mapped to code 0 by `Sequencer`. Both are reported as unsupported here.
pub const OPCODE_COUNT: u8 = 34;

/// Total byte length (opcode byte included) per opcode, from the `CurPtr +=` of each
/// `SeqExec*` and `AdvanceSeqCode` (which agree).
const OPCODE_LEN: [u8; 34] = [
    1, 2, 1, 4, 2, 2, 5, 1, 9, 1, 1, 1, 4, 3, 2, 3, 2, 2, 2, 8, 6, 2, 2, 2, 1, 6, 9, 8, 2, 2, 4, 2,
    2, 2,
];

/// Source names (the `SeqExec*` suffix) per opcode.
pub const OPCODE_NAMES: [&str; 34] = [
    "End",
    "Wait",
    "Calc",
    "MoveToTarget",
    "MoveToTurn",
    "Anim",
    "SVfx",
    "WaitAnim",
    "Vfx",
    "WaitLoadVfx",
    "StartVfx",
    "WaitVfx",
    "Scale",
    "MeshHide",
    "Message",
    "MeshShow",
    "SetCamera",
    "DefaultIdle",
    "RunCamera",
    "MoveToPoint",
    "Turn",
    "TexAnimPlay",
    "TexAnimOnce",
    "TexAnimStop",
    "FastEnd",
    "Sfx",
    "Vfx2",
    "MoveToOffset",
    "TargetBone",
    "FadeOut",
    "MoveToTargetZ",
    "Shadow",
    "RunCameraAlways",
    "MessageTitle",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnTarget {
    /// Bit 15 clear: `raw / 4096 * 360` degrees.
    Absolute { raw: i16 },
    /// Bit 15 set, low bits 0: the battle rotation `evt.rotBattle`.
    BattleRotation,
    /// Bit 15 set, low bits 1: face the average target position.
    FaceTarget,
    /// Bit 15 set, any other low value: keep the current rotation.
    KeepCurrent { mode: i16 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TexAnimOp {
    Play,
    PlayOnce,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CameraOp {
    /// Code 0x10: sets `CameraNo` unconditionally.
    Set,
    /// Code 0x12: reads `CameraNo` only when `SFX.ShouldPlayAlternateCamera` (engine state).
    RunIfAlternate,
    /// Code 0x20: always targets the average target position and sets `CameraNo`.
    Run,
}

/// One decoded opcode with its operands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SeqOp {
    End,
    Wait {
        frames: u8,
    },
    Calc,
    /// Code 3 (`z_offset` false: stop `distance` short along the line to the target) or
    /// code 0x1E (`z_offset` true: Dst.z -= distance).
    MoveToTarget {
        frames: u8,
        distance: i16,
        z_offset: bool,
    },
    /// Code 4: back to `base_pos`.
    MoveToBase {
        frames: u8,
    },
    Anim {
        code: u8,
    },
    SVfx {
        effect: u16,
        param: u8,
        delay: u8,
    },
    WaitAnim,
    /// Code 8 (`flag` 0) or 0x1A (`flag` 1; the source passes it as `array[3]`).
    Vfx {
        effect: u16,
        args: [i16; 3],
        flag: u8,
    },
    WaitLoadVfx,
    StartVfx,
    WaitVfx,
    Scale {
        factor: i16,
        frames: u8,
    },
    MeshHide {
        mask: u16,
    },
    /// Code 0xE (`title` false) or 0x21 (`title` true).
    Message {
        id: u8,
        title: bool,
    },
    MeshShow {
        mask: u16,
    },
    Camera {
        op: CameraOp,
        id: u8,
    },
    DefaultIdle {
        enabled: u8,
    },
    MoveToPoint {
        frames: u8,
        dst: [i16; 3],
    },
    Turn {
        target: TurnTarget,
        offset: i16,
        frames: u8,
    },
    TexAnim {
        op: TexAnimOp,
        id: u8,
    },
    FastEnd,
    /// `unused` is the byte `SeqExecSfx` reads and discards.
    Sfx {
        id: u16,
        delay: u8,
        unused: u8,
        volume: u8,
    },
    MoveToOffset {
        frames: u8,
        offset: [i16; 3],
    },
    TargetBone {
        bone: u8,
    },
    FadeOut {
        frames: u8,
    },
    Shadow {
        enabled: u8,
    },
}

impl SeqOp {
    /// The opcode byte this operation was decoded from (the `Vfx`, `Message`, `Camera` and
    /// `MoveToTarget` variants carry what distinguishes their two codes).
    pub fn code(&self) -> u8 {
        match *self {
            Self::End => 0,
            Self::Wait { .. } => 1,
            Self::Calc => 2,
            Self::MoveToTarget { z_offset, .. } => {
                if z_offset {
                    0x1E
                } else {
                    3
                }
            }
            Self::MoveToBase { .. } => 4,
            Self::Anim { .. } => 5,
            Self::SVfx { .. } => 6,
            Self::WaitAnim => 7,
            Self::Vfx { flag, .. } => {
                if flag == 0 {
                    8
                } else {
                    0x1A
                }
            }
            Self::WaitLoadVfx => 9,
            Self::StartVfx => 0xA,
            Self::WaitVfx => 0xB,
            Self::Scale { .. } => 0xC,
            Self::MeshHide { .. } => 0xD,
            Self::Message { title, .. } => {
                if title {
                    0x21
                } else {
                    0xE
                }
            }
            Self::MeshShow { .. } => 0xF,
            Self::Camera { op, .. } => match op {
                CameraOp::Set => 0x10,
                CameraOp::RunIfAlternate => 0x12,
                CameraOp::Run => 0x20,
            },
            Self::DefaultIdle { .. } => 0x11,
            Self::MoveToPoint { .. } => 0x13,
            Self::Turn { .. } => 0x14,
            Self::TexAnim { op, .. } => match op {
                TexAnimOp::Play => 0x15,
                TexAnimOp::PlayOnce => 0x16,
                TexAnimOp::Stop => 0x17,
            },
            Self::FastEnd => 0x18,
            Self::Sfx { .. } => 0x19,
            Self::MoveToOffset { .. } => 0x1B,
            Self::TargetBone { .. } => 0x1C,
            Self::FadeOut { .. } => 0x1D,
            Self::Shadow { .. } => 0x1F,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacedOp {
    /// Absolute file offset of the opcode byte.
    pub offset: usize,
    pub op: SeqOp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Terminator {
    End,
    FastEnd,
    /// An opcode byte outside the table (the source would index past it or map it to End).
    Unsupported {
        code: u8,
        offset: usize,
    },
    /// The stream ran off the end of the file.
    Truncated {
        offset: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecodedSequence {
    /// Absolute file offset of the first opcode.
    pub start: usize,
    /// Operations in order, including the terminating End or FastEnd when present.
    pub ops: Vec<PlacedOp>,
    /// One past the last byte consumed.
    pub end: usize,
    pub terminator: Terminator,
}

fn i16_at(b: &[u8], p: usize) -> i16 {
    i16::from_le_bytes([b[p], b[p + 1]])
}

fn u16_at(b: &[u8], p: usize) -> u16 {
    u16::from_le_bytes([b[p], b[p + 1]])
}

/// Decodes the operation at `p`; `None` when the opcode is unknown or its operands are cut off.
fn decode_op(bytes: &[u8], p: usize) -> Result<(SeqOp, usize), Terminator> {
    let code = bytes[p];
    if code >= OPCODE_COUNT {
        return Err(Terminator::Unsupported { code, offset: p });
    }
    let len = usize::from(OPCODE_LEN[usize::from(code)]);
    if p + len > bytes.len() {
        return Err(Terminator::Truncated { offset: p });
    }
    let b1 = bytes.get(p + 1).copied().unwrap_or(0);
    let op = match code {
        0 => SeqOp::End,
        1 => SeqOp::Wait { frames: b1 },
        2 => SeqOp::Calc,
        3 | 0x1E => SeqOp::MoveToTarget {
            frames: b1,
            distance: i16_at(bytes, p + 2),
            z_offset: code == 0x1E,
        },
        4 => SeqOp::MoveToBase { frames: b1 },
        5 => SeqOp::Anim { code: b1 },
        6 => SeqOp::SVfx {
            effect: u16_at(bytes, p + 1),
            param: bytes[p + 3],
            delay: bytes[p + 4],
        },
        7 => SeqOp::WaitAnim,
        8 | 0x1A => SeqOp::Vfx {
            effect: u16_at(bytes, p + 1),
            args: [
                i16_at(bytes, p + 3),
                i16_at(bytes, p + 5),
                i16_at(bytes, p + 7),
            ],
            flag: u8::from(code == 0x1A),
        },
        9 => SeqOp::WaitLoadVfx,
        0xA => SeqOp::StartVfx,
        0xB => SeqOp::WaitVfx,
        // `SeqInitScale`: i16 factor then u8 frames.
        0xC => SeqOp::Scale {
            factor: i16_at(bytes, p + 1),
            frames: bytes[p + 3],
        },
        0xD => SeqOp::MeshHide {
            mask: u16_at(bytes, p + 1),
        },
        0xE | 0x21 => SeqOp::Message {
            id: b1,
            title: code == 0x21,
        },
        0xF => SeqOp::MeshShow {
            mask: u16_at(bytes, p + 1),
        },
        0x10 | 0x12 | 0x20 => SeqOp::Camera {
            op: match code {
                0x10 => CameraOp::Set,
                0x12 => CameraOp::RunIfAlternate,
                _ => CameraOp::Run,
            },
            id: b1,
        },
        0x11 => SeqOp::DefaultIdle { enabled: b1 },
        // `SeqInitMoveToPoint`: u8 frames then three i16 (the Y sign flip is applied at run time).
        0x13 => SeqOp::MoveToPoint {
            frames: b1,
            dst: [
                i16_at(bytes, p + 2),
                i16_at(bytes, p + 4),
                i16_at(bytes, p + 6),
            ],
        },
        // `SeqExecTurn`: i16, i16, u8.
        0x14 => {
            let raw = i16_at(bytes, p + 1);
            let target = if raw as u16 & 0x8000 != 0 {
                match raw & 0x7FFF {
                    0 => TurnTarget::BattleRotation,
                    1 => TurnTarget::FaceTarget,
                    mode => TurnTarget::KeepCurrent { mode },
                }
            } else {
                TurnTarget::Absolute { raw }
            };
            SeqOp::Turn {
                target,
                offset: i16_at(bytes, p + 3),
                frames: bytes[p + 5],
            }
        }
        0x15..=0x17 => SeqOp::TexAnim {
            op: match code {
                0x15 => TexAnimOp::Play,
                0x16 => TexAnimOp::PlayOnce,
                _ => TexAnimOp::Stop,
            },
            id: b1,
        },
        0x18 => SeqOp::FastEnd,
        // `SeqExecSfx`: u16 id, u8 delay, one discarded byte, u8 volume.
        0x19 => SeqOp::Sfx {
            id: u16_at(bytes, p + 1),
            delay: bytes[p + 3],
            unused: bytes[p + 4],
            volume: bytes[p + 5],
        },
        0x1B => SeqOp::MoveToOffset {
            frames: b1,
            offset: [
                i16_at(bytes, p + 2),
                i16_at(bytes, p + 4),
                i16_at(bytes, p + 6),
            ],
        },
        0x1C => SeqOp::TargetBone { bone: b1 },
        0x1D => SeqOp::FadeOut { frames: b1 },
        0x1F => SeqOp::Shadow { enabled: b1 },
        _ => return Err(Terminator::Unsupported { code, offset: p }),
    };
    Ok((op, len))
}

/// Decodes one sequence starting at `start` until End, FastEnd, an unsupported opcode or the
/// end of the data.
pub fn decode_sequence(bytes: &[u8], start: usize) -> DecodedSequence {
    let mut ops = Vec::new();
    let mut p = start;
    let terminator = loop {
        if p >= bytes.len() {
            break Terminator::Truncated { offset: p };
        }
        match decode_op(bytes, p) {
            Ok((op, len)) => {
                ops.push(PlacedOp { offset: p, op });
                p += len;
                match op {
                    SeqOp::End => break Terminator::End,
                    SeqOp::FastEnd => break Terminator::FastEnd,
                    _ => {}
                }
            }
            Err(terminator) => break terminator,
        }
    };
    DecodedSequence {
        start,
        ops,
        end: p,
        terminator,
    }
}

// ---------------------------------------------------------------------------
// File layout
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegionKind {
    /// Header tables (`SequenceHeader::header_len` bytes).
    Header,
    /// Decoded opcode streams, one per distinct start offset.
    Sequence,
    /// Bytes between header, sequences and the camera block that no sequence claims; kept raw.
    Gap,
    /// From `camOffset` to the end of the file; kept raw apart from the offset table.
    CameraBlock,
    /// Bytes claimed by two sequences (overlap), counted once above.
    Overlap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    pub start: usize,
    pub len: usize,
    pub kind: RegionKind,
    /// Gap only: every byte is zero.
    pub all_zero: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequenceSlot {
    pub index: usize,
    /// The raw `seqOffset[i]`; 0 means no sequence.
    pub offset: i16,
    /// `seqBaseAnim[i]`: added to the `Anim` code to index `anim_list` (`AnmIDOfs`).
    pub base_anim: u8,
    /// Absolute start (`offset + 4`) when `offset != 0`.
    pub start: Option<usize>,
}

/// One camera entry from the camera block's offset table (`LoadFromBSC`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraEntry {
    /// Offset of the camera set relative to `camOffset`.
    pub set_offset: u16,
    /// First `u16` of the set (`SFXDataCamera.Flags`); the rest is not decoded.
    pub flags: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequenceFile {
    pub header: SequenceHeader,
    pub slots: Vec<SequenceSlot>,
    /// Distinct decoded streams keyed by absolute start offset.
    pub sequences: BTreeMap<usize, DecodedSequence>,
    pub cameras: Vec<CameraEntry>,
    pub regions: Vec<Region>,
}

impl SequenceFile {
    pub fn parse(bytes: &[u8], source: &str) -> Result<Self, ImportError> {
        let header = SequenceHeader::parse(bytes, source)?;
        let cam = usize::try_from(header.cam_offset)
            .map_err(|_| ImportError::invalid(source, "negative camera offset"))?;
        if cam > bytes.len() || cam < header.header_len {
            return Err(ImportError::invalid(
                source,
                format!("camera offset {cam} is outside the file or inside the header"),
            ));
        }
        let mut slots = Vec::new();
        let mut sequences = BTreeMap::new();
        for (index, offset) in header.seq_offsets.iter().copied().enumerate() {
            let start = (offset != 0).then(|| (i64::from(offset) + 4) as usize);
            if let Some(start) = start
                && start < bytes.len()
                && !sequences.contains_key(&start)
            {
                sequences.insert(start, decode_sequence(bytes, start));
            }
            slots.push(SequenceSlot {
                index,
                offset,
                base_anim: header.seq_base_anim[index],
                start,
            });
        }
        let cameras = parse_camera_table(bytes, cam);
        let regions = build_regions(bytes, header.header_len, cam, &sequences);
        Ok(Self {
            header,
            slots,
            sequences,
            cameras,
            regions,
        })
    }

    /// The decoded stream for sequence slot `index`, when the slot is used.
    pub fn sequence(&self, index: usize) -> Option<&DecodedSequence> {
        self.slots
            .get(index)?
            .start
            .and_then(|start| self.sequences.get(&start))
    }

    /// Bytes per region kind (distinct sequences counted once).
    pub fn region_totals(&self) -> BTreeMap<String, usize> {
        let mut totals = BTreeMap::new();
        for region in &self.regions {
            *totals.entry(format!("{:?}", region.kind)).or_insert(0) += region.len;
        }
        totals
    }
}

fn parse_camera_table(bytes: &[u8], cam: usize) -> Vec<CameraEntry> {
    if cam + 2 > bytes.len() {
        return Vec::new();
    }
    let count = usize::from(u16_at(bytes, cam)) / 2;
    let mut cameras = Vec::new();
    for i in 0..count {
        let at = cam + 2 * i;
        if at + 2 > bytes.len() {
            break;
        }
        let set_offset = u16_at(bytes, at);
        let flag_at = cam + usize::from(set_offset);
        let flags = (flag_at + 2 <= bytes.len()).then(|| u16_at(bytes, flag_at));
        cameras.push(CameraEntry { set_offset, flags });
    }
    cameras
}

fn build_regions(
    bytes: &[u8],
    header_len: usize,
    cam: usize,
    sequences: &BTreeMap<usize, DecodedSequence>,
) -> Vec<Region> {
    let mut regions = vec![Region {
        start: 0,
        len: header_len,
        kind: RegionKind::Header,
        all_zero: false,
    }];
    let mut pos = header_len;
    let gap = |regions: &mut Vec<Region>, from: usize, to: usize| {
        if to > from {
            regions.push(Region {
                start: from,
                len: to - from,
                kind: RegionKind::Gap,
                all_zero: bytes[from..to].iter().all(|b| *b == 0),
            });
        }
    };
    for seq in sequences.values() {
        let end = seq.end.min(bytes.len());
        if seq.start < pos {
            // Overlaps what was already claimed; count only the overlapping bytes once.
            let overlap_end = end.min(pos);
            if overlap_end > seq.start {
                regions.push(Region {
                    start: seq.start,
                    len: overlap_end - seq.start,
                    kind: RegionKind::Overlap,
                    all_zero: false,
                });
            }
            if end > pos {
                regions.push(Region {
                    start: pos,
                    len: end - pos,
                    kind: RegionKind::Sequence,
                    all_zero: false,
                });
                pos = end;
            }
            continue;
        }
        gap(&mut regions, pos, seq.start);
        regions.push(Region {
            start: seq.start,
            len: end - seq.start,
            kind: RegionKind::Sequence,
            all_zero: false,
        });
        pos = end;
    }
    if cam > pos {
        gap(&mut regions, pos, cam);
        pos = cam;
    }
    if bytes.len() > pos.max(cam) || cam < pos {
        // Sequences running into the camera block would be an overlap; account for it.
        if cam < pos {
            regions.push(Region {
                start: cam,
                len: pos.min(bytes.len()) - cam,
                kind: RegionKind::Overlap,
                all_zero: false,
            });
        }
        let from = pos.max(cam);
        regions.push(Region {
            start: from,
            len: bytes.len() - from,
            kind: RegionKind::CameraBlock,
            all_zero: false,
        });
    }
    regions
}

// ---------------------------------------------------------------------------
// Interpreter
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExternalWaitKind {
    /// `SeqExecWaitAnim` while the animation length is not supplied.
    AnimEnd,
    /// `SeqExecWaitLoadVfx` (effect plugin state).
    LoadVfx,
    /// `SeqExecWaitVfx` (effect plugin state).
    VfxEnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PendingKind {
    Sfx,
    SpecialVfx,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarkerKind {
    /// `Anim` with code 255 starts the enemy's default motion (`et.mot`); otherwise the
    /// animation is `anim_list[base_anim + code]` (`anim_list_index`).
    AnimStart {
        code: u8,
        anim_list_index: Option<u16>,
    },
    /// `Wait`: blocks exactly `frames` frames.
    Wait {
        frames: u8,
    },
    /// `WaitAnim` resolved against a supplied animation length (`frames` after the `Anim`).
    WaitAnimEnd {
        resolved_frames: Option<u32>,
    },
    ExternalWait {
        kind: ExternalWaitKind,
    },
    /// `Calc`: the damage/hit point (`ExecVfxCommand` on every target); `index` counts them.
    DamagePoint {
        index: u32,
    },
    MoveToTarget {
        frames: u8,
        distance: i16,
        z_offset: bool,
    },
    MoveToBase {
        frames: u8,
    },
    MoveToPoint {
        frames: u8,
        dst: [i16; 3],
    },
    MoveToOffset {
        frames: u8,
        offset: [i16; 3],
    },
    Turn {
        target: TurnTarget,
        offset: i16,
        frames: u8,
    },
    Scale {
        factor: i16,
        frames: u8,
    },
    /// `Vfx` op executed: `SetBattleVfx(effect, args)` called now.
    VfxStart {
        effect: u16,
        args: [i16; 3],
        flag: u8,
    },
    /// `StartVfx` op: `SFX.SetTaskMonsteraStart()`.
    VfxTaskStart,
    /// The `SVfx` timer expired: `SetBattleVfx(effect, [param, 0, 0, 0])`.
    SpecialVfxTrigger {
        effect: u16,
        param: u8,
    },
    /// The `Sfx` timer expired: `SetBattleSfx(id, volume)`.
    SfxTrigger {
        id: u16,
        volume: u8,
    },
    Camera {
        op: CameraOp,
        id: u8,
    },
    Message {
        id: u8,
        title: bool,
    },
    MeshHide {
        mask: u16,
    },
    MeshShow {
        mask: u16,
    },
    DefaultIdle {
        enabled: u8,
    },
    TexAnim {
        op: TexAnimOp,
        id: u8,
    },
    TargetBone {
        bone: u8,
    },
    FadeOut {
        frames: u8,
    },
    Shadow {
        enabled: u8,
    },
    /// The sequence reached End or FastEnd. `frame` is the frame it was first reached.
    SequenceEnd {
        fast: bool,
    },
    /// A timer was still pending when the sequence finished (it would only fire if the
    /// engine keeps the command alive longer).
    PendingAtEnd {
        kind: PendingKind,
        due_frame: u32,
    },
    /// The interpreter stopped on an opcode it does not implement.
    Unsupported {
        code: u8,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    pub frame: u32,
    pub kind: MarkerKind,
}

/// Caller-supplied facts the file cannot give.
#[derive(Clone, Copy)]
pub struct InterpretOptions<'a> {
    /// `seqBaseAnim[sequence]` (`AnmIDOfs`).
    pub base_anim: u8,
    /// Frames from an `Anim` op until `animEndFrame` becomes true, by `anim_list` index
    /// (`u16::MAX` for code 255). `None` or a `None` answer leaves the wait unresolved.
    pub anim_frames: Option<&'a dyn Fn(u16) -> Option<u32>>,
    /// Safety cap on simulated frames.
    pub max_frames: u32,
}

impl Default for InterpretOptions<'_> {
    fn default() -> Self {
        Self {
            base_anim: 0,
            anim_frames: None,
            max_frames: 20_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trace {
    pub markers: Vec<Marker>,
    /// Frame at which End or FastEnd was first reached (None when the stream is cut off).
    pub end_reached_frame: Option<u32>,
    /// Lower bound of the frame the command finishes: the first frame with no turn pending.
    /// `SeqExecEnd` also waits for the last animation and effect plugin (engine state).
    pub finish_frame: Option<u32>,
    /// False once a mid-sequence wait depended on engine state that was not supplied.
    pub exact: bool,
    pub external_waits: u32,
    /// `Sfx` or `SVfx` timers replaced by a later op before they fired.
    pub overwritten_timers: u32,
    pub hit_frame_cap: bool,
}

#[derive(Default)]
struct Timers {
    sfx: Option<(u32, u16, u8)>,
    svfx: Option<(u32, u16, u8)>,
}

/// Runs the `Sequencer()` loop for one sequence slot without any engine: see the module docs.
pub fn interpret(seq: &DecodedSequence, options: &InterpretOptions<'_>) -> Trace {
    let mut trace = Trace {
        markers: Vec::new(),
        end_reached_frame: None,
        finish_frame: None,
        exact: true,
        external_waits: 0,
        overwritten_timers: 0,
        hit_frame_cap: false,
    };
    let ops = &seq.ops;
    let mut cur = 0_usize;
    let mut old: Option<usize> = None;
    let mut inc_cnt: i32 = 0;
    let mut dec_cnt: i32 = 0;
    let mut anm_cnt: i32 = 0;
    // Counter start for the active timed op (`Init*` sets IncCnt = 1).
    let mut move_frames: i32 = 0;
    let mut scale_frames: i32 = 0;
    // Turn state: `TurnTime`, `TurnCnt`.
    let mut turn_time: i32 = 0;
    let mut turn_cnt: i32 = 0;
    let mut timers = Timers::default();
    let mut calc_index = 0_u32;
    let mut anim_end_at: Option<u32> = None;
    let mut wait_anim_announced = false;
    let mut end_announced = false;
    let mut done = false;
    let mut frame = 0_u32;

    while !done {
        if frame >= options.max_frames {
            trace.hit_frame_cap = true;
            break;
        }
        inc_cnt += 1;
        dec_cnt -= 1;
        anm_cnt += 1;
        let mut tick_done = false;
        while !tick_done {
            let Some(placed) = ops.get(cur) else {
                // Truncated or unsupported stream: stop where the data stops.
                if let Terminator::Unsupported { code, .. } = seq.terminator {
                    trace.markers.push(Marker {
                        frame,
                        kind: MarkerKind::Unsupported { code },
                    });
                }
                trace.exact = false;
                done = true;
                break;
            };
            let init = old != Some(cur);
            old = Some(cur);
            macro_rules! advance {
                () => {
                    cur += 1
                };
            }
            macro_rules! push {
                ($kind:expr) => {
                    trace.markers.push(Marker { frame, kind: $kind })
                };
            }
            match placed.op {
                SeqOp::Wait { frames } => {
                    if init {
                        dec_cnt = i32::from(frames);
                        push!(MarkerKind::Wait { frames });
                    }
                    if dec_cnt <= 0 {
                        advance!();
                    } else {
                        tick_done = true;
                    }
                }
                SeqOp::Calc => {
                    push!(MarkerKind::DamagePoint { index: calc_index });
                    calc_index += 1;
                    advance!();
                }
                SeqOp::MoveToTarget {
                    frames,
                    distance,
                    z_offset,
                } => {
                    if init {
                        inc_cnt = 1;
                        move_frames = i32::from(frames);
                        push!(MarkerKind::MoveToTarget {
                            frames,
                            distance,
                            z_offset,
                        });
                    }
                    if inc_cnt >= move_frames {
                        advance!();
                    }
                    tick_done = true;
                }
                SeqOp::MoveToBase { frames } => {
                    if init {
                        inc_cnt = 1;
                        move_frames = i32::from(frames);
                        push!(MarkerKind::MoveToBase { frames });
                    }
                    if inc_cnt >= move_frames {
                        advance!();
                    }
                    tick_done = true;
                }
                SeqOp::MoveToPoint { frames, dst } => {
                    if init {
                        inc_cnt = 1;
                        move_frames = i32::from(frames);
                        push!(MarkerKind::MoveToPoint { frames, dst });
                    }
                    if inc_cnt >= move_frames {
                        advance!();
                    }
                    tick_done = true;
                }
                SeqOp::MoveToOffset { frames, offset } => {
                    if init {
                        inc_cnt = 1;
                        move_frames = i32::from(frames);
                        push!(MarkerKind::MoveToOffset { frames, offset });
                    }
                    if inc_cnt >= move_frames {
                        advance!();
                    }
                    tick_done = true;
                }
                SeqOp::Anim { code } => {
                    let index =
                        (code != 255).then(|| u16::from(options.base_anim) + u16::from(code));
                    push!(MarkerKind::AnimStart {
                        code,
                        anim_list_index: index,
                    });
                    anim_end_at = options
                        .anim_frames
                        .and_then(|f| f(index.unwrap_or(u16::MAX)))
                        .map(|frames| frame + frames.max(1));
                    anm_cnt = 0;
                    wait_anim_announced = false;
                    advance!();
                }
                SeqOp::SVfx {
                    effect,
                    param,
                    delay,
                } => {
                    if timers.svfx.is_some() {
                        trace.overwritten_timers += 1;
                    }
                    // `SVfxTime = delay + 1`, decremented the same frame: fires at frame + delay.
                    timers.svfx = Some((frame + u32::from(delay), effect, param));
                    advance!();
                }
                SeqOp::WaitAnim => {
                    let ready = anm_cnt != 0 && anim_end_at.is_none_or(|at| frame >= at);
                    if !wait_anim_announced {
                        wait_anim_announced = true;
                        match anim_end_at {
                            Some(at) => push!(MarkerKind::WaitAnimEnd {
                                resolved_frames: Some(at.saturating_sub(frame)),
                            }),
                            None => {
                                trace.exact = false;
                                trace.external_waits += 1;
                                push!(MarkerKind::ExternalWait {
                                    kind: ExternalWaitKind::AnimEnd,
                                });
                            }
                        }
                    }
                    if ready {
                        advance!();
                    } else {
                        tick_done = true;
                    }
                }
                SeqOp::Vfx { effect, args, flag } => {
                    // `GetEffectOvRun() != 0` would block here; engine state, assumed clear.
                    push!(MarkerKind::VfxStart { effect, args, flag });
                    advance!();
                }
                SeqOp::WaitLoadVfx => {
                    trace.exact = false;
                    trace.external_waits += 1;
                    push!(MarkerKind::ExternalWait {
                        kind: ExternalWaitKind::LoadVfx,
                    });
                    advance!();
                }
                SeqOp::StartVfx => {
                    push!(MarkerKind::VfxTaskStart);
                    advance!();
                }
                SeqOp::WaitVfx => {
                    trace.exact = false;
                    trace.external_waits += 1;
                    push!(MarkerKind::ExternalWait {
                        kind: ExternalWaitKind::VfxEnd,
                    });
                    advance!();
                }
                SeqOp::Scale { factor, frames } => {
                    if init {
                        inc_cnt = 1;
                        scale_frames = i32::from(frames);
                        push!(MarkerKind::Scale { factor, frames });
                    }
                    if inc_cnt >= scale_frames {
                        advance!();
                    } else {
                        tick_done = true;
                    }
                }
                SeqOp::MeshHide { mask } => {
                    push!(MarkerKind::MeshHide { mask });
                    advance!();
                }
                SeqOp::MeshShow { mask } => {
                    push!(MarkerKind::MeshShow { mask });
                    advance!();
                }
                SeqOp::Message { id, title } => {
                    push!(MarkerKind::Message { id, title });
                    advance!();
                }
                SeqOp::Camera { op, id } => {
                    push!(MarkerKind::Camera { op, id });
                    advance!();
                }
                SeqOp::DefaultIdle { enabled } => {
                    push!(MarkerKind::DefaultIdle { enabled });
                    advance!();
                }
                SeqOp::Turn {
                    target,
                    offset,
                    frames,
                } => {
                    push!(MarkerKind::Turn {
                        target,
                        offset,
                        frames,
                    });
                    turn_time = i32::from(frames);
                    turn_cnt = 1;
                    advance!();
                }
                SeqOp::TexAnim { op, id } => {
                    push!(MarkerKind::TexAnim { op, id });
                    advance!();
                }
                SeqOp::Sfx {
                    id, delay, volume, ..
                } => {
                    if timers.sfx.is_some() {
                        trace.overwritten_timers += 1;
                    }
                    timers.sfx = Some((frame + u32::from(delay), id, volume));
                    advance!();
                }
                SeqOp::TargetBone { bone } => {
                    push!(MarkerKind::TargetBone { bone });
                    advance!();
                }
                SeqOp::FadeOut { frames } => {
                    push!(MarkerKind::FadeOut { frames });
                    advance!();
                }
                SeqOp::Shadow { enabled } => {
                    push!(MarkerKind::Shadow { enabled });
                    advance!();
                }
                SeqOp::End | SeqOp::FastEnd => {
                    let fast = matches!(placed.op, SeqOp::FastEnd);
                    if !end_announced {
                        end_announced = true;
                        trace.end_reached_frame = Some(frame);
                        push!(MarkerKind::SequenceEnd { fast });
                        // `SeqExecEnd` also waits for the last animation and the effect
                        // plugin; only the turn timer comes from the sequence data, so
                        // `finish_frame` is a lower bound whatever `exact` says.
                    }
                    if turn_time != 0 {
                        tick_done = true;
                    } else {
                        trace.finish_frame = Some(frame);
                        done = true;
                        tick_done = true;
                    }
                }
            }
        }
        // Post-opcode timers, in `Sequencer` order: turn, SFX, special VFX (fade has no marker).
        if turn_time != 0 {
            let before = turn_cnt;
            turn_cnt += 1;
            if before >= turn_time {
                turn_time = 0;
            }
        }
        if let Some((due, id, volume)) = timers.sfx
            && due == frame
        {
            trace.markers.push(Marker {
                frame,
                kind: MarkerKind::SfxTrigger { id, volume },
            });
            timers.sfx = None;
        }
        if let Some((due, effect, param)) = timers.svfx
            && due == frame
        {
            trace.markers.push(Marker {
                frame,
                kind: MarkerKind::SpecialVfxTrigger { effect, param },
            });
            timers.svfx = None;
        }
        frame += 1;
    }
    let last = frame.saturating_sub(1);
    if trace.finish_frame.is_some() {
        if let Some((due, ..)) = timers.sfx {
            trace.markers.push(Marker {
                frame: last,
                kind: MarkerKind::PendingAtEnd {
                    kind: PendingKind::Sfx,
                    due_frame: due,
                },
            });
        }
        if let Some((due, ..)) = timers.svfx {
            trace.markers.push(Marker {
                frame: last,
                kind: MarkerKind::PendingAtEnd {
                    kind: PendingKind::SpecialVfx,
                    due_frame: due,
                },
            });
        }
    }
    trace
}

// ---------------------------------------------------------------------------
// Whole-bundle audit
// ---------------------------------------------------------------------------

fn variant_name(value: &impl Serialize) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(name)) => name,
        Ok(serde_json::Value::Object(map)) => map.keys().next().cloned().unwrap_or_default(),
        _ => String::new(),
    }
}

/// Decodes and interprets every `.raw17` in `files` (keyed by scene) and returns the
/// `audit/battle-sequences.json` document.
pub fn audit(files: &BTreeMap<String, Vec<u8>>) -> serde_json::Value {
    use serde_json::json;
    let mut failed: Vec<serde_json::Value> = Vec::new();
    let (mut parsed, mut slots_total, mut slots_used, mut distinct) = (0_u64, 0_u64, 0_u64, 0_u64);
    let mut opcode_hist: BTreeMap<u8, u64> = BTreeMap::new();
    let mut terminators: BTreeMap<String, u64> = BTreeMap::new();
    let mut unknown: BTreeMap<u8, serde_json::Value> = BTreeMap::new();
    let mut unknown_count: BTreeMap<u8, u64> = BTreeMap::new();
    let mut region_bytes: BTreeMap<String, u64> = BTreeMap::new();
    let (mut total_bytes, mut gap_nonzero, mut overlap) = (0_u64, 0_u64, 0_u64);
    let mut camera_tables = 0_u64;
    let mut cameras_total = 0_u64;
    let mut camera_flags: BTreeMap<String, u64> = BTreeMap::new();
    let mut marker_hist: BTreeMap<String, u64> = BTreeMap::new();
    let (mut exact_traces, mut inexact_traces, mut external_waits) = (0_u64, 0_u64, 0_u64);
    let (mut overwritten, mut frame_caps, mut no_end_traces) = (0_u64, 0_u64, 0_u64);
    let mut damage_points: BTreeMap<String, u64> = BTreeMap::new();
    let mut anim_index_out_of_range = 0_u64;
    let mut max_end_frame = 0_u32;
    for (scene, bytes) in files {
        let file = match SequenceFile::parse(bytes, scene) {
            Ok(file) => file,
            Err(error) => {
                failed.push(json!({"scene": scene, "error": error.to_string()}));
                continue;
            }
        };
        parsed += 1;
        total_bytes += bytes.len() as u64;
        slots_total += file.slots.len() as u64;
        slots_used += file.slots.iter().filter(|s| s.start.is_some()).count() as u64;
        distinct += file.sequences.len() as u64;
        for region in &file.regions {
            *region_bytes
                .entry(format!("{:?}", region.kind))
                .or_insert(0) += region.len as u64;
            match region.kind {
                RegionKind::Gap if !region.all_zero => gap_nonzero += region.len as u64,
                RegionKind::Overlap => overlap += region.len as u64,
                _ => {}
            }
        }
        if !file.cameras.is_empty() {
            camera_tables += 1;
        }
        for camera in &file.cameras {
            cameras_total += 1;
            let key = camera
                .flags
                .map_or("unreadable".to_owned(), |f| format!("{f:#06x}"));
            *camera_flags.entry(key).or_insert(0) += 1;
        }
        for (start, seq) in &file.sequences {
            let slot = file.slots.iter().find(|s| s.start == Some(*start));
            for placed in &seq.ops {
                *opcode_hist.entry(placed.op.code()).or_insert(0) += 1;
            }
            let terminator = match seq.terminator {
                Terminator::End => "End".to_owned(),
                Terminator::FastEnd => "FastEnd".to_owned(),
                Terminator::Unsupported { code, offset } => {
                    *unknown_count.entry(code).or_insert(0) += 1;
                    unknown.entry(code).or_insert_with(
                        || json!({"scene": scene, "slot": slot.map(|s| s.index), "offset": offset}),
                    );
                    "Unsupported".to_owned()
                }
                Terminator::Truncated { .. } => "Truncated".to_owned(),
            };
            *terminators.entry(terminator).or_insert(0) += 1;
            let base_anim = slot.map_or(0, |s| s.base_anim);
            for placed in &seq.ops {
                if let SeqOp::Anim { code } = placed.op
                    && code != 255
                    && usize::from(base_anim) + usize::from(code) >= file.header.anim_list.len()
                {
                    anim_index_out_of_range += 1;
                }
            }
            let trace = interpret(
                seq,
                &InterpretOptions {
                    base_anim,
                    ..InterpretOptions::default()
                },
            );
            if trace.exact {
                exact_traces += 1;
            } else {
                inexact_traces += 1;
            }
            external_waits += u64::from(trace.external_waits);
            overwritten += u64::from(trace.overwritten_timers);
            frame_caps += u64::from(trace.hit_frame_cap);
            match trace.finish_frame {
                Some(frame) => max_end_frame = max_end_frame.max(frame),
                None => no_end_traces += 1,
            }
            let mut hits = 0_u32;
            for marker in &trace.markers {
                *marker_hist.entry(variant_name(&marker.kind)).or_insert(0) += 1;
                if matches!(marker.kind, MarkerKind::DamagePoint { .. }) {
                    hits += 1;
                }
            }
            *damage_points.entry(hits.to_string()).or_insert(0) += 1;
        }
    }
    let opcodes: Vec<serde_json::Value> = opcode_hist
        .iter()
        .map(|(code, count)| {
            json!({
                "code": format!("{code:#04x}"),
                "name": OPCODE_NAMES[usize::from(*code)],
                "ops": count,
            })
        })
        .collect();
    let unknown_list: Vec<serde_json::Value> = unknown
        .iter()
        .map(|(code, first)| {
            json!({
                "code": format!("{code:#04x}"),
                "count": unknown_count[code],
                "first": first,
            })
        })
        .collect();
    json!({
        "scope": "battle sequence data decoded from .raw17; interpreter output is a marker list with no rendering and no gameplay claim",
        "source": "Memoria Global/btlseq.cs (gSeqProg, SeqInit*/SeqExec*, AdvanceSeqCode, Sequencer)",
        "scenes_with_sequence_file": files.len(),
        "parsed": parsed,
        "failed": failed,
        "sequence_slots": slots_total,
        "slots_with_sequence": slots_used,
        "slots_without_sequence": slots_total - slots_used,
        "distinct_streams": distinct,
        "terminators": terminators,
        "opcode_histogram": opcodes,
        "unknown_opcodes": unknown_list,
        "unsupported_opcode_policy": "codes 0x22 and above end decoding and are reported here; the source indexes past gSeqProg for 0x22 and maps larger codes to End",
        "bytes": {
            "total": total_bytes,
            "by_region": region_bytes,
            "gap_nonzero": gap_nonzero,
            "overlap": overlap,
            "unconsumed": gap_nonzero + overlap,
            "note": "Header and Sequence are decoded; Gap is padding between claimed ranges; CameraBlock is kept raw except its offset table and each camera's first u16 (flags)",
        },
        "camera_block": {
            "files_with_table": camera_tables,
            "cameras": cameras_total,
            "flags_histogram": camera_flags,
            "decoded": "offset table and flags word only; camera sequences stay raw (SFXDataCamera.Sequence is not implemented here)",
        },
        "interpretation": {
            "traces": exact_traces + inexact_traces,
            "traces_exact_without_engine_state": exact_traces,
            "traces_with_engine_waits": inexact_traces,
            "external_waits": external_waits,
            "overwritten_timers": overwritten,
            "traces_without_end": no_end_traces,
            "frame_cap_hits": frame_caps,
            "max_finish_frame": max_end_frame,
            "marker_histogram": marker_hist,
            "damage_points_per_sequence": damage_points,
            "anim_index_out_of_range": anim_index_out_of_range,
            "assumptions": "WaitAnim, WaitLoadVfx, WaitVfx, the Vfx busy check and the SeqExecEnd animation and effect conditions depend on engine state; they pass at the first frame the data allows",
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trace_of(bytes: &[u8]) -> Trace {
        interpret(&decode_sequence(bytes, 0), &InterpretOptions::default())
    }

    fn kinds(trace: &Trace) -> Vec<(u32, MarkerKind)> {
        trace.markers.iter().map(|m| (m.frame, m.kind)).collect()
    }

    /// Runs only when a real install is configured through `FFIX_BUNDLE_P0DATA2`.
    #[test]
    fn real_bundle_sequences_decode_without_unknowns() {
        let Some(path) = std::env::var_os("FFIX_BUNDLE_P0DATA2") else {
            return;
        };
        let path = std::path::Path::new(&path);
        if !path.is_file() {
            return;
        }
        let data = crate::pc::battle_scene::BattleBundleData::load(path).unwrap();
        let report = audit(&data.sequence_bytes);
        assert_eq!(report["parsed"], 562);
        assert_eq!(report["unknown_opcodes"].as_array().unwrap().len(), 0);
        assert_eq!(report["bytes"]["unconsumed"], 0);
    }

    #[test]
    fn opcode_lengths_match_table() {
        // Every opcode, padded with zeros, decodes to exactly its table length.
        for code in 0..OPCODE_COUNT {
            let mut bytes = vec![code];
            bytes.extend(std::iter::repeat_n(1, 12));
            let (op, len) = decode_op(&bytes, 0).unwrap();
            assert_eq!(
                len,
                usize::from(OPCODE_LEN[usize::from(code)]),
                "code {code}"
            );
            assert_eq!(op.code(), code);
        }
        assert!(matches!(
            decode_op(&[34, 0, 0], 0),
            Err(Terminator::Unsupported { code: 34, .. })
        ));
        assert!(matches!(
            decode_op(&[8, 0, 0], 0),
            Err(Terminator::Truncated { .. })
        ));
    }

    #[test]
    fn wait_blocks_exactly_n_frames() {
        // Wait 3 at frame 0 blocks frames 0 to 2; Calc runs at frame 3; End at frame 3.
        let t = trace_of(&[1, 3, 2, 0]);
        assert_eq!(
            kinds(&t),
            vec![
                (0, MarkerKind::Wait { frames: 3 }),
                (3, MarkerKind::DamagePoint { index: 0 }),
                (3, MarkerKind::SequenceEnd { fast: false }),
            ]
        );
        assert_eq!(t.finish_frame, Some(3));
        // Wait 0 does not block.
        let t = trace_of(&[1, 0, 2, 0]);
        assert_eq!(t.markers[1].frame, 0);
    }

    #[test]
    fn move_occupies_max_frames_one() {
        // MoveToTarget 5 (opcode 3, distance 100): init frame 0, IncCnt reaches 5 at frame 4,
        // next op starts at frame 5. Frames 0 and 1 give 1 frame each.
        let t = trace_of(&[3, 5, 100, 0, 2, 0]);
        assert_eq!(t.markers[1].frame, 5);
        let t = trace_of(&[3, 0, 0, 0, 2, 0]);
        assert_eq!(t.markers[1].frame, 1);
        let t = trace_of(&[4, 1, 2, 0]);
        assert_eq!(t.markers[1].frame, 1);
        assert_eq!(t.markers[0].kind, MarkerKind::MoveToBase { frames: 1 });
    }

    #[test]
    fn scale_blocks_frames_minus_one() {
        // Scale to 2048 over 4 frames: IncCnt 1,2,3 block (frames 0 to 2); passes at frame 3.
        let t = trace_of(&[0xC, 0x00, 0x08, 4, 2, 0]);
        assert_eq!(t.markers[1].frame, 3);
        let t = trace_of(&[0xC, 0xFF, 0xFF, 1, 2, 0]);
        assert_eq!(t.markers[1].frame, 0);
        assert!(matches!(
            t.markers[0].kind,
            MarkerKind::Scale {
                factor: -1,
                frames: 1
            }
        ));
    }

    #[test]
    fn sfx_and_svfx_timers_fire_after_delay() {
        // Sfx id 0x0102, delay 2, unused 9, volume 7 at frame 0 fires at frame 2.
        // SVfx effect 0x0304 param 5 delay 0 fires at frame 0 (SVfxTime 1, decremented to 0).
        // Wait 4 then End at frame 4.
        let t = trace_of(&[0x19, 0x02, 0x01, 2, 9, 7, 6, 0x04, 0x03, 5, 0, 1, 4, 0]);
        assert_eq!(
            kinds(&t),
            vec![
                (0, MarkerKind::Wait { frames: 4 }),
                (
                    0,
                    MarkerKind::SpecialVfxTrigger {
                        effect: 0x0304,
                        param: 5
                    }
                ),
                (
                    2,
                    MarkerKind::SfxTrigger {
                        id: 0x0102,
                        volume: 7
                    }
                ),
                (4, MarkerKind::SequenceEnd { fast: false }),
            ]
        );
    }

    #[test]
    fn timers_overwrite_and_pend_at_end() {
        // Two Sfx in one frame: the first is overwritten. The second (delay 9) is still
        // pending when End is reached at frame 0.
        let t = trace_of(&[0x19, 1, 0, 1, 0, 1, 0x19, 2, 0, 9, 0, 1, 0]);
        assert_eq!(t.overwritten_timers, 1);
        assert!(
            t.markers
                .iter()
                .all(|m| !matches!(m.kind, MarkerKind::SfxTrigger { .. }))
        );
        assert!(matches!(
            t.markers.last().unwrap().kind,
            MarkerKind::PendingAtEnd {
                kind: PendingKind::Sfx,
                due_frame: 9
            }
        ));
    }

    #[test]
    fn turn_delays_finish_by_its_frames() {
        // Turn with 3 frames at frame 0: TurnTime clears after the post-loop of frame 2, so
        // End (reached at frame 0) finishes at frame 3.
        let t = trace_of(&[0x14, 0x01, 0x80, 0x00, 0x00, 3, 0]);
        assert_eq!(t.end_reached_frame, Some(0));
        assert_eq!(t.finish_frame, Some(3));
        assert!(matches!(
            t.markers[0].kind,
            MarkerKind::Turn {
                target: TurnTarget::FaceTarget,
                offset: 0,
                frames: 3
            }
        ));
        // FastEnd behaves the same for the turn, with the fast flag set.
        let t = trace_of(&[0x14, 0x00, 0x04, 0x00, 0x00, 1, 0x18]);
        assert_eq!(t.finish_frame, Some(1));
        assert!(matches!(
            t.markers[0].kind,
            MarkerKind::Turn {
                target: TurnTarget::Absolute { raw: 0x0400 },
                ..
            }
        ));
        assert!(matches!(
            t.markers[1].kind,
            MarkerKind::SequenceEnd { fast: true }
        ));
    }

    #[test]
    fn anim_wait_uses_supplied_length_or_reports_external() {
        let bytes = [5, 2, 7, 0];
        let seq = decode_sequence(&bytes, 0);
        let t = interpret(&seq, &InterpretOptions::default());
        assert!(t.external_waits >= 1);
        assert!(!t.exact);
        assert_eq!(
            t.markers[0].kind,
            MarkerKind::AnimStart {
                code: 2,
                anim_list_index: Some(2)
            }
        );
        let lengths = |index: u16| (index == 12).then_some(6);
        let options = InterpretOptions {
            base_anim: 10,
            anim_frames: Some(&lengths),
            ..InterpretOptions::default()
        };
        let t = interpret(&seq, &options);
        assert_eq!(
            t.markers[0].kind,
            MarkerKind::AnimStart {
                code: 2,
                anim_list_index: Some(12)
            }
        );
        assert_eq!(
            t.markers[1].kind,
            MarkerKind::WaitAnimEnd {
                resolved_frames: Some(6)
            }
        );
        // The animation ends 6 frames after it started, so End is reached at frame 6.
        assert_eq!(t.end_reached_frame, Some(6));
        // Code 255 is the default enemy motion.
        let seq = decode_sequence(&[5, 255, 0], 0);
        let t = interpret(&seq, &InterpretOptions::default());
        assert_eq!(
            t.markers[0].kind,
            MarkerKind::AnimStart {
                code: 255,
                anim_list_index: None
            }
        );
    }

    #[test]
    fn effect_camera_and_message_markers() {
        let t = trace_of(&[
            8, 0x10, 0x00, 1, 0, 2, 0, 3, 0, // Vfx 0x0010 args 1 2 3
            0x1A, 0x11, 0x00, 4, 0, 0, 0, 0xFF, 0xFF, // Vfx2 0x0011 args 4 0 -1
            0xA, 0x10, 5, 0x20, 6, 0x12, 7, 0xE, 0x81, 0x21, 3, 2, 0,
        ]);
        let k: Vec<MarkerKind> = t.markers.iter().map(|m| m.kind).collect();
        assert_eq!(
            k[0],
            MarkerKind::VfxStart {
                effect: 0x10,
                args: [1, 2, 3],
                flag: 0
            }
        );
        assert_eq!(
            k[1],
            MarkerKind::VfxStart {
                effect: 0x11,
                args: [4, 0, -1],
                flag: 1
            }
        );
        assert_eq!(k[2], MarkerKind::VfxTaskStart);
        assert_eq!(
            k[3],
            MarkerKind::Camera {
                op: CameraOp::Set,
                id: 5
            }
        );
        assert_eq!(
            k[4],
            MarkerKind::Camera {
                op: CameraOp::Run,
                id: 6
            }
        );
        assert_eq!(
            k[5],
            MarkerKind::Camera {
                op: CameraOp::RunIfAlternate,
                id: 7
            }
        );
        assert_eq!(
            k[6],
            MarkerKind::Message {
                id: 0x81,
                title: false
            }
        );
        assert_eq!(k[7], MarkerKind::Message { id: 3, title: true });
        assert_eq!(k[8], MarkerKind::DamagePoint { index: 0 });
        assert!(t.markers.iter().all(|m| m.frame == 0));
    }

    #[test]
    fn unsupported_opcode_stops_the_decoder() {
        let seq = decode_sequence(&[1, 2, 0x40, 0], 0);
        assert_eq!(
            seq.terminator,
            Terminator::Unsupported {
                code: 0x40,
                offset: 2
            }
        );
        assert_eq!(seq.ops.len(), 1);
        let t = interpret(&seq, &InterpretOptions::default());
        assert!(matches!(
            t.markers.last().unwrap().kind,
            MarkerKind::Unsupported { code: 0x40 }
        ));
        assert!(!t.exact);
    }

    #[test]
    fn file_layout_accounts_every_byte() {
        // Header: seqBlock 4, camOffset, 2 sequences, 1 animation.
        // 8 + 2*2 + 4 + 2 = 18 header bytes; seq0 at 18 (offset 14), seq1 at 21 (offset 17),
        // slot 1 shares nothing; then one zero pad byte and a 4-byte camera block at 24.
        let mut b = Vec::new();
        for v in [4_i16, 24, 2, 1] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&14_i16.to_le_bytes());
        b.extend_from_slice(&17_i16.to_le_bytes());
        b.extend_from_slice(&0x0000_0007_i32.to_le_bytes());
        b.extend_from_slice(&[0, 3]); // base anims
        assert_eq!(b.len(), 18);
        b.extend_from_slice(&[1, 2, 0]); // seq0: Wait 2, End  -> 18..21
        b.extend_from_slice(&[2, 0]); // seq1: Calc, End -> 21..23
        b.push(0); // pad -> 23
        assert_eq!(b.len(), 24);
        b.extend_from_slice(&[2, 0, 0, 0]); // camera block: one table entry (offset 2)
        let file = SequenceFile::parse(&b, "t").unwrap();
        assert_eq!(file.sequences.len(), 2);
        assert_eq!(file.slots[1].base_anim, 3);
        assert_eq!(file.sequence(1).unwrap().start, 21);
        let totals = file.region_totals();
        assert_eq!(totals["Header"], 18);
        assert_eq!(totals["Sequence"], 5);
        assert_eq!(totals["Gap"], 1);
        assert_eq!(totals["CameraBlock"], 4);
        assert!(
            file.regions
                .iter()
                .any(|r| r.kind == RegionKind::Gap && r.all_zero)
        );
        let covered: usize = file.regions.iter().map(|r| r.len).sum();
        assert_eq!(covered, b.len());
        assert_eq!(file.cameras.len(), 1);
        assert_eq!(file.cameras[0].set_offset, 2);
    }
}
