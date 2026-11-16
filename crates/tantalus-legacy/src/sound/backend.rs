//! Voice backend model and mixer.
//!
//! This models the control surface the game calls on its sound backend (`ISdLibAPI`) using the
//! SoLoud implementation in `Global/Sound/SoLoud/SdLibAPIWithSoloud.cs`. The managed source only
//! shows which SoLoud calls are made, not SoLoud's internals; every behavior that rests on SoLoud
//! itself is listed in `sound/mod.rs` under "Backend assumptions". There is no device I/O: the
//! mixer renders into a caller-owned buffer.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::profile::ProfileType;

/// Decoded audio for one asset plus its raw AKB2 loop fields (frames, not seconds).
#[derive(Debug, Clone, PartialEq)]
pub struct PcmClip {
    pub sample_rate: u32,
    pub channels: u16,
    /// Interleaved signed 16-bit samples.
    pub samples: Vec<i16>,
    pub loop_start: u32,
    pub loop_end: u32,
    pub loop_start_alternate: u32,
    pub loop_end_alternate: u32,
}

impl PcmClip {
    pub fn frames(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.samples.len() / usize::from(self.channels)
        }
    }

    fn sample(&self, frame: usize, channel: usize) -> f32 {
        let channels = usize::from(self.channels);
        let channel = channel.min(channels.saturating_sub(1));
        self.samples
            .get(frame * channels + channel)
            .map_or(0.0, |s| f32::from(*s) / 32768.0)
    }
}

/// Resource name that gets hand-set loop points. `SdLibAPIWithSoloud.cs:157` to `:163`.
const ALTAR_LOOP_RESOURCE: &str = "Sounds03/SE12/se120018";
const ALTAR_LOOP_START_FRAMES: u32 = 104_120;
const ALTAR_LOOP_END_FRAMES: u32 = 216_632;

/// Linear fade toward a target (`soloud.fadeVolume` and the pitch/pan equivalents).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Fade {
    from: f32,
    to: f32,
    total: u64,
    elapsed: u64,
}

impl Fade {
    fn value(&self) -> f32 {
        if self.elapsed >= self.total {
            self.to
        } else {
            self.from + (self.to - self.from) * (self.elapsed as f32 / self.total as f32)
        }
    }

    fn done(&self) -> bool {
        self.elapsed >= self.total
    }
}

#[derive(Debug, Clone)]
struct Bank {
    clip: Arc<PcmClip>,
    resource_id: String,
    #[allow(dead_code)]
    profile_type: ProfileType,
}

#[derive(Debug, Clone)]
struct Voice {
    bank: i32,
    /// SoLoud pause flag. Voices are created paused (`SdLibAPIWithSoloud.cs:152`, `play(.., 1)`).
    paused: bool,
    /// `Sound.pauseStack` (`SdLibAPIWithSoloud.cs:36`).
    pause_stack: i32,
    /// `Sound.volume`, the last value passed to `SetVolume` (`:36`, `:321`).
    stored_volume: f32,
    volume: f32,
    volume_fade: Option<Fade>,
    speed: f32,
    speed_fade: Option<Fade>,
    pan: f32,
    pan_fade: Option<Fade>,
    /// Playback position in source frames.
    position: f64,
    /// `getStreamTime`, seconds.
    stream_time: f64,
    loop_range: Option<(f64, f64)>,
    stop_in: Option<u64>,
    pause_in: Option<u64>,
}

/// The sound backend state: loaded banks and live voices.
#[derive(Debug, Clone)]
pub struct Backend {
    sample_rate: u32,
    banks: BTreeMap<i32, Bank>,
    voices: BTreeMap<i32, Voice>,
    next_bank: i32,
    next_voice: i32,
}

