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

use std::collections::BTreeMap;
use std::io::Write;
use std::num::{NonZeroU8, NonZeroU32};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tantalus_legacy::sound::PcmClip;
use tantalus_psxaudio::akao::{SampleBank, Song, read_songs};
use tantalus_psxaudio::disc::{Disc, find_discs};
use tantalus_psxaudio::img::Img;
use tantalus_psxaudio::render::{Rendered, render_capped, render_song};
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
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut a = Args {
        command: "render".to_owned(),
        disc_dir: PathBuf::new(),
        out: default_dir(),
        steam: PathBuf::from("E:/SteamLibrary/steamapps/common/FINAL FANTASY IX"),
        ids: None,
        dump: None,
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().cloned().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--disc-dir" => a.disc_dir = PathBuf::from(value("--disc-dir")?),
            "--out" => a.out = PathBuf::from(value("--out")?),
            "--dump" => a.dump = Some(PathBuf::from(value("--dump")?)),
            "--steam" => a.steam = PathBuf::from(value("--steam")?),
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
fn effect_sounds(a: &Args, stats: &mut String) -> Result<(Vec<EffectSound>, usize), String> {
    use tantalus_assets::pc::special_effect::{EffectFile, RecordDetail};
    let (lists, _resident) =
        crate::battle_sound::effect_sound_lists(&a.steam.join("x64/FF9_Data"))?;
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
    Ok((out, skipped))
}

fn render_sfx(a: &Args) -> Result<(), String> {
    let mut stats = String::new();
    let (sounds, skipped) = effect_sounds(a, &mut stats)?;
    let by_file = read_sfx_banks_by_file(&a.disc_dir)?;
    let bank_ids: std::collections::BTreeSet<u16> = by_file.values().map(|b| b.id).collect();
    let common = by_file
        .get(&COMMON_EFFECT_BANK)
        .ok_or("the discs have no common effect sound collection")?;
    // The articulations a program selects must exist in the collections its effect had loaded.
    let loaded = |effect: i32| -> (SampleBank, bool) {
        match u32::try_from(effect).ok().and_then(|e| by_file.get(&e)) {
            Some(own) if effect as u32 != COMMON_EFFECT_BANK => common.merged(own),
            _ => (common.clone(), false),
        }
    };
    // Resource -> program, checking that every effect naming the same resource agrees.
    let mut chosen: BTreeMap<String, (SfxEntry, &EffectSound)> = BTreeMap::new();
    let (mut agree, mut differ) = (0, 0);
    let (mut not_akao, mut no_art) = (0, 0);
    for s in &sounds {
        let Some(entry) = tantalus_psxaudio::sfx::parse_sound_file(&s.bytes, &bank_ids) else {
            not_akao += 1;
            continue;
        };
        let (bank, _) = loaded(s.effect);
        if entry
            .first_articulation(0)
            .is_none_or(|art| bank.art(art).is_none())
        {
            no_art += 1;
            continue;
        }
        match chosen.get(&s.resource) {
            Some((other, _)) => {
                if other.data == entry.data {
                    agree += 1;
                } else {
                    differ += 1;
                }
            }
            None => {
                chosen.insert(s.resource.clone(), (entry, s));
            }
        }
    }
    stats.push_str(&format!(
        "sound files {}, resources {}, resource repeated with identical program {agree}, with a variant program {differ}, not a program {not_akao}, first articulation missing {no_art}, resources of effects left out {}\n",
        sounds.len(),
        chosen.len(),
        skipped
    ));
    print!("{stats}");
    if a.command == "sfx-coverage" {
        return Ok(());
    }
    let dir = a.out.join("sfx");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let steam = SoundBank::open(
        &a.steam.join("StreamingAssets"),
        &a.steam.join("x64/FF9_Data"),
    )?;
    let by_name: BTreeMap<&str, i32> = steam
        .metadata
        .sound_effect
        .iter()
        .map(|(id, r)| (r.as_str(), *id))
        .collect();
    let mut manifest = Manifest {
        version: 1,
        entries: Vec::new(),
    };
    let (mut silent, mut runaway, mut overlapping) = (0, 0, 0);
    for (resource, (entry, s)) in &chosen {
        let (bank, overlap) = loaded(s.effect);
        overlapping += usize::from(overlap);
        let rendered = render_capped(&entry.sequence_all(), &bank, SFX_CAP_SECONDS);
        if rendered.peak < 1.0e-4 {
            silent += 1;
            continue;
        }
        // A program that is still playing at the length cap and has no loop never ended.
        if rendered.loop_frames.is_none() && rendered.seconds() >= SFX_CAP_SECONDS as f64 - 0.01 {
            runaway += 1;
            continue;
        }
        let name = format!("sfx/{}.ogg", basename(resource));
        encode_ogg(&rendered.samples, &a.out.join(&name))?;
        manifest.entries.push(ManifestEntry {
            kind: "sfx".to_owned(),
            resource: resource.clone(),
            sound_id: by_name.get(resource.as_str()).copied().unwrap_or(-1),
            file: name,
            sample_rate: 44_100,
            frames: rendered.frames() as u32,
            loop_frames: rendered.loop_frames,
            peak: rendered.peak,
            disc: s.disc,
            ps1_sequence_id: u16::try_from(s.effect * 16 + s.rank as i32).unwrap_or(u16::MAX),
        });
    }
    println!(
        "silent renders left out {silent}, never-ending renders left out {runaway}, collections overlapping in SPU memory {overlapping}"
    );
    let path = a.out.join(MANIFEST);
    if let Ok(text) = std::fs::read_to_string(&path)
        && let Ok(mut old) = serde_json::from_str::<Manifest>(&text)
    {
        old.entries.retain(|e| e.kind != "sfx");
        old.entries.extend(manifest.entries);
        manifest = old;
    }
    manifest.entries.sort_by(|x, y| {
        (&x.kind, x.sound_id, &x.resource).cmp(&(&y.kind, y.sound_id, &y.resource))
    });
    let text = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    println!(
        "wrote {} clips to {}",
        manifest.entries.len(),
        a.out.display()
    );
    Ok(())
}

/// The `SfxBanks` file id of the collection every effect's sound programs draw on besides the
/// effect's own (file id = effect id): the programs select articulations 128 and up, which this
/// collection holds (all 1433 programs checked have their first articulation in it or in their
/// effect's own collection).
const COMMON_EFFECT_BANK: u32 = 132;

/// Longest effect sound rendered, in seconds.
const SFX_CAP_SECONDS: u64 = 10;

/// `sfx-compare`: the length and peak of each rendered effect against the Steam clip it replaces,
/// as tab-separated rows (an objective check that the mapping pairs like sounds).
fn sfx_compare(a: &Args) -> Result<(), String> {
    let mut steam = SoundBank::open(
        &a.steam.join("StreamingAssets"),
        &a.steam.join("x64/FF9_Data"),
    )?;
    let clips = PsxClips::open(&a.out).ok_or("no psx-audio.json in the output folder")?;
    for (resource, entry) in &clips.by_resource {
        if entry.kind != "sfx" {
            continue;
        }
        let Ok(clip) = steam.load_clip(resource) else {
            continue;
        };
        let peak = clip
            .samples
            .iter()
            .map(|s| i32::from(*s).abs())
            .max()
            .unwrap_or(0);
        println!(
            "{resource}\t{:.3}\t{:.3}\t{:.3}\t{:.3}",
            f64::from(entry.frames) / 44_100.0,
            clip.frames() as f64 / f64::from(clip.sample_rate),
            entry.peak,
            f64::from(peak) / 32768.0
        );
    }
    Ok(())
}
