//! Theora video decode for the PC movie containers (`ma/*.bytes`, A5).
//!
//! The Theora bitstream decode itself (headers, Huffman tables, dequantization,
//! inverse DCT, motion compensation, loop filter) is the pure-Rust MIT crate
//! `oxideav-theora`, which follows the Theora specification section by section
//! and has no native dependency. This module adds what the engine needs around
//! it: Ogg packet reassembly over `Read`, a display-ready frame type (cropped,
//! top-down planes) and BT.601 limited-range YCbCr to RGBA conversion.

use crate::install::ImportError;
use oxideav_theora::{
    FrameDecoder, PixelFormat, TheoraIdentHeader, decode_identification_header, decode_setup_header,
};
use std::io::Read;

/// One reassembled Ogg packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OggPacket {
    pub serial: u32,
    pub data: Vec<u8>,
    /// Page granule position when this packet is the last one completed on
    /// its page; `None` otherwise (including the `-1` "no packet ends here"
    /// value).
    pub granule: Option<u64>,
}

/// Streaming Ogg demuxer yielding whole packets, in file order.
pub struct OggPacketReader<R: Read> {
    reader: R,
    partial: std::collections::BTreeMap<u32, Vec<u8>>,
    queue: std::collections::VecDeque<OggPacket>,
    done: bool,
}

impl<R: Read> OggPacketReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            partial: Default::default(),
            queue: Default::default(),
            done: false,
        }
    }

    /// Next packet, or `None` at a clean end of file.
    pub fn next_packet(&mut self) -> Result<Option<OggPacket>, ImportError> {
        loop {
            if let Some(packet) = self.queue.pop_front() {
                return Ok(Some(packet));
            }
            if self.done {
                return Ok(None);
            }
            self.read_page()?;
        }
    }

    fn read_page(&mut self) -> Result<(), ImportError> {
        let mut header = [0_u8; 27];
        let mut filled = 0;
        while filled < header.len() {
            let n = self
                .reader
                .read(&mut header[filled..])
                .map_err(|e| ImportError::invalid("ogg", format!("read failed: {e}")))?;
            if n == 0 {
                if filled == 0 {
                    self.done = true;
                    return Ok(());
                }
                return Err(ImportError::invalid("ogg", "truncated page header"));
            }
            filled += n;
        }
        if &header[..4] != b"OggS" || header[4] != 0 {
            return Err(ImportError::invalid("ogg", "bad page capture pattern"));
        }
        let granule = i64::from_le_bytes(header[6..14].try_into().unwrap());
        let serial = u32::from_le_bytes(header[14..18].try_into().unwrap());
        let segments = usize::from(header[26]);
        let mut lacing = vec![0_u8; segments];
        self.reader
            .read_exact(&mut lacing)
            .map_err(|_| ImportError::invalid("ogg", "truncated segment table"))?;
        let total: usize = lacing.iter().map(|b| usize::from(*b)).sum();
        let mut body = vec![0_u8; total];
        self.reader
            .read_exact(&mut body)
            .map_err(|_| ImportError::invalid("ogg", "truncated page body"))?;
        let continued = header[5] & 1 != 0;
        let buffer = self.partial.entry(serial).or_default();
        if !continued {
            buffer.clear();
        }
        let mut offset = 0;
        let first_new = self.queue.len();
        for lace in lacing {
            let lace = usize::from(lace);
            buffer.extend_from_slice(&body[offset..offset + lace]);
            offset += lace;
            if lace < 255 {
                self.queue.push_back(OggPacket {
                    serial,
                    data: std::mem::take(buffer),
                    granule: None,
                });
            }
        }
        if granule >= 0
            && self.queue.len() > first_new
            && let Some(last) = self.queue.back_mut()
        {
            last.granule = Some(granule as u64);
        }
        Ok(())
    }
}

/// Display-ready frame: planes are cropped to the picture region and stored
/// top row first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub chroma_width: u32,
    pub chroma_height: u32,
    pub y: Vec<u8>,
    pub cb: Vec<u8>,
    pub cr: Vec<u8>,
}

fn clamp8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

