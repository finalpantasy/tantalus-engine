//! Sound-effect scripts: the short AKAO programs that FF9's field and battle files carry next to
//! the small sample collections of the `SfxBanks` group.
//!
//! Layout, worked out from the discs (no public documentation exists for it):
//! - Every entry starts with `"AKAO"`, a `u16` kind (1, 2 or 3) and a `u16` 0 where music has its
//!   length, so these are told apart from sequences by that zero.
//! - Kind 1 (one voice): the `u32` at +8 is the id of the sample collection it plays, the `u32`
//!   at +0x14 is `length << 16`, and the opcode stream (the same opcodes as music) starts at
//!   +0x18.
//! - Kind 3 (several voices): the stream of channel 0 starts at +0x20 and is preceded by the pair
//!   `(0u16, length u16)`; the `u32` offsets at +0x14 and +0x18 (each relative to its own
//!   address) point at the other channels, each also preceded by `(0u16, length u16)`. A kind-3
//!   entry names no collection; the one it plays is found from the collections the same file's
//!   kind-1 entries name.
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
fn parse_entry(data: &[u8], at: usize, end: usize, banks: &BTreeSet<u16>) -> Option<SfxEntry> {
    if data.get(at..at + 4)? != b"AKAO" || u16le(data, at + 6)? != 0 {
        return None;
    }
    let kind = u16le(data, at + 4)?;
    let slice = &data[at..end.min(at + 4096)];
    let make = |bank_hint, take: usize, scripts| SfxEntry {
        disc: 0,
        group: 0,
        file_id: 0,
        offset: at,
        kind,
        bank_hint,
        file_banks: Vec::new(),
        data: slice[..take.min(slice.len())].to_vec(),
        scripts,
    };
    match kind {
        1 => {
            let bank = u32le(slice, 8)?;
            let bank = u16::try_from(bank).ok().filter(|b| banks.contains(b))?;
            let len = usize::from(u16le(slice, 0x16)?);
            if u16le(slice, 0x14)? != 0 || len == 0 || 0x18 + len > slice.len() {
                return None;
            }
            Some(make(Some(bank), 0x18 + len, vec![0x18]))
        }
        3 => {
            // Groups of two scripts: a `(0u16, length u16)` pair at +0x1C (and at the offsets of
            // the table at +0x14, +0x18, which are relative to +0x1C), the first script of
            // that length, then the second script directly after it.
            let mut heads = vec![0x1Cusize];
            for p in [0x14usize, 0x18] {
                let rel = u32le(slice, p)? as usize;
                if rel != 0 {
                    heads.push(0x1C + rel);
                }
            }
            let mut scripts = Vec::new();
            for h in heads {
                let len = usize::from(u16le(slice, h + 2)?);
                if u16le(slice, h)? != 0 || len == 0 || h + 4 + len >= slice.len() {
                    return None;
                }
                scripts.push(h + 4);
                scripts.push(h + 4 + len);
            }
            Some(make(None, slice.len(), scripts))
        }
        _ => None,
    }
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
