//! What the play loop asks the sound system to do, and the director that does it.
//!
//! `PlayGame` turns script sound opcodes, field changes, battles and menu presses into
//! [`AudioCue`]s. [`AudioDirector`] feeds them to the ported `SoundSystem`
//! (`tantalus_legacy::sound`), which renders interleaved stereo f32. Nothing here opens an audio
//! device: the client plays the rendered samples, `capture-play --audio-log` prints the cues.

use tantalus_legacy::HostEvent;
use tantalus_legacy::sound::{Backend, DispatchMode, SoundProjector, SoundSystem, code};

use crate::sound_bank::{BattleSongTable, SoundBank, clip_loader, music_for_battle};

/// `FF9SFX_Play` (`Global/ff9/FF9Sfx.cs:5`): `ff9snd_sndeffect_play(id, 8388608, 127, 128)`.
const SFX_ATTR: i32 = 8_388_608;
const SFX_VOLUME: i32 = 127;
const SFX_POSITION: i32 = 128;

/// Sound effect ids of the menu and dialog cursors (`FF9Sfx.FF9SFX_Play(103/101/102)` across
/// `Global/*UI.cs`, `MainMenuUI.cs`): 103 moves the cursor and confirms, 101 goes back, 102 refuses.
pub mod menu_sound {
    pub const CONFIRM: i32 = 103;
    pub const CANCEL: i32 = 101;
    pub const REFUSE: i32 = 102;
    pub const CURSOR: i32 = 103;
    /// `BattleResultUI`: a character gained a level.
    pub const LEVEL_UP: i32 = 683;
    /// `BattleResultUI.ApLearned`: an ability was learned.
    pub const ABILITY_LEARNED: i32 = 1043;
}

/// `QuadMistGame.MINIINT_SONG_DEFAULT`: the song a card game plays.
pub const CARD_SONG: i32 = 66;
/// Song ids `btl_util.ManageBattleSong` plays when a battle ends (`battle.cs:381`, `:276`).
pub const VICTORY_SONG: i32 = 5;
pub const DEFEAT_SONG: i32 = 6;
/// Ticks the battle song fades for before the end song starts (`ManageBattleSong(sys, 30, ..)`).
pub const BATTLE_SONG_FADE_TICKS: i32 = 30;

/// How a battle ended, as far as its music goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BattleEnd {
    /// `ManageBattleSong(sys, 30, 5)`: the victory fanfare.
    Victory,
    /// `ManageBattleSong(sys, 30, 6)`: the game over song.
    Defeat,
    /// Fled or scripted: no end song (`ff9ShutdownStateBattleResult` fades the battle song).
    Escape,
}

impl BattleEnd {
    /// From a `FieldBattles` outcome name.
    pub fn from_outcome(outcome: &str) -> Self {
        match outcome {
            "victory" => Self::Victory,
            "defeat" => Self::Defeat,
            _ => Self::Escape,
        }
    }
}

