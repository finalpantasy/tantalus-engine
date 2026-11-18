//! `tantalus-tools psx-audio`: read the owner's FF9 discs and render PS1 music to Ogg Vorbis, plus
//! the loader the client uses to prefer those files over the Steam ones.
//!
//! Nothing here ships audio. The renderer reads the user's own `.bin` images and writes under the
//! user's data folder (`%APPDATA%/tantalus/psx-audio`, or `TANTALUS_PSX_AUDIO_DIR`).
//!
//! The mapping to the Steam game is by sound id: the music file ids in `FF9.IMG` are the music
//! ids the game scripts use, and `MusicMetaData.txt` maps each to its `Sounds01/BGM_/musicNNN`
//! resource. A rendered track replaces the Steam resource of the same music id.
//!
//! Effect sounds (`render-sfx`) map by effect: the Steam `ef<NNN>` effect files are the PS1
//! effect files byte for byte (the archive is `FF9.IMG`'s kind-3 group, file index = effect id),
//! and `SpecialEffectMetaData.txt` lists each effect's sound resources by the rank of the
//! effect's sound files. A rendered program replaces the Steam resource at its rank.
//!
//! Field and menu sounds map by sound id: every field file, and the resident data of the first
//! group, keeps its sound programs in a typed list keyed by the sound id the scripts play
//! (`tantalus_psxaudio::cluster`), which is the `SoundEffectMetaData.txt` id. A rendered program
//! replaces the Steam resource of the same id.

use std::collections::BTreeMap;
use std::io::Write;
use std::num::{NonZeroU8, NonZeroU32};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tantalus_legacy::sound::PcmClip;
use tantalus_psxaudio::akao::{SampleBank, Song, read_songs};
use tantalus_psxaudio::disc::{Disc, find_discs};
use tantalus_psxaudio::img::Img;
use tantalus_psxaudio::render::{Rendered, render_capped, render_looping, render_song};
use tantalus_psxaudio::sfx::{SfxEntry, read_sfx_entries};

use crate::sound_bank::SoundBank;

/// File name of the manifest next to the rendered clips.
pub const MANIFEST: &str = "psx-audio.json";

/// The default output and lookup folder.
pub fn default_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("TANTALUS_PSX_AUDIO_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return PathBuf::from(appdata).join("tantalus").join("psx-audio");
    }
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home).join(".tantalus").join("psx-audio")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// `music` for now; effects get their own kind.
    pub kind: String,
    /// The Steam resource this clip replaces, e.g. `Sounds01/BGM_/music006`.
    pub resource: String,
    /// The game's music id (`MusicMetaData.txt` soundIndex).
    pub sound_id: i32,
    pub file: String,
    pub sample_rate: u32,
    pub frames: u32,
    /// Loop start and end frame (inclusive), as the Steam clips express them; `None` plays once.
    pub loop_frames: Option<(u32, u32)>,
    pub peak: f32,
    pub disc: u8,
    pub ps1_sequence_id: u16,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub entries: Vec<ManifestEntry>,
}

/// Rendered clips found on disk, keyed by Steam resource id.
pub struct PsxClips {
    dir: PathBuf,
    by_resource: BTreeMap<String, ManifestEntry>,
}

impl PsxClips {
    /// Opens a folder written by `psx-audio`; `None` when it has no manifest.
    pub fn open(dir: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(dir.join(MANIFEST)).ok()?;
        let manifest: Manifest = serde_json::from_str(&text).ok()?;
        Some(Self {
            dir: dir.to_owned(),
            by_resource: manifest
                .entries
                .into_iter()
                .map(|e| (e.resource.clone(), e))
                .collect(),
        })
    }

    pub fn len(&self) -> usize {
        self.by_resource.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_resource.is_empty()
    }

    pub fn has(&self, resource: &str) -> bool {
        self.by_resource.contains_key(resource)
    }

    /// Decodes the rendered clip that replaces `resource`, loop points included.
    pub fn load(&self, resource: &str) -> Result<Option<PcmClip>, String> {
        let Some(entry) = self.by_resource.get(resource) else {
            return Ok(None);
        };
        let path = self.dir.join(&entry.file);
        let file = std::fs::File::open(&path)
            .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        let mut reader = lewton::inside_ogg::OggStreamReader::new(std::io::BufReader::new(file))
            .map_err(|e| format!("{}: {e:?}", path.display()))?;
        let channels = u16::from(reader.ident_hdr.audio_channels);
        let mut samples: Vec<i16> = Vec::new();
        while let Some(block) = reader
            .read_dec_packet_itl()
            .map_err(|e| format!("{}: {e:?}", path.display()))?
        {
            samples.extend_from_slice(&block);
        }
        // The Vorbis decoder can return a few frames of padding past the real end.
        samples.truncate(entry.frames as usize * usize::from(channels));
        let (loop_start, loop_end) = entry.loop_frames.unwrap_or((0, 0));
        Ok(Some(PcmClip {
            sample_rate: reader.ident_hdr.audio_sample_rate,
            channels,
            samples,
            loop_start,
            loop_end,
            loop_start_alternate: 0,
            loop_end_alternate: 0,
        }))
    }
}

/// Every song on the discs, by music id. The first disc that has an id wins; the sample
/// collection comes from the song's own file or, when it has none, from any file whose bank has
/// the id the sequence names.
pub struct DiscSongs {
    pub songs: BTreeMap<u32, (Song, SampleBank)>,
    pub problems: Vec<String>,
}

pub fn read_discs(disc_dir: &Path) -> Result<DiscSongs, String> {
    let discs = find_discs(disc_dir);
    if discs.is_empty() {
        return Err(format!(
            "no `Disc<N>of4` .bin images in {}",
            disc_dir.display()
        ));
    }
    let mut all: Vec<Song> = Vec::new();
    let mut problems = Vec::new();
    for (n, path) in &discs {
        let mut disc = Disc::open(path)?;
        let img = Img::open(&mut disc)?;
        for song in read_songs(*n, &mut disc, &img) {
            match song {
                Ok(song) => all.push(song),
                Err(e) => problems.push(e),
            }
        }
    }
    let mut banks: BTreeMap<u16, SampleBank> = BTreeMap::new();
    for song in &all {
        if let Some(bank) = &song.bank {
            banks.entry(bank.id).or_insert_with(|| bank.clone());
        }
    }
    let mut songs = BTreeMap::new();
    for song in all {
        if songs.contains_key(&song.file_id) {
            continue;
        }
        let bank = song
            .bank
            .clone()
            .or_else(|| banks.get(&song.sequence.sample_set).cloned());
        match bank {
            Some(bank) => {
                songs.insert(song.file_id, (song, bank));
            }
            None => problems.push(format!(
                "disc {} music {}: no sample collection {:#x}",
                song.disc, song.file_id, song.sequence.sample_set
            )),
        }
    }
    Ok(DiscSongs { songs, problems })
}

