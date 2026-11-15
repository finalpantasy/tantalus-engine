//! `.mbgdata` movie-background reader.
//!
//! Source: Memoria `MBGDataReader`, `AUDIO_HDR_DEF.ExtractHeaderData` and
//! `MBG_CAM_DEF.ReadBinary`. Layout: `u32` frame count, then `2 * frame_count`
//! records of 100 bytes starting at offset 4 (shorter files yield fewer frames). Memoria reads the header only
//! from the odd records (1, 3, 5, ...); the even records (0, 2, 4, ...) are
//! never read by Memoria, so they are kept here as unread raw bytes
//! (their meaning is unknown).

use crate::install::ImportError;
use serde::{Deserialize, Serialize};

/// Size in bytes of one record (and of one header).
pub const MBG_RECORD_SIZE: usize = 100;

/// `MBG_CAM_DEF`: projection, 3x3 rotation (i16), translation (i32), center offset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MbgCamera {
    pub projection: u16,
    pub rotation: [[i16; 3]; 3],
    pub translation: [i32; 3],
    pub center_offset: [i16; 2],
}

/// `AUDIO_HDR_DEF` fields in file order. Names come from Memoria; Memoria
/// does not interpret `magic`, `sector_type`, `sector_index`, `frame_size`,
/// `audio_data_present`, `mbg_type` or `frame_count`, so those stay raw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MbgFrameHeader {
    pub magic: u16,
    pub sector_type: u16,
    pub sector_index: u16,
    pub sectors_in_frame: u16,
    pub frame_index: u32,
    pub frame_size: u32,
    pub audio_data_present: u16,
    pub mbg_type: u16,
    pub camera_a: MbgCamera,
    pub camera_b: MbgCamera,
    pub force_feedback_bits: u32,
    pub frame_count: u32,
}