/// One thing the sound system should do.
#[derive(Debug, Clone, PartialEq)]
pub enum AudioCue {
    /// A script sound opcode or position/flag event, as the event VM raised it.
    Script(HostEvent),
    /// A field started: sets the dispatcher's field state and stops the old field's map sounds
    /// (`ff9fieldsound_stopall_mapsndeffect`, `FF9Snd.cs:1419`).
    FieldStarted { map: i32 },
    /// The new field's init script has run (`HonoluluFieldMain.Start` after `StartEvents`,
    /// `HonoluluFieldMain.cs:141`): a song set aside before the field and still set aside comes
    /// back, any other is dropped, and the world song stops when the field played none.
    FieldReady,
    /// The world map plays the song of the ride (`ff9.w_musicPlay`, `ff9.cs:6433`): on
    /// entering the world map and on each change of ride (`w_musicRequestSend`). `song` is
    /// already `w_musicChoice`'s pick.
    WorldSong { song: i32 },
    /// A battle begins from field `map` (or world map `map` when `world`): picks its song from
    /// `BtlEncountBgmMetaData` or `WldBtlEncountBgmMetaData`
    /// (`BattleSwirl.RequestPlayBattleEncounterSongForField` / `ForWorld`, `BattleSwirl.cs:88`).
    BattleStarted { map: i32, battle: i32, world: bool },
    /// The battle ended; a won or lost battle plays its end song first.
    BattleEnded { end: BattleEnd },
    /// The result screen closed (`BattleResultUI.Hide` -> `battle.ff9ShutdownStateBattleResult`):
    /// the end song stops and the song set aside returns.
    BattleLeft,
    /// A Tetra Master game opens: the field song is set aside and the card song plays
    /// (`QuadMistGame.cs:156`, `ff9minisnd_song_play(66)`).
    CardStarted,
    /// The game is over: the card song stops and the field song returns (`QuadMistGame.cs:726`).
    CardEnded,
    /// A user interface sound effect by id (`FF9SFX_Play`).
    Effect { id: i32 },
    /// `SetSoundPosition` or `SetSoundObjectPosition` with the point already put through the
    /// field camera (`FF9FieldSoundGetPositionVolume`): the next effect's pan and volume.
    SoundPosition { pos: i32, vol: i32, volume: i32 },
    /// `btl_util.SetBattleSfx`: a battle sound effect by id at `volume` (0 to 127).
    BattleEffect { id: i32, volume: i32 },
    /// A sound effect by resource name (a spell effect's own sounds, `SFX.SoundPlay`).
    EffectSound { resource: String, pitch: f32 },
    /// The Config screen's music and sound volumes, 0 to 100 (`Configuration.Audio`).
    Volumes { music: i32, sound: i32 },
    /// A movie's sound starts (`Some`: the sound resource) or stops (`None`).
    Movie { resource: Option<String> },
}

/// The song id a movie's sound is registered under (above every real song id).
const MOVIE_STREAM_ID: i32 = 9000;
/// The first sound effect id handed out to an effect sound known by resource name.
const EFFECT_SOUND_BASE: i32 = 20_000;
/// `SoundLib.PlaySfxSound(.., 1f, ..)`: full volume.
const FULL_EFFECT_VOLUME: u8 = 127;

