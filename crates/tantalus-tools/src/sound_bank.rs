//! The game's sound assets: the id to resource tables and the decoded clips.
//!
//! `SoundMetaData` (`Global/Sound/Meta/SoundMetaData.cs:100`) reads its id to resource-name
//! tables from `EmbeddedAsset/Manifest/Sounds/*.txt`, which are TextAssets in `resources.assets`.
//! `SoundLoaderResources` then asks for `Sounds/<ResourceID>.akb`; the PC build keeps those as
//! `.akb.bytes` TextAssets in the `p0data61`, `p0data62` and `p0data63` bundles.
//! `FF9SndMetaData` (`Global/ff9/Snd/FF9SndMetaData.cs:14`) reads the battle song tables.

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tantalus_assets::pc::asset_bundle::AssetBundleMetadata;
use tantalus_assets::pc::audio_decode::{container_path_for_resource, decode_akb};
use tantalus_assets::pc::serialized::SerializedFile;
use tantalus_assets::pc::text_asset::TextAsset;
use tantalus_assets::pc::unity_raw::UnityRawBundle;
use tantalus_legacy::sound::{MetaEntry, PcmClip, SoundMetadata};

/// The bundles that hold sounds (`audit/audio-inventory.json`; the `*1` ones are the same
/// content for the other build flavor).
pub const SOUND_BUNDLES: [&str; 3] = ["p0data61.bin", "p0data62.bin", "p0data63.bin"];

/// Battle song tables: field map -> battle id -> song id.
pub type BattleSongTable = BTreeMap<i32, BTreeMap<i32, i32>>;

/// Where one `.akb.bytes` asset sits in its bundle file.
#[derive(Debug, Clone)]
struct AssetSlot {
    file: PathBuf,
    offset: u64,
    size: usize,
}

/// Sound metadata plus a lazily built index of the sound bundles.
pub struct SoundBank {
    pub metadata: SoundMetadata,
    /// `FF9SndMetaData.BtlBgmMapperForFieldMap`.
    pub field_battle_songs: BattleSongTable,
    /// `FF9SndMetaData.BtlBgmMapperForWorldMap`.
    pub world_battle_songs: BattleSongTable,
    streaming_assets: PathBuf,
    index: Option<BTreeMap<String, AssetSlot>>,
}

/// `{ "data": [ { "name", "soundIndex" } ] }` as `SoundMetaData.IndexingSoundData` reads it.
pub fn parse_entries(text: &[u8]) -> Result<Vec<MetaEntry>, String> {
    let text = text.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(text);
    let value: serde_json::Value =
        serde_json::from_slice(text).map_err(|e| format!("metadata JSON: {e}"))?;
    let data = value
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or("metadata has no `data` array")?;
    let mut out = Vec::with_capacity(data.len());
    for entry in data {
        let name = entry
            .get("name")
            .and_then(|n| n.as_str())
            .ok_or("metadata entry without `name`")?;
        let index = match entry.get("soundIndex") {
            Some(serde_json::Value::String(s)) => s.parse::<i32>().map_err(|e| e.to_string())?,
            Some(serde_json::Value::Number(n)) => {
                i32::try_from(n.as_i64().ok_or("soundIndex out of range")?)
                    .map_err(|e| e.to_string())?
            }
            _ => return Err(format!("metadata entry `{name}` has no soundIndex")),
        };
        out.push((index, name.to_owned()));
    }
    Ok(out)
}

/// `FF9SndMetaData.CreateBattleEncountBgmMetaData` (`FF9SndMetaData.cs:27`): `{ "<map>": { "<battle>": song } }`.
pub fn parse_battle_songs(text: &[u8]) -> Result<BattleSongTable, String> {
    let text = text.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(text);
    let value: serde_json::Value =
        serde_json::from_slice(text).map_err(|e| format!("battle song JSON: {e}"))?;
    let object = value
        .as_object()
        .ok_or("battle song table is not an object")?;
    let mut out = BattleSongTable::new();
    for (origin, battles) in object {
        let origin: i32 = origin
            .parse()
            .map_err(|_| format!("bad map id `{origin}`"))?;
        let battles = battles
            .as_object()
            .ok_or("battle song row is not an object")?;
        let row = out.entry(origin).or_default();
        for (battle, song) in battles {
            let battle: i32 = battle
                .parse()
                .map_err(|_| format!("bad battle id `{battle}`"))?;
            let song = song
                .as_i64()
                .or_else(|| song.as_str().and_then(|s| s.parse().ok()))
                .ok_or("battle song id is not a number")?;
            row.insert(battle, song as i32);
        }
    }
    Ok(out)
}

