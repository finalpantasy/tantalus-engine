//! The script-facing sound state machine: `FF9Snd` dispatch plus `AllSoundDispatchPlayer`.
//!
//! Source files: `Global/ff9/Snd/FF9Snd.cs` (written `FF9Snd.cs`) and `Global/AllSoundDispatchPlayer.cs`
//! (written `ASDP.cs`). All paths are relative to `Assembly-CSharp/`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::backend::{Backend, PcmClip};
use super::profile::{ProfileType, SoundMetadata, SoundProfile};

/// `ASDP.cs:1422`
pub const MINIMUM_SONG_FADE_MS: i32 = 500;
/// `ASDP.cs:1416`
pub const VOLUME_MAX: i32 = 127;
/// `ASDP.cs:1418`
pub const SNDEFFECTRES_SLOT_MAX: usize = 2;

/// Sound codes (`FF9Snd.cs:2287` onward). `code & 0xFFBF` strips the slot bit.
pub mod code {
    pub const SONG_PLAY: i32 = 0;
    pub const SONG_STOP: i32 = 256;
    pub const SONG_STOPCURRENT: i32 = 265;
    pub const SONG_NULL: i32 = 520;
    pub const SONG_TEMPOPITCH_FADE: i32 = 1028;
    pub const SONG_LOAD: i32 = 1792;
    pub const SONG_SUSPEND: i32 = 2048;
    pub const SONG_RESTORE: i32 = 2304;
    pub const SONG_JUMPPOINT: i32 = 2566;
    pub const SYNC: i32 = 3072;
    pub const SNDEFFECTRES_STOPCURRENT: i32 = 4489;
    pub const SNDEFFECT_NULL: i32 = 4616;
    pub const SNDEFFECTRES_SUSPEND: i32 = 6272;
    pub const SNDEFFECTRES_RESTORE: i32 = 6528;
    pub const STREAM_STOP: i32 = 8448;
    pub const STREAM_NULL: i32 = 8712;
    pub const SONG_VOL: i32 = 16897;
    pub const SONG_TEMPO: i32 = 16898;
    pub const SONG_PITCH: i32 = 16899;
    pub const SONG_TEMPOPITCH: i32 = 16900;
    pub const SONG_SKIPPHRASE: i32 = 16903;
    pub const SNDEFFECT_STOP: i32 = 20736;
    pub const SNDEFFECTRES_STOP: i32 = 20864;
    pub const SNDEFFECT_VOL_ALL: i32 = 21761;
    pub const SNDEFFECT_PITCH_ALL: i32 = 21763;
    pub const SNDEFFECT_POS_ALL: i32 = 21765;
    pub const SNDEFFECTRES_VOL_ALL: i32 = 21889;
    pub const SNDEFFECTRES_PITCH_ALL: i32 = 21891;
    pub const SNDEFFECTRES_POS_ALL: i32 = 21893;
    pub const STREAM_VOL: i32 = 25089;
    pub const STREAM_POS: i32 = 25093;
    pub const STREAMFMV_VOL: i32 = 25098;
    pub const STREAM_REVERB: i32 = 25100;
    pub const INSTR_LOAD: i32 = 30464;
    pub const SONG_VOL_INTPL: i32 = 33537;
    pub const SONG_TEMPO_INTPL: i32 = 33538;
    pub const SONG_PITCH_INTPL: i32 = 33539;
    pub const SONG_TEMPOPITCH_INTPL: i32 = 33540;
    pub const SONG_VOL_INTPLALL: i32 = 34305;
    pub const SNDEFFECT_VOL: i32 = 37377;
    pub const SNDEFFECT_POS: i32 = 37381;
    pub const SNDEFFECT_PITCH: i32 = 37379;
    pub const SNDEFFECTRES_VOL: i32 = 37505;
    pub const SNDEFFECTRES_PITCH: i32 = 37507;
    pub const SNDEFFECTRES_POS: i32 = 37509;
    pub const SNDEFFECT_VOL_INTPLALL: i32 = 38401;
    pub const SNDEFFECT_PITCH_INTPLALL: i32 = 38403;
    pub const SNDEFFECT_POS_INTPLALL: i32 = 38405;
    pub const SNDEFFECTRES_VOL_INTPLALL: i32 = 38529;
    pub const SNDEFFECTRES_PITCH_INTPLALL: i32 = 38531;
    pub const SNDEFFECTRES_POS_INTPLALL: i32 = 38533;
    pub const STREAM_PLAY: i32 = 40960;
    pub const SONG_VOL_FADE: i32 = 50177;
    pub const SONG_TEMPO_FADE: i32 = 50178;
    pub const SONG_PITCH_FADE: i32 = 50179;
    pub const SONG_VOL_FADEALL: i32 = 51969;
    pub const SNDEFFECT_PLAY: i32 = 53248;
    pub const SNDEFFECTRES_PLAY: i32 = 53376;
    pub const SNDEFFECT_VOL_INTPL: i32 = 54017;
    pub const SNDEFFECT_PITCH_INTPL: i32 = 54019;
    pub const SNDEFFECT_POS_INTPL: i32 = 54021;
    pub const SNDEFFECTRES_VOL_INTPL: i32 = 54145;
    pub const SNDEFFECTRES_PITCH_INTPL: i32 = 54147;
    pub const SNDEFFECTRES_POS_INTPL: i32 = 54149;
}

/// `ASDP.cs:9` `NormalizeVolume`: `Mathf.Clamp01(v / 127)`.
pub fn normalize_volume(original: i32) -> f32 {
    (original as f32 / 127.0).clamp(0.0, 1.0)
}

/// `ASDP.cs:14` `ReverseNormalizeVolume`.
pub fn reverse_normalize_volume(normalized: f32) -> i32 {
    ((normalized * 127.0) as i32).min(127)
}

/// `ASDP.cs:20` `NormalizePitch`: `original / 255 * 2 + 1`.
pub fn normalize_pitch(original: i32) -> f32 {
    (original as f32 / 255.0) * 2.0 + 1.0
}

/// `ASDP.cs:75` `ConvertTickToMillisec`: `Mathf.RoundToInt(ticks * 1000 / 60)` (half to even).
pub fn convert_tick_to_millisec(ticks: i32) -> i32 {
    (ticks as f32 * 1000.0 / 60.0).round_ties_even() as i32
}

/// `ASDP.cs:87` `ConvertMillisecToTick`: `(UInt16)(ms / 60)`. The divisor is 60 in the source.
pub fn convert_millisec_to_tick(ms: f32) -> u16 {
    (ms / 60.0) as u16
}

/// `FF9Snd.cs:224` `GetIsExtEnvObjNo`.
pub fn is_ext_env_obj_no(obj: i32) -> bool {
    matches!(
        obj,
        642 | 618
            | 347
            | 656
            | 650
            | 9060080
            | 9060084
            | 9060087
            | 1495
            | 9070035
            | 9080033
            | 1397
            | 9080075
            | 9090033
            | 9090097
            | 9100097
            | 9100105
            | 9100107
            | 9110004
            | 9110015
            | 9120016
            | 1887
            | 410
            | 1461
    )
}

/// `FF9Snd.cs:229` `IsShiftPitchInFastForwardMode`.
pub fn is_shift_pitch_in_fast_forward(obj: i32) -> bool {
    !matches!(
        obj,
        103 | 1044
            | 636
            | 635
            | 634
            | 101
            | 105
            | 109
            | 683
            | 1043
            | 108
            | 107
            | 106
            | 1046
            | 1045
            | 1047
            | 1261
            | 3096
            | 102
            | 682
            | 1362
            | 1853
            | 1854
            | 1855
            | 1856
            | 1857
            | 1858
            | 1859
            | 1852
            | 1861
            | 1862
            | 1863
            | 1864
            | 2331
            | 2332
            | 2333
    ) && !is_ext_env_obj_no(obj)
}