/// One line of the `--audio-log`: what the cue means in the reference's terms.
pub fn describe_cue(cue: &AudioCue, bank: &SoundBank) -> String {
    let music = |id: i32| {
        bank.metadata.music.get(&id).map_or_else(
            || format!("music {id} (unnamed)"),
            |n| format!("music {id} {n}"),
        )
    };
    let effect = |id: i32| {
        bank.metadata.sound_effect.get(&id).map_or_else(
            || format!("effect {id} (unnamed)"),
            |n| format!("effect {id} {n}"),
        )
    };
    match cue {
        AudioCue::Script(HostEvent::RunSound {
            code: sound_code,
            sound,
            arguments,
        }) => {
            let op = sound_code & 65471;
            let [a, b, c] = *arguments;
            let kind = (op >> 12) & 3;
            let subject = match kind {
                0 => music(*sound),
                1 => effect(*sound),
                _ => format!("object {sound}"),
            };
            format!(
                "sound code {op} ({}) {subject} args {a},{b},{c}",
                code_name(op)
            )
        }
        AudioCue::Script(HostEvent::SetSoundPosition { position, volume }) => {
            format!("sound position {position:?} volume {volume}")
        }
        AudioCue::Script(HostEvent::SetSoundObjectPosition { object, volume }) => {
            format!("sound object position {object:?} volume {volume}")
        }
        AudioCue::Script(HostEvent::SetBattleMusicContinuation { enabled }) => {
            format!("continue field song after battle: {enabled}")
        }
        AudioCue::Script(other) => format!("{other:?}"),
        AudioCue::FieldStarted { map } => format!("field {map} started"),
        AudioCue::FieldReady => "field init done: restore or drop the song set aside".to_owned(),
        AudioCue::WorldSong { song } => format!("world map: play {}", music(*song)),
        AudioCue::BattleStarted { map, battle, world } => {
            let (table, place) = if *world {
                (&bank.world_battle_songs, "world map")
            } else {
                (&bank.field_battle_songs, "field")
            };
            let song = music_for_battle(table, *map, *battle);
            if song < 0 {
                format!("battle {battle} from {place} {map}: no battle song, the song goes on")
            } else {
                format!("battle {battle} from {place} {map}: play {}", music(song))
            }
        }
        AudioCue::BattleEnded { end } => match end {
            BattleEnd::Victory => format!("battle won: fade, play {}", music(VICTORY_SONG)),
            BattleEnd::Defeat => format!("battle lost: fade, play {}", music(DEFEAT_SONG)),
            BattleEnd::Escape => {
                "battle ended without an end song: fade the battle song".to_owned()
            }
        },
        AudioCue::BattleLeft => "result screen closed: the song set aside returns".to_owned(),
        AudioCue::CardStarted => format!("card game: play {}", music(CARD_SONG)),
        AudioCue::CardEnded => "card game over: the field song returns".to_owned(),
        AudioCue::Effect { id } => format!("play {}", effect(*id)),
        AudioCue::EffectSound { resource, .. } => format!("effect sound {resource}"),
        AudioCue::Volumes { music, sound } => format!("music volume {music}, sound volume {sound}"),
        AudioCue::BattleEffect { id, volume } => {
            format!("battle {} at volume {volume}", effect(*id))
        }
        AudioCue::SoundPosition { pos, vol, volume } => {
            format!("sound position pan {pos} volume {vol} script volume {volume}")
        }
        AudioCue::Movie { resource } => match resource {
            Some(resource) => format!("movie sound {resource} starts"),
            None => "movie sound stops".to_owned(),
        },
    }
}

fn code_name(op: i32) -> &'static str {
    match op {
        code::SONG_PLAY => "song play",
        code::SONG_STOP => "song stop",
        code::SONG_STOPCURRENT => "song stop current",
        code::SONG_LOAD => "song load",
        code::SONG_SUSPEND => "song suspend",
        code::SONG_RESTORE => "song restore",
        code::SONG_VOL => "song volume",
        code::SONG_VOL_INTPL => "song volume fade",
        code::SONG_VOL_INTPLALL => "all songs volume fade",
        code::SONG_PITCH => "song pitch",
        code::SNDEFFECT_PLAY => "effect play",
        code::SNDEFFECTRES_PLAY => "resident effect play",
        code::SNDEFFECTRES_STOP => "resident effect stop",
        code::SNDEFFECTRES_VOL_INTPL => "resident effect volume fade",
        code::SNDEFFECT_NULL => "ignored",
        code::SONG_NULL => "ignored",
        code::SNDEFFECT_STOP => "effect stop",
        code::SNDEFFECT_VOL => "effect volume",
        code::SNDEFFECT_PITCH => "effect pitch",
        code::SNDEFFECT_VOL_ALL => "all effects volume",
        _ => "other",
    }
}

/// `FF9FieldSoundGetPositionVolume` needs the field camera and GTE projection, which the play
/// loop does not hand over: a centered, full-volume position (the dispatcher stores the position
/// and never pans with it).
struct CenteredProjector;

impl SoundProjector for CenteredProjector {
    fn project_position(&mut self, _position: [i32; 3]) -> (i32, i32) {
        (SFX_POSITION, SFX_VOLUME)
    }
    fn project_object(&mut self, _object: tantalus_contracts::EntityId) -> Option<(i32, i32)> {
        Some((SFX_POSITION, SFX_VOLUME))
    }
}

/// A cue waiting for the battle song to finish fading.
#[derive(Debug, Clone, Copy)]
struct Pending {
    ticks_left: i32,
    song: i32,
}