/// `FF9SndMetaData.GetMusicForBattle` (`FF9SndMetaData.cs:18`); -1 when the table has none.
pub fn music_for_battle(table: &BattleSongTable, origin_map: i32, battle: i32) -> i32 {
    table
        .get(&origin_map)
        .and_then(|row| row.get(&battle))
        .copied()
        .unwrap_or(-1)
}

/// Reads the named TextAssets of one serialized file in a single pass.
fn read_text_assets(path: &Path, names: &[&str]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let source = path.to_string_lossy();
    let serialized = SerializedFile::parse(&bytes, &source).map_err(|e| e.to_string())?;
    let mut found = BTreeMap::new();
    for (index, object) in serialized.report().objects.iter().enumerate() {
        if object.class_id != 49 {
            continue;
        }
        let data = serialized.object_data(index).map_err(|e| e.to_string())?;
        // The name comes first; skip assets whose name is not wanted without copying the body.
        let Ok(asset) = TextAsset::parse(data, &source) else {
            continue;
        };
        for name in names {
            if asset.name == *name || asset.name.ends_with(&format!("/{name}")) {
                found.insert((*name).to_owned(), asset.script.clone());
            }
        }
        if found.len() == names.len() {
            break;
        }
    }
    Ok(found)
}

impl SoundBank {
    /// Reads the metadata tables from `<game_data>/resources.assets`. Clips load on demand.
    pub fn open(streaming_assets: &Path, game_data: &Path) -> Result<Self, String> {
        const NAMES: [&str; 6] = [
            "MusicMetaData.txt",
            "SoundEffectMetaData.txt",
            "SoundEffectExtendedMetaData.txt",
            "SongMetaData.txt",
            "BtlEncountBgmMetaData.txt",
            "WldBtlEncountBgmMetaData.txt",
        ];
        let texts = read_text_assets(&game_data.join("resources.assets"), &NAMES)?;
        let need = |name: &str| {
            texts
                .get(name)
                .ok_or_else(|| format!("resources.assets has no {name}"))
        };
        let mut metadata = SoundMetadata {
            music: SoundMetadata::index(&parse_entries(need("MusicMetaData.txt")?)?)?,
            ..SoundMetadata::default()
        };
        metadata.set_sound_effects(
            &parse_entries(need("SoundEffectMetaData.txt")?)?,
            &parse_entries(need("SoundEffectExtendedMetaData.txt")?)?,
        )?;
        if let Some(song) = texts.get("SongMetaData.txt") {
            metadata.song = SoundMetadata::index(&parse_entries(song)?)?;
        }
        Ok(Self {
            metadata,
            field_battle_songs: parse_battle_songs(need("BtlEncountBgmMetaData.txt")?)?,
            world_battle_songs: parse_battle_songs(need("WldBtlEncountBgmMetaData.txt")?)?,
            streaming_assets: streaming_assets.to_owned(),
            index: None,
        })
    }

