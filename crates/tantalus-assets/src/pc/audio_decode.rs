//! Vorbis decoding of AKB2/AKB1 sound assets and the FMV-to-sound-resource table.
//!
//! The container wraps a plain Ogg Vorbis stream (`AkbHeader::payload_range`). Decoding
//! is streamed one packet at a time through `lewton`; callers see only the sample
//! count, duration and peak unless they pass a PCM sink (used for WAV spot checks).

use super::akb2::{AkbHeader, PayloadKind, classify_magic, inspect_ogg};
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::{Cursor, Seek, SeekFrom, Write};
use std::path::Path;

// ---------------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------------

/// Summary of one decoded Ogg Vorbis stream. No PCM is retained.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VorbisDecodeStats {
    pub channels: u8,
    pub sample_rate: u32,
    /// Decoded sample frames (samples per channel).
    pub frames: u64,
    pub duration_seconds: f64,
    /// Largest absolute 16-bit sample value seen (0..=32768).
    pub peak: u32,
    pub packets: u64,
}

/// Receives each decoded interleaved 16-bit block.
pub type PcmSink<'a> = dyn FnMut(&[i16]) + 'a;

/// Decodes an Ogg Vorbis byte stream packet by packet.
///
/// `sink`, when present, receives each interleaved 16-bit block.
pub fn decode_vorbis(
    ogg: &[u8],
    mut sink: Option<&mut PcmSink<'_>>,
) -> Result<VorbisDecodeStats, String> {
    let mut reader = lewton::inside_ogg::OggStreamReader::new(Cursor::new(ogg))
        .map_err(|e| format!("Vorbis headers: {e}"))?;
    let channels = reader.ident_hdr.audio_channels;
    let sample_rate = reader.ident_hdr.audio_sample_rate;
    if channels == 0 {
        return Err("Vorbis stream declares zero channels".to_owned());
    }
    let (mut frames, mut peak, mut packets) = (0u64, 0u32, 0u64);
    loop {
        match reader.read_dec_packet_itl() {
            Ok(Some(block)) => {
                packets += 1;
                frames += (block.len() / usize::from(channels)) as u64;
                for &s in &block {
                    peak = peak.max(i32::from(s).unsigned_abs());
                }
                if let Some(sink) = sink.as_mut() {
                    sink(&block);
                }
            }
            Ok(None) => break,
            Err(e) => return Err(format!("Vorbis packet {packets}: {e}")),
        }
    }
    Ok(VorbisDecodeStats {
        channels,
        sample_rate,
        frames,
        duration_seconds: if sample_rate == 0 {
            0.0
        } else {
            frames as f64 / f64::from(sample_rate)
        },
        peak,
        packets,
    })
}

/// Loop observations for one AKB asset. Header fields are kept raw.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LoopInfo {
    pub loop_start: u32,
    pub loop_end: u32,
    pub loop_start_alternate: u32,
    pub loop_end_alternate: u32,
    /// `loop_end > loop_start` (`AkbHeader::has_loop`).
    pub has_loop: bool,
    /// Loop end lies inside the decoded stream (`loop_end < frames`).
    pub loop_end_inside_stream: bool,
    /// `loop_end + 1 == frames`: the loop ends exactly at the stream end.
    pub loop_end_is_last_frame: bool,
}

/// Result of decoding one `.akb.bytes` asset.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AkbDecode {
    pub header_sample_rate: u16,
    pub header_sample_count: u32,
    pub decode: VorbisDecodeStats,
    pub sample_rate_matches_header: bool,
    /// Granule position of the last Ogg page: the stream length the encoder declared.
    pub final_granule: i64,
    /// Decoded frames capped at `final_granule` (lewton returns whole blocks, so the raw
    /// count can exceed the declared length by a partial window).
    pub playable_frames: u64,
    /// `header_sample_count == playable_frames` (placeholder values on looped assets).
    pub sample_count_matches_header: bool,
    pub loop_info: LoopInfo,
}