fn basename(resource: &str) -> &str {
    resource.rsplit('/').next().unwrap_or(resource)
}

/// Encodes interleaved stereo f32 to Ogg Vorbis (quality 0.8, roughly 200 kbit/s).
pub fn encode_ogg(samples: &[f32], path: &Path) -> Result<(), String> {
    let frames = samples.len() / 2;
    let mut left = Vec::with_capacity(frames);
    let mut right = Vec::with_capacity(frames);
    for pair in samples.chunks_exact(2) {
        left.push(pair[0]);
        right.push(pair[1]);
    }
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut builder = vorbis_rs::VorbisEncoderBuilder::new(
        NonZeroU32::new(44_100).expect("nonzero"),
        NonZeroU8::new(2).expect("nonzero"),
        std::io::BufWriter::new(file),
    )
    .map_err(|e| e.to_string())?;
    builder.bitrate_management_strategy(vorbis_rs::VorbisBitrateManagementStrategy::QualityVbr {
        target_quality: 0.8,
    });
    let mut encoder = builder.build().map_err(|e| e.to_string())?;
    for block in 0..frames.div_ceil(4096) {
        let a = block * 4096;
        let b = (a + 4096).min(frames);
        encoder
            .encode_audio_block([&left[a..b], &right[a..b]])
            .map_err(|e| e.to_string())?;
    }
    let mut out = encoder.finish().map_err(|e| e.to_string())?;
    out.flush().map_err(|e| e.to_string())
}

/// Minimal 16-bit PCM WAV writer for the analysis dump.
fn write_wav(path: &Path, rate: u32, channels: u16, samples: &[i16]) -> Result<(), String> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, out).map_err(|e| format!("{}: {e}", path.display()))
}

/// Every sample collection of the discs' `SfxBanks` groups, by collection id.
pub fn read_sfx_banks(disc_dir: &Path) -> Result<BTreeMap<u16, SampleBank>, String> {
    let mut banks = BTreeMap::new();
    for (_, path) in find_discs(disc_dir) {
        let mut disc = Disc::open(&path)?;
        let img = Img::open(&mut disc)?;
        for g in img.groups_of(&mut disc, tantalus_psxaudio::img::GroupKind::SfxBanks)? {
            for f in img.group(g) {
                let data = img.read(&mut disc, f)?;
                if let Ok(bank) = SampleBank::parse(&data, 64) {
                    banks.entry(bank.id).or_insert(bank);
                }
            }
        }
    }
    Ok(banks)
}

/// The `SfxBanks` collections by the archive file id (which is the special-effect id they belong to).
pub fn read_sfx_banks_by_file(disc_dir: &Path) -> Result<BTreeMap<u32, SampleBank>, String> {
    let mut banks = BTreeMap::new();
    for (_, path) in find_discs(disc_dir) {
        let mut disc = Disc::open(&path)?;
        let img = Img::open(&mut disc)?;
        for g in img.groups_of(&mut disc, tantalus_psxaudio::img::GroupKind::SfxBanks)? {
            for f in img.group(g) {
                let data = img.read(&mut disc, f)?;
                if let Ok(bank) = SampleBank::parse(&data, 64) {
                    banks.entry(f.id).or_insert(bank);
                }
            }
        }
    }
    Ok(banks)
}

/// Every distinct sound-effect script with the collection it plays: `(entry, script, bank id)`.
pub fn read_sfx_units(
    disc_dir: &Path,
    banks: &BTreeMap<u16, SampleBank>,
) -> Result<Vec<(SfxEntry, usize, u16)>, String> {
    let ids: std::collections::BTreeSet<u16> = banks.keys().copied().collect();
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for (n, path) in find_discs(disc_dir) {
        let mut disc = Disc::open(&path)?;
        let img = Img::open(&mut disc)?;
        for entry in read_sfx_entries(n, &mut disc, &img, &ids) {
            for script in 0..entry.scripts.len() {
                let bank = match entry.bank_hint {
                    Some(b) => b,
                    None => {
                        let Some(art) = entry.first_articulation(script) else {
                            continue;
                        };
                        let Some(b) = entry
                            .file_banks
                            .iter()
                            .copied()
                            .find(|b| banks.get(b).is_some_and(|bank| bank.art(art).is_some()))
                        else {
                            continue;
                        };
                        b
                    }
                };
                // The same script on the same collection is one effect, wherever it is stored.
                let body = &entry.data[entry.scripts[script]..];
                let body = &body[..body.len().min(512)];
                if seen.insert((bank, body.to_vec())) {
                    out.push((entry.clone(), script, bank));
                }
            }
        }
    }
    Ok(out)
}

/// Mono 16-bit WAV at `rate` from interleaved samples (linear resample), for analysis dumps.
fn mono_wav(
    path: &Path,
    samples: &[f32],
    channels: usize,
    rate: u32,
    out_rate: u32,
) -> Result<(), String> {
    let frames = samples.len() / channels;
    let mono: Vec<f32> = (0..frames)
        .map(|f| {
            samples[f * channels..(f + 1) * channels]
                .iter()
                .sum::<f32>()
                / channels as f32
        })
        .collect();
    let n = (frames as u64 * u64::from(out_rate) / u64::from(rate)) as usize;
    let pcm: Vec<i16> = (0..n)
        .map(|i| {
            let x = i as f64 * f64::from(rate) / f64::from(out_rate);
            let a = x as usize;
            let t = (x - a as f64) as f32;
            let v = mono[a.min(frames - 1)] * (1.0 - t) + mono[(a + 1).min(frames - 1)] * t;
            (v * 32767.0).clamp(-32768.0, 32767.0) as i16
        })
        .collect();
    write_wav(path, out_rate, 1, &pcm)
}