/// `FF9Snd.cs:234` `GetExtEnvObjNo`: resident effect id to its extra environment effect, or -1.
pub fn ext_env_obj_no(obj: i32) -> i32 {
    match obj {
        643 => 642,
        255 => 618,
        355 | 275 => 347,
        661 => 656,
        652 | 736 => 650,
        1490 => 9060080,
        1493 => 9060084,
        1511 => 9060087,
        1569 => 1495,
        1321 => 9070035,
        1366 => 9080033,
        1396 => 1397,
        1406 => 9080075,
        1431 => 9090033,
        1993 => 9090097,
        2287 => 9100097,
        2292 => 9100105,
        2293 => 9100107,
        1877 => 9110004,
        2648 => 9110015,
        2193 => 9120016,
        1888 => 1887,
        _ => -1,
    }
}

/// Which dispatch function `FF9Snd.sndFuncPtr` points at (`FF9Snd.cs:12`, `SceneDirector.cs:412`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchMode {
    /// `FF9AllSoundDispatch`. The battle dispatch (`FF9Snd.cs:1135`) forwards to it unchanged.
    All,
    /// `FF9FieldSoundDispatch`.
    Field,
    /// `FF9WorldSoundDispatch`.
    World,
}

/// Player volumes and host settings the source reads from `Configuration` and game state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoundSettings {
    /// `Configuration.Audio.MusicVolume / 100`.
    pub music_volume: f32,
    /// `Configuration.Audio.SoundVolume / 100`.
    pub sound_volume: f32,
    /// `HonoBehaviorSystem.GetFastForwardFactor()`.
    pub fast_forward_factor: i32,
    /// `FF9StateSystem.Settings.cfg.sound` (1 suppresses `SONG_VOL_INTPLALL`).
    pub cfg_sound: u64,
    /// `SoundLib.SoundEffectIsMute`.
    pub sound_effect_is_mute: bool,
}

impl Default for SoundSettings {
    fn default() -> Self {
        Self {
            music_volume: 1.0,
            sound_volume: 1.0,
            fast_forward_factor: 1,
            cfg_sound: 0,
            sound_effect_is_mute: false,
        }
    }
}

/// `AllSoundDispatchPlayer.PlayingSfx`, `ASDP.cs:1466`.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayingSfx {
    pub obj_no: i32,
    pub sound_id: i32,
    pub snd_effect_pos: i32,
    pub snd_effect_vol: i32,
    pub prev_snd_effect_map_id: i32,
    pub is_suspend: bool,
    pub pitch: f32,
    pub is_fast_forwarded_pitch: bool,
    /// `Time.time` in the source; here a monotonic play counter. Only its order is used.
    pub start_play_time: u64,
}

impl PlayingSfx {
    fn new(obj_no: i32, sound_id: i32) -> Self {
        Self {
            obj_no,
            sound_id,
            snd_effect_pos: 0,
            snd_effect_vol: 0,
            prev_snd_effect_map_id: 0,
            is_suspend: false,
            pitch: 1.0,
            is_fast_forwarded_pitch: false,
            start_play_time: 0,
        }
    }
}

type ClipFn = dyn FnMut(&str) -> Option<Arc<PcmClip>> + Send;

/// Loads a clip the first time its resource id is asked for (`SoundLoaderResources`): the
/// caller's asset reader. Unset, only clips placed in `SoundSystem::clips` can play.
#[derive(Clone, Default)]
pub struct ClipLoader(Option<Arc<std::sync::Mutex<Box<ClipFn>>>>);

impl ClipLoader {
    pub fn new(load: impl FnMut(&str) -> Option<Arc<PcmClip>> + Send + 'static) -> Self {
        Self(Some(Arc::new(std::sync::Mutex::new(Box::new(load)))))
    }

    fn load(&self, resource_id: &str) -> Option<Arc<PcmClip>> {
        let loader = self.0.as_ref()?;
        let mut load = loader.lock().ok()?;
        load(resource_id)
    }
}

impl std::fmt::Debug for ClipLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() {
            "ClipLoader(set)"
        } else {
            "ClipLoader(unset)"
        })
    }
}

/// The sound state machine.
#[derive(Debug, Clone)]
pub struct SoundSystem {
    pub backend: Backend,
    pub metadata: SoundMetadata,
    pub settings: SoundSettings,
    /// Decoded assets by resource id (`SoundProfile.ResourceID`). A missing entry is a load error.
    pub clips: BTreeMap<String, Arc<PcmClip>>,
    /// Fills `clips` on a miss.
    pub clip_loader: ClipLoader,
    pub dispatch_mode: DispatchMode,
    /// `EventEngine.gMode` (1 field, 3 world).
    pub game_mode: i32,
    /// `FF9StateSystem.Common.FF9.fldMapNo`.
    pub fld_map_no: i32,
    /// `FF9Snd.BGMFieldSongCounter`, `FF9Snd.cs:1415`.
    pub bgm_field_song_counter: i32,
    /// `FF9Snd.PlayedFieldSoundEffect`.
    pub played_field_sound_effect: BTreeSet<i32>,
    music_db: BTreeMap<i32, SoundProfile>,
    sfx_db: BTreeMap<i32, SoundProfile>,
    song_db: BTreeMap<i32, SoundProfile>,
    current_music_id: i32,
    suspend_bgm_no: i32,
    suspend_bgm_id: i32,
    sfx_channels: Vec<PlayingSfx>,
    sfx_res_slot: [Option<PlayingSfx>; SNDEFFECTRES_SLOT_MAX],
    current_song_id: i32,
    res_hooks_installed: bool,
    play_counter: u64,
    /// `sSEPos` / `sSEVol` (`EventEngine.cs:70`).
    pub se_pos: i32,
    pub se_vol: i32,
    /// `btl_flag & BTL_CONTI_FLD_SONG` (`EventEngine.DoEventCode.cs:269`).
    pub continue_field_song_after_battle: bool,
}

impl SoundSystem {
    pub fn new(backend: Backend, metadata: SoundMetadata) -> Self {
        Self {
            backend,
            metadata,
            settings: SoundSettings::default(),
            clips: BTreeMap::new(),
            clip_loader: ClipLoader::default(),
            dispatch_mode: DispatchMode::All,
            game_mode: 0,
            fld_map_no: 0,
            bgm_field_song_counter: 0,
            played_field_sound_effect: BTreeSet::new(),
            music_db: BTreeMap::new(),
            sfx_db: BTreeMap::new(),
            song_db: BTreeMap::new(),
            current_music_id: -1,
            suspend_bgm_no: -1,
            suspend_bgm_id: 0,
            sfx_channels: Vec::new(),
            sfx_res_slot: [None, None],
            current_song_id: -1,
            res_hooks_installed: false,
            play_counter: 0,
            se_pos: 0,
            se_vol: 0,
            continue_field_song_after_battle: false,
        }
    }

    pub fn current_music_id(&self) -> i32 {
        self.current_music_id
    }

    pub fn suspend_song_id(&self) -> i32 {
        self.suspend_bgm_no
    }

    pub fn current_song_id(&self) -> i32 {
        self.current_song_id
    }

    pub fn sfx_channels(&self) -> &[PlayingSfx] {
        &self.sfx_channels
    }

    pub fn sfx_res_slot(&self, slot: usize) -> Option<&PlayingSfx> {
        self.sfx_res_slot.get(slot).and_then(Option::as_ref)
    }

    pub fn profile(&self, sound_index: i32, profile_type: ProfileType) -> Option<&SoundProfile> {
        match profile_type {
            ProfileType::Music => self.music_db.get(&sound_index),
            ProfileType::SoundEffect => self.sfx_db.get(&sound_index),
            ProfileType::Song => self.song_db.get(&sound_index),
            _ => None,
        }
    }