/// Parses an AKB header, extracts the Ogg payload and decodes it.
pub fn decode_akb(script: &[u8], sink: Option<&mut PcmSink<'_>>) -> Result<AkbDecode, String> {
    match classify_magic(script) {
        PayloadKind::Akb1 | PayloadKind::Akb2 => {}
        PayloadKind::OggS => return Err("bare Ogg payload has no AKB header".to_owned()),
        PayloadKind::Other => return Err("unrecognized payload magic".to_owned()),
    }
    let h = AkbHeader::parse(script).map_err(|e| format!("AKB header: {e}"))?;
    let (offset, size) = h
        .payload_range(script.len())
        .map_err(|e| format!("AKB header: {e}"))?;
    let ogg = &script[offset..offset + size];
    let final_granule = inspect_ogg(ogg)
        .map_err(|e| format!("Ogg pages: {e}"))?
        .final_granule;
    let decode = decode_vorbis(ogg, sink)?;
    let frames =
        u64::try_from(final_granule.max(0)).map_or(decode.frames, |g| g.min(decode.frames));
    Ok(AkbDecode {
        header_sample_rate: h.sample_rate,
        header_sample_count: h.sample_count,
        sample_rate_matches_header: u32::from(h.sample_rate) == decode.sample_rate,
        final_granule,
        playable_frames: frames,
        sample_count_matches_header: u64::from(h.sample_count) == frames,
        loop_info: LoopInfo {
            loop_start: h.loop_start,
            loop_end: h.loop_end,
            loop_start_alternate: h.loop_start_alternate,
            loop_end_alternate: h.loop_end_alternate,
            has_loop: h.has_loop(),
            loop_end_inside_stream: u64::from(h.loop_end) < frames,
            loop_end_is_last_frame: u64::from(h.loop_end) + 1 == frames,
        },
        decode,
    })
}

// ---------------------------------------------------------------------------------
// WAV output (spot checks)
// ---------------------------------------------------------------------------------

/// Streaming 16-bit PCM WAV writer; sizes are patched in `finish`.
pub struct WavWriter<W: Write + Seek> {
    out: W,
    data_bytes: u64,
}

impl<W: Write + Seek> WavWriter<W> {
    pub fn new(mut out: W, channels: u16, sample_rate: u32) -> std::io::Result<Self> {
        let block_align = channels * 2;
        out.write_all(b"RIFF\0\0\0\0WAVEfmt ")?;
        out.write_all(&16u32.to_le_bytes())?;
        out.write_all(&1u16.to_le_bytes())?;
        out.write_all(&channels.to_le_bytes())?;
        out.write_all(&sample_rate.to_le_bytes())?;
        out.write_all(&(sample_rate * u32::from(block_align)).to_le_bytes())?;
        out.write_all(&block_align.to_le_bytes())?;
        out.write_all(&16u16.to_le_bytes())?;
        out.write_all(b"data\0\0\0\0")?;
        Ok(Self { out, data_bytes: 0 })
    }

    pub fn write_samples(&mut self, samples: &[i16]) -> std::io::Result<()> {
        for s in samples {
            self.out.write_all(&s.to_le_bytes())?;
        }
        self.data_bytes += samples.len() as u64 * 2;
        Ok(())
    }

    pub fn finish(mut self) -> std::io::Result<W> {
        let data = u32::try_from(self.data_bytes).unwrap_or(u32::MAX);
        self.out.seek(SeekFrom::Start(4))?;
        self.out.write_all(&data.saturating_add(36).to_le_bytes())?;
        self.out.seek(SeekFrom::Start(40))?;
        self.out.write_all(&data.to_le_bytes())?;
        self.out.flush()?;
        Ok(self.out)
    }
}

// ---------------------------------------------------------------------------------
// FMV -> sound resource table (Memoria SoundLib.GetMovieSoundIndex)
// ---------------------------------------------------------------------------------

/// How a movie name picks its sound resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MovieSoundSource {
    /// A fixed resource ID literal.
    Fixed(&'static str),
    /// `Localization.GetWithDefault("MovieAudioPathFMV060")`; see [`FMV060_LOCALIZED`].
    LocalizedFmv060,
}

/// One `case` of `SoundLib.GetMovieSoundIndex`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MovieSoundCase {
    pub movie: &'static str,
    pub source: MovieSoundSource,
    /// Line in `Assembly-CSharp/Global/Sound/Lib/SoundLib.cs` at the pinned commit.
    pub source_line: u32,
}

const fn fixed(movie: &'static str, resource: &'static str, line: u32) -> MovieSoundCase {
    MovieSoundCase {
        movie,
        source: MovieSoundSource::Fixed(resource),
        source_line: line,
    }
}