fn sfx_dump(a: &Args) -> Result<(), String> {
    let dir = a.dump.clone().ok_or("sfx-dump needs --dump <dir>")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let banks = read_sfx_banks(&a.disc_dir)?;
    let units = read_sfx_units(&a.disc_dir, &banks)?;
    println!("{} banks, {} distinct scripts", banks.len(), units.len());
    let mut tsv = String::new();
    for (i, (entry, script, bank)) in units.iter().enumerate() {
        let seq = entry.sequence(*script);
        let r = render_capped(&seq, &banks[bank], 10);
        mono_wav(
            &dir.join(format!("ps1_{i}.wav")),
            &r.samples,
            2,
            44_100,
            22_050,
        )?;
        tsv.push_str(&format!(
            "{i}	{}	{}	{}	{}	{}	{script}	{bank}	{:.3}	{:.3}	{:?}
",
            entry.disc,
            entry.group,
            entry.file_id,
            entry.offset,
            entry.kind,
            r.seconds(),
            r.peak,
            r.loop_frames
        ));
    }
    std::fs::write(dir.join("ps1_units.tsv"), tsv).map_err(|e| e.to_string())?;
    Ok(())
}

fn sfx_dump_steam(a: &Args, mut steam: SoundBank) -> Result<(), String> {
    let dir = a.dump.clone().ok_or("needs --dump <dir>")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut tsv = String::new();
    for (id, resource) in steam.metadata.sound_effect.clone() {
        let path = dir.join(format!("steam_{id}.wav"));
        if path.exists() {
            continue;
        }
        let Ok(clip) = steam.load_clip(&resource) else {
            continue;
        };
        let f: Vec<f32> = clip
            .samples
            .iter()
            .map(|s| f32::from(*s) / 32768.0)
            .collect();
        if f.len() < usize::from(clip.channels) {
            continue;
        }
        mono_wav(
            &path,
            &f,
            usize::from(clip.channels),
            clip.sample_rate,
            22_050,
        )?;
        tsv.push_str(&format!(
            "{id}	{resource}	{}	{}	{}	{}
",
            clip.sample_rate,
            clip.channels,
            clip.frames(),
            clip.loop_end
        ));
    }
    std::fs::write(dir.join("steam_sfx.tsv"), tsv).map_err(|e| e.to_string())?;
    Ok(())
}