impl MbgFrameHeader {
    /// Memoria's derived `vibrate` pair when bit 31 of the force-feedback
    /// bits is set: `[(bits >> 8 & 255) >= 128, bits & 255]`.
    pub fn vibrate(&self) -> [u32; 2] {
        let bits = self.force_feedback_bits;
        if bits & 0x8000_0000 != 0 {
            [u32::from((bits >> 8) & 255 >= 128), bits & 255]
        } else {
            [0, 0]
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MbgFile {
    /// Frame count declared by the file's first word. May exceed `frames.len()`
    /// when the file is shorter than declared (see `parse`).
    pub frame_count: u32,
    /// One header per frame actually present (odd records).
    pub frames: Vec<MbgFrameHeader>,
    /// Bytes Memoria never reads: the 100-byte even records, in order.
    /// Meaning unknown.
    pub unread_records: Vec<Vec<u8>>,
    /// Bytes after the last record (Memoria ignores them). Empty when exact.
    pub trailing: Vec<u8>,
}

impl MbgFile {
    /// Bytes read into typed headers plus the frame count word.
    pub fn header_bytes(&self) -> usize {
        4 + self.frames.len() * MBG_RECORD_SIZE
    }

    /// Bytes not read into a typed header: unread records plus trailing.
    pub fn unconsumed_bytes(&self) -> usize {
        self.unread_records.iter().map(Vec::len).sum::<usize>() + self.trailing.len()
    }

    /// Memoria `GetMBGCam`: camera A of the frame, clamped to the last frame.
    pub fn camera(&self, frame: usize) -> Option<&MbgCamera> {
        self.frames
            .get(frame)
            .or_else(|| self.frames.last())
            .map(|header| &header.camera_a)
    }

    pub fn parse(bytes: &[u8], source: &str) -> Result<Self, ImportError> {
        let count_bytes = bytes
            .get(..4)
            .ok_or_else(|| ImportError::invalid(source, "mbgdata is shorter than 4 bytes"))?;
        let frame_count = u32::from_le_bytes(count_bytes.try_into().expect("four bytes"));
        // Memoria's loop reads the header of record `2k + 1` only while the
        // read position (end of the previous header) is still inside the
        // file, so a file shorter than the declared count simply yields
        // fewer frames (three shipped files declare 2850 frames but hold
        // fewer). A header cut in half would make Memoria throw; that is
        // an error here too.
        let mut frames = Vec::new();
        let mut unread_records = Vec::new();
        let mut position = 4usize;
        for frame in 0..frame_count as usize {
            let header_start = frame
                .checked_mul(2 * MBG_RECORD_SIZE)
                .and_then(|offset| offset.checked_add(4 + MBG_RECORD_SIZE))
                .ok_or_else(|| ImportError::invalid(source, "mbgdata size overflow"))?;
            if position >= bytes.len() {
                break;
            }
            let end = header_start + MBG_RECORD_SIZE;
            if end > bytes.len() {
                return Err(ImportError::invalid(
                    source,
                    format!(
                        "mbgdata frame {frame} header at {header_start} is cut off by the {}-byte end of file",
                        bytes.len()
                    ),
                ));
            }
            unread_records.push(bytes[header_start - MBG_RECORD_SIZE..header_start].to_vec());
            frames.push(parse_header(&bytes[header_start..end]));
            position = end;
        }
        Ok(Self {
            frame_count,
            frames,
            unread_records,
            trailing: bytes[position..].to_vec(),
        })
    }
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn u16(&mut self) -> u16 {
        let value = u16::from_le_bytes(self.data[self.at..self.at + 2].try_into().unwrap());
        self.at += 2;
        value
    }
    fn i16(&mut self) -> i16 {
        self.u16() as i16
    }
    fn u32(&mut self) -> u32 {
        let value = u32::from_le_bytes(self.data[self.at..self.at + 4].try_into().unwrap());
        self.at += 4;
        value
    }
    fn i32(&mut self) -> i32 {
        self.u32() as i32
    }
    fn camera(&mut self) -> MbgCamera {
        let projection = self.u16();
        let mut rotation = [[0; 3]; 3];
        for row in &mut rotation {
            for cell in row {
                *cell = self.i16();
            }
        }
        let translation = [self.i32(), self.i32(), self.i32()];
        let center_offset = [self.i16(), self.i16()];
        MbgCamera {
            projection,
            rotation,
            translation,
            center_offset,
        }
    }
}

fn parse_header(data: &[u8]) -> MbgFrameHeader {
    let mut r = Reader { data, at: 0 };
    let header = MbgFrameHeader {
        magic: r.u16(),
        sector_type: r.u16(),
        sector_index: r.u16(),
        sectors_in_frame: r.u16(),
        frame_index: r.u32(),
        frame_size: r.u32(),
        audio_data_present: r.u16(),
        mbg_type: r.u16(),
        camera_a: r.camera(),
        camera_b: r.camera(),
        force_feedback_bits: r.u32(),
        frame_count: r.u32(),
    };
    debug_assert_eq!(r.at, MBG_RECORD_SIZE);
    header
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera_bytes(seed: i16) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(300u16.to_le_bytes());
        for i in 0..9 {
            out.extend((seed + i).to_le_bytes());
        }
        for i in 0..3 {
            out.extend((i32::from(seed) * 1000 + i).to_le_bytes());
        }
        out.extend((-5i16).to_le_bytes());
        out.extend(7i16.to_le_bytes());
        out
    }

    fn header_bytes(frame: u32, ffb: u32) -> Vec<u8> {
        let mut out = Vec::new();
        for v in [1u16, 2, 3, 4] {
            out.extend(v.to_le_bytes());
        }
        out.extend(frame.to_le_bytes());
        out.extend(99u32.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out.extend(camera_bytes(10));
        out.extend(camera_bytes(20));
        out.extend(ffb.to_le_bytes());
        out.extend(2u32.to_le_bytes());
        assert_eq!(out.len(), MBG_RECORD_SIZE);
        out
    }

    fn fixture() -> Vec<u8> {
        let mut out = 2u32.to_le_bytes().to_vec();
        for frame in 0..2 {
            out.extend(vec![0xEE; MBG_RECORD_SIZE]);
            out.extend(header_bytes(frame, 0x8000_0005));
        }
        out
    }

    #[test]
    fn parses_headers_from_odd_records() {
        let file = MbgFile::parse(&fixture(), "t").unwrap();
        assert_eq!(file.frames.len(), 2);
        assert_eq!(file.frames[1].frame_index, 1);
        assert_eq!(file.frames[0].camera_b.rotation[2][2], 28);
        assert_eq!(file.frames[0].camera_a.translation, [10000, 10001, 10002]);
        assert_eq!(file.frames[0].camera_a.center_offset, [-5, 7]);
        assert_eq!(file.frames[0].vibrate(), [0, 5]);
        assert_eq!(file.unread_records.len(), 2);
        assert!(file.trailing.is_empty());
        assert_eq!(
            file.header_bytes() + file.unconsumed_bytes(),
            fixture().len()
        );
    }

    #[test]
    fn camera_clamps_to_last_frame() {
        let file = MbgFile::parse(&fixture(), "t").unwrap();
        assert_eq!(file.camera(99), Some(&file.frames[1].camera_a));
    }

    #[test]
    fn rejects_truncated_and_reports_trailing() {
        let mut bytes = fixture();
        assert!(MbgFile::parse(&bytes[..bytes.len() - 1], "t").is_err());
        // declared 3 frames but only 2 present: tolerated like Memoria
        bytes[0] = 3;
        let short = MbgFile::parse(&bytes, "t").unwrap();
        assert_eq!((short.frame_count, short.frames.len()), (3, 2));
        bytes[0] = 2;
        assert!(MbgFile::parse(&[1, 2], "t").is_err());
        bytes.extend([1, 2, 3]);
        assert_eq!(MbgFile::parse(&bytes, "t").unwrap().trailing, [1, 2, 3]);
    }
}
