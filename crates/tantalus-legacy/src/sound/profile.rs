//! Sound profiles, the metadata tables that resolve ids to them, and the profile state graph.

use std::collections::BTreeMap;

/// `SoundProfileType`, `Global/Sound/Profile/SoundProfileType.cs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProfileType {
    Default,
    Music,
    SoundEffect,
    MovieAudio,
    Song,
    Sfx,
    Voice,
}

/// `SoundProfileState`, `Global/Sound/Profile/SoundProfileState.cs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileState {
    Idle,
    Created,
    CrossfadeIn,
    Played,
    Paused,
    Stopped,
    Released,
}

/// `SoundProfile` with its constructor defaults, `Global/Sound/Profile/SoundProfile.cs:5` to `:20`.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundProfile {
    pub code: String,
    pub name: String,
    pub resource_id: String,
    pub bank_id: i32,
    pub sound_index: i32,
    pub sound_id: i32,
    pub sound_volume: f32,
    pub panning: f32,
    pub pitch: f32,
    pub state: ProfileState,
    pub profile_type: ProfileType,
    pub start_play_time: f32,
}

impl SoundProfile {
    pub fn new(sound_index: i32, name: &str, profile_type: ProfileType) -> Self {
        Self {
            code: sound_index.to_string(),
            name: name.to_owned(),
            resource_id: name.to_owned(),
            bank_id: 0,
            sound_index,
            sound_id: 0,
            sound_volume: 1.0,
            panning: 0.0,
            pitch: 0.0,
            state: ProfileState::Idle,
            profile_type,
            start_play_time: 0.0,
        }
    }
}

/// One `{name, soundIndex}` metadata entry (`SoundMetaDataProfile.cs`).
pub type MetaEntry = (i32, String);

/// The id to resource-name tables behind `SoundMetaData.GetSoundProfile`
/// (`Global/Sound/Meta/SoundMetaData.cs:338` to `:383`).
///
/// The tables are plain data. The game fills them from embedded JSON manifests
/// (`SoundMetaData.cs:127` to `:152`); this port takes already parsed entries.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SoundMetadata {
    pub music: BTreeMap<i32, String>,
    pub sound_effect: BTreeMap<i32, String>,
    pub movie_audio: BTreeMap<i32, String>,
    pub song: BTreeMap<i32, String>,
}

impl SoundMetadata {
    /// `IndexingSoundData` (`SoundMetaData.cs:289`): a duplicate id throws in the source.
    pub fn index(entries: &[MetaEntry]) -> Result<BTreeMap<i32, String>, String> {
        let mut map = BTreeMap::new();
        for (id, name) in entries {
            if map.insert(*id, name.clone()).is_some() {
                return Err(format!("duplicate sound index {id}"));
            }
        }
        Ok(map)
    }

    /// `SoundEffectMetaData` setter (`SoundMetaData.cs:14` to `:30`): the base table wins and the
    /// extended table only adds ids the base lacks.
    pub fn set_sound_effects(
        &mut self,
        base: &[MetaEntry],
        extended: &[MetaEntry],
    ) -> Result<(), String> {
        let extended = Self::index(extended)?;
        let mut merged = Self::index(base)?;
        for (id, name) in extended {
            merged.entry(id).or_insert(name);
        }
        self.sound_effect = merged;
        Ok(())
    }

    /// `SoundMetaData.GetSoundProfile`. `Sfx` is unsupported and returns `None`; `Default` and
    /// `Voice` have no table, so they also return `None`.
    pub fn get_profile(&self, sound_index: i32, profile_type: ProfileType) -> Option<SoundProfile> {
        let table = match profile_type {
            ProfileType::Music => &self.music,
            ProfileType::SoundEffect => &self.sound_effect,
            ProfileType::MovieAudio => &self.movie_audio,
            ProfileType::Song => &self.song,
            ProfileType::Sfx | ProfileType::Default | ProfileType::Voice => return None,
        };
        let name = table.get(&sound_index)?;
        Some(SoundProfile::new(sound_index, name, profile_type))
    }
}

/// Transition delegates registered in the `MusicPlayer` constructor
/// (`Global/Sound/MusicPlayer.cs:8` to `:19`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    Create,
    StartCrossfadeIn,
    CrossfadeInFinish,
    Resume,
    Pause,
    Stop,
}

/// `SdLibSoundProfileStateGraph.Transition` table lookup (`Global/Sd/SdLibSoundProfileStateGraph.cs:24`
/// to `:43`). Returns the next state, or `None` where the source logs "does not exist" and returns 1.
pub fn music_graph_next(state: ProfileState, transition: Transition) -> Option<ProfileState> {
    use ProfileState::*;
    use Transition::*;
    match (transition, state) {
        (Create, Idle | Stopped) => Some(Created),
        (StartCrossfadeIn, Created) => Some(CrossfadeIn),
        (CrossfadeInFinish, CrossfadeIn) => Some(Played),
        (Resume, Paused) => Some(Played),
        (Pause, Played | CrossfadeIn) => Some(Paused),
        (Stop, Paused | Played | CrossfadeIn) => Some(Stopped),
        _ => None,
    }
}