    /// Container path -> file slot for every sound of the three bundles. Reads each bundle's
    /// directory once; the audio itself is read per asset afterwards.
    fn build_index(&self) -> Result<BTreeMap<String, AssetSlot>, String> {
        let mut index = BTreeMap::new();
        for name in SOUND_BUNDLES {
            let path = self.streaming_assets.join(name);
            if !path.is_file() {
                continue;
            }
            let bundle = UnityRawBundle::open(&path).map_err(|e| e.to_string())?;
            let base = bundle
                .report()
                .entries
                .first()
                .ok_or("bundle has no entries")?
                .absolute_offset;
            let bytes = bundle
                .read_entry(0, 600 * 1024 * 1024)
                .map_err(|e| e.to_string())?;
            let source = path.to_string_lossy();
            let serialized = SerializedFile::parse(&bytes, &source).map_err(|e| e.to_string())?;
            let objects = &serialized.report().objects;
            let by_id: BTreeMap<i64, usize> = objects
                .iter()
                .enumerate()
                .map(|(i, o)| (o.path_id, i))
                .collect();
            let (meta, _) = objects
                .iter()
                .enumerate()
                .find(|(_, o)| o.class_id == 142)
                .ok_or("AssetBundle metadata object is missing")?;
            let metadata = AssetBundleMetadata::parse(
                serialized.object_data(meta).map_err(|e| e.to_string())?,
                &source,
            )
            .map_err(|e| e.to_string())?;
            for (asset_path, info) in &metadata.container {
                let key = asset_path.to_ascii_lowercase();
                if !key.contains("/sounds/") {
                    continue;
                }
                let Some(&object) = by_id.get(&info.path_id) else {
                    continue;
                };
                let object = &objects[object];
                // A TextAsset is `name`, `script`, `path name`; the script is what the slot reads.
                index.entry(key).or_insert(AssetSlot {
                    file: path.clone(),
                    offset: base + object.absolute_offset,
                    size: object.size as usize,
                });
            }
        }
        Ok(index)
    }

    /// The raw `.akb.bytes` payload of `Sounds/<resource_id>.akb`.
    pub fn akb_bytes(&mut self, resource_id: &str) -> Result<Vec<u8>, String> {
        if self.index.is_none() {
            self.index = Some(self.build_index()?);
        }
        let key = container_path_for_resource(resource_id);
        let slot = self
            .index
            .as_ref()
            .and_then(|index| index.get(&key))
            .ok_or_else(|| format!("no sound asset `{key}`"))?;
        let mut file = std::fs::File::open(&slot.file)
            .map_err(|e| format!("cannot open {}: {e}", slot.file.display()))?;
        file.seek(SeekFrom::Start(slot.offset))
            .map_err(|e| e.to_string())?;
        let mut data = vec![0; slot.size];
        file.read_exact(&mut data).map_err(|e| e.to_string())?;
        let asset = TextAsset::parse(&data, &key).map_err(|e| e.to_string())?;
        Ok(asset.script)
    }

    /// Decodes one resource to PCM with its AKB2 loop fields kept as the file has them.
    pub fn load_clip(&mut self, resource_id: &str) -> Result<PcmClip, String> {
        let script = self.akb_bytes(resource_id)?;
        decode_clip(&script)
    }
}

/// Decodes an AKB asset: interleaved 16-bit samples cut at the stream's final granule, and the
/// header's loop fields (frames).
pub fn decode_clip(script: &[u8]) -> Result<PcmClip, String> {
    let mut samples: Vec<i16> = Vec::new();
    let mut sink = |block: &[i16]| samples.extend_from_slice(block);
    let decoded = decode_akb(script, Some(&mut sink))?;
    let channels = u16::from(decoded.decode.channels);
    let keep = usize::try_from(decoded.playable_frames)
        .unwrap_or(usize::MAX)
        .saturating_mul(usize::from(channels));
    samples.truncate(keep);
    let loops = &decoded.loop_info;
    Ok(PcmClip {
        sample_rate: decoded.decode.sample_rate,
        channels,
        samples,
        loop_start: loops.loop_start,
        loop_end: loops.loop_end,
        loop_start_alternate: loops.loop_start_alternate,
        loop_end_alternate: loops.loop_end_alternate,
    })
}