impl VideoFrame {
    /// Convert to tightly packed RGBA8 using BT.601 limited range
    /// (Y 16..235, Cb/Cr 16..240) with nearest-neighbor chroma upsampling.
    pub fn to_rgba(&self) -> Vec<u8> {
        let (w, h) = (self.width as usize, self.height as usize);
        let (cw, ch) = (self.chroma_width as usize, self.chroma_height as usize);
        let mut out = vec![255_u8; w * h * 4];
        for row in 0..h {
            let crow = (row * ch / h).min(ch.saturating_sub(1));
            for col in 0..w {
                let ccol = (col * cw / w).min(cw.saturating_sub(1));
                let y = i32::from(self.y[row * w + col]) - 16;
                let cb = i32::from(self.cb[crow * cw + ccol]) - 128;
                let cr = i32::from(self.cr[crow * cw + ccol]) - 128;
                let luma = 298 * y + 128;
                let o = (row * w + col) * 4;
                out[o] = clamp8((luma + 409 * cr) >> 8);
                out[o + 1] = clamp8((luma - 100 * cb - 208 * cr) >> 8);
                out[o + 2] = clamp8((luma + 516 * cb) >> 8);
            }
        }
        out
    }
}

/// Stream parameters from the identification header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamParams {
    pub picture_width: u32,
    pub picture_height: u32,
    pub fps_numerator: u32,
    pub fps_denominator: u32,
    pub keyframe_granule_shift: u8,
    pub pixel_format: &'static str,
}

pub struct TheoraDecoder {
    inner: FrameDecoder,
}

fn theora_err(context: &str, e: impl std::fmt::Display) -> ImportError {
    ImportError::invalid("theora", format!("{context}: {e}"))
}

impl TheoraDecoder {
    /// Build from the identification and setup header packets (the comment
    /// header carries nothing the decoder needs).
    pub fn from_headers(identification: &[u8], setup: &[u8]) -> Result<Self, ImportError> {
        let ident = decode_identification_header(identification)
            .map_err(|e| theora_err("identification header", e))?;
        let tables = decode_setup_header(setup).map_err(|e| theora_err("setup header", e))?;
        let inner = FrameDecoder::new(ident, tables).map_err(|e| theora_err("decoder setup", e))?;
        Ok(Self { inner })
    }

    pub fn ident(&self) -> &TheoraIdentHeader {
        self.inner.ident()
    }

    pub fn params(&self) -> StreamParams {
        let i = self.inner.ident();
        StreamParams {
            picture_width: i.picw,
            picture_height: i.pich,
            fps_numerator: i.frn,
            fps_denominator: i.frd,
            keyframe_granule_shift: i.kfgshift,
            pixel_format: match i.pf {
                PixelFormat::Yuv420 => "4:2:0",
                PixelFormat::Yuv422 => "4:2:2",
                PixelFormat::Yuv444 => "4:4:4",
            },
        }
    }

    /// Frames declared by a data-page granule position (stream version aware).
    pub fn frames_from_granule(&self, granule: u64) -> Result<u64, ImportError> {
        self.inner
            .ident()
            .frame_count_from_granule_position(granule)
            .map_err(|e| theora_err("granule position", e))
    }

    /// Decode one video data packet (a zero-length packet repeats the
    /// previous frame).
    pub fn decode(&mut self, packet: &[u8]) -> Result<VideoFrame, ImportError> {
        let decoded = self
            .inner
            .decode_frame(packet)
            .map_err(|e| theora_err("frame decode", e))?;
        let cropped = self
            .inner
            .crop_for_display(&decoded)
            .map_err(|e| theora_err("crop", e))?;
        let flip = |plane: &[u8], w: u32, h: u32| -> Vec<u8> {
            let (w, h) = (w as usize, h as usize);
            let mut out = Vec::with_capacity(w * h);
            for row in (0..h).rev() {
                out.extend_from_slice(&plane[row * w..(row + 1) * w]);
            }
            out
        };
        Ok(VideoFrame {
            width: cropped.dims_y.width,
            height: cropped.dims_y.height,
            chroma_width: cropped.dims_c.width,
            chroma_height: cropped.dims_c.height,
            y: flip(
                &cropped.samples_y,
                cropped.dims_y.width,
                cropped.dims_y.height,
            ),
            cb: flip(
                &cropped.samples_cb,
                cropped.dims_c.width,
                cropped.dims_c.height,
            ),
            cr: flip(
                &cropped.samples_cr,
                cropped.dims_c.width,
                cropped.dims_c.height,
            ),
        })
    }
}

/// Result of [`decode_movie`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovieDecodeSummary {
    pub params: StreamParams,
    pub packets_decoded: u64,
    /// Zero-length (duplicate frame) packets among those decoded.
    pub empty_packets: u64,
    /// Granule position of the last data page seen while reading (`None` when
    /// reading stopped before any data page ended).
    pub last_granule: Option<u64>,
    /// Frames declared by `last_granule` (keyframe count plus frames since the
    /// keyframe, the Theora A.2.3 3.2.1 convention, so a full stream of N data
    /// packets declares N).
    pub granule_frame_count: Option<u64>,
    /// True when every packet in the stream was decoded.
    pub complete: bool,
}