/// Every explicit `case` of `GetMovieSoundIndex`, verbatim (SoundLib.cs lines 39-55).
pub const MOVIE_SOUND_CASES: [MovieSoundCase; 17] = [
    fixed("FMV000", "Sounds01/BGM_/music033", 39),
    fixed("FMV059", "Sounds02/Movie_/FMV059A", 40),
    MovieSoundCase {
        movie: "FMV060",
        source: MovieSoundSource::LocalizedFmv060,
        source_line: 41,
    },
    fixed("mbg102", "Sounds02/song_/song0504_0", 42),
    fixed("mbg103", "Sounds02/song_/song0505_0", 43),
    fixed("mbg105", "Sounds02/song_/song0503_0", 44),
    fixed("mbg106", "Sounds02/song_/song0507_0", 45),
    fixed("mbg107", "Sounds02/song_/song0506_0", 46),
    fixed("mbg108", "Sounds02/song_/song0501_0", 47),
    fixed("mbg110", "Sounds02/song_/song0502_0", 48),
    fixed("mbg111", "Sounds02/song_/song0509_0", 49),
    fixed("mbg112", "Sounds02/song_/song0510_0", 50),
    fixed("mbg113", "Sounds02/song_/song0508_0", 51),
    fixed("mbg115", "Sounds02/song_/song0511_0", 52),
    fixed("mbg116", "Sounds02/song_/song1040_0", 53),
    fixed("mbg117", "Sounds02/song_/song0512_0", 54),
    fixed("mbg118", "Sounds02/song_/song0513_0", 55),
];

/// Prefix of the fall-through return (SoundLib.cs line 57): `"Sounds02/Movie_/" + movieName`.
pub const MOVIE_DEFAULT_PREFIX: &str = "Sounds02/Movie_/";

/// `MovieAudioPathFMV060` defaults (Localization.cs lines 190-196), as `(symbol, resource)`.
pub const FMV060_LOCALIZED: [(&str, &str); 7] = [
    ("US", "Sounds02/Movie_/FMV059C"),
    ("UK", "Sounds02/Movie_/FMV059C"),
    ("JP", "Sounds02/Movie_/FMV059B"),
    ("ES", "Sounds02/Movie_/FMV059C"),
    ("FR", "Sounds02/Movie_/FMV059C"),
    ("GR", "Sounds02/Movie_/FMV059C"),
    ("IT", "Sounds02/Movie_/FMV059C"),
];

/// How a resource ID was chosen for a movie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MovieSoundOrigin {
    ExplicitCase,
    LocalizedCase,
    DefaultFallback,
}

/// A movie's resource ID (for one localization where relevant).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MovieSoundChoice {
    pub movie: String,
    /// `None` unless the entry depends on a localization symbol.
    pub localization: Option<&'static str>,
    pub resource_id: String,
    pub origin: MovieSoundOrigin,
    /// Line of the literal in SoundLib.cs (or Localization.cs for localized rows).
    pub source_line: u32,
}

/// All resource IDs `GetMovieSoundIndex(movie)` can return (one per localization for FMV060).
pub fn movie_sound_choices(movie: &str) -> Vec<MovieSoundChoice> {
    if let Some(case) = MOVIE_SOUND_CASES.iter().find(|c| c.movie == movie) {
        return match case.source {
            MovieSoundSource::Fixed(resource) => vec![MovieSoundChoice {
                movie: movie.to_owned(),
                localization: None,
                resource_id: resource.to_owned(),
                origin: MovieSoundOrigin::ExplicitCase,
                source_line: case.source_line,
            }],
            MovieSoundSource::LocalizedFmv060 => FMV060_LOCALIZED
                .iter()
                .zip(190u32..)
                .map(|((symbol, resource), line)| MovieSoundChoice {
                    movie: movie.to_owned(),
                    localization: Some(symbol),
                    resource_id: (*resource).to_owned(),
                    origin: MovieSoundOrigin::LocalizedCase,
                    source_line: line,
                })
                .collect(),
        };
    }
    vec![MovieSoundChoice {
        movie: movie.to_owned(),
        localization: None,
        resource_id: format!("{MOVIE_DEFAULT_PREFIX}{movie}"),
        origin: MovieSoundOrigin::DefaultFallback,
        source_line: 57,
    }]
}

