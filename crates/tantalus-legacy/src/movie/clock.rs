//! `MovieMaterial.Update` and the `Frame` and `TotalFrame` properties.
//!
//! Source: `Assembly-CSharp/Assets/Sources/Graphics/Movie/MovieMaterial.cs` in the pinned
//! Memoria. Unity types are kept: `playSpeed` and the delta are `f32`, times are `f64`.

/// Audio and picture may drift this far before the clock jumps to the audio time.
const AUDIO_SYNC_TOLERANCE: f64 = 0.066;
/// A second, larger drift limit that also needs ten real seconds since the last forced sync.
const AUDIO_SYNC_HARD_LIMIT: f64 = 0.1;
const AUDIO_SYNC_COOLDOWN: f32 = 10.0;
/// Longest frame delta the clock accepts (`Mathf.Min(Time.deltaTime, 0.067f)`).
const MAX_DELTA: f32 = 0.067;
/// The first uploaded frame must be later than this before playback starts.
const FIRST_FRAME_THRESHOLD: f64 = 0.066_666_666_666_666_67;

/// What the host feeds the clock each tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClockInput {
    /// Elapsed playback time of the active movie sound, in seconds (`0` or less when none).
    pub audio_elapsed: f64,
    /// `Time.deltaTime`.
    pub delta_time: f32,
    /// `Time.realtimeSinceStartup`.
    pub realtime: f32,
    /// The decoder reports the stream as finished (`HasFinished`).
    pub native_finished: bool,
    /// Time of the last decoded frame (`GetDecodedFrameTime`).
    pub decoded_frame_time: f64,
    /// Time of the last uploaded frame (`GetUploadedFrameTime`).
    pub uploaded_frame_time: f64,
}

/// What the host must do after a tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClockEvent {
    /// `Seek(context, 0.0, false)`: the movie loops.
    SeekToStart,
    /// `SoundLib.PlayMovieMusic(movieKey, 0)`: the first frame arrived and the clock restarted.
    StartAudio,
    /// `SoundLib.StopMovieMusic` and `OnFinished`.
    Finished,
}

/// Result of one tick.
#[derive(Debug, Clone, PartialEq)]
pub struct ClockOutput {
    pub events: Vec<ClockEvent>,
    /// The time passed to `SetTargetDisplayDecodeTime`; `None` when the tick returned early.
    pub target_display_time: Option<f64>,
}

/// Playback clock for one movie.
#[derive(Debug, Clone)]
pub struct MovieClock {
    elapsed: f64,
    cycle: u8,
    play_speed: f32,
    is_fmv: bool,
    vsync: bool,
    loop_count: i32,
    finished: bool,
    got_first_frame: bool,
    advance: bool,
    last_sync: f32,
    fps: f64,
}

impl MovieClock {
    /// `Open()` followed by `Load()`: time zero, not advancing, `playSpeed = 15 / fps`.
    ///
    /// `file_name` decides the FMV rule (`MovieFile.StartsWith("FMV")`); `loop_count` is the
    /// initial `loopCount` (the field default is 1, `-1` loops forever).
    pub fn new(file_name: &str, fps: f64, vsync: bool, loop_count: i32) -> Self {
        Self {
            elapsed: 0.0,
            cycle: 0,
            play_speed: (15.0 / fps) as f32,
            is_fmv: file_name.starts_with("FMV"),
            vsync,
            loop_count,
            finished: false,
            got_first_frame: false,
            advance: false,
            last_sync: 0.0,
            fps,
        }
    }

    /// Starts advancing (`advance = true`).
    pub fn play(&mut self) {
        self.advance = true;
    }

    /// Stops advancing (`advance = false`).
    pub fn pause(&mut self) {
        self.advance = false;
    }

    pub fn elapsed(&self) -> f64 {
        self.elapsed
    }

    pub fn play_speed(&self) -> f32 {
        self.play_speed
    }