struct Args {
    command: String,
    disc_dir: PathBuf,
    out: PathBuf,
    steam: PathBuf,
    ids: Option<Vec<i32>>,
    /// `compare --dump <dir>`: also write both renders as 16-bit WAV for external analysis.
    dump: Option<PathBuf>,
    /// `render-sfx --families SE00,SE50`: render only these Steam sound families (the rest of the
    /// manifest is kept), so a slow machine can render in parts.
    families: Option<Vec<String>>,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut a = Args {
        command: "render".to_owned(),
        disc_dir: PathBuf::new(),
        out: default_dir(),
        steam: PathBuf::from("E:/SteamLibrary/steamapps/common/FINAL FANTASY IX"),
        ids: None,
        dump: None,
        families: None,
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().cloned().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--disc-dir" => a.disc_dir = PathBuf::from(value("--disc-dir")?),
            "--out" => a.out = PathBuf::from(value("--out")?),
            "--dump" => a.dump = Some(PathBuf::from(value("--dump")?)),
            "--steam" => a.steam = PathBuf::from(value("--steam")?),
            "--families" => {
                a.families = Some(
                    value("--families")?
                        .split(',')
                        .map(|s| s.trim().to_owned())
                        .collect(),
                )
            }
            "--ids" => {
                a.ids = Some(
                    value("--ids")?
                        .split(',')
                        .map(|s| s.trim().parse().map_err(|_| format!("bad id `{s}`")))
                        .collect::<Result<_, _>>()?,
                )
            }
            "render" | "compare" | "coverage" | "verify" | "sfx-dump" | "sfx-dump-steam"
            | "render-sfx" | "sfx-coverage" | "sfx-compare" => a.command = arg.clone(),
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    if a.disc_dir.as_os_str().is_empty() && !matches!(a.command.as_str(), "sfx-compare") {
        return Err("--disc-dir <folder with the four .bin images> is required".to_owned());
    }
    Ok(a)
}

pub fn run(args: &[String]) -> Result<(), String> {
    let a = parse(args)?;
    if a.command == "verify" {
        return verify(&a.out);
    }
    if a.command == "render-sfx" || a.command == "sfx-coverage" {
        return render_sfx(&a);
    }
    if a.command == "sfx-compare" {
        return sfx_compare(&a);
    }
    let steam = SoundBank::open(
        &a.steam.join("StreamingAssets"),
        &a.steam.join("x64/FF9_Data"),
    )?;
    if a.command == "sfx-dump" {
        return sfx_dump(&a);
    }
    if a.command == "sfx-dump-steam" {
        return sfx_dump_steam(&a, steam);
    }
    let discs = read_discs(&a.disc_dir)?;
    for p in &discs.problems {
        eprintln!("note: {p}");
    }
    let wanted = |id: i32| a.ids.as_ref().is_none_or(|ids| ids.contains(&id));
    match a.command.as_str() {
        "coverage" => {
            let covered: Vec<i32> = steam
                .metadata
                .music
                .keys()
                .copied()
                .filter(|id| *id >= 0 && discs.songs.contains_key(&(*id as u32)))
                .collect();
            let missing: Vec<i32> = steam
                .metadata
                .music
                .keys()
                .copied()
                .filter(|id| !covered.contains(id))
                .collect();
            let extra: Vec<u32> = discs
                .songs
                .keys()
                .copied()
                .filter(|id| !steam.metadata.music.contains_key(&(*id as i32)))
                .collect();
            println!(
                "steam music ids {}, covered by a disc song {}, missing {:?}, disc songs with no steam id {:?}",
                steam.metadata.music.len(),
                covered.len(),
                missing,
                extra
            );
            Ok(())
        }
        "compare" => {
            let mut steam = steam;
            for (id, resource) in steam.metadata.music.clone() {
                if !wanted(id) || id < 0 {
                    continue;
                }
                let Some((song, bank)) = discs.songs.get(&(id as u32)) else {
                    println!("music {id} {resource}: no disc song");
                    continue;
                };
                let r = render_song(&song.sequence, bank);
                let clip = steam.load_clip(&resource)?;
                if let Some(dir) = &a.dump {
                    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                    let ps1: Vec<i16> = r
                        .samples
                        .iter()
                        .map(|s| (s * 32767.0).round() as i16)
                        .collect();
                    write_wav(&dir.join(format!("ps1_{id}.wav")), 44_100, 2, &ps1)?;
                    write_wav(
                        &dir.join(format!("steam_{id}.wav")),
                        clip.sample_rate,
                        clip.channels,
                        &clip.samples,
                    )?;
                }
                println!(
                    "music {id} {resource}: ps1 {:.2}s loop {:?} peak {:.3} | steam {:.2}s ({} Hz) loop {}..{} peak {:.3} | {:?}",
                    r.seconds(),
                    r.loop_frames,
                    r.peak,
                    clip.frames() as f64 / f64::from(clip.sample_rate),
                    clip.sample_rate,
                    clip.loop_start,
                    clip.loop_end,
                    clip.samples
                        .iter()
                        .fold(0f32, |m, s| m.max((f32::from(*s) / 32768.0).abs())),
                    r.stats
                );
            }
            Ok(())
        }
        _ => render_all(&a, &steam, &discs),
    }
}

fn render_all(a: &Args, steam: &SoundBank, discs: &DiscSongs) -> Result<(), String> {
    let dir = a.out.join("music");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut manifest = Manifest {
        version: 1,
        entries: Vec::new(),
    };
    let wanted = |id: i32| a.ids.as_ref().is_none_or(|ids| ids.contains(&id));
    for (id, resource) in &steam.metadata.music {
        if *id < 0 || !wanted(*id) {
            continue;
        }
        let Some((song, bank)) = discs.songs.get(&(*id as u32)) else {
            continue;
        };
        let rendered: Rendered = render_song(&song.sequence, bank);
        let name = format!("music/{}.ogg", basename(resource));
        encode_ogg(&rendered.samples, &a.out.join(&name))?;
        println!(
            "music {id} {resource}: {:.1} s, loop {:?}, peak {:.3}",
            rendered.seconds(),
            rendered.loop_frames,
            rendered.peak
        );
        manifest.entries.push(ManifestEntry {
            kind: "music".to_owned(),
            resource: resource.clone(),
            sound_id: *id,
            file: name,
            sample_rate: 44_100,
            frames: rendered.frames() as u32,
            loop_frames: rendered.loop_frames,
            peak: rendered.peak,
            disc: song.disc,
            ps1_sequence_id: song.sequence.id,
        });
    }
    // A partial run (--ids) adds to an existing manifest instead of replacing it, and a full
    // music run keeps the sound-effect entries.
    let path = a.out.join(MANIFEST);
    if let Ok(text) = std::fs::read_to_string(&path)
        && let Ok(mut old) = serde_json::from_str::<Manifest>(&text)
    {
        let partial = a.ids.is_some();
        old.entries.retain(|e| {
            (partial || e.kind != "music")
                && !manifest.entries.iter().any(|n| n.resource == e.resource)
        });
        old.entries.extend(manifest.entries);
        manifest = old;
    }
    manifest.entries.sort_by_key(|e| e.sound_id);
    let text = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    println!(
        "wrote {} clips to {}",
        manifest.entries.len(),
        a.out.display()
    );
    Ok(())
}

/// Objective checks of a rendered folder: sample rate, length against the manifest, clipping, and
/// how far the loop seam jumps compared with the music's own sample-to-sample motion.
fn verify(dir: &Path) -> Result<(), String> {
    let clips = PsxClips::open(dir).ok_or("no psx-audio.json in the folder")?;
    let mut bad = 0;
    let mut worst_seam = 0f32;
    for (resource, entry) in &clips.by_resource {
        let clip = clips
            .load(resource)?
            .ok_or("manifest entry without a clip")?;
        let ch = usize::from(clip.channels);
        let frames = clip.frames();
        let peak = clip
            .samples
            .iter()
            .map(|s| i32::from(*s).abs())
            .max()
            .unwrap_or(0);
        let mut problems = Vec::new();
        if clip.sample_rate != 44_100 {
            problems.push(format!("rate {}", clip.sample_rate));
        }
        if frames != entry.frames as usize {
            problems.push(format!("frames {frames} != manifest {}", entry.frames));
        }
        if peak >= 32_767 {
            problems.push("clipped".to_owned());
        }
        let mut seam = String::new();
        if let Some((start, end)) = entry.loop_frames {
            let (start, end) = (start as usize, end as usize);
            if end >= frames || start >= end {
                problems.push(format!("loop {start}..{end} outside {frames} frames"));
            } else {
                // Jump from the last loop frame to the first, against the typical step nearby.
                let at = |f: usize, c: usize| f32::from(clip.samples[f * ch + c]);
                let mut jump = 0f32;
                let mut step = 0f32;
                let mut n = 0f32;
                for c in 0..ch {
                    jump = jump.max((at(start, c) - at(end, c)).abs());
                    for f in end.saturating_sub(256)..end {
                        step += (at(f + 1, c) - at(f, c)).abs();
                        n += 1.0;
                    }
                }
                let ratio = jump / (step / n).max(1.0);
                worst_seam = worst_seam.max(ratio);
                seam = format!(" seam jump {jump:.0} ({ratio:.1}x the local step)");
            }
        }
        if !problems.is_empty() {
            bad += 1;
        }
        println!(
            "{resource}: {:.1}s {} Hz peak {:.3} loop {:?}{seam}{}",
            frames as f64 / 44_100.0,
            clip.sample_rate,
            peak as f32 / 32768.0,
            entry.loop_frames,
            if problems.is_empty() {
                String::new()
            } else {
                format!(" PROBLEMS {problems:?}")
            }
        );
    }
    println!(
        "{} clips, {bad} with problems, worst loop seam {worst_seam:.1}x the local step",
        clips.len()
    );
    if bad > 0 {
        Err(format!("{bad} clips failed"))
    } else {
        Ok(())
    }
}

/// One Steam sound resource and the disc effect file that holds its program.
struct EffectSound {
    resource: String,
    effect: i32,
    rank: usize,
    disc: u8,
    bytes: Vec<u8>,
}

/// Walks the discs' effect archives and the Steam effect sound lists together.
///
/// The Steam `ef<NNN>` effect files are the PS1 effect files byte for byte, and
/// `SpecialEffectMetaData.txt` lists each effect's sound resources by the rank of the effect's
/// sound files (`battle_sound::effect_timeline`: `PLAY_SOUND` numbers sound files by their rank
/// among the files the sequence plays). So effect E's rank-N sound file is the program behind
/// list entry N. Only effects whose list length equals their number of sound files are used.
/// Also returns the battle's resident sound list (`ResidentSpecialEffectMetaData.txt`).
fn effect_sounds(
    a: &Args,
    stats: &mut String,
) -> Result<(Vec<EffectSound>, usize, Vec<String>), String> {
    use tantalus_assets::pc::special_effect::{EffectFile, RecordDetail};
    let (lists, resident) = crate::battle_sound::effect_sound_lists(&a.steam.join("x64/FF9_Data"))?;
    let mut effects: BTreeMap<usize, (u8, Vec<u8>)> = BTreeMap::new();
    for (n, path) in find_discs(&a.disc_dir) {
        let mut disc = Disc::open(&path)?;
        let img = Img::open(&mut disc)?;
        for f in &img.effects {
            if let std::collections::btree_map::Entry::Vacant(v) = effects.entry(f.index) {
                v.insert((n, img.read(&mut disc, f)?));
            }
        }
    }
    let mut out = Vec::new();
    let mut skipped = 0;
    let (mut used, mut length_mismatch, mut unparsed, mut no_disc) = (0, 0, 0, 0);
    for (effect, list) in &lists {
        let Some((n, bytes)) = usize::try_from(*effect).ok().and_then(|i| effects.get(&i)) else {
            no_disc += 1;
            continue;
        };
        let name = format!("ef{effect:03}");
        let Ok(mut file) = EffectFile::parse(bytes, &name) else {
            unparsed += 1;
            continue;
        };
        if file.resolve_small_files(bytes, &name).is_err() {
            unparsed += 1;
            continue;
        }
        let files: BTreeMap<usize, usize> = file
            .sequence_summary
            .sound_refs
            .iter()
            .filter_map(|r| Some((r.file?, r.group)))
            .collect();
        if files.len() != list.len() {
            length_mismatch += 1;
            skipped += list.len();
            continue;
        }
        used += 1;
        for (rank, (index, group)) in files.iter().enumerate() {
            let Some(table) = file
                .records
                .iter()
                .filter(|r| r.group == *group)
                .find_map(|r| match &r.detail {
                    RecordDetail::SmallFiles(t) => Some(t),
                    _ => None,
                })
            else {
                continue;
            };
            let Some(small) = table.files.get(*index) else {
                continue;
            };
            let start = table.base_offset + small.offset;
            let Some(slice) = bytes.get(start..start + small.size) else {
                continue;
            };
            out.push(EffectSound {
                resource: list[rank].clone(),
                effect: *effect,
                rank,
                disc: *n,
                bytes: slice.to_vec(),
            });
        }
    }
    stats.push_str(&format!(
        "effects with a Steam list {}, used {used}, list length differs from the sound file count {length_mismatch}, no disc file {no_disc}, unparsed {unparsed}\n",
        lists.len()
    ));
    Ok((out, skipped, resident))
}

/// Sound programs of the discs' field and resident lists, by sound id (the `SoundEffectMetaData.txt`
/// id), with the disc they were read from. Each id has the same program on every disc that has it
/// (checked: 1060 ids, no variants), so the first disc wins.
pub fn read_sound_programs(disc_dir: &Path) -> Result<BTreeMap<u16, (u8, Vec<u8>)>, String> {
    use tantalus_psxaudio::cluster::{CLUSTER, SOUND_PROGRAMS, objects};
    let mut out: BTreeMap<u16, (u8, Vec<u8>)> = BTreeMap::new();
    for (n, path) in find_discs(disc_dir) {
        let mut disc = Disc::open(&path)?;
        let img = Img::open(&mut disc)?;
        for f in &img.files {
            // Backgrounds and movies are big and are not clusters.
            if f.blocks == 0 || f.blocks > 6000 {
                continue;
            }
            match disc.read(img.base_lba + f.start_block, 16) {
                Ok(head) if head.first() == Some(&CLUSTER) => {}
                _ => continue,
            }
            let Ok(data) = img.read(&mut disc, f) else {
                continue;
            };
            for o in objects(&data) {
                if o.list_type == SOUND_PROGRAMS && data[o.range.clone()].starts_with(b"AKAO") {
                    out.entry(o.id)
                        .or_insert_with(|| (n, data[o.range.clone()].to_vec()));
                }
            }
        }
    }
    Ok(out)
}

/// The battle's resident sound programs: the `AKAO` programs inside the code files of the
/// archive's first group (the battle overlay, file 11 on disc 1, holds twelve, each naming
/// collection 1), in file order, with the disc read.
pub fn read_resident_programs(
    disc_dir: &Path,
    banks: &std::collections::BTreeSet<u16>,
) -> Result<Vec<(u8, SfxEntry)>, String> {
    let Some((n, path)) = find_discs(disc_dir).into_iter().next() else {
        return Ok(Vec::new());
    };
    let mut disc = Disc::open(&path)?;
    let img = Img::open(&mut disc)?;
    let Some(first) = img.files.first().map(|f| f.group) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for f in img.group(first) {
        if f.blocks == 0 || f.blocks > 6000 {
            continue;
        }
        let Ok(data) = img.read(&mut disc, f) else {
            continue;
        };
        let hits = tantalus_psxaudio::akao::find_akao(&data);
        for (i, &at) in hits.iter().enumerate() {
            let end = hits
                .get(i + 1)
                .copied()
                .unwrap_or(data.len())
                .min(at + 4096);
            if let Some(e) = tantalus_psxaudio::sfx::parse_program(&data[at..end], banks)
                && e.bank_hint.is_some()
            {
                out.push((n, e));
            }
        }
    }
    Ok(out)
}

/// Where a program's samples come from.
#[derive(Clone, Copy)]
enum SoundSource {
    /// A spell or ability effect: the common effect collection plus the effect's own.
    Effect(i32),
    /// A field or resident program: the resident collection (id 0) plus the one it names.
    Field(u16),
    /// A battle resident program (same collections as a field program).
    Resident(u16),
}

/// The Steam sound family of a resource (`Sounds02/SE03/se030012` -> `Sounds02/SE03`).
fn family(resource: &str) -> &str {
    resource.rsplit_once('/').map_or(resource, |(f, _)| f)
}

fn render_sfx(a: &Args) -> Result<(), String> {
    let mut stats = String::new();
    let mut steam = SoundBank::open(
        &a.steam.join("StreamingAssets"),
        &a.steam.join("x64/FF9_Data"),
    )?;
    let (sounds, skipped, resident_list) = effect_sounds(a, &mut stats)?;
    let by_file = read_sfx_banks_by_file(&a.disc_dir)?;
    let by_id = read_sfx_banks(&a.disc_dir)?;
    let bank_ids: std::collections::BTreeSet<u16> = by_file
        .values()
        .map(|b| b.id)
        .chain(by_id.keys().copied())
        .collect();
    let common = by_file
        .get(&COMMON_EFFECT_BANK)
        .ok_or("the discs have no common effect sound collection")?;
    let resident = by_id
        .get(&RESIDENT_BANK)
        .ok_or("the discs have no resident sound collection")?;
    // The articulations a program selects must exist in the collections it had loaded.
    let loaded = |source: SoundSource| -> Option<(SampleBank, bool)> {
        match source {
            SoundSource::Effect(effect) => Some(
                match u32::try_from(effect).ok().and_then(|e| by_file.get(&e)) {
                    Some(own) if effect as u32 != COMMON_EFFECT_BANK => common.merged(own),
                    _ => (common.clone(), false),
                },
            ),
            SoundSource::Field(RESIDENT_BANK) | SoundSource::Resident(RESIDENT_BANK) => {
                Some((resident.clone(), false))
            }
            SoundSource::Field(id) | SoundSource::Resident(id) => {
                by_id.get(&id).map(|own| resident.merged(own))
            }
        }
    };
    let has_arts = |entry: &SfxEntry, bank: &SampleBank| {
        let arts: Vec<Option<u32>> = (0..entry.scripts.len())
            .map(|s| entry.first_articulation(s))
            .collect();
        arts.iter().any(Option::is_some)
            && arts.iter().flatten().all(|art| bank.art(*art).is_some())
    };
    // Resource -> program, checking that every effect naming the same resource agrees.
    let mut chosen: BTreeMap<String, (SfxEntry, SoundSource, u8, u16)> = BTreeMap::new();
    let (mut agree, mut differ) = (0, 0);
    let (mut not_akao, mut no_art) = (0, 0);
    for s in &sounds {
        let Some(entry) = tantalus_psxaudio::sfx::parse_sound_file(&s.bytes, &bank_ids) else {
            not_akao += 1;
            continue;
        };
        let source = SoundSource::Effect(s.effect);
        if !loaded(source).is_some_and(|(bank, _)| has_arts(&entry, &bank)) {
            no_art += 1;
            continue;
        }
        match chosen.get(&s.resource) {
            Some((other, ..)) => {
                if other.data == entry.data {
                    agree += 1;
                } else {
                    differ += 1;
                }
            }
            None => {
                let tag = u16::try_from(s.effect * 16 + s.rank as i32).unwrap_or(u16::MAX);
                chosen.insert(s.resource.clone(), (entry, source, s.disc, tag));
            }
        }
    }
    stats.push_str(&format!(
        "effect sound files {}, resources {}, resource repeated with identical program {agree}, with a variant program {differ}, not a program {not_akao}, an articulation missing {no_art}, resources of effects left out {}\n",
        sounds.len(),
        chosen.len(),
        skipped
    ));
    // Field and resident programs: the sound id is the Steam sound id.
    let programs = read_sound_programs(&a.disc_dir)?;
    let (mut f_used, mut f_no_steam, mut f_effect, mut f_bad, mut f_no_art) = (0, 0, 0, 0, 0);
    for (id, (disc, bytes)) in &programs {
        let Some(resource) = steam.metadata.sound_effect.get(&i32::from(*id)).cloned() else {
            f_no_steam += 1;
            continue;
        };
        if chosen.contains_key(&resource) {
            f_effect += 1;
            continue;
        }
        let Some(entry) = tantalus_psxaudio::sfx::parse_program(bytes, &bank_ids) else {
            f_bad += 1;
            continue;
        };
        let source = SoundSource::Field(entry.bank_hint.unwrap_or(RESIDENT_BANK));
        if !loaded(source).is_some_and(|(bank, _)| has_arts(&entry, &bank)) {
            f_no_art += 1;
            continue;
        }
        f_used += 1;
        chosen.insert(resource, (entry, source, *disc, *id));
    }
    stats.push_str(&format!(
        "field and resident programs {}, used {f_used}, no Steam sound id {f_no_steam}, Steam resource already an effect sound {f_effect}, not a program {f_bad}, an articulation missing {f_no_art}\n",
        programs.len()
    ));
    // The battle's resident sounds (`ResidentSpecialEffectMetaData.txt`, twelve resources) pair
    // with the twelve programs of the battle overlay in order; used only when the counts agree.
    let residents = read_resident_programs(&a.disc_dir, &bank_ids)?;
    let (mut r_used, mut r_same, mut r_other) = (0, 0, 0);
    if residents.len() == resident_list.len() {
        for (i, (resource, (disc, entry))) in resident_list.iter().zip(residents.iter()).enumerate()
        {
            // An effect list naming the same resource is a check on the pairing.
            if let Some((other, ..)) = chosen.get(resource) {
                if other.scripts.len() == entry.scripts.len()
                    && other.scripts.iter().zip(&entry.scripts).all(|(x, y)| {
                        other.data.get(*x..).map(|d| &d[..d.len().min(64)])
                            == entry.data.get(*y..).map(|d| &d[..d.len().min(64)])
                    })
                {
                    r_same += 1;
                } else {
                    r_other += 1;
                }
                continue;
            }
            let source = SoundSource::Resident(entry.bank_hint.unwrap_or(RESIDENT_BANK));
            if !loaded(source).is_some_and(|(bank, _)| has_arts(entry, &bank)) {
                continue;
            }
            chosen.insert(resource.clone(), (entry.clone(), source, *disc, i as u16));
            r_used += 1;
        }
    }
    stats.push_str(&format!(
        "battle resident sounds {}, overlay programs {}, used {r_used}, already an effect sound with the same voices {r_same}, with other voices {r_other}\n",
        resident_list.len(),
        residents.len()
    ));
    print!("{stats}");
    let by_name: BTreeMap<String, i32> = steam
        .metadata
        .sound_effect
        .iter()
        .map(|(id, r)| (r.clone(), *id))
        .collect();
    let mut total: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for r in steam.metadata.sound_effect.values() {
        total.entry(family(r).to_owned()).or_default().0 += 1;
    }
    if a.command == "sfx-coverage" {
        for r in chosen.keys() {
            if let Some(t) = total.get_mut(family(r)) {
                t.1 += 1;
            }
        }
        print_coverage(&total, "programs mapped");
        return Ok(());
    }
    let dir = a.out.join("sfx");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut manifest = Manifest {
        version: 1,
        entries: Vec::new(),
    };
    let (mut silent, mut capped, mut overlapping, mut steam_loops) = (0, 0, 0, 0);
    let wanted = |resource: &str| {
        a.families.as_ref().is_none_or(|f| {
            f.iter()
                .any(|f| family(resource).rsplit('/').next() == Some(f.as_str()))
        })
    };
    for (resource, (entry, source, disc, tag)) in &chosen {
        if !wanted(resource) {
            continue;
        }
        let Some((bank, overlap)) = loaded(*source) else {
            continue;
        };
        overlapping += usize::from(overlap);
        let mut rendered = render_looping(
            &entry.sequence_all(),
            &bank,
            SFX_CAP_SECONDS,
            SFX_LOOPED_CAP_SECONDS,
        );
        if rendered.peak < 1.0e-4 {
            silent += 1;
            continue;
        }
        let clip = if rendered.loop_frames.is_none() {
            let Ok(clip) = steam.load_clip(resource) else {
                continue;
            };
            // The game loops this sound until a script stops it; a one-shot render would end
            // early, so the Steam clip stays.
            if clip.loop_end > 0 {
                steam_loops += 1;
                continue;
            }
            Some(clip)
        } else {
            None
        };
        // A program still playing at the cap with no loop of its own (a held note the game
        // stops by script) is cut at the Steam clip's length, with a short fade.
        if let Some(clip) = clip
            && rendered.seconds() >= SFX_CAP_SECONDS as f64 - 0.01
        {
            let frames =
                (clip.frames() as u64 * 44_100 / u64::from(clip.sample_rate.max(1))) as usize;
            cut_with_fade(&mut rendered.samples, frames.max(1));
            capped += 1;
        }
        let name = format!("sfx/{}.ogg", basename(resource));
        encode_ogg(&rendered.samples, &a.out.join(&name))?;
        let kind = match source {
            SoundSource::Effect(_) => "sfx",
            SoundSource::Field(_) => "sfx-field",
            SoundSource::Resident(_) => "sfx-resident",
        };
        manifest.entries.push(ManifestEntry {
            kind: kind.to_owned(),
            resource: resource.clone(),
            sound_id: by_name.get(resource).copied().unwrap_or(-1),
            file: name,
            sample_rate: 44_100,
            frames: (rendered.samples.len() / 2) as u32,
            loop_frames: rendered.loop_frames,
            peak: rendered.peak,
            disc: *disc,
            ps1_sequence_id: *tag,
        });
    }
    println!(
        "silent renders left out {silent}, one-shot renders of a looping Steam clip left out {steam_loops}, renders cut at the Steam clip length {capped}, collections overlapping in SPU memory {overlapping}"
    );
    let path = a.out.join(MANIFEST);
    if let Ok(text) = std::fs::read_to_string(&path)
        && let Ok(mut old) = serde_json::from_str::<Manifest>(&text)
    {
        old.entries
            .retain(|e| !e.kind.starts_with("sfx") || !wanted(&e.resource));
        old.entries.extend(manifest.entries);
        manifest = old;
    }
    manifest.entries.sort_by(|x, y| {
        (&x.kind, x.sound_id, &x.resource).cmp(&(&y.kind, y.sound_id, &y.resource))
    });
    let text = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    for e in manifest
        .entries
        .iter()
        .filter(|e| e.kind.starts_with("sfx"))
    {
        if let Some(t) = total.get_mut(family(&e.resource)) {
            t.1 += 1;
        }
    }
    print_coverage(&total, "rendered (whole manifest)");
    println!(
        "wrote {} clips to {}",
        manifest.entries.len(),
        a.out.display()
    );
    Ok(())
}

/// Per Steam sound family: `mapped/total`, and the sum.
fn print_coverage(total: &BTreeMap<String, (usize, usize)>, what: &str) {
    let (mut all, mut hit) = (0, 0);
    for (family, (n, h)) in total {
        println!("coverage {family}: {h}/{n} {what}");
        all += n;
        hit += h;
    }
    println!("coverage all: {hit}/{all} {what}");
}

/// Truncates interleaved stereo to `frames` with a 10 ms fade to silence at the end.
fn cut_with_fade(samples: &mut Vec<f32>, frames: usize) {
    samples.truncate(frames * 2);
    let n = samples.len() / 2;
    let fade = 441.min(n);
    for i in 0..fade {
        let g = i as f32 / fade as f32;
        let f = n - 1 - i;
        samples[f * 2] *= g;
        samples[f * 2 + 1] *= g;
    }
}

/// The `SfxBanks` file id of the collection every effect's sound programs draw on besides the
/// effect's own (file id = effect id): the programs select articulations 128 and up, which this
/// collection holds (all 1433 programs checked have their first articulation in it or in their
/// effect's own collection).
const COMMON_EFFECT_BANK: u32 = 132;

/// The collection id of the resident sounds every field and menu program draws on besides the
/// one it names (articulations 128 to 255; 209 of the 1060 programs select articulations only it
/// holds).
const RESIDENT_BANK: u16 = 0;

/// Longest a program whose voices all loop may run to close its loop, in seconds.
const SFX_LOOPED_CAP_SECONDS: u64 = 150;

/// Longest effect sound rendered, in seconds.
const SFX_CAP_SECONDS: u64 = 10;

/// `sfx-compare`: each rendered sound against the Steam clip it replaces, as tab-separated rows
/// (resource, kind, PS1 and Steam length in seconds, both sounding lengths, both peaks, and the
/// correlation of their band spectra), then a summary per kind against a control pairing (each render with the Steam
/// clip of the next resource in order): an objective check that the mapping pairs like sounds.
fn sfx_compare(a: &Args) -> Result<(), String> {
    let mut steam = SoundBank::open(
        &a.steam.join("StreamingAssets"),
        &a.steam.join("x64/FF9_Data"),
    )?;
    let clips = PsxClips::open(&a.out).ok_or("no psx-audio.json in the output folder")?;
    // kind -> rows of (PS1 seconds, Steam seconds, PS1 profile, Steam profile)
    type Row = (f64, f64, Vec<f32>, Vec<f32>);
    let mut by_kind: BTreeMap<String, Vec<Row>> = BTreeMap::new();
    for (resource, entry) in &clips.by_resource {
        if !entry.kind.starts_with("sfx") {
            continue;
        }
        let (Ok(clip), Ok(Some(ps1))) = (steam.load_clip(resource), clips.load(resource)) else {
            continue;
        };
        let (s_mono, p_mono) = (mono(&clip), mono(&ps1));
        let (sp, pp) = (
            band_profile(&s_mono, clip.sample_rate),
            band_profile(&p_mono, ps1.sample_rate),
        );
        let peak = |c: &PcmClip| {
            c.samples
                .iter()
                .map(|s| i32::from(*s).abs())
                .max()
                .unwrap_or(0) as f64
                / 32768.0
        };
        let (p_len, s_len) = (
            ps1.frames() as f64 / f64::from(ps1.sample_rate),
            clip.frames() as f64 / f64::from(clip.sample_rate),
        );
        // Lengths compare until the sound falls 40 dB under its own peak: the Steam clips carry
        // tails of near silence that the renders trim.
        let (p_on, s_on) = (
            sounding_seconds(&p_mono, ps1.sample_rate, 0.01),
            sounding_seconds(&s_mono, clip.sample_rate, 0.01),
        );
        // And until it falls 12 dB under: the body of the sound without its tail.
        let (p_body, s_body) = (
            sounding_seconds(&p_mono, ps1.sample_rate, 0.25),
            sounding_seconds(&s_mono, clip.sample_rate, 0.25),
        );
        println!(
            "{resource}\t{}\t{p_len:.3}\t{s_len:.3}\t{p_on:.3}\t{s_on:.3}\t{p_body:.3}\t{s_body:.3}\t{:.3}\t{:.3}\t{:.3}",
            entry.kind,
            peak(&ps1),
            peak(&clip),
            pearson(&pp, &sp)
        );
        by_kind
            .entry(entry.kind.clone())
            .or_default()
            .push((p_on, s_on, pp, sp));
    }
    let median = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v.get(v.len() / 2).copied().unwrap_or(f64::NAN)
    };
    let ratio = |p: f64, s: f64| (p.max(0.01) / s.max(0.01)).log2().abs();
    for (kind, rows) in &by_kind {
        let n = rows.len();
        let other = |i: usize| &rows[(i + 1) % n];
        let len_true: Vec<f64> = rows.iter().map(|r| ratio(r.0, r.1)).collect();
        let len_ctrl: Vec<f64> = (0..n).map(|i| ratio(rows[i].0, other(i).1)).collect();
        let cor_true: Vec<f64> = rows.iter().map(|r| pearson(&r.2, &r.3)).collect();
        let cor_ctrl: Vec<f64> = (0..n).map(|i| pearson(&rows[i].2, &other(i).3)).collect();
        let closer = (0..n).filter(|&i| len_true[i] < len_ctrl[i]).count();
        let likelier = (0..n).filter(|&i| cor_true[i] > cor_ctrl[i]).count();
        let within = len_true.iter().filter(|r| **r < 0.5).count();
        println!(
            "summary {kind}: {n} clips; sounding length within 1.41x of Steam {within}; median |log2 sounding length ratio| {:.2} (control {:.2}), closer than control {closer}; median band-spectrum correlation {:.2} (control {:.2}), higher than control {likelier}",
            median(len_true.clone()),
            median(len_ctrl),
            median(cor_true),
            median(cor_ctrl)
        );
    }
    Ok(())
}

