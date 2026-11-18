//! Sound-effect scripts: the short AKAO programs that FF9's field and battle files carry next to
//! the small sample collections of the `SfxBanks` group.
//!
//! Layout, worked out from the discs (no public documentation exists for it):
//! - Every entry starts with `"AKAO"`, a `u16` count of voice pairs (1 to 4) and a `u16` 0 where
//!   music has its length, so these are told apart from sequences by that zero.
//! - The `u32` at +8 is the id of the sample collection it plays (0 is the resident collection,
//!   articulations 128 to 255, that the field and menu programs also draw on).
//! - Pair heads follow: the first at `0x14 + 4 * (pairs - 1)`, the others at the `u32` offsets
//!   from +0x14 on (relative to the first head). A head is two `u16` offsets from its end: voice
//!   A's (always 0) and voice B's; `0xFFFF` marks a voice the pair does not have. Voice A is often a short
//!   prefix (a rest or a detune) that runs on into voice B's opcodes: the two then play the same
//!   notes as a chorus. Both play.
//! - The opcode stream ends with `0xA0`.

use std::collections::BTreeSet;

use crate::akao::{Sequence, find_akao};
use crate::disc::Disc;
use crate::img::{GroupKind, Img};

/// One sound-effect program as found in a file: one or more scripts (voices) on one entry.
#[derive(Debug, Clone)]
pub struct SfxEntry {
    pub disc: u8,
    pub group: u32,
    pub file_id: u32,
    /// Byte offset of the `AKAO` tag in the file.
    pub offset: usize,
    /// 1 for a single-voice program, 3 for a container of several.
    pub kind: u16,
    /// Collection id named by a kind-1 entry.
    pub bank_hint: Option<u16>,
    /// Collections named by the kind-1 entries of the same file (candidates for a kind-3 entry).
    pub file_banks: Vec<u16>,
    /// The entry's bytes, from the `AKAO` tag up to the next entry (or a size cap).
    pub data: Vec<u8>,
    /// Start of each script within `data`; each is played on its own.
    pub scripts: Vec<usize>,
}

impl SfxEntry {
    /// One script as a one-channel sequence the player can run. Reverb type 0 is the neutral
    /// default; the driver's own reverb switch (`0xC2`) still selects the voices that feed it.
    pub fn sequence(&self, script: usize) -> Sequence {
        Sequence {
            id: u16::from_le_bytes([self.data[4], self.data[5]]),
            reverb_type: 0,
            sample_set: self.bank_hint.unwrap_or(0),
            data: self.data.clone(),
            channels: vec![self.scripts[script]],
            instrument_map: None,
            drum_map: None,
        }
    }

    /// The first articulation the script selects (`0xA1 <id>`), found by walking opcodes.
    pub fn first_articulation(&self, script: usize) -> Option<u32> {
        let mut pc = self.scripts[script];
        for _ in 0..64 {
            let op = *self.data.get(pc)?;
            let size = match op {
                0xA1 => return self.data.get(pc + 1).map(|v| u32::from(*v)),
                0xA0 | 0x9A..=0x9F | 0xFF => return None,
                0xA2..=0xA5 | 0xA8 | 0xAA | 0xAC | 0xAD..=0xAF | 0xB1..=0xB2 | 0xB5 | 0xB7 => 2,
                0xB9 | 0xBB | 0xBF | 0xC0 | 0xC1 | 0xCE | 0xCF | 0xD2 | 0xD3 | 0xD8 | 0xD9 => 2,
                0xA9 | 0xAB | 0xB0 | 0xB4 | 0xB8 | 0xBC | 0xDD..=0xDF | 0xE4..=0xE6 => 3,
                0xF0..=0xFD => 3,
                _ => 1,
            };
            pc += size;
        }
        None
    }
}