    pub fn has_first_frame(&self) -> bool {
        self.got_first_frame
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// `MovieMaterial.Frame`: FMVs round to the nearest frame (ties to even, as Unity does),
    /// other movies take the floor.
    pub fn frame(&self) -> i32 {
        let scaled = (self.fps * self.elapsed) as f32;
        if self.is_fmv {
            scaled.round_ties_even() as i32
        } else {
            scaled.floor() as i32
        }
    }

    /// `MovieMaterial.TotalFrame` for a movie of `duration` seconds (`Duration` is an `f32`).
    pub fn total_frame(&self, duration: f64) -> i32 {
        (self.fps * f64::from(duration as f32)) as i32
    }

    /// One call of `MovieMaterial.Update`.
    pub fn update(&mut self, input: ClockInput) -> ClockOutput {
        let mut events = Vec::new();
        if self.finished {
            return ClockOutput {
                events,
                target_display_time: None,
            };
        }
        self.finished = input.native_finished;
        if !self.finished {
            if self.advance {
                self.advance_clock(&input);
            }
        } else if self.loop_count - 1 <= 0 && self.loop_count != -1 {
            self.elapsed = input.decoded_frame_time;
            events.push(ClockEvent::Finished);
            return ClockOutput {
                events,
                target_display_time: None,
            };
        } else {
            if self.loop_count != -1 {
                self.loop_count -= 1;
            }
            self.elapsed %= input.decoded_frame_time;
            events.push(ClockEvent::SeekToStart);
            self.finished = false;
        }
        if !self.got_first_frame && input.uploaded_frame_time > FIRST_FRAME_THRESHOLD {
            self.elapsed = 0.0;
            self.cycle = 0;
            self.got_first_frame = true;
            if self.advance {
                events.push(ClockEvent::StartAudio);
            }
        }
        ClockOutput {
            events,
            target_display_time: Some(self.elapsed),
        }
    }

    fn advance_clock(&mut self, input: &ClockInput) {
        let audio = input.audio_elapsed;
        if audio > 0.0 && (audio - self.elapsed).abs() > AUDIO_SYNC_TOLERANCE {
            self.elapsed = audio;
        }
        if audio > 0.0
            && (audio - self.elapsed).abs() > AUDIO_SYNC_HARD_LIMIT
            && input.realtime - self.last_sync > AUDIO_SYNC_COOLDOWN
        {
            self.elapsed = audio;
            self.last_sync = input.realtime;
        }
        if self.is_fmv && !self.vsync {
            // Three-step cycle 0.066, 0.067, 0.067 s: 0.2 s per three ticks (15 fps).
            let step = if self.cycle == 0 { 0.066 } else { 0.067 };
            self.elapsed += step * f64::from(self.play_speed);
            self.cycle = if self.cycle == 2 { 0 } else { self.cycle + 1 };
        } else {
            let delta = input.delta_time.min(MAX_DELTA) * self.play_speed.max(0.0);
            self.elapsed += f64::from(delta);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> ClockInput {
        ClockInput {
            audio_elapsed: 0.0,
            delta_time: 0.016,
            realtime: 100.0,
            native_finished: false,
            decoded_frame_time: 10.0,
            uploaded_frame_time: 0.0,
        }
    }

    fn started(file: &str, fps: f64, vsync: bool, loops: i32) -> MovieClock {
        let mut clock = MovieClock::new(file, fps, vsync, loops);
        clock.play();
        clock.got_first_frame = true;
        clock
    }

    #[test]
    fn fmv_without_vsync_steps_066_067_067_and_wraps_the_cycle() {
        // 15 fps gives playSpeed 1.0, so three ticks add 0.066 + 0.067 + 0.067 = 0.2 s.
        let mut clock = started("FMV000", 15.0, false, 1);
        assert_eq!(clock.play_speed(), 1.0);
        for expected in [0.066, 0.133, 0.2] {
            let out = clock.update(input());
            assert!((clock.elapsed() - expected).abs() < 1e-9);
            assert_eq!(out.target_display_time, Some(clock.elapsed()));
        }
        assert_eq!(clock.cycle, 0);
    }

    #[test]
    fn other_movies_and_vsync_use_the_capped_frame_delta() {
        let mut clock = started("MBG119", 15.0, false, 1);
        let mut tick = input();
        tick.delta_time = 0.5;
        clock.update(tick);
        assert!((clock.elapsed() - f64::from(0.067_f32)).abs() < 1e-9);
        let mut clock = started("FMV000", 15.0, true, 1);
        tick.delta_time = 0.01;
        clock.update(tick);
        assert!((clock.elapsed() - f64::from(0.01_f32)).abs() < 1e-9);
    }

    #[test]
    fn play_speed_scales_with_the_frame_rate() {
        let clock = MovieClock::new("MBG001", 30.0, true, 1);
        assert_eq!(clock.play_speed(), 0.5);
        let mut clock = started("MBG001", 30.0, true, 1);
        let mut tick = input();
        tick.delta_time = 0.04;
        clock.update(tick);
        assert!((clock.elapsed() - f64::from(0.04_f32 * 0.5)).abs() < 1e-9);
    }

    #[test]
    fn audio_more_than_66_ms_away_pulls_the_clock_before_the_step() {
        let mut clock = started("FMV000", 15.0, false, 1);
        let mut tick = input();
        tick.audio_elapsed = 0.5;
        clock.update(tick);
        // Jumped to 0.5, then the first cycle step of 0.066.
        assert!((clock.elapsed() - 0.566).abs() < 1e-9);
        // Within tolerance (0.566 vs 0.6): no jump, only the 0.067 step.
        tick.audio_elapsed = 0.6;
        clock.update(tick);
        assert!((clock.elapsed() - 0.633).abs() < 1e-9);
        // No audio position: the clock free-runs.
        tick.audio_elapsed = 0.0;
        clock.update(tick);
        assert!((clock.elapsed() - 0.700).abs() < 1e-9);
    }

    #[test]
    fn frame_rounds_fmvs_to_even_and_floors_the_rest() {
        let mut fmv = MovieClock::new("FMV010", 5.0, true, 1);
        let mut other = MovieClock::new("MBG010", 5.0, true, 1);
        for (elapsed, fmv_frame, other_frame) in [(0.5, 2, 2), (0.7, 4, 3), (0.9, 4, 4)] {
            fmv.elapsed = elapsed;
            other.elapsed = elapsed;
            assert_eq!(fmv.frame(), fmv_frame, "fmv at {elapsed}");
            assert_eq!(other.frame(), other_frame, "other at {elapsed}");
        }
        assert_eq!(fmv.total_frame(4.5), 22);
    }

    #[test]
    fn looping_wraps_the_time_and_the_last_loop_finishes() {
        let mut clock = started("MBG119", 15.0, true, 2);
        clock.elapsed = 12.0;
        let mut tick = input();
        tick.native_finished = true;
        let out = clock.update(tick);
        assert_eq!(out.events, [ClockEvent::SeekToStart]);
        assert!((clock.elapsed() - 2.0).abs() < 1e-9);
        assert!(!clock.is_finished());
        // loopCount is now 1: the next finish ends playback at the decoded time.
        let out = clock.update(tick);
        assert_eq!(out.events, [ClockEvent::Finished]);
        assert_eq!(out.target_display_time, None);
        assert_eq!(clock.elapsed(), 10.0);
        assert!(clock.is_finished());
        assert!(clock.update(input()).events.is_empty());
    }

    #[test]
    fn a_forever_loop_never_decrements() {
        let mut clock = started("MBG119", 15.0, true, -1);
        let mut tick = input();
        tick.native_finished = true;
        for _ in 0..3 {
            assert_eq!(clock.update(tick).events, [ClockEvent::SeekToStart]);
        }
        assert_eq!(clock.loop_count, -1);
    }

    #[test]
    fn the_first_uploaded_frame_restarts_the_clock_and_starts_audio_when_advancing() {
        let mut clock = MovieClock::new("FMV000", 15.0, false, 1);
        clock.play();
        let mut tick = input();
        tick.uploaded_frame_time = 0.066;
        assert!(clock.update(tick).events.is_empty());
        assert!(!clock.has_first_frame());
        tick.uploaded_frame_time = 0.07;
        let out = clock.update(tick);
        assert_eq!(out.events, [ClockEvent::StartAudio]);
        assert_eq!(out.target_display_time, Some(0.0));
        assert!(clock.has_first_frame());

        let mut paused = MovieClock::new("FMV000", 15.0, false, 1);
        let out = paused.update(tick);
        assert!(out.events.is_empty());
        assert!(paused.has_first_frame());
    }
}