/// Seconds until the last sample at or above `fraction` of the clip's peak.
fn sounding_seconds(x: &[f32], rate: u32, fraction: f32) -> f64 {
    let peak = x.iter().fold(0f32, |m, v| m.max(v.abs()));
    let last = x
        .iter()
        .rposition(|v| v.abs() >= peak * fraction)
        .unwrap_or(0);
    (last + 1) as f64 / f64::from(rate.max(1))
}

/// The clip's channels averaged, as floats.
fn mono(clip: &PcmClip) -> Vec<f32> {
    let ch = usize::from(clip.channels.max(1));
    clip.samples
        .chunks(ch)
        .map(|f| f.iter().map(|s| f32::from(*s)).sum::<f32>() / (32768.0 * ch as f32))
        .collect()
}

/// Number of log-spaced bands in [`band_profile`], 100 Hz to 10 kHz.
const BANDS: usize = 20;

/// Average power per log-spaced band (dB) over the first 4 seconds, from 1024-point frames.
fn band_profile(x: &[f32], rate: u32) -> Vec<f32> {
    const N: usize = 1024;
    let mut power = [0f64; BANDS];
    let limit = x.len().min(rate as usize * 4);
    let edges: Vec<f64> = (0..=BANDS)
        .map(|b| 100.0 * 100f64.powf(b as f64 / BANDS as f64))
        .collect();
    let mut frame = 0;
    while frame + N <= limit.max(N) {
        let mut re: Vec<f64> = (0..N)
            .map(|i| {
                let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / N as f64).cos();
                f64::from(x.get(frame + i).copied().unwrap_or(0.0)) * w
            })
            .collect();
        let mut im = vec![0f64; N];
        fft(&mut re, &mut im);
        for k in 1..N / 2 {
            let hz = k as f64 * f64::from(rate) / N as f64;
            if let Some(b) = edges.windows(2).position(|e| hz >= e[0] && hz < e[1]) {
                power[b] += re[k] * re[k] + im[k] * im[k];
            }
        }
        frame += N;
    }
    power
        .iter()
        .map(|p| (10.0 * (p + 1e-9).log10()) as f32)
        .collect()
}

