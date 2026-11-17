//! `tantalus-tools psx-audio`: read the owner's FF9 discs and render PS1 music to Ogg Vorbis, plus
//! the loader the client uses to prefer those files over the Steam ones.
//!
//! Nothing here ships audio. The renderer reads the user's own `.bin` images and writes under the
//! user's data folder (`%APPDATA%/tantalus/psx-audio`, or `TANTALUS_PSX_AUDIO_DIR`).
//!
//! The mapping to the Steam game is by sound id: the music file ids in `FF9.IMG` are the music
//! ids the game scripts use, and `MusicMetaData.txt` maps each to its `Sounds01/BGM_/musicNNN`
//! resource. A rendered track replaces the Steam resource of the same music id.

use std::collections::BTreeMap;
use std::io::Write;
use std::num::{NonZeroU8, NonZeroU32};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tantalus_legacy::sound::PcmClip;
use tantalus_psxaudio::akao::{SampleBank, Song, read_songs};
use tantalus_psxaudio::disc::{Disc, find_discs};
use tantalus_psxaudio::img::Img;
use tantalus_psxaudio::render::{Rendered, render_song};

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

struct Args {
    command: String,
    disc_dir: PathBuf,
    out: PathBuf,
    steam: PathBuf,
    ids: Option<Vec<i32>>,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut a = Args {
        command: "render".to_owned(),
        disc_dir: PathBuf::new(),
        out: default_dir(),
        steam: PathBuf::from("E:/SteamLibrary/steamapps/common/FINAL FANTASY IX"),
        ids: None,
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().cloned().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--disc-dir" => a.disc_dir = PathBuf::from(value("--disc-dir")?),
            "--out" => a.out = PathBuf::from(value("--out")?),
            "--steam" => a.steam = PathBuf::from(value("--steam")?),
            "--ids" => {
                a.ids = Some(
                    value("--ids")?
                        .split(',')
                        .map(|s| s.trim().parse().map_err(|_| format!("bad id `{s}`")))
                        .collect::<Result<_, _>>()?,
                )
            }
            "render" | "compare" | "coverage" | "verify" => a.command = arg.clone(),
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    if a.disc_dir.as_os_str().is_empty() {
        return Err("--disc-dir <folder with the four .bin images> is required".to_owned());
    }
    Ok(a)
}

pub fn run(args: &[String]) -> Result<(), String> {
    let a = parse(args)?;
    if a.command == "verify" {
        return verify(&a.out);
    }
    let steam = SoundBank::open(
        &a.steam.join("StreamingAssets"),
        &a.steam.join("x64/FF9_Data"),
    )?;
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
    // A partial run (--ids) adds to an existing manifest instead of replacing it.
    let path = a.out.join(MANIFEST);
    if a.ids.is_some()
        && let Ok(text) = std::fs::read_to_string(&path)
        && let Ok(mut old) = serde_json::from_str::<Manifest>(&text)
    {
        old.entries
            .retain(|e| !manifest.entries.iter().any(|n| n.resource == e.resource));
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