/// Runs cues against a `SoundSystem` and renders it.
pub struct AudioDirector {
    pub system: SoundSystem,
    field_battle_songs: BattleSongTable,
    world_battle_songs: BattleSongTable,
    pending: Option<Pending>,
    /// On the world map (`EventEngine.gMode == 3`) since the last `WorldSong`.
    in_world: bool,
    /// `FF9Snd.LatestWorldPlayedSong`.
    latest_world_song: i32,
    /// The song set aside when the field started (`HonoluluFieldMain.cs:129`), until `FieldReady`.
    field_suspend_before: Option<i32>,
    /// Sound ids handed out to effect sounds by resource name.
    effect_sounds: Vec<(i32, String)>,
}

impl AudioDirector {
    /// `sample_rate` must be a multiple of 60 (one script tick is a whole number of frames).
    pub fn new(bank: SoundBank, sample_rate: u32) -> Result<Self, String> {
        let field_battle_songs = bank.field_battle_songs.clone();
        let world_battle_songs = bank.world_battle_songs.clone();
        let metadata = bank.metadata.clone();
        let mut system = SoundSystem::new(Backend::new(sample_rate)?, metadata);
        system.clip_loader = clip_loader(bank);
        let mut director = Self::with_system(system, field_battle_songs);
        director.world_battle_songs = world_battle_songs;
        Ok(director)
    }

    /// A director over a ready `SoundSystem` (its clips and metadata already set).
    pub fn with_system(mut system: SoundSystem, field_battle_songs: BattleSongTable) -> Self {
        system.dispatch_mode = DispatchMode::Field;
        system.game_mode = 1;
        Self {
            system,
            field_battle_songs,
            world_battle_songs: BattleSongTable::new(),
            pending: None,
            in_world: false,
            latest_world_song: -1,
            field_suspend_before: None,
            effect_sounds: Vec::new(),
        }
    }

    /// Applies one cue.
    pub fn apply(&mut self, cue: &AudioCue) {
        match cue {
            AudioCue::Script(event) => {
                self.system.apply_host_event(event, &mut CenteredProjector);
            }
            AudioCue::FieldStarted { map } => {
                self.system.stopall_map_sndeffects();
                self.in_world = false;
                self.system.dispatch_mode = DispatchMode::Field;
                self.system.game_mode = 1;
                self.system.fld_map_no = *map;
                self.system.bgm_field_song_counter = 0;
                self.field_suspend_before = Some(self.system.suspend_song_id());
            }
            AudioCue::FieldReady => self.field_ready(),
            AudioCue::WorldSong { song } => self.world_song(*song),
            AudioCue::BattleStarted { map, battle, world } => {
                self.battle_started(*map, *battle, *world)
            }
            AudioCue::BattleEnded { end } => self.battle_ended(*end),
            AudioCue::BattleLeft => self.battle_left(),
            AudioCue::CardStarted => self.card_started(),
            AudioCue::CardEnded => self.card_ended(),
            AudioCue::Volumes { music, sound } => {
                // `SoundLib.TryUpdateMusicVolume` / `TryUpdateSoundVolume`.
                self.system.settings.music_volume = *music as f32 / 100.0;
                self.system.settings.sound_volume = *sound as f32 / 100.0;
                self.system.update_playing_music_volume();
                self.system.update_playing_sound_effect_volume();
            }
            AudioCue::EffectSound { resource, pitch } => {
                let known = self
                    .effect_sounds
                    .iter()
                    .find(|(_, name)| name == resource)
                    .map(|(id, _)| *id);
                let id = known.unwrap_or_else(|| {
                    let id = EFFECT_SOUND_BASE + self.effect_sounds.len() as i32;
                    self.system
                        .metadata
                        .sound_effect
                        .insert(id, resource.clone());
                    self.effect_sounds.push((id, resource.clone()));
                    id
                });
                self.system.ff9_sound(
                    code::SNDEFFECT_PLAY,
                    id,
                    0,
                    0,
                    i32::from(FULL_EFFECT_VOLUME),
                );
                // `PlaySfxSound(.., pitch)`: the sound plays at `pitch` times its own speed.
                if (*pitch - 1.0).abs() > f32::EPSILON {
                    self.system.set_last_effect_pitch(*pitch);
                }
            }
            AudioCue::BattleEffect { id, volume } => {
                // `ff9btlsnd_sndeffect_play(id, 0, volume, S_SeGetPos)`: no attribute bits; the
                // dispatcher takes no pan for it.
                self.system
                    .ff9_sound(code::SNDEFFECT_PLAY, *id, 0, 0, *volume);
            }
            AudioCue::SoundPosition { pos, vol, volume } => {
                self.system.set_se_position_volume(*pos, *vol, *volume);
            }
            AudioCue::Movie { resource } => match resource {
                Some(resource) => {
                    self.system
                        .metadata
                        .song
                        .insert(MOVIE_STREAM_ID, resource.clone());
                    self.system.stream_play(MOVIE_STREAM_ID, 0, 0);
                }
                None => self.system.stream_stop(),
            },
            AudioCue::Effect { id } => {
                self.system.ff9_sound(
                    code::SNDEFFECT_PLAY,
                    *id,
                    SFX_ATTR,
                    SFX_POSITION,
                    SFX_VOLUME,
                );
            }
        }
    }