    /// Advances `ticks` engine ticks: each runs `update` then renders one tick of audio.
    /// Output is interleaved stereo f32. The source runs `Update` once per rendered frame
    /// (`SoundLib.cs:827`); one call per tick is a host choice recorded in `sound/mod.rs`.
    pub fn run_ticks(&mut self, ticks: u32) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.backend.frames_per_tick() * 2 * ticks as usize);
        for _ in 0..ticks {
            self.update();
            out.extend(self.backend.mix_ticks(1));
        }
        out
    }

    // --- profile database plumbing -------------------------------------------------------

    fn with_profile<R>(
        &mut self,
        index: i32,
        profile_type: ProfileType,
        f: impl FnOnce(&mut Backend, &mut SoundProfile, &SoundSettings) -> R,
    ) -> Option<R> {
        let settings = self.settings;
        let db = match profile_type {
            ProfileType::Music => &mut self.music_db,
            ProfileType::SoundEffect => &mut self.sfx_db,
            ProfileType::Song => &mut self.song_db,
            _ => return None,
        };
        let profile = db.get_mut(&index)?;
        Some(f(&mut self.backend, profile, &settings))
    }

    /// `GetSoundProfileIfExist`, `ASDP.cs:946`.
    fn profile_exists(&self, index: i32, profile_type: ProfileType) -> bool {
        self.profile(index, profile_type).is_some()
    }

    /// `CreateSoundProfileIfNotExist`, `ASDP.cs:972`. True when the profile is (now) in its
    /// database, which is when the source runs the create callback.
    fn ensure_profile(&mut self, index: i32, profile_type: ProfileType) -> bool {
        if !matches!(
            profile_type,
            ProfileType::Music | ProfileType::SoundEffect | ProfileType::Song
        ) {
            return false;
        }
        if self.profile_exists(index, profile_type) {
            return true;
        }
        let Some(mut profile) = self.metadata.get_profile(index, profile_type) else {
            return false;
        };
        self.delete_immediate_profiles_if_unused();
        // `SoundPlayer.LoadResource` -> loader. A missing clip is the loader's error path.
        if !self.clips.contains_key(&profile.resource_id)
            && let Some(clip) = self.clip_loader.load(&profile.resource_id)
        {
            self.clips.insert(profile.resource_id.clone(), clip);
        }
        let Some(clip) = self.clips.get(&profile.resource_id).cloned() else {
            return false;
        };
        profile.bank_id = self
            .backend
            .add_data(clip, &profile.resource_id, profile_type);
        let db = match profile_type {
            ProfileType::Music => &mut self.music_db,
            ProfileType::SoundEffect => &mut self.sfx_db,
            _ => &mut self.song_db,
        };
        // `SoundDatabase.Create` keeps an existing entry; the miss above makes this a plain insert.
        db.entry(index).or_insert(profile);
        true
    }

    /// `SoundPlayer.StaticCreateSound`, `Global/Sound/SoundPlayer.cs:57`.
    fn static_create_sound(backend: &mut Backend, profile: &mut SoundProfile) {
        if profile.sound_id > 0 {
            if backend.is_exist(profile.sound_id) {
                return;
            }
            profile.sound_id = 0;
        }
        let id = backend.create_sound(profile.bank_id);
        if id <= 0 {
            return;
        }
        profile.sound_id = id;
    }

    /// `StaticUnregisterBank`, `SoundPlayer.cs:197`.
    fn unload(backend: &mut Backend, profile: &SoundProfile) {
        if profile.bank_id != 0 {
            backend.remove_data(profile.bank_id);
        }
    }

    /// `DeleteImmediateSoundProfileIfUnused`, `ASDP.cs:998`.
    fn delete_immediate_profiles_if_unused(&mut self) {
        if self.music_db.len() >= 4 {
            let dead: Vec<i32> = self
                .music_db
                .iter()
                .filter(|(_, p)| !self.backend.is_exist(p.sound_id))
                .map(|(k, _)| *k)
                .collect();
            for key in dead {
                if let Some(p) = self.music_db.remove(&key) {
                    self.backend.stop(p.sound_id, 0);
                    Self::unload(&mut self.backend, &p);
                }
            }
        }
        if self.sfx_db.len() >= 20 {
            let backend = &self.backend;
            self.sfx_channels
                .retain(|c| backend.is_exist(c.sound_id) && c.obj_no != -1);
            for slot in &mut self.sfx_res_slot {
                if let Some(s) = slot
                    && !s.is_suspend
                    && !self.backend.is_exist(s.sound_id)
                {
                    *slot = None;
                }
            }
            let in_use: Vec<i32> = self
                .sfx_channels
                .iter()
                .map(|c| c.sound_id)
                .chain(self.sfx_res_slot.iter().flatten().map(|s| s.sound_id))
                .collect();
            let dead: Vec<i32> = self
                .sfx_db
                .iter()
                .filter(|(_, p)| !in_use.contains(&p.sound_id))
                .map(|(k, _)| *k)
                .collect();
            for key in dead {
                if let Some(p) = self.sfx_db.remove(&key) {
                    self.backend.stop(p.sound_id, 0);
                    Self::unload(&mut self.backend, &p);
                }
            }
        }
        if self.song_db.len() >= 4 {
            let dead: Vec<i32> = self
                .song_db
                .iter()
                .filter(|(_, p)| !self.backend.is_exist(p.sound_id))
                .map(|(k, _)| *k)
                .collect();
            for key in dead {
                if let Some(p) = self.song_db.remove(&key) {
                    self.backend.stop(p.sound_id, 0);
                    Self::unload(&mut self.backend, &p);
                }
            }
        }
    }

    fn music_sound_id(&self, index: i32) -> Option<i32> {
        self.music_db.get(&index).map(|p| p.sound_id)
    }

    // --- music ---------------------------------------------------------------------------

    /// `FF9SOUND_SONG_LOAD`, `ASDP.cs:112`.
    pub fn song_load(&mut self, obj_no: i32) {
        if self.game_mode != 1 {
            self.ensure_profile(obj_no, ProfileType::Music);
            return;
        }
        if self.current_music_id == obj_no {
            return;
        }
        if self.current_music_id != -1 {
            if let Some(id) = self.music_sound_id(self.current_music_id) {
                self.backend.stop(id, 0);
            }
            self.current_music_id = -1;
        }
        if self.ensure_profile(obj_no, ProfileType::Music) {
            self.with_profile(obj_no, ProfileType::Music, |be, p, s| {
                Self::static_create_sound(be, p);
                p.sound_volume = 1.0;
                be.set_volume(p.sound_id, s.music_volume, 0);
                // Cargo Ship bridge special case (`ASDP.cs:132` to `:140`) needs script variables
                // and is not ported; see `sound/mod.rs`.
                be.start(p.sound_id, 0);
                be.set_pause(p.sound_id, 1, 0);
            });
            self.current_music_id = obj_no;
            self.stop_and_clear_suspend_bgm(obj_no, true);
        }
    }

    /// `StopAndClearSuspendBGM`, `ASDP.cs:157`.
    pub fn stop_and_clear_suspend_bgm(&mut self, obj_no: i32, set_current_music: bool) {
        if self.suspend_bgm_no != -1 && obj_no != self.suspend_bgm_no {
            if self.game_mode == 1 && self.dispatch_mode == DispatchMode::Field {
                self.backend.stop(self.suspend_bgm_id, 0);
                self.suspend_bgm_no = -1;
                self.suspend_bgm_id = 0;
            }
            if self.game_mode == 3 && self.dispatch_mode == DispatchMode::World {
                self.backend.stop(self.suspend_bgm_id, 0);
                self.suspend_bgm_no = -1;
                self.suspend_bgm_id = 0;
            }
        }
        if set_current_music
            && self.current_music_id != -1
            && self.game_mode == 1
            && self.dispatch_mode == DispatchMode::Field
        {
            self.suspend_bgm_no = -1;
            self.suspend_bgm_id = 0;
        }
    }

    /// `FF9SOUND_SONG_PLAY`, `ASDP.cs:182`.
    pub fn song_play(&mut self, obj_no: i32, vol: i32, time: i32) {
        let mut time = time;
        if self.current_music_id != obj_no {
            if self.current_music_id != -1 {
                if let Some(id) = self.music_sound_id(self.current_music_id) {
                    time = time.max(MINIMUM_SONG_FADE_MS);
                    self.backend.stop(id, time);
                }
                self.current_music_id = -1;
            }
            if self.ensure_profile(obj_no, ProfileType::Music) {
                self.with_profile(obj_no, ProfileType::Music, |be, p, s| {
                    Self::static_create_sound(be, p);
                    p.sound_volume = normalize_volume(vol);
                    be.set_volume(p.sound_id, p.sound_volume * s.music_volume, 0);
                    be.start(p.sound_id, 0);
                });
                self.current_music_id = obj_no;
                self.stop_and_clear_suspend_bgm(obj_no, true);
            }
        } else if self.ensure_profile(obj_no, ProfileType::Music) {
            self.with_profile(obj_no, ProfileType::Music, |be, p, s| {
                be.set_pause(p.sound_id, 0, time);
                p.sound_volume = normalize_volume(vol);
                be.set_volume(p.sound_id, p.sound_volume * s.music_volume, 0);
            });
            self.stop_and_clear_suspend_bgm(obj_no, true);
        }
    }

    /// `FF9SOUND_SONG_SUSPEND`, `ASDP.cs:228`.
    pub fn song_suspend(&mut self, obj_no: i32, is_skip_pause: bool) {
        match self.music_sound_id(obj_no) {
            Some(id) => {
                if !is_skip_pause {
                    self.backend.set_pause(id, 1, 0);
                }
                self.suspend_bgm_no = obj_no;
                self.suspend_bgm_id = id;
                self.current_music_id = -1;
            }
            None => {
                self.suspend_bgm_no = -1;
                self.suspend_bgm_id = 0;
            }
        }
    }

    /// `FF9SOUND_SONG_RESTORE`, `ASDP.cs:250`.
    pub fn song_restore(&mut self) {
        if self.suspend_bgm_no != -1 {
            self.current_music_id = self.suspend_bgm_no;
            self.backend.set_pause(self.suspend_bgm_id, 0, 0);
            self.suspend_bgm_no = -1;
            self.suspend_bgm_id = 0;
        }
    }

    /// `FF9SOUND_SONG_STOP`, `ASDP.cs:265`.
    pub fn song_stop(&mut self, obj_no: i32) {
        if let Some(id) = self.music_sound_id(obj_no) {
            self.backend.stop(id, MINIMUM_SONG_FADE_MS);
        }
        if obj_no == self.suspend_bgm_no {
            self.suspend_bgm_no = -1;
            self.suspend_bgm_id = 0;
        }
        if obj_no == self.current_music_id {
            self.current_music_id = -1;
        }
    }

    /// `StopCurrentSong`, `ASDP.cs:284`.
    pub fn stop_current_song(&mut self, ticks: i32) {
        if let Some(id) = self.music_sound_id(self.current_music_id) {
            self.backend.stop(id, convert_tick_to_millisec(ticks));
        }
        self.current_music_id = -1;
    }

    /// `FF9SOUND_SONG_STOPCURRENT`, `ASDP.cs:297`.
    pub fn song_stop_current(&mut self) {
        if self.current_music_id != -1 {
            if let Some(id) = self.music_sound_id(self.current_music_id) {
                self.backend.stop(id, MINIMUM_SONG_FADE_MS);
            }
            self.current_music_id = -1;
        }
    }

    /// `FF9SOUND_SONG_VOL`, `ASDP.cs:316`.
    pub fn song_vol(&mut self, obj_no: i32, vol: i32) {
        self.with_profile(obj_no, ProfileType::Music, |be, p, s| {
            be.set_pause(p.sound_id, 0, 0);
            p.sound_volume = normalize_volume(vol);
            be.set_volume(p.sound_id, p.sound_volume * s.music_volume, 0);
        });
    }

    /// `FF9SOUND_SONG_VOL_INTPL`, `ASDP.cs:329`.
    pub fn song_vol_intpl(&mut self, obj_no: i32, ticks: i32, to: i32) {
        self.with_profile(obj_no, ProfileType::Music, |be, p, s| {
            be.set_pause(p.sound_id, 0, 0);
            p.sound_volume = normalize_volume(to);
            be.set_volume(
                p.sound_id,
                p.sound_volume * s.music_volume,
                convert_tick_to_millisec(ticks),
            );
        });
    }

    /// `FF9SOUND_SONG_VOL_FADE`, `ASDP.cs:342`.
    pub fn song_vol_fade(&mut self, obj_no: i32, ticks: i32, from: i32, to: i32) {
        self.with_profile(obj_no, ProfileType::Music, |be, p, s| {
            be.set_pause(p.sound_id, 0, 0);
            be.set_volume(p.sound_id, normalize_volume(from) * s.music_volume, 0);
            p.sound_volume = normalize_volume(to);
            be.set_volume(
                p.sound_id,
                p.sound_volume * s.music_volume,
                convert_tick_to_millisec(ticks),
            );
        });
    }

    /// `FF9SOUND_SONG_VOL_INTPLALL`, `ASDP.cs:356`.
    pub fn song_vol_intplall(&mut self, ticks: i32, to: i32) {
        if self.settings.cfg_sound == 1 {
            return;
        }
        if self.current_music_id != -1 {
            self.song_vol_intpl_current(ticks, to);
        }
    }

    fn song_vol_intpl_current(&mut self, ticks: i32, to: i32) {
        let current = self.current_music_id;
        self.song_vol_intpl(current, ticks, to);
    }

    /// `FF9SOUND_SONG_VOL_FADEALL`, `ASDP.cs:376`. The source un-pauses, then calls
    /// `FF9SOUND_SONG_VOL_FADE` with the profile's own index.
    pub fn song_vol_fadeall(&mut self, ticks: i32, from: i32, to: i32) {
        if self.current_music_id != -1 {
            let current = self.current_music_id;
            self.with_profile(current, ProfileType::Music, |be, p, _| {
                be.set_pause(p.sound_id, 0, 0);
            });
            if self.profile_exists(current, ProfileType::Music) {
                self.song_vol_fade(current, ticks, from, to);
            }
        }
    }

    /// `FF9SOUND_SONG_PITCH`, `ASDP.cs:391`.
    pub fn song_pitch(&mut self, obj_no: i32, pitch: i32) {
        if self.current_music_id != -1 {
            self.with_profile(obj_no, ProfileType::Music, |be, p, _| {
                be.set_pitch(p.sound_id, normalize_pitch(pitch), 0);
            });
        }
    }

    /// `FF9SOUND_SONG_PITCH_INTPL`, `ASDP.cs:406`.
    pub fn song_pitch_intpl(&mut self, obj_no: i32, tick: i32, to: i32) {
        if self.current_music_id != -1 {
            self.with_profile(obj_no, ProfileType::Music, |be, p, _| {
                be.set_pitch(
                    p.sound_id,
                    normalize_pitch(to),
                    convert_tick_to_millisec(tick),
                );
            });
        }
    }

    /// `FF9SOUND_SONG_PITCH_FADE`, `ASDP.cs:422`.
    pub fn song_pitch_fade(&mut self, obj_no: i32, tick: i32, from: i32, to: i32) {
        if self.current_music_id != -1 {
            self.with_profile(obj_no, ProfileType::Music, |be, p, _| {
                be.set_pitch(p.sound_id, normalize_pitch(from), 0);
                be.set_pitch(
                    p.sound_id,
                    normalize_pitch(to),
                    convert_tick_to_millisec(tick),
                );
            });
        }
    }

    /// `FF9SOUND_SONG_JUMPPOINT`, `ASDP.cs:440`: switches the current song to its alternate loop.
    pub fn song_jumppoint(&mut self) {
        if self.current_music_id != -1 {
            self.with_profile(self.current_music_id, ProfileType::Music, |be, p, _| {
                be.set_next_loop_region(p.sound_id);
            });
        }
    }

    /// `FF9SOUND_SONG_SKIPPHRASE_MILLISEC`, `ASDP.cs:454`, SoLoud branch (`Backend > 0`).
    pub fn song_skipphrase_millisec(&mut self, offset_ms: i32) {
        self.with_profile(self.current_music_id, ProfileType::Music, |be, p, _| {
            be.start(p.sound_id, offset_ms);
        });
    }

    /// `UpdatePlayingMusicVolume`, `ASDP.cs:34`.
    pub fn update_playing_music_volume(&mut self) {
        self.with_profile(self.current_music_id, ProfileType::Music, |be, p, s| {
            be.set_volume(p.sound_id, p.sound_volume * s.music_volume, 0);
        });
    }

    // --- sound effects -------------------------------------------------------------------

    /// `LimitPlayingSfxByObjNo`, `ASDP.cs:502`. Equal start times pick the first entry.
    fn limit_playing_sfx_by_obj_no(&mut self, obj_no: i32, limit_obj_no: i32, limit: usize) {
        if obj_no != limit_obj_no {
            return;
        }
        let mut oldest: Option<usize> = None;
        let mut count = 0;
        for (i, c) in self.sfx_channels.iter().enumerate() {
            if c.obj_no == obj_no {
                count += 1;
                if oldest.is_none_or(|o| c.start_play_time < self.sfx_channels[o].start_play_time) {
                    oldest = Some(i);
                }
            }
        }
        if count >= limit
            && let Some(i) = oldest
        {
            let channel = self.sfx_channels.remove(i);
            self.backend.stop(channel.sound_id, 0);
        }
    }

    /// `TuneUpSoundEffectByObjNo`, `ASDP.cs:530`. Returns true when it handled the start.
    fn tune_up_sound_effect(
        backend: &mut Backend,
        settings: &SoundSettings,
        obj_no: i32,
        p: &mut SoundProfile,
    ) -> bool {
        if obj_no == 1748 {
            backend.set_volume(p.sound_id, 0.6 * settings.sound_volume, 0);
            p.pitch *= 0.6;
            backend.set_pitch(p.sound_id, p.pitch, 0);
            backend.start(p.sound_id, 0);
            return true;
        }
        // `Configuration.Audio.Backend > 0` holds for the SoLoud model.
        if obj_no == 58 {
            backend.set_volume(p.sound_id, 0.0, 0);
            backend.set_volume(p.sound_id, settings.sound_volume, 300);
            p.pitch *= 0.8;
            backend.set_pitch(p.sound_id, p.pitch, 0);
            backend.start(p.sound_id, 900);
            return true;
        }
        false
    }

    /// `ShiftPitchIfFastForward`, `ASDP.cs:563`.
    fn shift_pitch_if_fast_forward(
        backend: &mut Backend,
        settings: &SoundSettings,
        p: &mut SoundProfile,
    ) {
        let factor = settings.fast_forward_factor;
        if factor == 1 {
            return;
        }
        if is_shift_pitch_in_fast_forward(p.sound_index) {
            p.pitch *= factor as f32;
            backend.set_pitch(p.sound_id, factor as f32, 0);
        }
    }

    /// `FF9SOUND_SNDEFFECT_PLAY`, `ASDP.cs:477`.
    pub fn sndeffect_play(&mut self, obj_no: i32, _attr: i32, _pos: i32, vol: i32) {
        self.limit_playing_sfx_by_obj_no(obj_no, 2295, 1);
        self.limit_playing_sfx_by_obj_no(obj_no, 1966, 3);
        if !self.ensure_profile(obj_no, ProfileType::SoundEffect) {
            return;
        }
        let (sound_id, pitch) = self
            .with_profile(obj_no, ProfileType::SoundEffect, |be, p, s| {
                Self::static_create_sound(be, p);
                p.pitch = 1.0;
                if !Self::tune_up_sound_effect(be, s, obj_no, p) {
                    be.set_volume(p.sound_id, normalize_volume(vol) * s.sound_volume, 0);
                    Self::shift_pitch_if_fast_forward(be, s, p);
                    be.start(p.sound_id, 0);
                }
                (p.sound_id, p.pitch)
            })
            .unwrap_or((0, 1.0));
        self.play_counter += 1;
        let mut channel = PlayingSfx::new(obj_no, sound_id);
        channel.snd_effect_vol = vol;
        channel.pitch = pitch;
        channel.is_fast_forwarded_pitch = self.settings.fast_forward_factor != 1;
        channel.start_play_time = self.play_counter;
        self.sfx_channels.push(channel);
    }

    /// `FF9SOUND_SNDEFFECT_STOP`, `ASDP.cs:577`. `attr` is the fade time in ms.
    pub fn sndeffect_stop(&mut self, obj_no: i32, attr: i32) {
        let backend = &mut self.backend;
        self.sfx_channels.retain(|c| {
            if c.obj_no == obj_no {
                backend.stop(c.sound_id, attr);
                false
            } else {
                true
            }
        });
    }

    /// `FF9SOUND_SNDEFFECT_STOP_ALL`, `ASDP.cs:594`.
    pub fn sndeffect_stop_all(&mut self, except: Option<&BTreeSet<i32>>) {
        let backend = &mut self.backend;
        self.sfx_channels.retain(|c| {
            if !is_ext_env_obj_no(c.obj_no) && except.is_none_or(|e| !e.contains(&c.obj_no)) {
                backend.stop(c.sound_id, 0);
                false
            } else {
                true
            }
        });
    }

    /// `FF9SOUND_SNDEFFECT_VOL`, `ASDP.cs:614`.
    pub fn sndeffect_vol(&mut self, obj_no: i32, _attr: i32, vol: i32) {
        let sound_volume = self.settings.sound_volume;
        for c in &mut self.sfx_channels {
            if c.obj_no == obj_no {
                c.snd_effect_vol = vol;
                self.backend
                    .set_volume(c.sound_id, normalize_volume(vol) * sound_volume, 0);
            }
        }
    }

    /// `FF9SOUND_SNDEFFECT_VOL_INTPL`, `ASDP.cs:626`.
    pub fn sndeffect_vol_intpl(&mut self, obj_no: i32, _attr: i32, ticks: i32, to: i32) {
        let sound_volume = self.settings.sound_volume;
        for c in &mut self.sfx_channels {
            if c.obj_no == obj_no {
                c.snd_effect_vol = to;
                self.backend.set_volume(
                    c.sound_id,
                    normalize_volume(to) * sound_volume,
                    convert_tick_to_millisec(ticks),
                );
            }
        }
    }

    /// `FF9SOUND_SNDEFFECT_VOL_ALL`, `ASDP.cs:638`.
    pub fn sndeffect_vol_all(&mut self, vol: i32) {
        let sound_volume = self.settings.sound_volume;
        for c in &mut self.sfx_channels {
            if !is_ext_env_obj_no(c.obj_no) {
                c.snd_effect_vol = vol;
                self.backend
                    .set_volume(c.sound_id, normalize_volume(vol) * sound_volume, 0);
            }
        }
    }

    /// `FF9SOUND_SNDEFFECT_VOL_INTPLALL`, `ASDP.cs:650`.
    pub fn sndeffect_vol_intplall(&mut self, ticks: i32, to: i32) {
        let sound_volume = self.settings.sound_volume;
        for c in &mut self.sfx_channels {
            if !is_ext_env_obj_no(c.obj_no) || c.obj_no == 656 {
                c.snd_effect_vol = to;
                self.backend.set_volume(
                    c.sound_id,
                    normalize_volume(to) * sound_volume,
                    convert_tick_to_millisec(ticks),
                );
            }
        }
    }

    /// Sets the relative play speed of the sound effect started last (`PlaySfxSound`'s pitch,
    /// which `SFX.SoundPlay` gives 0.725 to 1.25 for a spell's sounds).
    pub fn set_last_effect_pitch(&mut self, pitch: f32) {
        if let Some(channel) = self.sfx_channels.last_mut() {
            channel.pitch = pitch;
            self.backend.set_pitch(channel.sound_id, pitch, 0);
        }
    }

    /// `FF9SOUND_SNDEFFECT_PITCH`, `ASDP.cs:662`.
    pub fn sndeffect_pitch(&mut self, obj_no: i32, _attr: i32, pitch: i32) {
        let factor = self.settings.fast_forward_factor;
        for c in &mut self.sfx_channels {
            if c.obj_no == obj_no && !is_ext_env_obj_no(c.obj_no) {
                c.pitch = normalize_pitch(pitch);
                if is_shift_pitch_in_fast_forward(obj_no) {
                    c.pitch = factor as f32;
                }
                self.backend.set_pitch(c.sound_id, c.pitch, 0);
            }
        }
    }

    /// `UpdatePlayingSoundEffectVolume`, `ASDP.cs:44`.
    pub fn update_playing_sound_effect_volume(&mut self) {
        let sound_volume = self.settings.sound_volume;
        for s in self.sfx_res_slot.iter().flatten() {
            if self.backend.is_exist(s.sound_id) {
                self.backend.set_volume(
                    s.sound_id,
                    normalize_volume(s.snd_effect_vol) * sound_volume,
                    0,
                );
            }
        }
        for c in &self.sfx_channels {
            if self.backend.is_exist(c.sound_id) && c.obj_no != 1261 && c.obj_no != 3096 {
                self.backend.set_volume(
                    c.sound_id,
                    normalize_volume(c.snd_effect_vol) * sound_volume,
                    0,
                );
            }
        }
        let song = self.current_song_id;
        self.with_profile(song, ProfileType::Song, |be, p, s| {
            be.set_volume(p.sound_id, p.sound_volume * s.sound_volume, 0);
        });
    }

    // --- resident sound effect slots (`SNDEFFECTRES_*`) ----------------------------------

    /// `OnSndEffectResPlay`, `FF9Snd.cs:367`.
    fn hook_res_play(&mut self, obj_no: i32, attr: i32, pos: i32, vol: i32) {
        let ext = ext_env_obj_no(obj_no);
        if ext != -1 {
            self.sndeffect_stop(ext, attr);
            self.sndeffect_play(ext, attr, pos, vol);
            if ext == 618 {
                self.sndeffect_stop(410, attr);
                self.sndeffect_play(410, attr, pos, vol);
            }
        }
    }

    /// `FF9SOUND_SNDEFFECTRES_PLAY`, `ASDP.cs:699`.
    pub fn sndeffectres_play(&mut self, slot: usize, obj_no: i32, attr: i32, pos: i32, vol: i32) {
        if self.res_hooks_installed {
            self.hook_res_play(obj_no, attr, pos, vol);
        }
        if let Some(existing) = self.sfx_res_slot[slot].clone() {
            if existing.obj_no == obj_no && !existing.is_suspend {
                if let Some(s) = &mut self.sfx_res_slot[slot] {
                    s.prev_snd_effect_map_id = self.fld_map_no;
                }
                return;
            }
            if self.profile_exists(existing.obj_no, ProfileType::SoundEffect) {
                self.backend.stop(existing.sound_id, 0);
            }
            self.sfx_res_slot[slot] = None;
        }
        if self.ensure_profile(obj_no, ProfileType::SoundEffect) {
            let sound_id = self
                .with_profile(obj_no, ProfileType::SoundEffect, |be, p, s| {
                    Self::static_create_sound(be, p);
                    be.set_volume(p.sound_id, normalize_volume(vol) * s.sound_volume, 0);
                    be.start(p.sound_id, 0);
                    p.sound_id
                })
                .unwrap_or(0);
            let mut s = PlayingSfx::new(obj_no, sound_id);
            s.snd_effect_pos = pos;
            s.snd_effect_vol = vol;
            s.prev_snd_effect_map_id = self.fld_map_no;
            self.sfx_res_slot[slot] = Some(s);
        }
    }

    /// `FF9SOUND_SNDEFFECTRES_STOP`, `ASDP.cs:738`.
    pub fn sndeffectres_stop(&mut self, slot: usize, obj_no: i32, attr: i32) {
        if self.res_hooks_installed {
            let ext = ext_env_obj_no(obj_no);
            if ext != -1 {
                self.sndeffect_stop(ext, attr);
                if ext == 618 {
                    self.sndeffect_stop(410, attr);
                }
            }
        }
        let _ = slot;
        for i in 0..SNDEFFECTRES_SLOT_MAX {
            if let Some(s) = self.sfx_res_slot[i].clone()
                && s.obj_no == obj_no
            {
                self.backend.stop(s.sound_id, 0);
                self.sfx_res_slot[i] = None;
            }
        }
    }

    /// `FF9SOUND_SNDEFFECTRES_STOPCURRENT`, `ASDP.cs:758`.
    pub fn sndeffectres_stopcurrent(&mut self) {
        if self.res_hooks_installed {
            for i in 0..SNDEFFECTRES_SLOT_MAX {
                if let Some(s) = self.sfx_res_slot[i].clone() {
                    let ext = ext_env_obj_no(s.obj_no);
                    if ext != -1 {
                        self.sndeffect_stop(ext, 0);
                        if ext == 618 {
                            self.sndeffect_stop(410, 0);
                        }
                    }
                }
            }
        }
        for i in 0..SNDEFFECTRES_SLOT_MAX {
            if let Some(s) = self.sfx_res_slot[i].take() {
                self.backend.stop(s.sound_id, 0);
            }
        }
    }

    /// `FF9SOUND_SNDEFFECTRES_SUSPEND`, `ASDP.cs:775`.
    pub fn sndeffectres_suspend(&mut self, slot: usize) {
        if self.res_hooks_installed
            && let Some(s) = self.sfx_res_slot[slot].clone()
        {
            let ext = ext_env_obj_no(s.obj_no);
            if ext != -1 {
                self.sndeffect_stop(ext, 0);
                if ext == 618 {
                    self.sndeffect_stop(410, 0);
                }
            }
        }
        let fld = self.fld_map_no;
        if let Some(s) = &mut self.sfx_res_slot[slot] {
            s.prev_snd_effect_map_id = fld;
            s.is_suspend = true;
            self.backend.stop(s.sound_id, 0);
        }
    }

    /// `FF9SOUND_SNDEFFECTRES_RESTORE`, `ASDP.cs:794`.
    pub fn sndeffectres_restore(&mut self, slot: usize) {
        if self.res_hooks_installed
            && let Some(s) = self.sfx_res_slot[slot].clone()
        {
            let ext = ext_env_obj_no(s.obj_no);
            if ext != -1 {
                self.sndeffect_play(ext, 0, s.snd_effect_pos, s.snd_effect_vol);
                if ext == 618 {
                    self.sndeffect_play(410, 0, s.snd_effect_pos, s.snd_effect_vol);
                }
            }
        }
        if let Some(s) = self.sfx_res_slot[slot].clone()
            && s.prev_snd_effect_map_id == self.fld_map_no
        {
            self.sndeffectres_play(slot, s.obj_no, 0, 0, s.snd_effect_vol);
        }
    }

    /// `FF9SOUND_SNDEFFECTRES_VOL`, `ASDP.cs:811`.
    pub fn sndeffectres_vol(&mut self, slot: usize, obj_no: i32, attr: i32, vol: i32) {
        let _ = slot;
        if self.res_hooks_installed {
            let ext = ext_env_obj_no(obj_no);
            if ext != -1 {
                self.sndeffect_vol(ext, attr, vol);
                if ext == 618 {
                    self.sndeffect_vol(410, attr, vol);
                }
            }
        }
        let sound_volume = self.settings.sound_volume;
        for s in self.sfx_res_slot.iter_mut().flatten() {
            if s.obj_no == obj_no {
                s.snd_effect_vol = vol;
                self.backend
                    .set_volume(s.sound_id, normalize_volume(vol) * sound_volume, 0);
            }
        }
    }

    /// `FF9SOUND_SNDEFFECTRES_VOL_INTPL`, `ASDP.cs:831`.
    pub fn sndeffectres_vol_intpl(
        &mut self,
        slot: usize,
        obj_no: i32,
        attr: i32,
        ticks: i32,
        to: i32,
    ) {
        let _ = slot;
        if self.res_hooks_installed {
            let ext = ext_env_obj_no(obj_no);
            if ext != -1 {
                self.sndeffect_vol_intpl(ext, attr, ticks, to);
                if ext == 618 {
                    self.sndeffect_vol_intpl(410, attr, ticks, to);
                }
            }
        }
        let sound_volume = self.settings.sound_volume;
        for s in self.sfx_res_slot.iter_mut().flatten() {
            if s.obj_no == obj_no {
                s.snd_effect_vol = to;
                self.backend.set_volume(
                    s.sound_id,
                    normalize_volume(to) * sound_volume,
                    convert_tick_to_millisec(ticks),
                );
            }
        }
    }

    /// `FF9SOUND_SNDEFFECTRES_VOL_ALL`, `ASDP.cs:851`.
    pub fn sndeffectres_vol_all(&mut self, vol: i32) {
        if self.res_hooks_installed {
            for i in 0..SNDEFFECTRES_SLOT_MAX {
                if let Some(s) = self.sfx_res_slot[i].clone() {
                    let ext = ext_env_obj_no(s.obj_no);
                    if ext != -1 {
                        self.sndeffect_vol(ext, 0, vol);
                    }
                }
            }
        }
        let sound_volume = self.settings.sound_volume;
        for s in self.sfx_res_slot.iter_mut().flatten() {
            s.snd_effect_vol = vol;
            self.backend
                .set_volume(s.sound_id, normalize_volume(vol) * sound_volume, 0);
        }
    }

    /// `FF9SOUND_SNDEFFECTRES_VOL_INTPLALL`, `ASDP.cs:868`.
    pub fn sndeffectres_vol_intplall(&mut self, ticks: i32, to: i32) {
        if self.res_hooks_installed {
            for i in 0..SNDEFFECTRES_SLOT_MAX {
                if let Some(s) = self.sfx_res_slot[i].clone() {
                    let ext = ext_env_obj_no(s.obj_no);
                    if ext != -1 {
                        self.sndeffect_vol_intpl(ext, 0, ticks, to);
                    }
                }
            }
        }
        let sound_volume = self.settings.sound_volume;
        for s in self.sfx_res_slot.iter_mut().flatten() {
            s.snd_effect_vol = to;
            self.backend.set_volume(
                s.sound_id,
                normalize_volume(to) * sound_volume,
                convert_tick_to_millisec(ticks),
            );
        }
    }

    /// `ff9fieldSoundSuspendAllResidentSndEffect`, `FF9Snd.cs:527`.
    pub fn suspend_all_resident_sndeffects(&mut self) {
        for i in 0..SNDEFFECTRES_SLOT_MAX {
            if self.sfx_res_slot[i].is_some() {
                self.sndeffectres_suspend(i);
            }
        }
    }

    /// `ff9fieldSoundRestoreAllResidentSndEffect`, `FF9Snd.cs:535`.
    pub fn restore_all_resident_sndeffects(&mut self) {
        for i in 0..SNDEFFECTRES_SLOT_MAX {
            if self.sfx_res_slot[i].is_some() {
                self.sndeffectres_restore(i);
            }
        }
    }

    // --- streams -------------------------------------------------------------------------

    /// `FF9SOUND_STREAM_PLAY`, `ASDP.cs:885`.
    pub fn stream_play(&mut self, stream_id: i32, _pos: i32, _reverb: i32) {
        self.stream_stop();
        if self.ensure_profile(stream_id, ProfileType::Song) {
            let index = self.with_profile(stream_id, ProfileType::Song, |be, p, s| {
                Self::static_create_sound(be, p);
                be.set_volume(p.sound_id, s.sound_volume, 0);
                be.start(p.sound_id, 0);
                p.sound_index
            });
            if let Some(index) = index {
                self.current_song_id = index;
            }
        }
    }

    /// `FF9SOUND_STREAM_STOP`, `ASDP.cs:911`.
    pub fn stream_stop(&mut self) {
        if self.current_song_id != -1 {
            let song = self.current_song_id;
            self.with_profile(song, ProfileType::Song, |be, p, _| {
                be.stop(p.sound_id, 0);
            });
            self.current_song_id = -1;
        }
    }

    /// `FF9SOUND_STREAM_VOL`, `ASDP.cs:930`.
    pub fn stream_vol(&mut self, vol: i32) {
        if self.current_song_id != -1 {
            let song = self.current_song_id;
            if self.ensure_profile(song, ProfileType::Song) {
                self.with_profile(song, ProfileType::Song, |be, p, s| {
                    p.sound_volume = normalize_volume(vol);
                    be.set_volume(p.sound_id, p.sound_volume * s.sound_volume, 0);
                });
            }
        }
    }

    // --- housekeeping --------------------------------------------------------------------

    /// `ClearSuspendedSounds`, `ASDP.cs:1152`.
    pub fn clear_suspended_sounds(&mut self) {
        self.song_stop(self.suspend_bgm_no);
        self.sfx_res_slot = [None, None];
    }

    /// `AllSoundDispatchPlayer.Update`, `ASDP.cs:1344`.
    pub fn update(&mut self) {
        if self.current_music_id != -1 {
            self.with_profile(self.current_music_id, ProfileType::Music, |_, p, _| {
                p.start_play_time += 20.0;
                if p.start_play_time > 245_000.0 {
                    p.start_play_time %= 245_000.0;
                }
            });
        }
        let backend = &mut self.backend;
        self.sfx_channels.retain(|c| {
            if backend.is_exist(c.sound_id) {
                true
            } else {
                backend.stop(c.sound_id, 0);
                false
            }
        });
    }

    // --- FF9Snd dispatch -----------------------------------------------------------------

    /// `FF9Snd.FF9Sound`, `FF9Snd.cs:165`: routes through the current dispatch function.
    pub fn ff9_sound(
        &mut self,
        sound_code: i32,
        obj_no: i32,
        arg1: i32,
        arg2: i32,
        arg3: i32,
    ) -> i32 {
        match self.dispatch_mode {
            DispatchMode::All => self.ff9_all_sound_dispatch(sound_code, obj_no, arg1, arg2, arg3),
            DispatchMode::Field => {
                self.ff9_field_sound_dispatch(sound_code, obj_no, arg1, arg2, arg3)
            }
            DispatchMode::World => {
                self.ff9_world_sound_dispatch(sound_code, obj_no, arg1, arg2, arg3)
            }
        }
    }

    /// `FF9Snd.ParameterChanger`, `FF9Snd.cs:186`: id remaps applied before every dispatch.
    fn parameter_changer(
        &mut self,
        parm_type: i32,
        obj_no: &mut i32,
        arg1: i32,
        arg2: i32,
        arg3: i32,
    ) {
        if (parm_type >> 12) & 3 == 0 {
            match *obj_no {
                93 => *obj_no = 5,
                137 => *obj_no = 10,
                78 => *obj_no = 22,
                _ => {}
            }
        }
        if (parm_type >> 12) & 3 == 1 {
            match *obj_no {
                3067 | 3068 => *obj_no = 1748,
                293 => *obj_no = 633,
                679 => *obj_no = 9010197,
                1230 | 1231 => {
                    self.ff9_all_sound_dispatch(parm_type, 9000023, arg1, arg2, arg3);
                    self.ff9_all_sound_dispatch(parm_type, 9000024, arg1, arg2, arg3);
                    self.ff9_all_sound_dispatch(parm_type, 9000025, arg1, arg2, arg3);
                }
                681 => *obj_no = 3092,
                672 => *obj_no = 9010138,
                3084 => *obj_no = 370,
                _ => {}
            }
        }
    }

    /// `FF9Snd.FF9AllSoundDispatch`, `FF9Snd.cs:543`. Always returns 0.
    pub fn ff9_all_sound_dispatch(
        &mut self,
        parm_type: i32,
        obj_no: i32,
        arg1: i32,
        arg2: i32,
        arg3: i32,
    ) -> i32 {
        // `SetSfxResDelegate`: the resident-effect hooks exist from the first dispatch on.
        self.res_hooks_installed = true;
        let mut obj_no = obj_no;
        let parm_type = parm_type & 65535;
        let slot = ((parm_type >> 6) & 1) as usize;
        self.parameter_changer(parm_type, &mut obj_no, arg1, arg2, arg3);
        let num = parm_type & 65471;
        use code::*;
        match num {
            SONG_PLAY => self.song_play(obj_no, 127, 0),
            SONG_STOP => self.song_stop(obj_no),
            SONG_STOPCURRENT => self.song_stop_current(),
            SONG_LOAD => self.song_load(obj_no),
            SONG_SUSPEND => self.song_suspend(obj_no, false),
            SONG_RESTORE => self.song_restore(),
            SONG_JUMPPOINT => self.song_jumppoint(),
            SNDEFFECTRES_STOPCURRENT => self.sndeffectres_stopcurrent(),
            SNDEFFECTRES_SUSPEND => self.sndeffectres_suspend(slot),
            SNDEFFECTRES_RESTORE => self.sndeffectres_restore(slot),
            STREAM_STOP => self.stream_stop(),
            SONG_VOL => self.song_vol(obj_no, arg1),
            SONG_PITCH => self.song_pitch(obj_no, arg1),
            SONG_SKIPPHRASE => {
                let offset = self.skipphrase_offset_ms(obj_no, arg1);
                if offset != -1 {
                    self.song_skipphrase_millisec(offset);
                }
            }
            SNDEFFECT_STOP => self.sndeffect_stop(obj_no, arg1),
            SNDEFFECTRES_STOP => self.sndeffectres_stop(slot, obj_no, arg1),
            SNDEFFECT_VOL_ALL => self.sndeffect_vol_all(arg1),
            SNDEFFECTRES_VOL_ALL => self.sndeffectres_vol_all(arg1),
            STREAM_VOL => self.stream_vol(arg1),
            SONG_VOL_INTPL => self.song_vol_intpl(obj_no, arg1, arg2),
            SONG_PITCH_INTPL => self.song_pitch_intpl(obj_no, arg1, arg2),
            SONG_VOL_INTPLALL => self.song_vol_intplall(arg1, arg2),
            SNDEFFECT_VOL => self.sndeffect_vol(obj_no, arg1, arg2),
            SNDEFFECT_PITCH => self.sndeffect_pitch(obj_no, arg1, arg2),
            SNDEFFECTRES_VOL => self.sndeffectres_vol(slot, obj_no, arg1, arg2),
            SNDEFFECT_VOL_INTPLALL => self.sndeffect_vol_intplall(arg1, arg2),
            SNDEFFECTRES_VOL_INTPLALL => self.sndeffectres_vol_intplall(arg1, arg2),
            STREAM_PLAY => {
                if !self.settings.sound_effect_is_mute {
                    self.stream_play(obj_no, arg1, arg2);
                }
            }
            SONG_VOL_FADE => self.song_vol_fade(obj_no, arg1, arg2, arg3),
            SONG_PITCH_FADE => self.song_pitch_fade(obj_no, arg1, arg2, arg3),
            SONG_VOL_FADEALL => self.song_vol_fadeall(arg1, arg2, arg3),
            SNDEFFECT_PLAY => self.sndeffect_play(obj_no, arg1, arg2, arg3),
            SNDEFFECTRES_PLAY => self.sndeffectres_play(slot, obj_no, arg1, arg2, arg3),
            SNDEFFECT_VOL_INTPL => self.sndeffect_vol_intpl(obj_no, arg1, arg2, arg3),
            SNDEFFECTRES_VOL_INTPL => self.sndeffectres_vol_intpl(slot, obj_no, arg1, arg2, arg3),
            // Cases the source logs as "stubbed", "ignore", "no implementation" or unknown.
            _ => {}
        }
        0
    }

    /// `GetOffsetMillisecondFromFF9SOUND_SONG_SKIPPHRASE`, `FF9Snd.cs:335`.
    fn skipphrase_offset_ms(&self, obj_no: i32, phrase: i32) -> i32 {
        let field = self.game_mode == 1;
        if field
            && (self.fld_map_no == 613 || self.fld_map_no == 615)
            && obj_no == 113
            && phrase == 14
        {
            48000
        } else if field && self.fld_map_no == 3010 && obj_no == 145 && phrase == 11 {
            16500
        } else {
            -1
        }
    }

    /// `FF9Snd.FF9FieldSoundDispatch`, `FF9Snd.cs:1381`.
    pub fn ff9_field_sound_dispatch(
        &mut self,
        parm_type: i32,
        obj_no: i32,
        arg1: i32,
        arg2: i32,
        arg3: i32,
    ) -> i32 {
        let parm_type = parm_type & 65535;
        let mut obj_no = obj_no;
        let mut arg3 = arg3;
        self.parameter_changer(parm_type, &mut obj_no, arg1, arg2, arg3);
        let num = parm_type & 65471;
        if num != code::SONG_PLAY && num != code::SONG_LOAD {
            if num == code::SNDEFFECT_PLAY {
                self.played_field_sound_effect.insert(obj_no);
                let map = self.fld_map_no;
                if (map == 1911 || map == 911) && obj_no == 1395 {
                    self.sndeffect_stop(1394, 0);
                }
                if map == 157 && obj_no == 3064 {
                    self.sndeffect_play(9050122, 0, 0, arg3);
                    self.sndeffect_play(9050123, 0, 0, arg3);
                    arg3 /= 2;
                    self.played_field_sound_effect.insert(9050122);
                    self.played_field_sound_effect.insert(9050123);
                }
                if (map == 302 || map == 303 || map == 304) && obj_no == 2491 {
                    obj_no = 77;
                }
            }
        } else {
            self.bgm_field_song_counter += 1;
        }
        self.ff9_all_sound_dispatch(parm_type, obj_no, arg1, arg2, arg3)
    }

    /// `FF9Snd.FF9WorldSoundDispatch`, `FF9Snd.cs:2075`. The two `if`s run in sequence, so 890
    /// ends at 2728.
    pub fn ff9_world_sound_dispatch(
        &mut self,
        parm_type: i32,
        obj_no: i32,
        arg1: i32,
        arg2: i32,
        arg3: i32,
    ) -> i32 {
        let parm_type = parm_type & 65535;
        let mut obj_no = obj_no;
        self.parameter_changer(parm_type, &mut obj_no, arg1, arg2, arg3);
        if parm_type & 65471 == code::SNDEFFECT_PLAY {
            if obj_no == 890 {
                obj_no = 891;
            }
            if obj_no == 891 {
                obj_no = 2728;
            }
        }
        self.ff9_all_sound_dispatch(parm_type, obj_no, arg1, arg2, arg3)
    }

    /// `ff9fieldsound_stopall_mapsndeffect`, `FF9Snd.cs:1419`.
    pub fn stopall_map_sndeffects(&mut self) {
        let played = std::mem::take(&mut self.played_field_sound_effect);
        for obj in played {
            self.sndeffect_stop(obj, 0);
        }
    }
}
