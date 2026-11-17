//! Square's AKAO containers as FF9 (PS1) stores them: sample collections and sequences.
//!
//! Layouts follow the public notes cited in the crate docs (header sizes and field offsets for
//! "version 3.2", the one FF9 uses) and were checked against the discs.
//!
//! Both kinds start with `"AKAO"` and a `u16` id. The `u16` at offset 6 is the sequence length
//! (header included); it is 0 in a sample collection.

use crate::disc::Disc;
use crate::img::{GroupKind, Img, ImgFile};

fn u16le(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(at..at + 2)?.try_into().ok()?))
}
fn u32le(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

/// One articulation (a sample with tuning and a default envelope), 16 bytes on disc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Articulation {
    pub id: u32,
    /// Byte offset of the first ADPCM block, from the start of the sample section.
    pub sample_offset: u32,
    /// Byte offset of the loop target, from the start of the sample section.
    pub loop_offset: u32,
    pub fine_tune: i16,
    pub unity_key: u8,
    pub adsr1: u16,
    pub adsr2: u16,
}

impl Articulation {
    /// Pitch multiplier of the fine-tune word (positive values add up to +100 %, negative ones
    /// scale the unsigned value over 0x10000).
    pub fn tune_multiplier(&self) -> f64 {
        if self.fine_tune >= 0 {
            1.0 + f64::from(self.fine_tune) / 32768.0
        } else {
            f64::from(self.fine_tune as u16) / 65536.0
        }
    }
}

/// A sample collection: articulations plus the raw SPU-ADPCM sample section.
#[derive(Debug, Clone)]
pub struct SampleBank {
    pub id: u16,
    pub spu_dest: u32,
    pub start_art: u32,
    pub arts: Vec<Articulation>,
    pub adpcm: Vec<u8>,
}

impl SampleBank {
    pub fn parse(data: &[u8], at: usize) -> Result<Self, String> {
        let head = data
            .get(at..at + 0x40)
            .ok_or("sample bank header cut off")?;
        if &head[0..4] != b"AKAO" || u16le(head, 6) != Some(0) {
            return Err("not an AKAO sample collection".to_owned());
        }
        let size = u32le(head, 0x14).expect("in range") as usize;
        let start_art = u32le(head, 0x18).expect("in range");
        let count = u32le(head, 0x1C).expect("in range") as usize;
        if count == 0 || count > 300 {
            return Err(format!("implausible articulation count {count}"));
        }
        let arts_at = at + 0x40;
        let section = arts_at + count * 0x10;
        let mut arts = Vec::with_capacity(count);
        for i in 0..count {
            let a = arts_at + i * 0x10;
            let rec = data.get(a..a + 0x10).ok_or("articulation table cut off")?;
            arts.push(Articulation {
                id: start_art + i as u32,
                sample_offset: u32le(rec, 0).expect("in range"),
                loop_offset: u32le(rec, 4).expect("in range"),
                fine_tune: u16le(rec, 8).expect("in range") as i16,
                unity_key: rec[10],
                adsr1: u16le(rec, 12).expect("in range"),
                adsr2: u16le(rec, 14).expect("in range"),
            });
        }
        let end = (section + size).min(data.len());
        Ok(Self {
            id: u16le(head, 4).expect("in range"),
            spu_dest: u32le(head, 0x10).expect("in range"),
            start_art,
            arts,
            adpcm: data[section.min(end)..end].to_vec(),
        })
    }

    pub fn art(&self, id: u32) -> Option<&Articulation> {
        id.checked_sub(self.start_art)
            .and_then(|i| self.arts.get(i as usize))
    }
}

/// An AKAO sequence: header fields and the whole byte range (header included) so channel
/// offsets, jumps and instrument maps stay relative to it.
#[derive(Debug, Clone)]
pub struct Sequence {
    pub id: u16,
    /// Reverb type 0..=9 (index into the SDK reverb presets).
    pub reverb_type: u16,
    /// The sample collection this song plays on (`id` of a [`SampleBank`]).
    pub sample_set: u16,
    pub data: Vec<u8>,
    /// Start of each channel's opcode stream within `data`.
    pub channels: Vec<usize>,
    /// Custom instrument table (`0xFE 0x14`), as an offset within `data`.
    pub instrument_map: Option<usize>,
    /// Drum map (`0xFE 0x04`), as an offset within `data`.
    pub drum_map: Option<usize>,
}

impl Sequence {
    pub fn parse(data: &[u8], at: usize) -> Result<Self, String> {
        let head = data.get(at..at + 0x40).ok_or("sequence header cut off")?;
        if &head[0..4] != b"AKAO" {
            return Err("not an AKAO header".to_owned());
        }
        let length = u16le(head, 6).expect("in range") as usize;
        if length < 0x40 {
            return Err("AKAO header is a sample collection, not a sequence".to_owned());
        }
        let end = (at + length).min(data.len());
        let seq = data[at..end].to_vec();
        let mask = u32le(&seq, 0x20).expect("in range");
        let mut channels = Vec::new();
        for i in 0..mask.count_ones() as usize {
            let p = 0x40 + i * 2;
            let rel = u16le(&seq, p).ok_or("channel table cut off")? as usize;
            channels.push(p + rel);
        }
        let map = |field: usize| match u32le(&seq, field) {
            Some(0) | None => None,
            Some(rel) => Some(field + rel as usize),
        };
        Ok(Self {
            id: u16le(&seq, 4).expect("in range"),
            reverb_type: u16le(&seq, 8).expect("in range"),
            sample_set: u16le(&seq, 0x14).expect("in range"),
            channels,
            instrument_map: map(0x30),
            drum_map: map(0x34),
            data: seq,
        })
    }
}

/// Offsets of every `AKAO` signature in `data`.
pub fn find_akao(data: &[u8]) -> Vec<usize> {
    data.windows(4)
        .enumerate()
        .filter(|(_, w)| *w == b"AKAO")
        .map(|(i, _)| i)
        .collect()
}

/// A music file of the archive: its own sample collection plus the song.
#[derive(Debug, Clone)]
pub struct Song {
    pub disc: u8,
    /// `FF9.IMG` file id within the music group.
    pub file_id: u32,
    /// `None` when the file carries only the sequence (the bank is in another file).
    pub bank: Option<SampleBank>,
    pub sequence: Sequence,
}

impl Song {
    pub fn parse(disc: u8, file: &ImgFile, data: &[u8]) -> Result<Self, String> {
        let mut bank = None;
        let mut sequence = None;
        for at in find_akao(data) {
            let Some(length) = u16le(data, at + 6) else {
                continue;
            };
            if length == 0 {
                if bank.is_none() {
                    bank = SampleBank::parse(data, at).ok();
                }
            } else if sequence.is_none() {
                sequence = Sequence::parse(data, at).ok();
            }
        }
        Ok(Self {
            disc,
            file_id: file.id,
            bank,
            sequence: sequence.ok_or("music file has no sequence")?,
        })
    }
}

/// Reads every song of one disc's music group.
pub fn read_songs(disc_no: u8, disc: &mut Disc, img: &Img) -> Vec<Result<Song, String>> {
    let groups = img.groups_of(disc, GroupKind::Music).unwrap_or_default();
    img.files
        .iter()
        .filter(|f| groups.contains(&f.group))
        .map(|file| {
            let data = img.read(disc, file)?;
            Song::parse(disc_no, file, &data)
                .map_err(|e| format!("disc {disc_no} music file {}: {e}", file.id))
        })
        .collect()
}