/// Demux one Theora stream and decode up to `limit` data packets (`None` for
/// all), calling `on_frame(index, &frame)` for each. When decoding stops early
/// the granule position is that of the last page read, so
/// `granule_frame_count` only relates to `packets_decoded` for a complete
/// decode.
pub fn decode_movie<R: Read>(
    reader: R,
    limit: Option<u64>,
    mut on_frame: impl FnMut(u64, &VideoFrame) -> Result<(), ImportError>,
) -> Result<MovieDecodeSummary, ImportError> {
    let mut packets = OggPacketReader::new(reader);
    let mut headers: Vec<Vec<u8>> = Vec::new();
    let mut serial = None;
    let mut decoder: Option<TheoraDecoder> = None;
    let (mut decoded, mut empty) = (0_u64, 0_u64);
    let mut last_granule = None;
    let mut complete = true;
    while let Some(packet) = packets.next_packet()? {
        if serial.is_none() && packet.data.starts_with(b"\x80theora") {
            serial = Some(packet.serial);
        }
        if serial != Some(packet.serial) {
            continue;
        }
        let Some(dec) = decoder.as_mut() else {
            headers.push(packet.data);
            if headers.len() == 3 {
                decoder = Some(TheoraDecoder::from_headers(&headers[0], &headers[2])?);
            }
            continue;
        };
        if limit.is_some_and(|l| decoded >= l) {
            complete = false;
            break;
        }
        if packet.granule.is_some() {
            last_granule = packet.granule;
        }
        if packet.data.is_empty() {
            empty += 1;
        }
        let frame = dec.decode(&packet.data)?;
        on_frame(decoded, &frame)?;
        decoded += 1;
    }
    let decoder = decoder.ok_or_else(|| ImportError::invalid("theora", "no Theora stream"))?;
    let granule_frame_count = match last_granule {
        Some(g) => Some(decoder.frames_from_granule(g)?),
        None => None,
    };
    Ok(MovieDecodeSummary {
        params: decoder.params(),
        packets_decoded: decoded,
        empty_packets: empty,
        last_granule,
        granule_frame_count,
        complete,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pc::ogg::ogg_crc32;
    use oxideav_theora::{
        ColorSpace, FrameEncoder, SetupHeaderTables, SourceFrame, encode_identification_header,
        encode_setup_header,
    };

    fn page(serial: u32, seq: u32, flags: u8, granule: i64, lacing: &[u8], body: &[u8]) -> Vec<u8> {
        let mut p = b"OggS".to_vec();
        p.push(0);
        p.push(flags);
        p.extend_from_slice(&granule.to_le_bytes());
        p.extend_from_slice(&serial.to_le_bytes());
        p.extend_from_slice(&seq.to_le_bytes());
        p.extend_from_slice(&[0; 4]);
        p.push(lacing.len() as u8);
        p.extend_from_slice(lacing);
        p.extend_from_slice(body);
        let crc = ogg_crc32(&p);
        p[22..26].copy_from_slice(&crc.to_le_bytes());
        p
    }

    fn lace(len: usize) -> Vec<u8> {
        let mut out = vec![255; len / 255];
        out.push((len % 255) as u8);
        out
    }

    #[test]
    fn packet_reader_reassembles_lacing_and_continuation() {
        let big: Vec<u8> = (0..300_u32).map(|i| i as u8).collect();
        // Page 1: packet A (3 bytes) then the first 255 bytes of `big`.
        let mut body1 = vec![1, 2, 3];
        body1.extend_from_slice(&big[..255]);
        let mut file = page(7, 0, 2, -1, &[3, 255], &body1);
        // Page 2 continues `big` with 45 bytes, then an empty packet.
        file.extend(page(7, 1, 1, 9, &[45, 0], &big[255..]));
        let mut reader = OggPacketReader::new(file.as_slice());
        let a = reader.next_packet().unwrap().unwrap();
        assert_eq!((a.data, a.granule), (vec![1, 2, 3], None));
        let b = reader.next_packet().unwrap().unwrap();
        assert_eq!((b.data, b.granule), (big, None));
        let c = reader.next_packet().unwrap().unwrap();
        assert_eq!((c.data.len(), c.granule), (0, Some(9)));
        assert!(reader.next_packet().unwrap().is_none());
    }

    #[test]
    fn packet_reader_rejects_bad_capture_pattern() {
        let mut reader = OggPacketReader::new(&[0_u8; 40][..]);
        assert!(reader.next_packet().is_err());
    }

    #[test]
    fn rgba_conversion_matches_bt601_limited_range() {
        let frame = VideoFrame {
            width: 2,
            height: 1,
            chroma_width: 1,
            chroma_height: 1,
            y: vec![16, 235],
            cb: vec![128],
            cr: vec![128],
        };
        assert_eq!(
            frame.to_rgba(),
            vec![0, 0, 0, 255, 255, 255, 255, 255],
            "black and white map to full range"
        );
        let red = VideoFrame {
            width: 1,
            height: 1,
            chroma_width: 1,
            chroma_height: 1,
            y: vec![81],
            cb: vec![90],
            cr: vec![240],
        };
        let px = red.to_rgba();
        assert!(px[0] >= 250 && px[1] <= 6 && px[2] <= 6, "{px:?}");
    }

    /// Header and frame packets come from the encoder in the same dependency
    /// and are muxed by the page writer above. This proves demux, header
    /// parse, intra decode and granule bookkeeping are wired together; it does
    /// not validate the codec against an independent reference decode.
    #[test]
    fn synthetic_stream_round_trips_and_counts_frames() {
        let (w, h) = (32_u32, 32_u32);
        let ident = TheoraIdentHeader {
            vmaj: 3,
            vmin: 2,
            vrev: 1,
            fmbw: (w / 16) as u16,
            fmbh: (h / 16) as u16,
            picw: w,
            pich: h,
            picx: 0,
            picy: 0,
            frn: 15,
            frd: 1,
            parn: 1,
            pard: 1,
            cs: ColorSpace::Undefined,
            nombr: 0,
            qual: 63,
            kfgshift: 6,
            pf: PixelFormat::Yuv420,
        };
        let setup = SetupHeaderTables::vp3_defaults();
        let id_packet = encode_identification_header(&ident).unwrap();
        let setup_packet = encode_setup_header(&setup).unwrap();
        let mut comment = b"\x81theora".to_vec();
        comment.extend_from_slice(&0_u32.to_le_bytes());
        comment.extend_from_slice(&0_u32.to_le_bytes());
        let encoder = FrameEncoder::new(ident, setup, 63).unwrap();
        let (cw, ch) = ((w / 2) as usize, (h / 2) as usize);
        let src = SourceFrame {
            samples_y: (0..w * h).map(|i| (16 + (i % w) * 6) as u8).collect(),
            samples_cb: vec![128; cw * ch],
            samples_cr: vec![128; cw * ch],
        };
        let frame_packet = encoder.encode_intra_frame(&src).unwrap();

        let serial = 0x1234;
        let mut file = page(serial, 0, 2, 0, &lace(id_packet.len()), &id_packet);
        let mut lacing = lace(comment.len());
        lacing.extend(lace(setup_packet.len()));
        let mut hdr_body = comment.clone();
        hdr_body.extend_from_slice(&setup_packet);
        file.extend(page(serial, 1, 0, 0, &lacing, &hdr_body));
        // One key frame and two zero-length duplicates. A 3.2.1 granule is the
        // count of frames decoded so far split at KFGSHIFT: key frame count 1
        // in the high half, two frames since the key frame in the low half.
        let mut lacing = lace(frame_packet.len());
        lacing.extend_from_slice(&[0, 0]);
        file.extend(page(serial, 2, 4, (1 << 6) | 2, &lacing, &frame_packet));

        let mut frames = Vec::new();
        let summary = decode_movie(file.as_slice(), None, |_, f| {
            frames.push(f.clone());
            Ok(())
        })
        .unwrap();
        assert_eq!(summary.packets_decoded, 3);
        assert_eq!(summary.empty_packets, 2);
        assert!(summary.complete);
        assert_eq!(summary.last_granule, Some(66));
        assert_eq!(summary.granule_frame_count, Some(3));
        assert_eq!(summary.params.picture_width, 32);
        assert_eq!(summary.params.pixel_format, "4:2:0");
        assert_eq!(frames[0].width, 32);
        assert_eq!(frames[0], frames[1]);
        assert_eq!(frames[0], frames[2]);
        // The gradient survives a high-quality round trip; the source is
        // bottom-up and the output top-down, so row 0 maps to source row 31.
        for row in [0_usize, 31] {
            for col in [0_usize, 31] {
                let want = i32::from(src.samples_y[(31 - row) * 32 + col]);
                let got = i32::from(frames[0].y[row * 32 + col]);
                assert!((want - got).abs() <= 12, "({row},{col}) {want} vs {got}");
            }
        }
        let limited = decode_movie(file.as_slice(), Some(1), |_, _| Ok(())).unwrap();
        assert_eq!(limited.packets_decoded, 1);
        assert!(!limited.complete);
        assert_eq!(frames[0].to_rgba().len(), 32 * 32 * 4);
    }
}