    /// `HonoluluFieldMain.cs:141-151`, after the field's `StartEvents`.
    fn field_ready(&mut self) {
        let Some(before) = self.field_suspend_before.take() else {
            return;
        };
        let now = self.system.suspend_song_id();
        if before != -1 && now != -1 && before == now {
            // `FF9Snd.ff9fldsnd_song_restore`.
            self.system.song_restore();
        } else {
            self.system.stop_and_clear_suspend_bgm(-1, false);
        }
        if self.latest_world_song != -1
            && self.latest_world_song == self.system.current_music_id()
            && self.system.bgm_field_song_counter == 0
        {
            self.system.song_stop_current();
        }
        self.latest_world_song = -1;
        self.system.bgm_field_song_counter = 0;
    }

    /// `ff9.w_musicPlay` (`ff9.cs:6433`): the song set aside comes back when it is the world
    /// song; otherwise the song set aside is dropped and the world song plays.
    fn world_song(&mut self, song: i32) {
        self.in_world = true;
        self.system.dispatch_mode = DispatchMode::World;
        self.system.game_mode = 3;
        let suspended = self.system.suspend_song_id();
        if song == suspended && suspended != -1 {
            self.system.song_restore();
            let restored = self.system.current_music_id();
            if restored >= 0 {
                self.system.song_vol(restored, 127);
            }
        } else {
            self.system.stop_and_clear_suspend_bgm(-1, false);
            self.system.song_play(song, 127, 0);
        }
        self.latest_world_song = song;
    }

    /// `HonoluluFieldMain.cs:313` and `BattleSwirl.RequestPlayBattleEncounterSongForField` /
    /// `ForWorld` (`BattleSwirl.cs:88`): the song is set aside and the table's song plays, unless
    /// the table has none or that song already plays.
    fn battle_started(&mut self, map: i32, battle: i32, world: bool) {
        self.system.dispatch_mode = DispatchMode::All;
        let table = if world {
            &self.world_battle_songs
        } else {
            &self.field_battle_songs
        };
        let song = music_for_battle(table, map, battle);
        let current = self.system.current_music_id();
        if song < 0 || song == current {
            return;
        }
        if current >= 0 {
            self.system.song_suspend(current, false);
        }
        self.system.song_play(song, 127, 0);
    }

    /// `btl_util.ManageBattleSong` (`btl_util.cs:244`): every song fades over 30 ticks, the field
    /// song is set aside (unless a battle song is what plays), then the end song starts.
    fn battle_ended(&mut self, end: BattleEnd) {
        let song = match end {
            BattleEnd::Victory => VICTORY_SONG,
            BattleEnd::Defeat => DEFEAT_SONG,
            BattleEnd::Escape => return,
        };
        self.system.song_vol_intplall(BATTLE_SONG_FADE_TICKS, 0);
        self.pending = Some(Pending {
            ticks_left: BATTLE_SONG_FADE_TICKS,
            song,
        });
    }

