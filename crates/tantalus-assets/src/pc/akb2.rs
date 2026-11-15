//! AKB container header reader and Ogg page inspector for FFIX PC sound assets.
//!
//! Layout follows Memoria's `AKB2Header` (304 bytes for `AKB2`, 204 for `AKB1`).
//! The payload after the header is expected to be an Ogg stream; this module
//! identifies the codec and counts pages but never decodes audio.

use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

pub const AKB2_SIGNATURE: u32 = 0x3242_4B41;
pub const AKB1_SIGNATURE: u32 = 0x2042_4B41;
pub const AKB2_HEADER_SIZE: usize = 304;
pub const AKB1_HEADER_SIZE: usize = 204;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum AudioError {
    TooShort {
        len: usize,
        need: usize,
    },
    BadSignature {
        found: u32,
    },
    PayloadOutOfRange {
        offset: usize,
        size: usize,
        file_len: usize,
    },
    BadOgg(String),
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort { len, need } => write!(f, "too short: {len} bytes, need {need}"),
            Self::BadSignature { found } => write!(f, "bad signature 0x{found:08x}"),
            Self::PayloadOutOfRange {
                offset,
                size,
                file_len,
            } => write!(f, "payload {offset}+{size} exceeds file length {file_len}"),
            Self::BadOgg(reason) => write!(f, "bad Ogg stream: {reason}"),
        }
    }
}

impl std::error::Error for AudioError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AkbVersion {
    Akb1,
    Akb2,
}

/// Parsed AKB header. Fields Memoria names are typed; the rest stay in
/// named raw fields. `raw` holds the complete header bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AkbHeader {
    pub version: AkbVersion,
    pub header_size: usize,
    /// `Constant01` (word 1); zero for AKB1.
    pub constant01: u32,
    /// Declared total file size (word 2).
    pub file_size: u32,
    /// `Unknown09` (word 9), "looks like a sound ID".
    pub unknown09: u32,
    /// `Constant32` (u16).
    pub constant32: u16,
    /// `Unknown33` (u16).
    pub unknown33: u16,
    /// `Constant34` (u16).
    pub constant34: u16,
    pub sample_rate: u16,
    /// Payload byte count. For AKB1 this is `file_size - 204`.
    pub content_size: u32,
    pub sample_count: u32,
    pub loop_start: u32,
    pub loop_end: u32,
    /// `Constant36`.
    pub constant36: u32,
    pub loop_start_alternate: u32,
    pub loop_end_alternate: u32,
    /// `Optional46` ("Loop?"), `Optional47` ("can be zero").
    pub optional46: u32,
    pub optional47: u32,
    #[serde(skip)]
    pub raw: Vec<u8>,
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

impl AkbHeader {
    /// Parses the header from the start of an `.akb.bytes` file.
    pub fn parse(bytes: &[u8]) -> Result<Self, AudioError> {
        if bytes.len() < 4 {
            return Err(AudioError::TooShort {
                len: bytes.len(),
                need: 4,
            });
        }
        let signature = u32_at(bytes, 0);
        let (version, header_size) = match signature {
            AKB2_SIGNATURE => (AkbVersion::Akb2, AKB2_HEADER_SIZE),
            AKB1_SIGNATURE => (AkbVersion::Akb1, AKB1_HEADER_SIZE),
            found => return Err(AudioError::BadSignature { found }),
        };
        if bytes.len() < header_size {
            return Err(AudioError::TooShort {
                len: bytes.len(),
                need: header_size,
            });
        }
        let raw = bytes[..header_size].to_vec();
        if version == AkbVersion::Akb1 {
            // Memoria: skip u32, ContentSize = u32 - 204, skip u16, SampleRate u16,
            // skip u32, LoopStart u32, LoopEnd u32.
            let file_size = u32_at(bytes, 8);
            return Ok(Self {
                version,
                header_size,
                constant01: 0,
                file_size,
                unknown09: 0,
                constant32: 0,
                unknown33: 0,
                constant34: 0,
                sample_rate: u16_at(bytes, 14),
                content_size: file_size.wrapping_sub(header_size as u32),
                sample_count: 0,
                loop_start: u32_at(bytes, 20),
                loop_end: u32_at(bytes, 24),
                constant36: 0,
                loop_start_alternate: 0,
                loop_end_alternate: 0,
                optional46: 0,
                optional47: 0,
                raw,
            });
        }
        // AKB2: 31 words (124 bytes) + four 24-byte sections (96) + Zero31 (4)
        // puts the u16 block at byte 224.
        let base = 224;
        Ok(Self {
            version,
            header_size,
            constant01: u32_at(bytes, 4),
            file_size: u32_at(bytes, 8),
            unknown09: u32_at(bytes, 36),
            constant32: u16_at(bytes, base),
            unknown33: u16_at(bytes, base + 2),
            constant34: u16_at(bytes, base + 4),
            sample_rate: u16_at(bytes, base + 6),
            content_size: u32_at(bytes, base + 8),
            sample_count: u32_at(bytes, base + 12),
            loop_start: u32_at(bytes, base + 16),
            loop_end: u32_at(bytes, base + 20),
            constant36: u32_at(bytes, base + 24),
            loop_start_alternate: u32_at(bytes, base + 28),
            loop_end_alternate: u32_at(bytes, base + 32),
            optional46: u32_at(bytes, base + 70),
            optional47: u32_at(bytes, base + 74),
            raw,
        })
    }