/// In-place radix-2 FFT (`re.len()` a power of two).
fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -std::f64::consts::TAU / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (wr, wi) = ((ang * k as f64).cos(), (ang * k as f64).sin());
                let (a, b) = (start + k, start + k + len / 2);
                let (xr, xi) = (re[b] * wr - im[b] * wi, re[b] * wi + im[b] * wr);
                re[b] = re[a] - xr;
                im[b] = im[a] - xi;
                re[a] += xr;
                im[a] += xi;
            }
        }
        len <<= 1;
    }
}

/// Pearson correlation of two equal-length profiles.
fn pearson(a: &[f32], b: &[f32]) -> f64 {
    let n = a.len().min(b.len()) as f64;
    let (ma, mb) = (
        a.iter().map(|v| f64::from(*v)).sum::<f64>() / n,
        b.iter().map(|v| f64::from(*v)).sum::<f64>() / n,
    );
    let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (f64::from(*x) - ma, f64::from(*y) - mb);
        sab += x * y;
        saa += x * x;
        sbb += y * y;
    }
    if saa == 0.0 || sbb == 0.0 {
        return 0.0;
    }
    sab / (saa * sbb).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_profile_peaks_at_the_tone_and_matches_itself() {
        let rate = 22_050;
        let tone: Vec<f32> = (0..rate * 2)
            .map(|i| (std::f32::consts::TAU * 1500.0 * i as f32 / rate as f32).sin() * 0.5)
            .collect();
        let p = band_profile(&tone, rate as u32);
        let top = (0..BANDS).max_by(|x, y| p[*x].total_cmp(&p[*y])).unwrap();
        // 1.5 kHz is in band 11 of 20 (100 Hz * 100^(11/20) = 1.26 kHz to 1.58 kHz).
        assert_eq!(top, 11);
        assert!((pearson(&p, &p) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_cut_ends_at_the_length_with_a_fade() {
        let mut s = vec![1.0f32; 2000 * 2];
        cut_with_fade(&mut s, 1000);
        assert_eq!(s.len(), 2000);
        assert_eq!(s[1999], 0.0);
        assert_eq!(s[0], 1.0);
    }
}
