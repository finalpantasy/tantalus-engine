//! Milestone A7: the sound state machine as a pure, deterministic core with no device I/O.
//!
//! Layout:
//! - `profile`: profile types, id to resource metadata tables, the music state graph.
//! - `backend`: the voice model behind the game's `ISdLibAPI` calls (from the SoLoud backend) and
//!   the mixer that renders interleaved stereo f32 for N ticks from supplied PCM.
//! - `dispatch`: `FF9Snd` dispatch and `AllSoundDispatchPlayer`: song, sfx, resident sfx slots, streams.
//! - `events`: typed event-VM sound events to state transitions.
//!
//! Time base: `AllSoundDispatchPlayer.ConvertTickToMillisec` (`ASDP.cs:75`) fixes one script tick at
//! 1/60 s. The backend sample rate must be a multiple of 60 so a tick is a whole number of frames
//! (735 at 44.1 kHz, 800 at 48 kHz).
//!
//! # Not ported, recorded instead
//!
//! Backend assumptions (SoLoud internals the managed source calls but does not show). Each is
//! implemented one specific way and marked here so it can be checked against the real library:
//! - Fader shape: linear in time from the current value. Fades and scheduled stop/pause timers run
//!   in output frames and do not advance while a voice is paused.
//! - Loop wrap: when the position reaches `loop_end` it becomes `loop_start + (position - loop_end)`.
//!   Whether `loop_end` is inclusive is unconfirmed. A clip with `loop_end == 0` plays once
//!   (`SdLibAPIWithSoloud.cs:157`).
//! - Resampling: pitch and the clip-to-output rate ratio advance the read position per output
//!   frame with linear interpolation. SoLoud's resampler is not in the source.
//! - Mono clips are copied to both channels. No pan law is applied: the script path never sets
//!   a non-zero pan (`SndEffectPos` is stored but never fed to `SetPanning`), and the one caller
//!   that does (`MusicPlayer`) is not ported.
//! - Output is the plain sum of voices, unclamped; SoLoud's clip behavior is not modeled.
//! - Voice limit: SoLoud's default active voice cap is not modeled.
//! - `getStreamTime` counts output time while unpaused and is not scaled by pitch; `seek` sets it.
//! - The 48 kHz backend rate (`soloud.init(1, 0, 48000)`) is a parameter here.
//!
//! Source behavior left out on purpose:
//! - `MusicPlayer` playback (`MusicPlayer.cs`): needs `Time.deltaTime`. Only its state graph is
//!   ported (`profile::music_graph_next`). `SoundEffectPlayer`, `SfxSoundPlayer`, the world
//!   sound service and `PauseAllSounds`/`ResumeAllSounds` are not ported.
//! - Tempo and tempo-pitch codes are stubs in the source and do nothing here too.
//! - `FF9SOUND_SONG_LOAD` Cargo Ship bridge volume special case (`ASDP.cs:132`) needs script variables.
//! - Language-dependent Cleyra tempo skip (`DoEventCode.cs:2491`): guards a no-op code.
//! - Camera projection for `SetSoundPosition`/`SetSoundObjectPosition`: supplied through
//!   `events::SoundProjector`.
//! - Asset loading (`SoundLoader*`, AKB2 header parse, info-file overrides): callers pass decoded
//!   `PcmClip`s keyed by resource id. A missing clip is treated as the loader's error path.
//! - The `Update` cadence: the source runs it once per rendered frame; `SoundSystem::run_ticks`
//!   runs it once per tick. `Time.time` ordering of sfx start times is a monotonic counter.
//! - The legacy (`Backend == 0`) `SdLib` seek path.

pub mod backend;
pub mod dispatch;
pub mod events;
pub mod profile;

pub use backend::{Backend, PcmClip};
pub use dispatch::{
    DispatchMode, PlayingSfx, SoundSettings, SoundSystem, code, convert_millisec_to_tick,
    convert_tick_to_millisec, ext_env_obj_no, is_ext_env_obj_no, is_shift_pitch_in_fast_forward,
    normalize_pitch, normalize_volume, reverse_normalize_volume,
};
pub use events::{EventSoundOutcome, SoundProjector};
pub use profile::{
    MetaEntry, ProfileState, ProfileType, SoundMetadata, SoundProfile, Transition, music_graph_next,
};