    /// True when the header declares a loop region.
    pub fn has_loop(&self) -> bool {
        self.loop_end > self.loop_start
    }

    /// Payload byte range `(offset, size)` inside the file, validated.
    pub fn payload_range(&self, file_len: usize) -> Result<(usize, usize), AudioError> {
        let offset = self.header_size;
        let size = self.content_size as usize;
        match offset.checked_add(size) {
            Some(end) if end <= file_len => Ok((offset, size)),
            _ => Err(AudioError::PayloadOutOfRange {
                offset,
                size,
                file_len,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OggCodec {
    Vorbis,
    Opus,
    Flac,
    Speex,
    Theora,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OggSummary {
    pub codec: OggCodec,
    pub page_count: usize,
    /// Largest valid granule position seen (samples for audio codecs).
    pub final_granule: i64,
    pub serial_numbers: Vec<u32>,
    /// Vorbis identification header fields when the codec is Vorbis.
    pub vorbis_channels: Option<u8>,
    pub vorbis_sample_rate: Option<u32>,
}

/// Walks Ogg pages from the start of `bytes`; identifies the codec from the first packet.
pub fn inspect_ogg(bytes: &[u8]) -> Result<OggSummary, AudioError> {
    let bad = |m: &str| AudioError::BadOgg(m.to_owned());
    let mut pos = 0usize;
    let mut pages = 0usize;
    let mut granule = -1i64;
    let mut serials = Vec::new();
    let mut codec = None;
    let (mut channels, mut rate) = (None, None);
    while pos < bytes.len() {
        let head = bytes
            .get(pos..pos + 27)
            .ok_or_else(|| bad("truncated page header"))?;
        if &head[..4] != b"OggS" {
            return Err(AudioError::BadOgg(format!("no capture pattern at {pos}")));
        }
        if head[4] != 0 {
            return Err(bad("unsupported stream structure version"));
        }
        let page_granule = i64::from_le_bytes(head[6..14].try_into().expect("8 bytes"));
        let serial = u32::from_le_bytes(head[14..18].try_into().expect("4 bytes"));
        let segments = head[26] as usize;
        let table = bytes
            .get(pos + 27..pos + 27 + segments)
            .ok_or_else(|| bad("truncated segment table"))?;
        let body: usize = table.iter().map(|&s| s as usize).sum();
        let body_start = pos + 27 + segments;
        let body_bytes = bytes
            .get(body_start..body_start + body)
            .ok_or_else(|| bad("truncated page body"))?;
        if codec.is_none() {
            codec = Some(if body_bytes.starts_with(b"\x01vorbis") {
                if body_bytes.len() >= 16 {
                    channels = Some(body_bytes[11]);
                    rate = Some(u32::from_le_bytes(
                        body_bytes[12..16].try_into().expect("4 bytes"),
                    ));
                }
                OggCodec::Vorbis
            } else if body_bytes.starts_with(b"OpusHead") {
                OggCodec::Opus
            } else if body_bytes.starts_with(b"\x7fFLAC") {
                OggCodec::Flac
            } else if body_bytes.starts_with(b"Speex   ") {
                OggCodec::Speex
            } else if body_bytes.starts_with(b"\x80theora") {
                OggCodec::Theora
            } else {
                OggCodec::Other
            });
        }
        if page_granule != -1 && page_granule > granule {
            granule = page_granule;
        }
        if !serials.contains(&serial) {
            serials.push(serial);
        }
        pages += 1;
        pos = body_start + body;
    }
    let codec = codec.ok_or_else(|| bad("empty stream"))?;
    Ok(OggSummary {
        codec,
        page_count: pages,
        final_granule: granule,
        serial_numbers: serials,
        vorbis_channels: channels,
        vorbis_sample_rate: rate,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadKind {
    Akb2,
    Akb1,
    OggS,
    Other,
}

/// Classifies a sound TextAsset script by its leading magic bytes.
pub fn classify_magic(bytes: &[u8]) -> PayloadKind {
    match bytes.get(..4) {
        Some(b"AKB2") => PayloadKind::Akb2,
        Some(b"AKB ") => PayloadKind::Akb1,
        Some(b"OggS") => PayloadKind::OggS,
        _ => PayloadKind::Other,
    }
}

/// Aggregate audio statistics for one bundle.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AudioBundleReport {
    pub bundle: String,
    pub container_entries: usize,
    pub sound_assets: usize,
    /// Count by `class<id>:<extension>`.
    pub by_container_type: BTreeMap<String, usize>,
    /// Count by payload magic (`akb2`, `akb1`, `ogg_s`, `other`).
    pub by_magic: BTreeMap<String, usize>,
    pub akb_parsed: usize,
    pub akb_payload_in_range: usize,
    pub min_sample_rate: Option<u16>,
    pub max_sample_rate: Option<u16>,
    pub looped: usize,
    pub with_alternate_loop: usize,
    /// Sum of header `sample_count` (a placeholder on looped assets).
    pub total_samples: u64,
    /// Sum of the last Ogg granule position (true stream length).
    pub total_granule_samples: u64,
    /// Looped assets whose last granule equals `loop_end + 1`.
    pub loop_end_matches_granule: usize,
    pub total_payload_bytes: u64,
    /// Count by identified payload codec (`vorbis`, `other`, ...).
    pub codecs: BTreeMap<String, usize>,
    pub ogg_pages: usize,
    /// Assets whose declared `sample_count` differs from the last Ogg granule.
    pub sample_count_mismatches: usize,
    /// First few mismatching assets: `path: header sample_count, loop_start, loop_end, last granule`.
    pub mismatch_examples: Vec<String>,
    pub failures: Vec<AudioFailure>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioFailure {
    pub path: String,
    pub reason: String,
}

/// Classifies every `/sounds/` container entry of a UnityRaw bundle.
pub fn inspect_audio_bundle(path: &Path) -> Result<AudioBundleReport, String> {
    use super::asset_bundle::AssetBundleMetadata;
    use super::serialized::SerializedFile;
    use super::text_asset::TextAsset;
    use super::unity_raw::UnityRawBundle;

    let source = path.to_string_lossy();
    let bundle = UnityRawBundle::open(path).map_err(|e| e.to_string())?;
    let bytes = bundle
        .read_entry(0, 600 * 1024 * 1024)
        .map_err(|e| e.to_string())?;
    let serialized = SerializedFile::parse(&bytes, &source).map_err(|e| e.to_string())?;
    let report = serialized.report();
    let index_of: BTreeMap<i64, usize> = report
        .objects
        .iter()
        .enumerate()
        .map(|(i, o)| (o.path_id, i))
        .collect();
    let class_of: BTreeMap<i64, u16> = report
        .objects
        .iter()
        .map(|o| (o.path_id, o.class_id))
        .collect();
    let (meta_index, _) = report
        .objects
        .iter()
        .enumerate()
        .find(|(_, o)| o.class_id == 142)
        .ok_or("AssetBundle metadata object is missing")?;
    let metadata = AssetBundleMetadata::parse(
        serialized
            .object_data(meta_index)
            .map_err(|e| e.to_string())?,
        &source,
    )
    .map_err(|e| e.to_string())?;

    let mut out = AudioBundleReport {
        bundle: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        container_entries: metadata.container.len(),
        ..Default::default()
    };
    for (asset_path, info) in &metadata.container {
        if !asset_path.to_ascii_lowercase().contains("/sounds/") {
            continue;
        }
        out.sound_assets += 1;
        let class = class_of
            .get(&info.path_id)
            .map_or_else(|| "none".to_owned(), |c| c.to_string());
        let extension = asset_path.rsplit('.').next().unwrap_or("");
        *out.by_container_type
            .entry(format!("class{class}:{extension}"))
            .or_insert(0) += 1;
        let mut fail = |reason: String| {
            out.failures.push(AudioFailure {
                path: asset_path.clone(),
                reason,
            })
        };
        let Some(&index) = index_of.get(&info.path_id) else {
            fail("container path ID has no object".to_owned());
            continue;
        };
        let script = match serialized
            .object_data(index)
            .map_err(|e| e.to_string())
            .and_then(|d| TextAsset::parse(d, asset_path).map_err(|e| e.to_string()))
        {
            Ok(asset) => asset.script,
            Err(reason) => {
                fail(format!("not a TextAsset: {reason}"));
                continue;
            }
        };
        let magic = classify_magic(&script);
        let key = serde_json::to_value(magic)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        *out.by_magic.entry(key).or_insert(0) += 1;
        let payload: &[u8] = match magic {
            PayloadKind::Akb1 | PayloadKind::Akb2 => {
                let header = match AkbHeader::parse(&script) {
                    Ok(h) => h,
                    Err(e) => {
                        fail(format!("AKB header: {e}"));
                        continue;
                    }
                };
                out.akb_parsed += 1;
                let (offset, size) = match header.payload_range(script.len()) {
                    Ok(r) => r,
                    Err(e) => {
                        fail(format!("AKB header: {e}"));
                        continue;
                    }
                };
                out.akb_payload_in_range += 1;
                out.total_payload_bytes += size as u64;
                out.total_samples += u64::from(header.sample_count);
                out.min_sample_rate = Some(
                    out.min_sample_rate
                        .map_or(header.sample_rate, |m| m.min(header.sample_rate)),
                );
                out.max_sample_rate = Some(
                    out.max_sample_rate
                        .map_or(header.sample_rate, |m| m.max(header.sample_rate)),
                );
                if header.has_loop() {
                    out.looped += 1;
                }
                if header.loop_end_alternate > header.loop_start_alternate {
                    out.with_alternate_loop += 1;
                }
                let payload = &script[offset..offset + size];
                match inspect_ogg(payload) {
                    Ok(summary) => {
                        out.ogg_pages += summary.page_count;
                        out.total_granule_samples += summary.final_granule.max(0) as u64;
                        if header.has_loop()
                            && summary.final_granule == i64::from(header.loop_end) + 1
                        {
                            out.loop_end_matches_granule += 1;
                        }
                        if summary.final_granule != i64::from(header.sample_count) {
                            out.sample_count_mismatches += 1;
                            if out.mismatch_examples.len() < 6 {
                                out.mismatch_examples.push(format!(
                                    "{asset_path}: {}, {}, {}, {}",
                                    header.sample_count,
                                    header.loop_start,
                                    header.loop_end,
                                    summary.final_granule
                                ));
                            }
                        }
                        let name = serde_json::to_value(summary.codec)
                            .ok()
                            .and_then(|v| v.as_str().map(str::to_owned))
                            .unwrap_or_default();
                        *out.codecs.entry(name).or_insert(0) += 1;
                    }
                    Err(e) => fail(format!("payload: {e}")),
                }
                continue;
            }
            PayloadKind::OggS => &script,
            PayloadKind::Other => {
                fail("unrecognized payload magic".to_owned());
                continue;
            }
        };
        match inspect_ogg(payload) {
            Ok(summary) => {
                out.ogg_pages += summary.page_count;
                let name = serde_json::to_value(summary.codec)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default();
                *out.codecs.entry(name).or_insert(0) += 1;
            }
            Err(e) => fail(format!("Ogg: {e}")),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ogg_page(first: bool, granule: i64, body: &[u8]) -> Vec<u8> {
        let mut p = b"OggS".to_vec();
        p.push(0);
        p.push(if first { 2 } else { 0 });
        p.extend(granule.to_le_bytes());
        p.extend(7u32.to_le_bytes());
        p.extend(0u32.to_le_bytes());
        p.extend(0u32.to_le_bytes());
        p.push(1);
        p.push(body.len() as u8);
        p.extend(body);
        p
    }

    fn vorbis_id() -> Vec<u8> {
        let mut b = b"\x01vorbis".to_vec();
        b.extend(0u32.to_le_bytes());
        b.push(2);
        b.extend(44100u32.to_le_bytes());
        b
    }

    fn akb2(payload: &[u8], declared: u32) -> Vec<u8> {
        let mut h = vec![0u8; AKB2_HEADER_SIZE];
        h[..4].copy_from_slice(b"AKB2");
        h[8..12].copy_from_slice(&((AKB2_HEADER_SIZE + payload.len()) as u32).to_le_bytes());
        h[36..40].copy_from_slice(&99u32.to_le_bytes());
        h[230..232].copy_from_slice(&32000u16.to_le_bytes());
        h[232..236].copy_from_slice(&declared.to_le_bytes());
        h[236..240].copy_from_slice(&1000u32.to_le_bytes());
        h[240..244].copy_from_slice(&10u32.to_le_bytes());
        h[244..248].copy_from_slice(&900u32.to_le_bytes());
        h.extend(payload);
        h
    }

    #[test]
    fn parses_akb2_header_fields() {
        let mut ogg = ogg_page(true, 0, &vorbis_id());
        ogg.extend(ogg_page(false, 1000, b"data"));
        let file = akb2(&ogg, ogg.len() as u32);
        let h = AkbHeader::parse(&file).unwrap();
        assert_eq!(h.version, AkbVersion::Akb2);
        assert_eq!(h.sample_rate, 32000);
        assert_eq!(h.unknown09, 99);
        assert_eq!((h.sample_count, h.loop_start, h.loop_end), (1000, 10, 900));
        assert!(h.has_loop());
        let (off, size) = h.payload_range(file.len()).unwrap();
        assert_eq!((off, size), (304, ogg.len()));
        assert_eq!(classify_magic(&file), PayloadKind::Akb2);
        assert_eq!(classify_magic(&file[off..]), PayloadKind::OggS);
        let s = inspect_ogg(&file[off..off + size]).unwrap();
        assert_eq!(s.codec, OggCodec::Vorbis);
        assert_eq!((s.page_count, s.final_granule), (2, 1000));
        assert_eq!(
            (s.vorbis_channels, s.vorbis_sample_rate),
            (Some(2), Some(44100))
        );
    }

    #[test]
    fn rejects_out_of_range_payload_and_bad_input() {
        let file = akb2(b"abcd", 5000);
        let h = AkbHeader::parse(&file).unwrap();
        assert!(matches!(
            h.payload_range(file.len()),
            Err(AudioError::PayloadOutOfRange { .. })
        ));
        assert!(matches!(
            AkbHeader::parse(b"XXXXzzzz"),
            Err(AudioError::BadSignature { .. })
        ));
        assert!(matches!(
            AkbHeader::parse(b"AKB2"),
            Err(AudioError::TooShort { .. })
        ));
        assert!(inspect_ogg(b"OggS\0\0").is_err());
    }

    #[test]
    fn identifies_non_vorbis_codecs() {
        let s = inspect_ogg(&ogg_page(true, 5, b"OpusHead....")).unwrap();
        assert_eq!(s.codec, OggCodec::Opus);
        let s = inspect_ogg(&ogg_page(true, -1, b"zzzz")).unwrap();
        assert_eq!((s.codec, s.final_granule), (OggCodec::Other, -1));
    }

    #[test]
    fn parses_akb1_header() {
        let mut h = vec![0u8; AKB1_HEADER_SIZE];
        h[..4].copy_from_slice(b"AKB ");
        h[8..12].copy_from_slice(&210u32.to_le_bytes());
        h[14..16].copy_from_slice(&22050u16.to_le_bytes());
        h[20..24].copy_from_slice(&1u32.to_le_bytes());
        h[24..28].copy_from_slice(&2u32.to_le_bytes());
        h.extend([0u8; 6]);
        let p = AkbHeader::parse(&h).unwrap();
        assert_eq!(
            (p.version, p.sample_rate, p.content_size),
            (AkbVersion::Akb1, 22050, 6)
        );
        assert!(p.payload_range(h.len()).is_ok());
    }

    /// Real-install check; skips when the install is absent.
    #[test]
    fn real_install_every_sound_asset_is_classified() {
        let root = Path::new("E:/SteamLibrary/steamapps/common/FINAL FANTASY IX/StreamingAssets");
        if !root.join("p0data61.bin").is_file() {
            eprintln!("skipping: FFIX install not present");
            return;
        }
        let report = inspect_audio_bundle(&root.join("p0data61.bin")).expect("bundle");
        assert!(report.sound_assets > 0);
        assert_eq!(report.by_magic.values().sum::<usize>(), report.sound_assets);
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert_eq!(report.akb_parsed, report.akb_payload_in_range);
    }
}