impl Backend {
    /// `sample_rate` must be a multiple of 60 so one engine tick is a whole number of frames.
    pub fn new(sample_rate: u32) -> Result<Self, String> {
        if sample_rate == 0 || !sample_rate.is_multiple_of(60) {
            return Err(format!(
                "sample rate {sample_rate} is not a multiple of the 60 Hz tick rate"
            ));
        }
        Ok(Self {
            sample_rate,
            banks: BTreeMap::new(),
            voices: BTreeMap::new(),
            next_bank: 1,
            next_voice: 1,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Output frames per engine tick (`ConvertTickToMillisec` fixes the tick at 60 Hz).
    pub fn frames_per_tick(&self) -> usize {
        (self.sample_rate / 60) as usize
    }

    pub fn voice_count(&self) -> usize {
        self.voices.len()
    }

    fn ms_to_frames(&self, ms: i32) -> u64 {
        let ms = u64::try_from(ms.max(0)).unwrap_or(0);
        (ms * u64::from(self.sample_rate) + 500) / 1000
    }

    /// `SdSoundSystem_AddData`, `SdLibAPIWithSoloud.cs:103`. Returns the bank id.
    pub fn add_data(
        &mut self,
        clip: Arc<PcmClip>,
        resource_id: &str,
        profile_type: ProfileType,
    ) -> i32 {
        let id = self.next_bank;
        self.next_bank += 1;
        self.banks.insert(
            id,
            Bank {
                clip,
                resource_id: resource_id.to_owned(),
                profile_type,
            },
        );
        id
    }

    /// `SdSoundSystem_RemoveData`, `SdLibAPIWithSoloud.cs:131`. Live voices keep playing.
    pub fn remove_data(&mut self, bank: i32) -> i32 {
        i32::from(self.banks.remove(&bank).is_some())
    }

    /// `SdSoundSystem_CreateSound`, `SdLibAPIWithSoloud.cs:137`. Returns 0 for an unknown bank.
    pub fn create_sound(&mut self, bank_id: i32) -> i32 {
        let Some(bank) = self.banks.get(&bank_id) else {
            return 0;
        };
        let clip = &bank.clip;
        let mut loop_range = None;
        if clip.loop_end > 0 {
            let (mut start, mut end) = (clip.loop_start, clip.loop_end);
            if bank.resource_id == ALTAR_LOOP_RESOURCE {
                start = ALTAR_LOOP_START_FRAMES;
                end = ALTAR_LOOP_END_FRAMES;
            }
            loop_range = Some((f64::from(start), f64::from(end)));
        }
        let id = self.next_voice;
        self.next_voice += 1;
        self.voices.insert(
            id,
            Voice {
                bank: bank_id,
                paused: true,
                pause_stack: 0,
                stored_volume: 1.0,
                volume: 1.0,
                volume_fade: None,
                speed: 1.0,
                speed_fade: None,
                pan: 0.0,
                pan_fade: None,
                position: 0.0,
                stream_time: 0.0,
                loop_range,
                stop_in: None,
                pause_in: None,
            },
        );
        id
    }

    /// `SdSoundSystem_SoundCtrl_IsExist` (`isValidVoiceHandle`), `:264`.
    pub fn is_exist(&self, id: i32) -> bool {
        self.voices.contains_key(&id)
    }

    /// `SdSoundSystem_SoundCtrl_GetElapsedPlaybackTime`, `:187`.
    pub fn elapsed_ms(&self, id: i32) -> i32 {
        self.voices
            .get(&id)
            .map_or(0, |v| (v.stream_time * 1000.0) as i32)
    }

    /// `SdSoundSystem_SoundCtrl_IsPaused`, `:306`.
    pub fn is_paused(&self, id: i32) -> bool {
        self.voices.get(&id).is_some_and(|v| v.paused)
    }

    /// `SdSoundSystem_SoundCtrl_GetVolume` (`getVolume`), `:331`.
    pub fn get_volume(&self, id: i32) -> f32 {
        self.voices.get(&id).map_or(0.0, |v| v.volume)
    }

    pub fn get_pitch(&self, id: i32) -> f32 {
        self.voices.get(&id).map_or(0.0, |v| v.speed)
    }

    pub fn get_panning(&self, id: i32) -> f32 {
        self.voices.get(&id).map_or(0.0, |v| v.pan)
    }

    pub fn get_position_frames(&self, id: i32) -> Option<f64> {
        self.voices.get(&id).map(|v| v.position)
    }

    /// `SdSoundSystem_SoundCtrl_Start`, `SdLibAPIWithSoloud.cs:200`.
    pub fn start(&mut self, id: i32, offset_ms: i32) -> i32 {
        let Some(bank) = self.voices.get(&id).map(|v| v.bank) else {
            return 0;
        };
        let mut count = 0;
        let mut stop_id = 0;
        let mut max_pos = 0.0_f64;
        let same_bank: Vec<(i32, f64)> = self
            .voices
            .iter()
            .filter(|(_, v)| v.bank == bank && !v.paused)
            .map(|(key, v)| (*key, v.stream_time))
            .collect();
        for (key, pos) in same_bank {
            if pos < 0.01 {
                // Same sound within 10 ms: stopped outright (`:219` to `:224`).
                self.voices.remove(&key);
            } else {
                count += 1;
                if pos > max_pos {
                    stop_id = key;
                    max_pos = pos;
                }
            }
        }
        if count >= 2 {
            // Never more than two audible copies: fade out the oldest over 100 ms (`:237` to `:242`).
            let frames = self.ms_to_frames(100);
            if let Some(voice) = self.voices.get_mut(&stop_id) {
                voice.volume_fade = Some(Fade {
                    from: voice.volume,
                    to: 0.0,
                    total: frames,
                    elapsed: 0,
                });
                voice.stop_in = Some(frames);
            }
        }
        let rate = self.banks.get(&bank).map_or(1, |b| b.clip.sample_rate);
        if let Some(voice) = self.voices.get_mut(&id) {
            if offset_ms > 0 {
                let seconds = f64::from(offset_ms) / 1000.0;
                voice.position = seconds * f64::from(rate);
                voice.stream_time = seconds;
            }
            if voice.pause_stack == 0 {
                voice.paused = false;
            }
        }
        1
    }

    /// `SdSoundSystem_SoundCtrl_Stop`, `:253`. Minimum fade is 100 ms.
    pub fn stop(&mut self, id: i32, trans_ms: i32) {
        let trans_ms = trans_ms.max(100);
        let frames = self.ms_to_frames(trans_ms);
        if let Some(voice) = self.voices.get_mut(&id) {
            voice.volume_fade = Some(Fade {
                from: voice.volume,
                to: 0.0,
                total: frames,
                elapsed: 0,
            });
            voice.stop_in = Some(frames);
        }
    }

    /// `SdSoundSystem_SoundCtrl_SetPause`, `:269`.
    pub fn set_pause(&mut self, id: i32, pause_on: i32, trans_ms: i32) {
        let frames = self.ms_to_frames(trans_ms);
        let Some(voice) = self.voices.get_mut(&id) else {
            return;
        };
        if pause_on > 0 {
            voice.pause_stack += 1;
        } else if voice.pause_stack > 0 {
            voice.pause_stack -= 1;
        }
        if pause_on == 0 && voice.pause_stack != 0 {
            return;
        }
        if trans_ms > 0 {
            if pause_on != 0 {
                voice.volume_fade = Some(Fade {
                    from: voice.volume,
                    to: 0.0,
                    total: frames,
                    elapsed: 0,
                });
                voice.pause_in = Some(frames);
            } else {
                voice.paused = false;
                voice.volume = 0.0;
                voice.volume_fade = Some(Fade {
                    from: 0.0,
                    to: voice.stored_volume,
                    total: frames,
                    elapsed: 0,
                });
            }
        } else {
            voice.paused = pause_on != 0;
            voice.volume = voice.stored_volume;
            voice.volume_fade = None;
        }
    }

    /// `SdSoundSystem_SoundCtrl_SetVolume`, `:312`. Volume clamps to 0..=5.
    pub fn set_volume(&mut self, id: i32, volume: f32, trans_ms: i32) {
        let frames = self.ms_to_frames(trans_ms);
        let Some(voice) = self.voices.get_mut(&id) else {
            return;
        };
        let volume = volume.clamp(0.0, 5.0);
        voice.stored_volume = volume;
        if trans_ms > 0 {
            voice.volume_fade = Some(Fade {
                from: voice.volume,
                to: volume,
                total: frames,
                elapsed: 0,
            });
        } else {
            voice.volume = volume;
            voice.volume_fade = None;
        }
    }

    /// `SdSoundSystem_SoundCtrl_SetPitch` (relative play speed), `:337`.
    pub fn set_pitch(&mut self, id: i32, pitch: f32, trans_ms: i32) {
        let frames = self.ms_to_frames(trans_ms);
        let Some(voice) = self.voices.get_mut(&id) else {
            return;
        };
        if trans_ms > 0 {
            voice.speed_fade = Some(Fade {
                from: voice.speed,
                to: pitch,
                total: frames,
                elapsed: 0,
            });
        } else {
            voice.speed = pitch;
            voice.speed_fade = None;
        }
    }

    /// `SdSoundSystem_SoundCtrl_SetPanning`, `:354`. The value is stored; the mixer does not
    /// apply a pan law (see `sound/mod.rs`).
    pub fn set_panning(&mut self, id: i32, pan: f32, trans_ms: i32) {
        let frames = self.ms_to_frames(trans_ms);
        let Some(voice) = self.voices.get_mut(&id) else {
            return;
        };
        if trans_ms > 0 {
            voice.pan_fade = Some(Fade {
                from: voice.pan,
                to: pan,
                total: frames,
                elapsed: 0,
            });
        } else {
            voice.pan = pan;
            voice.pan_fade = None;
        }
    }

    /// `SdSoundSystem_SoundCtrl_SetNextLoopRegion`, `:369`.
    pub fn set_next_loop_region(&mut self, id: i32) {
        let Some(bank) = self.voices.get(&id).map(|v| v.bank) else {
            return;
        };
        let Some(clip) = self.banks.get(&bank).map(|b| Arc::clone(&b.clip)) else {
            return;
        };
        if clip.loop_end_alternate > 0
            && let Some(voice) = self.voices.get_mut(&id)
        {
            voice.loop_range = Some((
                f64::from(clip.loop_start_alternate),
                f64::from(clip.loop_end_alternate),
            ));
        }
    }

    /// Renders `ticks` engine ticks (`frames_per_tick` frames each) of interleaved stereo f32.
    ///
    /// Per output frame, for each live voice in id order: a paused voice is skipped and does not
    /// advance; otherwise its sample is added at the current fader value, then positions, faders
    /// and scheduled stop/pause timers advance by one frame.
    pub fn mix_ticks(&mut self, ticks: u32) -> Vec<f32> {
        self.mix_frames(self.frames_per_tick() * ticks as usize)
    }

    /// Renders an exact number of output frames (interleaved stereo f32).
    pub fn mix_frames(&mut self, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0_f32; frames * 2];
        for frame in 0..frames {
            let (left, right) = self.mix_frame();
            out[frame * 2] = left;
            out[frame * 2 + 1] = right;
        }
        out
    }

    fn mix_frame(&mut self) -> (f32, f32) {
        let rate = f64::from(self.sample_rate);
        let mut left = 0.0_f32;
        let mut right = 0.0_f32;
        let mut remove = Vec::new();
        for (id, voice) in &mut self.voices {
            if voice.paused {
                continue;
            }
            let Some(bank) = self.banks.get(&voice.bank) else {
                remove.push(*id);
                continue;
            };
            let clip = &bank.clip;
            let frames = clip.frames();
            let index = voice.position.floor();
            let fraction = (voice.position - index) as f32;
            let index = index as usize;
            let gain = voice.volume_fade.as_ref().map_or(voice.volume, Fade::value);
            let sample = |channel: usize| {
                let a = clip.sample(index, channel);
                if fraction == 0.0 || index + 1 >= frames {
                    a
                } else {
                    a + (clip.sample(index + 1, channel) - a) * fraction
                }
            };
            if index < frames {
                if clip.channels == 1 {
                    let s = sample(0) * gain;
                    left += s;
                    right += s;
                } else {
                    left += sample(0) * gain;
                    right += sample(1) * gain;
                }
            }
            // Advance one output frame.
            if let Some(fade) = &mut voice.volume_fade {
                fade.elapsed += 1;
                if fade.done() {
                    voice.volume = fade.to;
                    voice.volume_fade = None;
                }
            }
            if let Some(fade) = &mut voice.speed_fade {
                fade.elapsed += 1;
                voice.speed = fade.value();
                if fade.done() {
                    voice.speed_fade = None;
                }
            }
            if let Some(fade) = &mut voice.pan_fade {
                fade.elapsed += 1;
                voice.pan = fade.value();
                if fade.done() {
                    voice.pan_fade = None;
                }
            }
            voice.stream_time += 1.0 / rate;
            voice.position += f64::from(voice.speed) * f64::from(clip.sample_rate) / rate;
            if let Some((start, end)) = voice.loop_range
                && voice.position >= end
            {
                voice.position = start + (voice.position - end);
            }
            if voice.loop_range.is_none() && voice.position >= frames as f64 {
                remove.push(*id);
                continue;
            }
            if let Some(left_frames) = &mut voice.stop_in {
                *left_frames = left_frames.saturating_sub(1);
                if *left_frames == 0 {
                    remove.push(*id);
                    continue;
                }
            }
            if let Some(left_frames) = &mut voice.pause_in {
                *left_frames = left_frames.saturating_sub(1);
                if *left_frames == 0 {
                    voice.paused = true;
                    voice.pause_in = None;
                }
            }
        }
        for id in remove {
            self.voices.remove(&id);
        }
        (left, right)
    }
}
