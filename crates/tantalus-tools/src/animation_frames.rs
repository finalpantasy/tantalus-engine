//! Animation lengths in frames, for scripts that wait on an actor's
//! animation frame (`ReadObjectMember` 7) or on `WaitAnimation`.
//!
//! The reference counts `round(clip.length * clip.frameRate) + 1` frames
//! (`EventEngineUtils.GetCharAnimFrame`, used by `ExecAnim` for `frameN`).
//! The clips live in `p0data5.bin`; [`build`] reads them all once and the
//! result is kept as a small CSV (`audit/animation-frames.csv`) that
//! [`read_csv`] loads.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use tantalus_assets::pc::field_models::{animation_first_key, animation_name, resolve_animation};
use tantalus_assets::pc::model_assembly::{SerializedStore, clip_entries};
use tantalus_assets::pc::serialized::SerializedFile;
use tantalus_assets::pc::unity_object::animation::AnimationClip;
use tantalus_assets::pc::unity_raw::UnityRawBundle;

const MAX_BUNDLE_BYTES: usize = 600 * 1024 * 1024;

/// The last key time over every curve of the clip, in seconds.
fn clip_length(clip: &AnimationClip) -> f32 {
    let mut last = 0.0f32;
    for time in clip
        .rotation_curves
        .iter()
        .filter_map(|curve| curve.keys.last().map(|key| key.time))
        .chain(
            clip.position_curves
                .iter()
                .filter_map(|curve| curve.keys.last().map(|key| key.time)),
        )
        .chain(
            clip.scale_curves
                .iter()
                .filter_map(|curve| curve.keys.last().map(|key| key.time)),
        )
    {
        last = last.max(time);
    }
    last
}

/// `GetCharAnimFrame` for every animation id whose clip is in the bundle.
pub fn build(streaming_assets: &Path) -> Result<BTreeMap<u16, u16>, String> {
    let path = streaming_assets.join("p0data5.bin");
    let bundle = UnityRawBundle::open(&path).map_err(|error| error.to_string())?;
    let bytes = bundle
        .read_entry(0, MAX_BUNDLE_BYTES)
        .map_err(|error| error.to_string())?;
    let file = SerializedFile::parse(&bytes, "p0data5.bin").map_err(|error| error.to_string())?;
    let store = SerializedStore::new(&file, "p0data5.bin").map_err(|error| error.to_string())?;
    let entries = store.container().map_err(|error| error.to_string())?;
    let (clips, _) = clip_entries(&entries);
    let by_name: HashMap<(u32, &str), i64> = clips
        .iter()
        .map(|clip| ((clip.geo_id, clip.name.as_str()), clip.path_id))
        .collect();
    let mut table = BTreeMap::new();
    for id in 0..=u16::MAX {
        if animation_name(u32::from(id)).is_none() {
            continue;
        }
        let resolved = resolve_animation(id, None, &HashMap::new());
        let Some(path_id) = resolved
            .folder_geo_id
            .zip(animation_first_key(u32::from(id)))
            .and_then(|(folder, key)| by_name.get(&(folder, key.to_string().as_str())).copied())
        else {
            continue;
        };
        let Ok(Some(clip)) = store.clip(path_id) else {
            continue;
        };
        let rate = if clip.sample_rate > 0.0 {
            clip.sample_rate
        } else {
            30.0
        };
        let frames = (clip_length(&clip) * rate).round() as i64 + 1;
        table.insert(id, u16::try_from(frames).unwrap_or(u16::MAX));
    }
    Ok(table)
}

/// `id,frames` lines.
pub fn to_csv(table: &BTreeMap<u16, u16>) -> String {
    let mut text = String::from("# animation id,frames (round(length * frameRate) + 1)\n");
    for (id, frames) in table {
        text.push_str(&format!("{id},{frames}\n"));
    }
    text
}

/// Reads the CSV [`to_csv`] writes.
pub fn read_csv(text: &str) -> BTreeMap<u16, u16> {
    text.lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let (id, frames) = line.split_once(',')?;
            Some((id.trim().parse().ok()?, frames.trim().parse().ok()?))
        })
        .collect()
}
/// The table shipped with the engine (`audit/animation-frames.csv`, written
/// by `tantalus-trace animation-frames`).
const EMBEDDED: &str = include_str!("../../../audit/animation-frames.csv");

/// The shipped table: frames by animation id.
pub fn table() -> std::sync::Arc<BTreeMap<u16, u16>> {
    static TABLE: std::sync::OnceLock<std::sync::Arc<BTreeMap<u16, u16>>> =
        std::sync::OnceLock::new();
    TABLE
        .get_or_init(|| std::sync::Arc::new(read_csv(EMBEDDED)))
        .clone()
}

/// Gives `world` the real clip lengths unless it already has a provider
/// (`ExecAnim` waits `GetCharAnimFrame` frames, not a fixed 32).
pub fn install(world: &mut tantalus_game::FieldWorld) {
    if world.animation_frame_counts.is_some() {
        return;
    }
    let table = table();
    world.animation_frame_counts = Some(std::sync::Arc::new(move |id| table.get(&id).copied()));
}