/// A clip loader for `SoundSystem::clip_loader` that remembers failures.
pub fn clip_loader(mut bank: SoundBank) -> tantalus_legacy::sound::ClipLoader {
    let mut failed = std::collections::BTreeSet::new();
    tantalus_legacy::sound::ClipLoader::new(move |resource| {
        if failed.contains(resource) {
            return None;
        }
        match bank.load_clip(resource) {
            Ok(clip) => Some(Arc::new(clip)),
            Err(error) => {
                eprintln!("audio: cannot load {resource}: {error}");
                failed.insert(resource.to_owned());
                None
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_metadata_rows() {
        let rows = parse_entries(
            b"\xEF\xBB\xBF{\"data\":[{\"name\":\"Sounds01/BGM_/music000\",\"soundIndex\":\"0\"},{\"name\":\"x\",\"soundIndex\":7}]}",
        )
        .unwrap();
        assert_eq!(
            rows,
            vec![(0, "Sounds01/BGM_/music000".into()), (7, "x".into())]
        );
    }

    #[test]
    fn battle_song_lookup_follows_the_game() {
        let table = parse_battle_songs(br#"{"100":{"5":1,"6":"35"}}"#).unwrap();
        assert_eq!(music_for_battle(&table, 100, 5), 1);
        assert_eq!(music_for_battle(&table, 100, 6), 35);
        assert_eq!(music_for_battle(&table, 100, 7), -1);
        assert_eq!(music_for_battle(&table, 101, 5), -1);
    }

    /// 0.1 s stereo 440 Hz sine at 44.1 kHz (synthetic, no game data).
    const SINE: &[u8] = include_bytes!("../../tantalus-assets/tests/fixtures/sine-440-stereo.ogg");

    fn akb2_wrap(ogg: &[u8], loop_start: u32, loop_end: u32) -> Vec<u8> {
        let mut h = vec![0u8; 304];
        h[..4].copy_from_slice(b"AKB2");
        h[8..12].copy_from_slice(&((304 + ogg.len()) as u32).to_le_bytes());
        h[230..232].copy_from_slice(&44100u16.to_le_bytes());
        h[232..236].copy_from_slice(&(ogg.len() as u32).to_le_bytes());
        h[236..240].copy_from_slice(&123u32.to_le_bytes());
        h[240..244].copy_from_slice(&loop_start.to_le_bytes());
        h[244..248].copy_from_slice(&loop_end.to_le_bytes());
        h.extend(ogg);
        h
    }

    #[test]
    fn decoded_clip_has_the_stream_length_and_the_header_loop() {
        let clip = decode_clip(&akb2_wrap(SINE, 100, 2000)).unwrap();
        assert_eq!((clip.sample_rate, clip.channels), (44100, 2));
        // The length is the final Ogg granule, not the placeholder sample count in the header.
        assert_eq!(clip.frames(), 4410);
        assert_eq!((clip.loop_start, clip.loop_end), (100, 2000));
        assert_eq!(clip.samples.len(), 4410 * 2);
        let unlooped = decode_clip(&akb2_wrap(SINE, 0, 0)).unwrap();
        assert_eq!((unlooped.loop_start, unlooped.loop_end), (0, 0));
    }

    /// The first field song of the install: its length and loop points are the file's.
    /// Skips when the install is absent.
    #[test]
    fn real_install_track_length_and_loop_points() {
        let root = Path::new("E:/SteamLibrary/steamapps/common/FINAL FANTASY IX");
        if !root.join("StreamingAssets/p0data61.bin").is_file() {
            eprintln!("skipping: FFIX install not present");
            return;
        }
        let mut bank =
            SoundBank::open(&root.join("StreamingAssets"), &root.join("x64/FF9_Data")).unwrap();
        // Music id 1 is `Sounds01/BGM_/music004` in `MusicMetaData.txt`.
        assert_eq!(
            bank.metadata.music.get(&1).map(String::as_str),
            Some("Sounds01/BGM_/music004")
        );
        let clip = bank.load_clip("Sounds01/BGM_/music004").unwrap();
        assert_eq!((clip.sample_rate, clip.channels), (44100, 2));
        assert_eq!(clip.frames(), 4_264_734);
        assert_eq!((clip.loop_start, clip.loop_end), (86_811, 4_264_733));
        // The loop ends on the last frame, as 100 of the 101 looped songs do.
        assert_eq!(u64::from(clip.loop_end) + 1, clip.frames() as u64);
        // Menu cursor sound: effect 103 is a short mono clip that does not loop.
        let cursor = bank.metadata.sound_effect[&103].clone();
        let cursor = bank.load_clip(&cursor).unwrap();
        assert_eq!((cursor.channels, cursor.loop_end), (1, 0));
        // Victory fanfare and the battle song for the first fought battle exist in the table.
        assert!(bank.metadata.music.contains_key(&VICTORY_SONG_FOR_TEST));
        assert!(!bank.field_battle_songs.is_empty());
    }

    const VICTORY_SONG_FOR_TEST: i32 = 5;
}