    /// `QuadMistGame` start: the field song is set aside and song 66 plays.
    fn card_started(&mut self) {
        self.system.dispatch_mode = DispatchMode::All;
        let current = self.system.current_music_id();
        if current == CARD_SONG {
            return;
        }
        if current >= 0 {
            self.system.song_suspend(current, false);
        }
        self.system.song_play(CARD_SONG, 127, 0);
    }

    /// `QuadMistGame` end: song 66 stops and the field song comes back.
    fn card_ended(&mut self) {
        self.finish_battle_return();
    }

    /// The result screen closed (`BattleResultUI.Hide`): the end song stops at once, looping
    /// or not, and the song set aside returns.
    fn battle_left(&mut self) {
        self.finish_battle_return();
    }

    /// `HonoluluFieldMain.cs:145` and `battle.ff9ShutdownStateBattleResult` (`battle.cs:581`): a
    /// field song set aside comes back at full volume; otherwise the battle song fades out
    /// unless a script asked to keep the field song.
    fn finish_battle_return(&mut self) {
        self.pending = None;
        if self.system.suspend_song_id() >= 0 {
            let current = self.system.current_music_id();
            if current >= 0 {
                self.system.song_stop(current);
            }
            self.system.song_restore();
            let restored = self.system.current_music_id();
            if restored >= 0 {
                self.system.song_vol(restored, 127);
            }
        } else if !self.system.continue_field_song_after_battle {
            self.system.song_vol_intplall(120, 0);
            self.system.stop_current_song(120);
        }
        self.system.dispatch_mode = if self.in_world {
            DispatchMode::World
        } else {
            DispatchMode::Field
        };
    }