/// `SoundLoaderResources.Load` asks for `Sounds/<ResourceID>.akb`; the bundle container
/// stores it lowercase under `assets/resources/` with a `.bytes` suffix.
pub fn container_path_for_resource(resource_id: &str) -> String {
    format!(
        "assets/resources/sounds/{}.akb.bytes",
        resource_id.to_ascii_lowercase()
    )
}

/// `{ "data": [ { "name", "soundIndex" } ] }` entries of `MovieAudioMetaData.txt`.
pub fn parse_movie_audio_metadata(text: &[u8]) -> Result<BTreeMap<String, i64>, String> {
    let text = text.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(text);
    let value: serde_json::Value =
        serde_json::from_slice(text).map_err(|e| format!("metadata JSON: {e}"))?;
    let data = value
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or("metadata has no `data` array")?;
    let mut out = BTreeMap::new();
    for entry in data {
        let name = entry
            .get("name")
            .and_then(|n| n.as_str())
            .ok_or("metadata entry without `name`")?;
        let index = match entry.get("soundIndex") {
            Some(serde_json::Value::String(s)) => s.parse::<i64>().map_err(|e| e.to_string())?,
            Some(serde_json::Value::Number(n)) => n.as_i64().ok_or("soundIndex out of range")?,
            _ => return Err(format!("metadata entry `{name}` has no soundIndex")),
        };
        if out.insert(name.to_owned(), index).is_some() {
            return Err(format!("duplicate metadata name `{name}`"));
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------
// Bundle driver
// ---------------------------------------------------------------------------------

/// Per-asset decode result in the bundle report.
#[derive(Debug, Clone, Serialize)]
pub struct AssetDecodeRecord {
    pub path: String,
    pub frames: u64,
    pub duration_seconds: f64,
    pub peak: u32,
    pub channels: u8,
    pub sample_rate: u32,
    pub has_loop: bool,
    pub loop_start: u32,
    pub loop_end: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct DecodeFailure {
    pub path: String,
    pub reason: String,
}

/// Aggregate decode result for one bundle.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BundleDecodeReport {
    pub bundle: String,
    pub sound_assets: usize,
    pub decoded_ok: usize,
    pub decode_failed: usize,
    pub total_frames: u64,
    pub total_duration_seconds: f64,
    pub sample_rates: BTreeMap<u32, usize>,
    pub channel_counts: BTreeMap<u8, usize>,
    pub silent_assets: usize,
    pub clipped_assets: usize,
    pub sample_rate_header_mismatches: usize,
    pub sample_count_header_matches: usize,
    pub looped: usize,
    pub with_alternate_loop: usize,
    pub loop_end_inside_stream: usize,
    pub loop_end_is_last_frame: usize,
    pub loop_end_beyond_stream: usize,
    /// Assets with `loop_end <= loop_start` but a nonzero loop field.
    pub non_loop_with_nonzero_fields: usize,
    pub failures: Vec<DecodeFailure>,
    #[serde(skip)]
    pub assets: Vec<AssetDecodeRecord>,
}

/// Decodes every `/sounds/` asset of a bundle, one at a time.
///
/// Assets whose container path satisfies `wav_filter` are also written as WAV files
/// into `wav_dir` when it is set.
pub fn decode_audio_bundle(
    path: &Path,
    wav_dir: Option<&Path>,
    wav_filter: &dyn Fn(&str) -> bool,
) -> Result<BundleDecodeReport, String> {
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

    let mut out = BundleDecodeReport {
        bundle: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        ..Default::default()
    };
    for (asset_path, info) in &metadata.container {
        if !asset_path.to_ascii_lowercase().contains("/sounds/") {
            continue;
        }
        out.sound_assets += 1;
        let Some(&index) = index_of.get(&info.path_id) else {
            out.decode_failed += 1;
            out.failures.push(DecodeFailure {
                path: asset_path.clone(),
                reason: "container path ID has no object".to_owned(),
            });
            continue;
        };
        let script = match serialized
            .object_data(index)
            .map_err(|e| e.to_string())
            .and_then(|d| TextAsset::parse(d, asset_path).map_err(|e| e.to_string()))
        {
            Ok(asset) => asset.script,
            Err(reason) => {
                out.decode_failed += 1;
                out.failures.push(DecodeFailure {
                    path: asset_path.clone(),
                    reason: format!("not a TextAsset: {reason}"),
                });
                continue;
            }
        };
        let wav_target = wav_dir.filter(|_| wav_filter(asset_path));
        let result = decode_with_optional_wav(&script, wav_target, asset_path);
        match result {
            Ok(d) => out.record(asset_path, &d),
            Err(reason) => {
                out.decode_failed += 1;
                out.failures.push(DecodeFailure {
                    path: asset_path.clone(),
                    reason,
                });
            }
        }
    }
    Ok(out)
}

fn decode_with_optional_wav(
    script: &[u8],
    wav_dir: Option<&Path>,
    asset_path: &str,
) -> Result<AkbDecode, String> {
    let Some(dir) = wav_dir else {
        return decode_akb(script, None);
    };
    let h = AkbHeader::parse(script).map_err(|e| format!("AKB header: {e}"))?;
    let (offset, size) = h
        .payload_range(script.len())
        .map_err(|e| format!("AKB header: {e}"))?;
    let reader =
        lewton::inside_ogg::OggStreamReader::new(Cursor::new(&script[offset..offset + size]))
            .map_err(|e| format!("Vorbis headers: {e}"))?;
    let (channels, rate) = (
        u16::from(reader.ident_hdr.audio_channels),
        reader.ident_hdr.audio_sample_rate,
    );
    drop(reader);
    let file = std::fs::File::create(dir.join(asset_path.replace('/', "_") + ".wav"))
        .map_err(|e| format!("WAV create: {e}"))?;
    let mut wav = WavWriter::new(std::io::BufWriter::new(file), channels, rate)
        .map_err(|e| format!("WAV header: {e}"))?;
    let mut io_error = None;
    let mut sink = |block: &[i16]| {
        if io_error.is_none()
            && let Err(e) = wav.write_samples(block)
        {
            io_error = Some(e.to_string());
        }
    };
    let result = decode_akb(script, Some(&mut sink));
    if let Some(e) = io_error {
        return Err(format!("WAV write: {e}"));
    }
    wav.finish().map_err(|e| format!("WAV finish: {e}"))?;
    result
}

impl BundleDecodeReport {
    fn record(&mut self, asset_path: &str, d: &AkbDecode) {
        self.decoded_ok += 1;
        let seconds = d.playable_frames as f64 / f64::from(d.decode.sample_rate.max(1));
        self.total_frames += d.playable_frames;
        self.total_duration_seconds += seconds;
        *self.sample_rates.entry(d.decode.sample_rate).or_default() += 1;
        *self.channel_counts.entry(d.decode.channels).or_default() += 1;
        if d.decode.peak == 0 {
            self.silent_assets += 1;
        }
        if d.decode.peak >= 32767 {
            self.clipped_assets += 1;
        }
        if !d.sample_rate_matches_header {
            self.sample_rate_header_mismatches += 1;
        }
        if d.sample_count_matches_header {
            self.sample_count_header_matches += 1;
        }
        let l = &d.loop_info;
        if l.has_loop {
            self.looped += 1;
            if l.loop_end_inside_stream {
                self.loop_end_inside_stream += 1;
            } else {
                self.loop_end_beyond_stream += 1;
            }
            if l.loop_end_is_last_frame {
                self.loop_end_is_last_frame += 1;
            }
        } else if l.loop_end != 0 || l.loop_start != 0 {
            self.non_loop_with_nonzero_fields += 1;
        }
        if l.loop_end_alternate > l.loop_start_alternate {
            self.with_alternate_loop += 1;
        }
        self.assets.push(AssetDecodeRecord {
            path: asset_path.to_owned(),
            frames: d.playable_frames,
            duration_seconds: seconds,
            peak: d.decode.peak,
            channels: d.decode.channels,
            sample_rate: d.decode.sample_rate,
            has_loop: l.has_loop,
            loop_start: l.loop_start,
            loop_end: l.loop_end,
        });
    }
}

/// One movie-to-asset resolution row.
#[derive(Debug, Clone, Serialize)]
pub struct MovieAudioRow {
    pub movie: String,
    pub localization: Option<&'static str>,
    pub resource_id: String,
    pub origin: MovieSoundOrigin,
    pub source_line: u32,
    /// `soundIndex` of the resource ID in `MovieAudioMetaData.txt`, when listed.
    pub metadata_sound_index: Option<i64>,
    pub container_path: String,
    /// Bundle that holds `container_path`, when found.
    pub bundle: Option<String>,
    pub decoded: Option<AssetDecodeRecord>,
    pub resolved: bool,
    /// Why the row is unresolved, when it is.
    pub unresolved_reason: Option<String>,
}

/// Resolves every movie name to metadata and bundle assets.
///
/// `metadata` is `None` when `MovieAudioMetaData.txt` was not available; rows then
/// resolve on the bundle asset alone.
pub fn resolve_movie_audio(
    movies: &[String],
    metadata: Option<&BTreeMap<String, i64>>,
    bundles: &[BundleDecodeReport],
) -> Vec<MovieAudioRow> {
    let mut by_path: BTreeMap<String, (&str, &AssetDecodeRecord)> = BTreeMap::new();
    for b in bundles {
        for a in &b.assets {
            by_path.insert(a.path.to_ascii_lowercase(), (b.bundle.as_str(), a));
        }
    }
    let mut names: Vec<String> = movies.to_vec();
    for case in &MOVIE_SOUND_CASES {
        if !names.iter().any(|m| m == case.movie) {
            names.push(case.movie.to_owned());
        }
    }
    let mut rows = Vec::new();
    for movie in &names {
        for choice in movie_sound_choices(movie) {
            let container_path = container_path_for_resource(&choice.resource_id);
            let sound_index = metadata.and_then(|m| m.get(&choice.resource_id).copied());
            let found = by_path.get(&container_path);
            let reason = if found.is_none() {
                Some("no asset at the expected container path".to_owned())
            } else if metadata.is_some() && sound_index.is_none() {
                Some("asset exists but the resource ID is not in MovieAudioMetaData.txt".to_owned())
            } else {
                None
            };
            rows.push(MovieAudioRow {
                movie: choice.movie,
                localization: choice.localization,
                resource_id: choice.resource_id,
                origin: choice.origin,
                source_line: choice.source_line,
                metadata_sound_index: sound_index,
                container_path,
                bundle: found.map(|(b, _)| (*b).to_owned()),
                decoded: found.map(|(_, a)| (*a).clone()),
                resolved: reason.is_none(),
                unresolved_reason: reason,
            });
        }
    }
    rows
}

/// Reads one named TextAsset from a standalone serialized file (e.g. `resources.assets`).
pub fn read_text_asset(serialized_path: &Path, name: &str) -> Result<Vec<u8>, String> {
    use super::serialized::SerializedFile;
    use super::text_asset::TextAsset;
    let bytes = std::fs::read(serialized_path)
        .map_err(|e| format!("cannot read {}: {e}", serialized_path.display()))?;
    let serialized = SerializedFile::parse(&bytes, &serialized_path.to_string_lossy())
        .map_err(|e| e.to_string())?;
    for (index, object) in serialized.report().objects.iter().enumerate() {
        if object.class_id != 49 {
            continue;
        }
        let asset = TextAsset::parse(
            serialized.object_data(index).map_err(|e| e.to_string())?,
            name,
        )
        .map_err(|e| e.to_string())?;
        if asset.name == name || asset.name.ends_with(&format!("/{name}")) {
            return Ok(asset.script);
        }
    }
    Err(format!("text asset `{name}` not found"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 0.1 s stereo 440 Hz sine at 44.1 kHz, encoded with libvorbis (synthetic, no game data).
    const SINE: &[u8] = include_bytes!("../../tests/fixtures/sine-440-stereo.ogg");

    fn akb2_wrap(ogg: &[u8], rate: u16, count: u32, ls: u32, le: u32) -> Vec<u8> {
        let mut h = vec![0u8; 304];
        h[..4].copy_from_slice(b"AKB2");
        h[8..12].copy_from_slice(&((304 + ogg.len()) as u32).to_le_bytes());
        h[230..232].copy_from_slice(&rate.to_le_bytes());
        h[232..236].copy_from_slice(&(ogg.len() as u32).to_le_bytes());
        h[236..240].copy_from_slice(&count.to_le_bytes());
        h[240..244].copy_from_slice(&ls.to_le_bytes());
        h[244..248].copy_from_slice(&le.to_le_bytes());
        h.extend(ogg);
        h
    }

    #[test]
    fn decodes_synthetic_vorbis() {
        let stats = decode_vorbis(SINE, None).unwrap();
        assert_eq!((stats.channels, stats.sample_rate), (2, 44100));
        assert!((3000..=4410 + 2048).contains(&stats.frames), "{stats:?}");
        assert!(stats.peak > 1000 && stats.peak < 32768, "{stats:?}");
        assert!(stats.packets > 1);
    }

    #[test]
    fn decodes_through_akb2_wrapper_with_loop() {
        let file = akb2_wrap(SINE, 44100, 123, 100, 2000);
        let d = decode_akb(&file, None).unwrap();
        assert!(d.sample_rate_matches_header);
        assert!(!d.sample_count_matches_header);
        assert!(d.loop_info.has_loop);
        assert!(d.loop_info.loop_end_inside_stream);
        assert_eq!(d.playable_frames, 4410);
        assert_eq!(d.final_granule, 4410);
        let mut pcm = 0usize;
        let mut sink = |b: &[i16]| pcm += b.len();
        let d2 = decode_akb(&file, Some(&mut sink)).unwrap();
        assert_eq!(pcm as u64, d2.decode.frames * 2);
    }

    #[test]
    fn rejects_bad_payloads() {
        let file = akb2_wrap(b"OggS-not-vorbis", 44100, 1, 0, 0);
        assert!(decode_akb(&file, None).is_err());
        assert!(decode_akb(b"junkjunk", None).is_err());
        assert!(decode_vorbis(&SINE[..60], None).is_err());
    }

    #[test]
    fn wav_writer_patches_sizes() {
        let mut w = WavWriter::new(Cursor::new(Vec::new()), 2, 44100).unwrap();
        w.write_samples(&[1, -1, 2, -2]).unwrap();
        let bytes = w.finish().unwrap().into_inner();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 36 + 8);
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 8);
        assert_eq!(bytes.len(), 44 + 8);
    }

    #[test]
    fn movie_table_matches_source_cases() {
        assert_eq!(MOVIE_SOUND_CASES.len(), 17);
        let c = movie_sound_choices("FMV000");
        assert_eq!(c[0].resource_id, "Sounds01/BGM_/music033");
        assert_eq!(c[0].origin, MovieSoundOrigin::ExplicitCase);
        let c = movie_sound_choices("FMV060");
        assert_eq!(c.len(), 7);
        assert_eq!(
            c.iter()
                .find(|x| x.localization == Some("JP"))
                .unwrap()
                .resource_id,
            "Sounds02/Movie_/FMV059B"
        );
        let c = movie_sound_choices("FMV012");
        assert_eq!(c[0].resource_id, "Sounds02/Movie_/FMV012");
        assert_eq!(c[0].origin, MovieSoundOrigin::DefaultFallback);
        assert_eq!(
            container_path_for_resource("Sounds02/Movie_/FMV059A"),
            "assets/resources/sounds/sounds02/movie_/fmv059a.akb.bytes"
        );
    }

    #[test]
    fn parses_metadata_and_resolves_rows() {
        let meta = parse_movie_audio_metadata(
            b"\xEF\xBB\xBF{\"data\":[{\"name\":\"Sounds02/Movie_/FMV001\",\"soundIndex\":\"0\"}]}",
        )
        .unwrap();
        assert_eq!(meta["Sounds02/Movie_/FMV001"], 0);
        let bundle = BundleDecodeReport {
            bundle: "p0data62.bin".to_owned(),
            assets: vec![AssetDecodeRecord {
                path: "assets/resources/sounds/sounds02/movie_/fmv001.akb.bytes".to_owned(),
                frames: 10,
                duration_seconds: 0.0,
                peak: 1,
                channels: 2,
                sample_rate: 44100,
                has_loop: false,
                loop_start: 0,
                loop_end: 0,
            }],
            ..Default::default()
        };
        let rows = resolve_movie_audio(&["FMV001".to_owned()], Some(&meta), &[bundle]);
        let r = rows.iter().find(|r| r.movie == "FMV001").unwrap();
        assert!(r.resolved && r.bundle.as_deref() == Some("p0data62.bin"));
        let missing = rows.iter().find(|r| r.movie == "FMV000").unwrap();
        assert!(!missing.resolved);
    }
}