fn u16le(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(at..at + 2)?.try_into().ok()?))
}
fn u32le(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

/// Parses the entry at `at`, or `None` when the bytes are not a sound-effect program. `end` is the
/// start of the next entry (or the end of the file).
///
/// The `u16` after the tag is the number of voice pairs (1 to 4). The pair heads follow the
/// header: the first at `0x14 + 4 * (pairs - 1)`, the others at the `u32` offsets at `+0x14`
/// onward, relative to the first head. A head is two `u16` offsets from its end, voice A's (0)
/// and voice B's; `0xFFFF` marks a voice the pair does not have. Voice A often is only a short prefix (a
/// rest, a detune) that runs on into voice B's opcodes, so the entry keeps all its bytes. The
/// `u32` at `+8` names the sample collection the program plays.
/// A pair head's offset for a voice the pair does not have.
const NO_VOICE: u16 = 0xFFFF;

fn parse_entry(data: &[u8], at: usize, end: usize, banks: &BTreeSet<u16>) -> Option<SfxEntry> {
    if data.get(at..at + 4)? != b"AKAO" || u16le(data, at + 6)? != 0 {
        return None;
    }
    let kind = u16le(data, at + 4)?;
    if !(1..=4).contains(&kind) {
        return None;
    }
    let slice = &data[at..end.min(at + 4096)];
    let named = u16::try_from(u32le(slice, 8)?)
        .ok()
        .filter(|b| banks.contains(b));
    // A single-pair entry naming no known collection is a false match of the tag.
    if kind == 1 && named.is_none() {
        return None;
    }
    let first = 0x14 + 4 * (usize::from(kind) - 1);
    let mut heads = vec![first];
    for p in 1..usize::from(kind) {
        let rel = u32le(slice, 0x14 + 4 * (p - 1))? as usize;
        if rel != 0 {
            heads.push(first + rel);
        }
    }
    let mut scripts = Vec::new();
    for h in heads {
        let (a, b) = (u16le(slice, h)?, u16le(slice, h + 2)?);
        // Voice A is at offset 0 or absent; a pair with neither voice is a false match.
        if (a != 0 && a != NO_VOICE) || a == b {
            return None;
        }
        for v in [a, b] {
            let at = h + 4 + usize::from(v);
            if v != NO_VOICE && at < slice.len() {
                scripts.push(at);
            }
        }
    }
    if scripts.is_empty() {
        return None;
    }
    Some(SfxEntry {
        disc: 0,
        group: 0,
        file_id: 0,
        offset: at,
        kind,
        bank_hint: named,
        file_banks: Vec::new(),
        data: slice.to_vec(),
        scripts,
    })
}

/// Parses one sound program held as its own object (a field's or the resident sound list, see
/// [`crate::cluster`]): the `AKAO` tag at byte 0.
pub fn parse_program(data: &[u8], banks: &BTreeSet<u16>) -> Option<SfxEntry> {
    parse_entry(data, 0, data.len(), banks)
}

/// Parses one stand-alone sound file (an effect's small file): the `AKAO` tag at byte 0 and the
/// program filling the rest.
pub fn parse_sound_file(data: &[u8], banks: &BTreeSet<u16>) -> Option<SfxEntry> {
    parse_entry(data, 0, data.len(), banks)
}

impl SfxEntry {
    /// Every script of the entry as the channels of one sequence, so a container's voices play
    /// together as the console played the file.
    pub fn sequence_all(&self) -> Sequence {
        Sequence {
            id: u16::from_le_bytes([self.data[4], self.data[5]]),
            reverb_type: 0,
            sample_set: self.bank_hint.unwrap_or(0),
            data: self.data.clone(),
            channels: self.scripts.clone(),
            instrument_map: None,
            drum_map: None,
        }
    }
}

/// All sound-effect programs of one disc, in file order. `banks` are the collection ids that
/// exist (kind-1 entries naming anything else are rejected as false matches).
pub fn read_sfx_entries(
    disc_no: u8,
    disc: &mut Disc,
    img: &Img,
    banks: &BTreeSet<u16>,
) -> Vec<SfxEntry> {
    let mut groups: Vec<u32> = img.files.iter().map(|f| f.group).collect();
    groups.dedup();
    let mut out = Vec::new();
    for group in groups {
        if !matches!(img.classify(disc, group), Ok(GroupKind::Other)) {
            continue;
        }
        for file in img.group(group) {
            // Models and backgrounds are big and never carry these programs.
            if file.blocks == 0 || file.blocks > 6000 {
                continue;
            }
            let Ok(data) = img.read(disc, file) else {
                continue;
            };
            let hits = find_akao(&data);
            let mut found: Vec<SfxEntry> = hits
                .iter()
                .enumerate()
                .filter_map(|(i, &at)| {
                    let end = hits.get(i + 1).copied().unwrap_or(data.len());
                    parse_entry(&data, at, end, banks)
                })
                .collect();
            let mut file_banks: Vec<u16> = found.iter().filter_map(|e| e.bank_hint).collect();
            file_banks.sort_unstable();
            file_banks.dedup();
            for e in &mut found {
                e.disc = disc_no;
                e.group = group;
                e.file_id = file.id;
                e.file_banks = file_banks.clone();
            }
            out.extend(found);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(kind: u16) -> Vec<u8> {
        let mut h = b"AKAO".to_vec();
        h.extend_from_slice(&kind.to_le_bytes());
        h.extend_from_slice(&0u16.to_le_bytes());
        h
    }

    #[test]
    fn parses_a_single_voice_program() {
        let script = [0xA1, 0x90, 0xA5, 0x04, 0x00, 0xA0];
        let mut d = header(1);
        d.extend_from_slice(&0x136u32.to_le_bytes());
        d.resize(0x14, 0);
        d.extend_from_slice(&0u16.to_le_bytes());
        d.extend_from_slice(&(script.len() as u16).to_le_bytes());
        d.extend_from_slice(&script);
        let banks = BTreeSet::from([0x136u16]);
        let e = parse_entry(&d, 0, d.len(), &banks).expect("parses");
        assert_eq!(
            (e.kind, e.bank_hint, e.scripts.clone()),
            (1, Some(0x136), vec![0x18])
        );
        assert_eq!(e.first_articulation(0), Some(0x90));
        // A collection that does not exist is a false match.
        assert!(parse_entry(&d, 0, d.len(), &BTreeSet::new()).is_none());
    }

    #[test]
    fn a_short_voice_a_runs_on_into_voice_b() {
        // Field program 108's shape: voice A is a one-byte rest, then voice B's opcodes, which
        // voice A also runs through.
        let script = [0xC2, 0xA1, 0x85, 0xA5, 0x07, 0xA0];
        let mut d = header(1);
        d.extend_from_slice(&0u32.to_le_bytes());
        d.resize(0x14, 0);
        d.extend_from_slice(&[0, 0, 1, 0, 0x99]);
        d.extend_from_slice(&script);
        let e = parse_program(&d, &BTreeSet::from([0u16])).expect("parses");
        assert_eq!(
            (e.bank_hint, e.scripts.clone()),
            (Some(0), vec![0x18, 0x19])
        );
        assert_eq!(e.data.len(), d.len());
        assert_eq!(e.first_articulation(0), Some(0x85));
        assert_eq!(e.first_articulation(1), Some(0x85));
    }

    #[test]
    fn a_pair_of_length_ffff_has_one_voice() {
        // Two pairs: the second head (at 0x18 + 0x0C) has no voice B.
        let s1 = [0xA1, 0x8B, 0xA0];
        let mut d = header(2);
        d.extend_from_slice(&2u32.to_le_bytes());
        d.resize(0x14, 0);
        d.extend_from_slice(&0x0Cu32.to_le_bytes());
        d.extend_from_slice(&[0, 0, 3, 0]);
        d.extend_from_slice(&s1);
        d.extend_from_slice(&s1);
        d.extend_from_slice(&[0xFF, 0xFF]);
        d.extend_from_slice(&[0, 0, 0xFF, 0xFF]);
        d.extend_from_slice(&s1);
        let e = parse_program(&d, &BTreeSet::from([2u16])).expect("parses");
        assert_eq!(e.bank_hint, Some(2));
        assert_eq!(e.scripts, vec![0x1C, 0x1F, 0x28]);
    }

    #[test]
    fn a_pair_can_have_only_voice_b() {
        // Field program 892's head `(0xFFFF, 0)`: no voice A, voice B right after the head.
        let script = [0xC2, 0xA1, 0xA8, 0xA5, 0x06, 0xA0];
        let mut d = header(1);
        d.extend_from_slice(&0x2BCu32.to_le_bytes());
        d.resize(0x14, 0);
        d.extend_from_slice(&[0xFF, 0xFF, 0, 0]);
        d.extend_from_slice(&script);
        let e = parse_program(&d, &BTreeSet::from([0x2BCu16])).expect("parses");
        assert_eq!(e.scripts, vec![0x18]);
        // Both voices absent, or both at the same place, is not a program.
        d[0x14..0x18].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
        assert!(parse_program(&d, &BTreeSet::from([0x2BCu16])).is_none());
        d[0x14..0x18].copy_from_slice(&[0, 0, 0, 0]);
        assert!(parse_program(&d, &BTreeSet::from([0x2BCu16])).is_none());
    }

    #[test]
    fn parses_a_container_of_voice_pairs() {
        // Header, then two groups of two scripts: the table at +0x14 is relative to +0x1C.
        let s1 = [0xA1, 0x8B, 0xA0];
        let s2 = [0xA1, 0xAD, 0xA0];
        let mut d = header(3);
        d.resize(0x14, 0);
        d.extend_from_slice(&0x0Du32.to_le_bytes()); // second group at 0x1C + 0x0D
        d.extend_from_slice(&0u32.to_le_bytes());
        d.extend_from_slice(&[0, 0, s1.len() as u8, 0]);
        d.extend_from_slice(&s1);
        d.extend_from_slice(&s2);
        d.extend_from_slice(&[0xFF; 3]);
        d.extend_from_slice(&[0, 0, s2.len() as u8, 0]);
        d.extend_from_slice(&s2);
        d.extend_from_slice(&s1);
        d.resize(d.len() + 8, 0);
        let e = parse_entry(&d, 0, d.len(), &BTreeSet::new()).expect("parses");
        assert_eq!(e.kind, 3);
        assert_eq!(e.scripts, vec![0x20, 0x23, 0x2D, 0x30]);
        assert_eq!(e.first_articulation(1), Some(0xAD));
        assert_eq!(e.first_articulation(2), Some(0xAD));
    }

    #[test]
    fn a_stand_alone_sound_file_carries_all_its_voices() {
        let s1 = [0xA1, 0x8B, 0xA0];
        let s2 = [0xA1, 0xAD, 0xA0];
        let mut d = header(3);
        d.resize(0x14, 0);
        d.extend_from_slice(&0x0Du32.to_le_bytes());
        d.extend_from_slice(&0u32.to_le_bytes());
        d.extend_from_slice(&[0, 0, s1.len() as u8, 0]);
        d.extend_from_slice(&s1);
        d.extend_from_slice(&s2);
        d.extend_from_slice(&[0xFF; 3]);
        d.extend_from_slice(&[0, 0, s2.len() as u8, 0]);
        d.extend_from_slice(&s2);
        d.extend_from_slice(&s1);
        d.resize(d.len() + 8, 0);
        let e = parse_sound_file(&d, &BTreeSet::new()).expect("parses");
        assert_eq!(e.sequence_all().channels, e.scripts);
        assert!(parse_sound_file(&d[4..], &BTreeSet::new()).is_none());
    }
}