    /// Renders `ticks` script ticks (1/60 s each) of interleaved stereo f32.
    pub fn render(&mut self, ticks: u32) -> Vec<f32> {
        let mut out = Vec::new();
        for _ in 0..ticks {
            if let Some(pending) = &mut self.pending {
                pending.ticks_left -= 1;
                if pending.ticks_left <= 0 {
                    let song = pending.song;
                    self.pending = None;
                    let current = self.system.current_music_id();
                    // Battle (0), boss battle (35) and faerie battle (61) are not set aside.
                    if current >= 0
                        && self.system.suspend_song_id() < 0
                        && !matches!(current, 0 | 35 | 61)
                    {
                        self.system.song_vol(current, 127);
                        self.system.song_suspend(current, false);
                    }
                    self.system.song_load(song);
                    self.system.song_play(song, 127, 0);
                }
            }
            out.extend(self.system.run_ticks(1));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tantalus_legacy::sound::PcmClip;

    const RATE: u32 = 6000; // 100 frames a tick

    fn clip(value: i16, frames: usize, looped: bool) -> Arc<PcmClip> {
        Arc::new(PcmClip {
            sample_rate: RATE,
            channels: 1,
            samples: vec![value; frames],
            loop_start: 0,
            loop_end: if looped { frames as u32 } else { 0 },
            loop_start_alternate: 0,
            loop_end_alternate: 0,
        })
    }

    fn director() -> AudioDirector {
        let mut metadata = tantalus_legacy::sound::SoundMetadata::default();
        // Field song 10, battle song 0, victory fanfare 5, cursor effect 103.
        for (id, name) in [(10, "field"), (0, "battle"), (5, "fanfare")] {
            metadata.music.insert(id, name.to_owned());
        }
        metadata.sound_effect.insert(103, "cursor".to_owned());
        let mut system = SoundSystem::new(Backend::new(RATE).unwrap(), metadata);
        system
            .clips
            .insert("field".into(), clip(4000, 60_000, true));
        system
            .clips
            .insert("battle".into(), clip(8000, 60_000, true));
        system
            .clips
            .insert("fanfare".into(), clip(12000, 300, false));
        system.clips.insert("cursor".into(), clip(2000, 200, false));
        let table = BTreeMap::from([(7, BTreeMap::from([(3, 0)]))]);
        AudioDirector::with_system(system, table)
    }

    use std::collections::BTreeMap;

    fn play(song: i32) -> AudioCue {
        AudioCue::Script(HostEvent::RunSound {
            code: code::SONG_PLAY,
            sound: song,
            arguments: [0, 0, 0],
        })
    }

    fn level(director: &mut AudioDirector) -> i32 {
        let out = director.render(2);
        out.iter()
            .map(|v| (v * 32768.0).abs() as i32)
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn script_song_plays_and_loops_past_its_length() {
        let mut d = director();
        d.apply(&play(10));
        assert_eq!(d.system.current_music_id(), 10);
        // 60,000 frames at 100 a tick is 600 ticks; the loop keeps it going well past that.
        let _ = d.render(700);
        assert!(level(&mut d) > 0);
    }

    #[test]
    fn menu_cursor_effect_is_audible() {
        let mut d = director();
        d.apply(&AudioCue::Effect { id: 103 });
        assert!(level(&mut d) > 0);
    }

    #[test]
    fn battle_takes_the_song_and_the_field_song_returns() {
        let mut d = director();
        d.apply(&play(10));
        let _ = d.render(5);
        d.apply(&AudioCue::BattleStarted {
            map: 7,
            battle: 3,
            world: false,
        });
        assert_eq!(d.system.current_music_id(), 0);
        assert_eq!(d.system.suspend_song_id(), 10);
        d.apply(&AudioCue::BattleEnded {
            end: BattleEnd::Victory,
        });
        // 30 ticks of fade, then the fanfare, which goes on (the game's loops) until the
        // result screen closes; then the field song again.
        let _ = d.render(31);
        assert_eq!(d.system.current_music_id(), VICTORY_SONG);
        let _ = d.render(200);
        assert_eq!(d.system.current_music_id(), VICTORY_SONG);
        d.apply(&AudioCue::BattleLeft);
        assert_eq!(d.system.current_music_id(), 10);
        assert_eq!(d.system.suspend_song_id(), -1);
        assert!(level(&mut d) > 0);
    }

    #[test]
    fn the_world_song_replaces_the_field_song_and_a_field_without_a_song_stops_it() {
        let mut d = director();
        d.system.metadata.music.insert(69, "battle".to_owned());
        d.apply(&play(10));
        d.apply(&AudioCue::WorldSong { song: 69 });
        assert_eq!(d.system.current_music_id(), 69);
        assert_eq!(d.system.game_mode, 3);
        // A world battle with a table song sets the world song aside; it comes back.
        d.world_battle_songs = BTreeMap::from([(9000, BTreeMap::from([(359, 0)]))]);
        d.apply(&AudioCue::BattleStarted {
            map: 9000,
            battle: 359,
            world: true,
        });
        assert_eq!(d.system.current_music_id(), 0);
        assert_eq!(d.system.suspend_song_id(), 69);
        d.apply(&AudioCue::BattleEnded {
            end: BattleEnd::Victory,
        });
        let _ = d.render(31);
        assert_eq!(d.system.current_music_id(), VICTORY_SONG);
        d.apply(&AudioCue::BattleLeft);
        assert_eq!(d.system.current_music_id(), 69);
        // A field whose init plays no song: the world song stops.
        d.apply(&AudioCue::FieldStarted { map: 7 });
        d.apply(&AudioCue::FieldReady);
        assert_eq!(d.system.current_music_id(), -1);
        // A field whose init plays its song: that song plays.
        d.apply(&AudioCue::WorldSong { song: 69 });
        d.apply(&AudioCue::FieldStarted { map: 8 });
        d.apply(&play(10));
        d.apply(&AudioCue::FieldReady);
        assert_eq!(d.system.current_music_id(), 10);
    }

    #[test]
    fn a_battle_without_a_table_song_leaves_the_field_song_alone() {
        let mut d = director();
        d.apply(&play(10));
        d.apply(&AudioCue::BattleStarted {
            map: 7,
            battle: 99,
            world: false,
        });
        assert_eq!(d.system.current_music_id(), 10);
        assert_eq!(d.system.suspend_song_id(), -1);
    }

    #[test]
    fn an_effect_sound_by_resource_name_plays_and_is_reused() {
        let mut d = director();
        d.system
            .clips
            .insert("spell".into(), clip(9000, 400, false));
        let cue = AudioCue::EffectSound {
            resource: "spell".to_owned(),
            pitch: 0.75,
        };
        d.apply(&cue);
        assert!(d.render(1).iter().any(|sample| sample.abs() > 0.0));
        d.apply(&cue);
        assert_eq!(d.effect_sounds.len(), 1);
    }

    /// Runs only with `FFIX_ASSETS` and `FFIX_GAME_DATA` set: a real spell sound decodes and plays.
    #[test]
    fn a_real_spell_sound_is_heard() {
        let (Some(assets), Some(data)) = (
            std::env::var_os("FFIX_ASSETS"),
            std::env::var_os("FFIX_GAME_DATA"),
        ) else {
            return;
        };
        let bank =
            SoundBank::open(std::path::Path::new(&assets), std::path::Path::new(&data)).unwrap();
        let mut d = AudioDirector::new(bank, 48_000).unwrap();
        d.apply(&AudioCue::EffectSound {
            resource: "Sounds02/SE02/se020036".to_owned(),
            pitch: 1.0,
        });
        let peak = d
            .render(30)
            .iter()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
        println!("PEAK {peak}");
        assert!(peak > 0.01);
        // A weapon swing (Zidane's dagger) and an enemy death effect.
        for id in [899, 21, 121] {
            d.apply(&AudioCue::BattleEffect { id, volume: 127 });
            let peak = d
                .render(30)
                .iter()
                .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
            assert!(peak > 0.005, "sound effect {id} is silent");
        }
    }

    #[test]
    fn a_card_game_takes_the_song_and_the_field_song_returns() {
        let mut d = director();
        d.system
            .metadata
            .music
            .insert(CARD_SONG, "battle".to_owned());
        d.apply(&play(10));
        let _ = d.render(5);
        d.apply(&AudioCue::CardStarted);
        assert_eq!(d.system.current_music_id(), CARD_SONG);
        assert_eq!(d.system.suspend_song_id(), 10);
        d.apply(&AudioCue::CardEnded);
        assert_eq!(d.system.current_music_id(), 10);
        assert_eq!(d.system.suspend_song_id(), -1);
    }

    /// Runs only with `FFIX_ASSETS` and `FFIX_GAME_DATA` set: every card game sound is heard,
    /// and the card song plays.
    #[test]
    fn real_card_game_sounds_and_song_are_heard() {
        let (Some(assets), Some(data)) = (
            std::env::var_os("FFIX_ASSETS"),
            std::env::var_os("FFIX_GAME_DATA"),
        ) else {
            return;
        };
        let bank =
            SoundBank::open(std::path::Path::new(&assets), std::path::Path::new(&data)).unwrap();
        let mut d = AudioDirector::new(bank, 48_000).unwrap();
        for id in [
            1852, 1853, 1854, 1855, 1856, 1857, 1858, 1859, 1861, 1862, 1863, 1864, 2331, 2332,
            2333,
        ] {
            d.apply(&AudioCue::Effect { id });
            let peak = d
                .render(30)
                .iter()
                .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
            assert!(peak > 0.005, "card sound {id} is silent");
        }
        d.apply(&AudioCue::CardStarted);
        let peak = d
            .render(120)
            .iter()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
        println!("CARD SONG PEAK {peak}");
        assert!(peak > 0.005, "the card song is silent");
    }
}
