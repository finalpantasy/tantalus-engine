//! Streaming Ogg page reader for the PC movie containers (`ma/*.bytes`).
//!
//! Measurement only: parses page headers, optionally verifies page CRCs,
//! groups pages into logical streams and identifies the codec of each stream
//! from its first packet. No video or audio is decoded. Payloads are skipped
//! with `seek` unless CRC verification is requested.

use crate::install::ImportError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

const CRC_POLY: u32 = 0x04C1_1DB7;
const FIRST_PACKET_CAPTURE: usize = 64;

fn crc_table() -> &'static [u32; 256] {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [0_u32; 256];
        for (i, slot) in table.iter_mut().enumerate() {
            let mut r = (i as u32) << 24;
            for _ in 0..8 {
                r = if r & 0x8000_0000 != 0 {
                    (r << 1) ^ CRC_POLY
                } else {
                    r << 1
                };
            }
            *slot = r;
        }
        table
    })
}

fn crc_update(mut crc: u32, data: &[u8]) -> u32 {
    let table = crc_table();
    for &byte in data {
        crc = (crc << 8) ^ table[(((crc >> 24) as u8) ^ byte) as usize];
    }
    crc
}

/// Ogg CRC32 of `data` (polynomial 0x04C11DB7, initial value 0, no reflection).
pub fn ogg_crc32(data: &[u8]) -> u32 {
    crc_update(0, data)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TheoraInfo {
    pub version: String,
    pub frame_width: u32,
    pub frame_height: u32,
    pub picture_width: u32,
    pub picture_height: u32,
    pub picture_x: u8,
    pub picture_y: u8,
    pub fps_numerator: u32,
    pub fps_denominator: u32,
    pub pixel_format: String,
    pub keyframe_granule_shift: u8,
    pub nominal_bitrate: u32,
    pub quality: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VorbisInfo {
    pub channels: u8,
    pub sample_rate: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "codec")]
pub enum StreamCodec {
    Theora(TheoraInfo),
    Vorbis(VorbisInfo),
    Opus { channels: u8, pre_skip: u16 },
    Flac,
    Other { first_bytes_hex: String },
}

impl StreamCodec {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Theora(_) => "theora",
            Self::Vorbis(_) => "vorbis",
            Self::Opus { .. } => "opus",
            Self::Flac => "flac",
            Self::Other { .. } => "other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OggStream {
    pub serial: u32,
    pub codec: StreamCodec,
    pub pages: u64,
    pub last_granule: Option<i64>,
    pub has_eos: bool,
    pub duration_seconds: Option<f64>,
    pub frame_count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrcFailure {
    pub offset: u64,
    pub serial: u32,
    pub sequence: u32,
    pub stored: u32,
    pub computed: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OggReport {
    pub file_size: u64,
    pub page_count: u64,
    pub crc_verified: bool,
    pub crc_failures: Vec<CrcFailure>,
    pub sequence_gaps: u64,
    pub streams: Vec<OggStream>,
    /// Set when parsing stopped before the end of the file.
    pub parse_error: Option<String>,
}

struct Builder {
    first_packet: Vec<u8>,
    pages: u64,
    last_granule: Option<i64>,
    has_eos: bool,
    next_sequence: u32,
}

/// Scan an Ogg file page by page. Set `verify_crc` to read every payload and
/// check page checksums; otherwise payloads are skipped by seeking.
pub fn scan_ogg_file(path: &Path, verify_crc: bool) -> Result<OggReport, ImportError> {
    let file = File::open(path).map_err(|e| ImportError::io("open Ogg file", path, e))?;
    let size = file
        .metadata()
        .map_err(|e| ImportError::io("stat Ogg file", path, e))?
        .len();
    scan_ogg(BufReader::with_capacity(1 << 16, file), size, verify_crc)
        .map_err(|e| ImportError::io("read Ogg file", path, e))
}

/// Scan any seekable Ogg byte source of known length.
pub fn scan_ogg<R: Read + Seek>(
    mut reader: R,
    size: u64,
    verify_crc: bool,
) -> Result<OggReport, std::io::Error> {
    let mut builders: BTreeMap<u32, Builder> = BTreeMap::new();
    let mut order: Vec<u32> = Vec::new();
    let mut crc_failures = Vec::new();
    let mut page_count = 0_u64;
    let mut sequence_gaps = 0_u64;
    let mut parse_error = None;
    let mut offset = 0_u64;
    let mut buf = vec![0_u8; 1 << 16];

    while offset < size {
        let mut header = [0_u8; 27];
        if let Err(e) = reader.read_exact(&mut header) {
            parse_error = Some(format!("truncated page header at offset {offset}: {e}"));
            break;
        }
        if &header[0..4] != b"OggS" {
            parse_error = Some(format!("missing OggS capture pattern at offset {offset}"));
            break;
        }
        let version = header[4];
        if version != 0 {
            parse_error = Some(format!(
                "unsupported Ogg version {version} at offset {offset}"
            ));
            break;
        }
        let header_type = header[5];
        let granule = i64::from_le_bytes(header[6..14].try_into().unwrap());
        let serial = u32::from_le_bytes(header[14..18].try_into().unwrap());
        let sequence = u32::from_le_bytes(header[18..22].try_into().unwrap());
        let stored_crc = u32::from_le_bytes(header[22..26].try_into().unwrap());
        let nseg = header[26] as usize;
        let mut table = [0_u8; 255];
        if let Err(e) = reader.read_exact(&mut table[..nseg]) {
            parse_error = Some(format!("truncated segment table at offset {offset}: {e}"));
            break;
        }
        let payload: u64 = table[..nseg].iter().map(|&b| u64::from(b)).sum();
        let page_end = offset + 27 + nseg as u64 + payload;
        if page_end > size {
            parse_error = Some(format!(
                "page at offset {offset} extends past end of file ({page_end} > {size})"
            ));
            break;
        }

        let builder = builders.entry(serial).or_insert_with(|| {
            order.push(serial);
            Builder {
                first_packet: Vec::new(),
                pages: 0,
                last_granule: None,
                has_eos: false,
                next_sequence: sequence,
            }
        });
        if builder.next_sequence != sequence {
            sequence_gaps += 1;
        }
        builder.next_sequence = sequence.wrapping_add(1);
        builder.pages += 1;
        if header_type & 0x04 != 0 {
            builder.has_eos = true;
        }
        if granule != -1 {
            builder.last_granule = Some(granule);
        }

        let capture_first = builder.pages == 1;
        let mut remaining = payload;
        let mut crc = 0_u32;
        if verify_crc {
            let mut h = header;
            h[22..26].fill(0);
            crc = crc_update(crc, &h);
            crc = crc_update(crc, &table[..nseg]);
        }
        let want_first = if capture_first {
            FIRST_PACKET_CAPTURE.min(payload as usize)
        } else {
            0
        };
        let mut first_done = 0_usize;
        while remaining > 0 && (verify_crc || first_done < want_first) {
            let n = (remaining as usize).min(buf.len());
            reader.read_exact(&mut buf[..n])?;
            if first_done < want_first {
                let take = (want_first - first_done).min(n);
                builder.first_packet.extend_from_slice(&buf[..take]);
                first_done += take;
            }
            if verify_crc {
                crc = crc_update(crc, &buf[..n]);
            }
            remaining -= n as u64;
        }
        if remaining > 0 {
            reader.seek(SeekFrom::Current(remaining as i64))?;
        }
        if verify_crc && crc != stored_crc {
            crc_failures.push(CrcFailure {
                offset,
                serial,
                sequence,
                stored: stored_crc,
                computed: crc,
            });
        }
        page_count += 1;
        offset = page_end;
    }

    let streams = order
        .into_iter()
        .map(|serial| finish_stream(serial, &builders[&serial]))
        .collect();
    Ok(OggReport {
        file_size: size,
        page_count,
        crc_verified: verify_crc,
        crc_failures,
        sequence_gaps,
        streams,
        parse_error,
    })
}

fn finish_stream(serial: u32, b: &Builder) -> OggStream {
    let codec = identify_codec(&b.first_packet);
    let granule = b.last_granule.filter(|g| *g >= 0).map(|g| g as u64);
    let (duration_seconds, frame_count) = match (&codec, granule) {
        (StreamCodec::Theora(t), Some(g)) if t.fps_numerator > 0 => {
            let shift = u32::from(t.keyframe_granule_shift);
            let mask = (1_u64 << shift) - 1;
            let mut frames = (g >> shift) + (g & mask);
            // Theora 3.2.1 and later: granule is the index of the last frame.
            if !t.version.starts_with("3.2.0") && !t.version.starts_with("3.1") {
                frames += 1;
            }
            let secs = frames as f64 * f64::from(t.fps_denominator) / f64::from(t.fps_numerator);
            (Some(secs), Some(frames))
        }
        (StreamCodec::Vorbis(v), Some(g)) if v.sample_rate > 0 => {
            (Some(g as f64 / f64::from(v.sample_rate)), None)
        }
        (StreamCodec::Opus { pre_skip, .. }, Some(g)) => (
            Some(g.saturating_sub(u64::from(*pre_skip)) as f64 / 48_000.0),
            None,
        ),
        _ => (None, None),
    };
    OggStream {
        serial,
        codec,
        pages: b.pages,
        last_granule: b.last_granule,
        has_eos: b.has_eos,
        duration_seconds,
        frame_count,
    }
}

fn be24(b: &[u8]) -> u32 {
    (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2])
}

/// Identify a logical stream's codec from its first packet.
pub fn identify_codec(p: &[u8]) -> StreamCodec {
    if p.len() >= 42 && p.starts_with(b"\x80theora") {
        let bits = u16::from_be_bytes([p[40], p[41]]);
        let pf = (bits >> 3) & 3;
        return StreamCodec::Theora(TheoraInfo {
            version: format!("{}.{}.{}", p[7], p[8], p[9]),
            frame_width: u32::from(u16::from_be_bytes([p[10], p[11]])) * 16,
            frame_height: u32::from(u16::from_be_bytes([p[12], p[13]])) * 16,
            picture_width: be24(&p[14..17]),
            picture_height: be24(&p[17..20]),
            picture_x: p[20],
            picture_y: p[21],
            fps_numerator: u32::from_be_bytes(p[22..26].try_into().unwrap()),
            fps_denominator: u32::from_be_bytes(p[26..30].try_into().unwrap()),
            pixel_format: match pf {
                0 => "4:2:0",
                2 => "4:2:2",
                3 => "4:4:4",
                _ => "reserved",
            }
            .to_owned(),
            keyframe_granule_shift: ((bits >> 5) & 0x1f) as u8,
            nominal_bitrate: be24(&p[37..40]),
            quality: ((bits >> 10) & 0x3f) as u8,
        });
    }
    if p.len() >= 16 && p.starts_with(b"\x01vorbis") {
        return StreamCodec::Vorbis(VorbisInfo {
            channels: p[11],
            sample_rate: u32::from_le_bytes(p[12..16].try_into().unwrap()),
        });
    }
    if p.len() >= 12 && p.starts_with(b"OpusHead") {
        return StreamCodec::Opus {
            channels: p[9],
            pre_skip: u16::from_le_bytes([p[10], p[11]]),
        };
    }
    if p.starts_with(b"\x7fFLAC") {
        return StreamCodec::Flac;
    }
    let hex: String = p.iter().take(8).map(|b| format!("{b:02x}")).collect();
    StreamCodec::Other {
        first_bytes_hex: hex,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn build_page(
        header_type: u8,
        granule: i64,
        serial: u32,
        seq: u32,
        packet: &[u8],
        bad_crc: bool,
    ) -> Vec<u8> {
        assert!(packet.len() < 255);
        let mut p = Vec::new();
        p.extend_from_slice(b"OggS");
        p.push(0);
        p.push(header_type);
        p.extend_from_slice(&granule.to_le_bytes());
        p.extend_from_slice(&serial.to_le_bytes());
        p.extend_from_slice(&seq.to_le_bytes());
        p.extend_from_slice(&[0; 4]);
        p.push(1);
        p.push(packet.len() as u8);
        p.extend_from_slice(packet);
        let mut crc = ogg_crc32(&p);
        if bad_crc {
            crc ^= 1;
        }
        p[22..26].copy_from_slice(&crc.to_le_bytes());
        p
    }

    fn theora_header() -> Vec<u8> {
        let mut h = b"\x80theora".to_vec();
        h.extend_from_slice(&[3, 2, 1]);
        h.extend_from_slice(&40_u16.to_be_bytes());
        h.extend_from_slice(&30_u16.to_be_bytes());
        h.extend_from_slice(&[0, 2, 0x80]);
        h.extend_from_slice(&[0, 1, 0xE0]);
        h.extend_from_slice(&[0, 0]);
        h.extend_from_slice(&15_u32.to_be_bytes());
        h.extend_from_slice(&1_u32.to_be_bytes());
        h.extend_from_slice(&[0, 0, 1, 0, 0, 1]);
        h.push(0);
        h.extend_from_slice(&[0, 0, 0]);
        let bits: u16 = (32 << 10) | (6 << 5);
        h.extend_from_slice(&bits.to_be_bytes());
        assert_eq!(h.len(), 42);
        h
    }

    fn vorbis_header() -> Vec<u8> {
        let mut h = b"\x01vorbis".to_vec();
        h.extend_from_slice(&0_u32.to_le_bytes());
        h.push(2);
        h.extend_from_slice(&44_100_u32.to_le_bytes());
        h.extend_from_slice(&[0; 12]);
        h
    }

    #[test]
    fn crc_known_vector() {
        assert_eq!(ogg_crc32(b""), 0);
        // CRC-32/MPEG-2-style (poly 04C11DB7, init 0, no reflection, no xorout)
        // of "123456789" is the Ogg checksum.
        assert_eq!(ogg_crc32(b"123456789"), 0x89A1_897F);
    }

    #[test]
    fn scans_two_stream_fixture() {
        let mut data = Vec::new();
        data.extend(build_page(0x02, 0, 7, 0, &theora_header(), false));
        data.extend(build_page(0x02, 0, 9, 0, &vorbis_header(), false));
        data.extend(build_page(0x04, 29, 7, 1, b"frame", false));
        data.extend(build_page(0x04, 88_200, 9, 1, b"audio", false));
        let len = data.len() as u64;
        let report = scan_ogg(Cursor::new(data), len, true).unwrap();
        assert_eq!(report.page_count, 4);
        assert!(report.crc_failures.is_empty());
        assert!(report.parse_error.is_none());
        assert_eq!(report.streams.len(), 2);
        let StreamCodec::Theora(t) = &report.streams[0].codec else {
            panic!("expected theora");
        };
        assert_eq!((t.picture_width, t.picture_height), (640, 480));
        assert_eq!((t.fps_numerator, t.fps_denominator), (15, 1));
        assert_eq!(t.keyframe_granule_shift, 6);
        assert_eq!(t.pixel_format, "4:2:0");
        assert_eq!(report.streams[0].duration_seconds, Some(2.0));
        assert_eq!(
            report.streams[1].codec,
            StreamCodec::Vorbis(VorbisInfo {
                channels: 2,
                sample_rate: 44_100
            })
        );
        assert_eq!(report.streams[1].duration_seconds, Some(2.0));
    }

    #[test]
    fn reports_crc_mismatch_and_truncation() {
        let mut data = build_page(0x02, 0, 1, 0, &vorbis_header(), true);
        data.extend(build_page(0x04, 10, 1, 1, b"x", false));
        let len = data.len() as u64;
        let report = scan_ogg(Cursor::new(data.clone()), len, true).unwrap();
        assert_eq!(report.crc_failures.len(), 1);
        assert_eq!(report.crc_failures[0].sequence, 0);
        let no_crc = scan_ogg(Cursor::new(data.clone()), len, false).unwrap();
        assert!(no_crc.crc_failures.is_empty());
        let cut = &data[..data.len() - 2];
        let report = scan_ogg(Cursor::new(cut.to_vec()), cut.len() as u64, false).unwrap();
        assert!(report.parse_error.is_some());
        assert_eq!(report.page_count, 1);
    }

    #[test]
    fn real_install_movie_parses() {
        let path = std::path::Path::new(
            "E:/SteamLibrary/steamapps/common/FINAL FANTASY IX/StreamingAssets/ma/mbg101.bytes",
        );
        if !path.exists() {
            eprintln!("skipping: FFIX install not present");
            return;
        }
        let report = scan_ogg_file(path, false).unwrap();
        assert!(report.parse_error.is_none(), "{:?}", report.parse_error);
        assert!(report.page_count > 0);
    }
}
