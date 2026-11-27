//! Smooth high frame rate: the game logic stays on its vanilla tick (fields
//! and the world map 30 Hz, battles 15 Hz) and only the drawing runs at the
//! display rate, between the last two logic states.
//!
//! Memoria's high-FPS option (`FPSManager`, `SmoothFrameUpdater_Field`,
//! `SmoothFrameUpdater_World`) does the same: it registers the actors' and
//! the camera's state at every logic tick and, in between, draws the
//! previous state moved a fraction of the way to the actual one. A move
//! longer than `ActorSmoothMovementMaxSqr` (400 units), a turn past
//! `ActorSmoothTurnMaxDeg` (45 degrees) or a camera step past
//! `CameraSmoothMovementMaxSqr` (450 units) is a cut and is not blended.
//!
//! Nothing here can reach the game: every function takes copies of drawn
//! state (a `TrackFrame`, a camera center) and returns new values, so the
//! logic, its timing and the regression chain are the same at any rate.

use std::time::Duration;

use bevy::window::PresentMode;

use crate::actor_models::ActorPose;
use crate::track::TrackFrame;

/// Field and world map logic ticks a second (`FPS_FIELD`, `FPS_WORLD`).
pub const LOGIC_HZ: f64 = 30.0;
/// The tick length in seconds.
pub const LOGIC_STEP: f64 = 1.0 / LOGIC_HZ;

/// `SmoothFrameUpdater_Field.ActorSmoothMovementMaxSqr`: the longest step an
/// actor makes in one tick and still moves smoothly.
pub const ACTOR_MOVE_MAX: f32 = 400.0;
/// `SmoothFrameUpdater_Field.ActorSmoothTurnMaxDeg`, in 256ths of a turn.
pub const ACTOR_TURN_MAX: f32 = 45.0 * 256.0 / 360.0;
/// `SmoothFrameUpdater_Field.CameraSmoothMovementMaxSqr`.
pub const CAMERA_MOVE_MAX: f32 = 450.0;

/// What the frame rate is held to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FpsCap {
    /// The display's refresh rate (vsync).
    Display,
    /// No limit and no vsync.
    Uncapped,
    /// At most this many frames a second.
    Limit(u32),
}

/// The frame rate setting: `--fps-cap N`, `TANTALUS_FPS_CAP`, or the
/// `fps_cap` entry of `tantalus.cfg`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FpsSettings {
    pub cap: FpsCap,
    /// Draw the states between logic ticks. Off is vanilla: one picture a tick.
    pub interpolate: bool,
}

impl Default for FpsSettings {
    fn default() -> Self {
        Self {
            cap: FpsCap::Display,
            interpolate: true,
        }
    }
}

impl FpsSettings {
    /// `cap` is the `--fps-cap` text (0 or `uncapped` for no limit, `display`
    /// for the refresh rate); `interpolation` is `on`/`off`. A cap of 30 or
    /// less is the vanilla mode, without interpolation, unless it is asked for.
    pub fn parse(cap: Option<&str>, interpolation: Option<&str>) -> Result<Self, String> {
        let mut settings = Self::default();
        if let Some(text) = cap {
            settings.cap = match text.trim().to_ascii_lowercase().as_str() {
                "display" | "vsync" | "auto" => FpsCap::Display,
                "0" | "uncapped" | "off" | "none" => FpsCap::Uncapped,
                number => FpsCap::Limit(
                    number
                        .parse::<u32>()
                        .ok()
                        .filter(|rate| *rate > 0)
                        .ok_or_else(|| format!("bad frame rate cap `{text}`"))?,
                ),
            };
            if matches!(settings.cap, FpsCap::Limit(rate) if f64::from(rate) <= LOGIC_HZ) {
                settings.interpolate = false;
            }
        }
        if let Some(text) = interpolation {
            settings.interpolate = match text.trim().to_ascii_lowercase().as_str() {
                "on" | "true" | "1" | "yes" => true,
                "off" | "false" | "0" | "no" => false,
                _ => return Err(format!("bad interpolation setting `{text}`")),
            };
        }
        Ok(settings)
    }

    /// The `fps_cap` and `interpolation` texts of a config file's `key = value`
    /// lines (`#` starts a comment).
    pub fn config_values(text: &str) -> (Option<String>, Option<String>) {
        let (mut cap, mut interpolation) = (None, None);
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "fps_cap" => cap = Some(value.trim().to_owned()),
                "interpolation" => interpolation = Some(value.trim().to_owned()),
                _ => {}
            }
        }
        (cap, interpolation)
    }

    pub fn from_config(text: &str) -> Result<Self, String> {
        let (cap, interpolation) = Self::config_values(text);
        Self::parse(cap.as_deref(), interpolation.as_deref())
    }

    /// The setting for a run: a flag beats the `TANTALUS_FPS_CAP` and
    /// `TANTALUS_FPS_INTERP` environment, which beats the config file. A cap
    /// given outside the file brings its own interpolation default, so the
    /// file's `interpolation` line does not undo `--fps-cap 30`.
    pub fn resolve(
        config: Option<&std::path::Path>,
        flag_cap: Option<&str>,
        flag_interpolation: Option<&str>,
    ) -> Result<Self, String> {
        let file = config
            .map(std::fs::read_to_string)
            .transpose()
            .map_err(|error| format!("config: {error}"))?;
        let (file_cap, file_interp) = file.as_deref().map_or((None, None), Self::config_values);
        let env_cap = std::env::var("TANTALUS_FPS_CAP").ok();
        let env_interp = std::env::var("TANTALUS_FPS_INTERP").ok();
        let outside_cap = flag_cap.or(env_cap.as_deref());
        let cap = outside_cap.or(file_cap.as_deref());
        let interpolation =
            flag_interpolation
                .or(env_interp.as_deref())
                .or(if outside_cap.is_none() {
                    file_interp.as_deref()
                } else {
                    None
                });
        Self::parse(cap, interpolation)
    }

    /// Vsync on the display's rate, or off when a limit or none is asked for.
    pub fn present_mode(&self) -> PresentMode {
        match self.cap {
            FpsCap::Display => PresentMode::AutoVsync,
            FpsCap::Uncapped | FpsCap::Limit(_) => PresentMode::AutoNoVsync,
        }
    }

    /// The shortest time a frame may take under a `--fps-cap N` limit.
    pub fn frame_budget(&self) -> Option<Duration> {
        match self.cap {
            FpsCap::Limit(rate) => Some(Duration::from_secs_f64(1.0 / f64::from(rate))),
            _ => None,
        }
    }
}

/// How far into the logic tick the drawn frame is: 0 at the tick, 1 at the
/// next. `elapsed` is the time since the last tick (the leftover of the
/// fixed-step loop).
pub fn alpha(elapsed: f64, interpolate: bool) -> f32 {
    if !interpolate {
        return 1.0;
    }
    (elapsed / LOGIC_STEP).clamp(0.0, 1.0) as f32
}

/// Whether the fixed-step loop runs a logic tick now, with `elapsed` seconds
/// banked (this frame's `frame` seconds included).
///
/// A capped display's frames start on a fixed grid (`FrameLimiter` adds the
/// budget to the last deadline), so each frame is the tick length plus or
/// minus a few microseconds of wake-up jitter. With nothing banked (as after
/// a load hitch, a battle or a movie, which reset the bank) a strict
/// `elapsed >= LOGIC_STEP` then misses the tick on every frame that is a hair
/// short and runs two on the next: at `--fps-cap 30 --fps-interp off` about
/// half the frames showed no motion and half jumped two ticks. Without
/// interpolation (one picture a tick, the vanilla mode) the tick goes to the
/// display frame nearest its time: one up to half a frame early takes it and
/// the debt carries, so the logic rate is unchanged and a 30 fps display gets
/// exactly one tick a frame. With interpolation the drawn alpha follows the
/// banked time, so only rounding slack is allowed.
pub fn tick_due(elapsed: f64, frame: f64, interpolate: bool) -> bool {
    let slack = if interpolate {
        TICK_ROUNDING
    } else {
        (frame.min(LOGIC_STEP) / 2.0).max(TICK_ROUNDING)
    };
    elapsed + slack >= LOGIC_STEP
}

/// Clock rounding a tick may be early by (a `Duration` holds whole
/// nanoseconds, so 1/30 s of frames sums a little short of `LOGIC_STEP`).
pub const TICK_ROUNDING: f64 = 1e-6;

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        lerp(a[0], b[0], t),
        lerp(a[1], b[1], t),
        lerp(a[2], b[2], t),
    ]
}

/// The signed shortest turn from `from` to `to`, in the same unit as `turn`
/// (a full turn): in `-turn/2..=turn/2`.
pub fn shortest_delta(from: f32, to: f32, turn: f32) -> f32 {
    let mut delta = (to - from).rem_euclid(turn);
    if delta > turn / 2.0 {
        delta -= turn;
    }
    delta
}

/// An angle `t` of the way from `a` to `b` by the shortest arc, kept in
/// `0..turn`.
pub fn lerp_angle(a: f32, b: f32, t: f32, turn: f32) -> f32 {
    (a + shortest_delta(a, b, turn) * t).rem_euclid(turn)
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// The pose `t` of the way from `prev` to `cur`. A different model, clip or
/// visibility, an animation that restarted (its time went back) or a turn
/// past 45 degrees is a cut: `cur` as it is.
pub fn blend_pose(prev: &ActorPose, cur: &ActorPose, t: f32) -> ActorPose {
    let mut pose = *cur;
    pose.facing_frac = 0.0;
    let same_look = prev.model == cur.model
        && prev.animation == cur.animation
        && prev.visible == cur.visible
        && prev.scale == cur.scale;
    if !same_look {
        return pose;
    }
    // Animation time runs forward a tick at a time; going back is a restart.
    let advance = cur.time - prev.time;
    if (0.0..=0.5).contains(&advance) {
        pose.time = prev.time + advance * t;
    }
    let from = f32::from(prev.facing) + prev.facing_frac;
    let to = f32::from(cur.facing) + cur.facing_frac;
    if shortest_delta(from, to, 256.0).abs() <= ACTOR_TURN_MAX {
        let facing = lerp_angle(from, to, t, 256.0);
        pose.facing = facing.floor() as u8;
        pose.facing_frac = facing - facing.floor();
    }
    pose
}

/// `cur` drawn `t` of the way from `prev`: actors' positions, facings and
/// animation times move between the two ticks. An actor new in `cur`, or one
/// that moved past [`ACTOR_MOVE_MAX`], is not blended.
pub fn blend_frame(prev: &TrackFrame, cur: &TrackFrame, t: f32) -> TrackFrame {
    let mut frame = cur.clone();
    if t >= 1.0 {
        return frame;
    }
    for (id, position) in &mut frame.actors {
        let Some(before) = prev
            .actors
            .iter()
            .find(|(other, _)| other == id)
            .map(|(_, position)| *position)
        else {
            continue;
        };
        let moved = distance(before, *position) <= ACTOR_MOVE_MAX;
        if moved {
            *position = lerp3(before, *position, t);
        }
        if let (Some(pose), Some(before)) = (frame.poses.get_mut(id), prev.poses.get(id)) {
            // A cut in place (it was teleported) cuts its turn and clip too.
            *pose = if moved {
                blend_pose(before, pose, t)
            } else {
                *pose
            };
        }
    }
    frame
}

/// A 2D camera center `t` of the way from `prev` to `cur`, except past a cut
/// ([`CAMERA_MOVE_MAX`]).
pub fn blend_view(prev: [f32; 2], cur: [f32; 2], t: f32) -> [f32; 2] {
    let step = ((cur[0] - prev[0]).powi(2) + (cur[1] - prev[1]).powi(2)).sqrt();
    if step > CAMERA_MOVE_MAX {
        return cur;
    }
    [lerp(prev[0], cur[0], t), lerp(prev[1], cur[1], t)]
}

/// The last two logic states of something drawn, with a snap that drops the
/// older one (a field change, a camera cut).
#[derive(Debug, Clone, PartialEq)]
pub struct Pair<T> {
    prev: Option<T>,
    cur: Option<T>,
}

impl<T> Default for Pair<T> {
    fn default() -> Self {
        Self {
            prev: None,
            cur: None,
        }
    }
}

impl<T> Pair<T> {
    /// Records the state of a new tick. With `snap` the older state is
    /// dropped, so the next frames show the new one without a blend.
    pub fn push(&mut self, state: T, snap: bool) {
        self.prev = if snap { None } else { self.cur.take() };
        self.cur = Some(state);
    }

    pub fn current(&self) -> Option<&T> {
        self.cur.as_ref()
    }

    pub fn previous(&self) -> Option<&T> {
        self.prev.as_ref()
    }

    pub fn clear(&mut self) {
        self.prev = None;
        self.cur = None;
    }
}

impl Pair<TrackFrame> {
    /// The actors drawn `t` into the tick.
    pub fn frame(&self, t: f32) -> Option<TrackFrame> {
        let cur = self.cur.as_ref()?;
        Some(match &self.prev {
            Some(prev) => blend_frame(prev, cur, t),
            None => cur.clone(),
        })
    }
}

impl Pair<WorldPose> {
    /// The world map's player and camera drawn `t` into the tick.
    pub fn world(&self, t: f32) -> Option<WorldPose> {
        let cur = self.cur?;
        Some(match self.prev {
            Some(prev) => blend_world(&prev, &cur, t),
            None => cur,
        })
    }
}

impl Pair<[f32; 2]> {
    /// The camera center drawn `t` into the tick.
    pub fn view(&self, t: f32) -> Option<[f32; 2]> {
        let cur = self.cur?;
        Some(match self.prev {
            Some(prev) => blend_view(prev, cur, t),
            None => cur,
        })
    }
}

/// Where the world map's player and camera are, for [`blend_world`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldPose {
    pub position: [f32; 3],
    /// Heading in degrees.
    pub yaw: f32,
    pub eye: [f32; 3],
    pub aim: [f32; 3],
    pub fov: f32,
    /// The mount (`WorldPlay::ride`): a change is a cut.
    pub ride: u8,
}

/// The world map's player, vehicle and camera `t` of the way between two
/// ticks. A jump past [`CAMERA_MOVE_MAX`] (a warp, a place entered) or a
/// change of mount is not blended.
pub fn blend_world(prev: &WorldPose, cur: &WorldPose, t: f32) -> WorldPose {
    if prev.ride != cur.ride
        || distance(prev.position, cur.position) > CAMERA_MOVE_MAX
        || distance(prev.eye, cur.eye) > CAMERA_MOVE_MAX
    {
        return *cur;
    }
    WorldPose {
        position: lerp3(prev.position, cur.position, t),
        yaw: lerp_angle(prev.yaw, cur.yaw, t, 360.0),
        eye: lerp3(prev.eye, cur.eye, t),
        aim: lerp3(prev.aim, cur.aim, t),
        fov: lerp(prev.fov, cur.fov, t),
        ride: cur.ride,
    }
}

/// Holds a frame to its `--fps-cap` budget: sleeps most of what is left of
/// the budget and spins the last stretch, as the frame limiters of games do.
pub struct FrameLimiter {
    budget: Option<Duration>,
    next: std::time::Instant,
}

impl FrameLimiter {
    pub fn new(budget: Option<Duration>) -> Self {
        Self {
            budget,
            next: std::time::Instant::now(),
        }
    }

    pub fn wait(&mut self) {
        let Some(budget) = self.budget else {
            return;
        };
        let now = std::time::Instant::now();
        if self.next > now {
            let left = self.next - now;
            if left > Duration::from_millis(2) {
                std::thread::sleep(left - Duration::from_millis(1));
            }
            while std::time::Instant::now() < self.next {
                std::hint::spin_loop();
            }
            self.next += budget;
        } else {
            // Late: start the next budget from now, not from the debt.
            self.next = now + budget;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(facing: u8, time: f32) -> ActorPose {
        ActorPose {
            facing,
            time,
            model: Some(1),
            animation: Some(2),
            visible: true,
            ..ActorPose::default()
        }
    }

    fn frame(position: [f32; 3], facing: u8, time: f32) -> TrackFrame {
        TrackFrame {
            tick: 0,
            player: Some(7),
            actors: vec![(7, position)],
            poses: [(7, pose(facing, time))].into(),
            attachments: Vec::new(),
        }
    }

    #[test]
    fn yaw_takes_the_shortest_arc() {
        // 250 to 6 crosses the seam: 12 steps, not 244.
        let facing = lerp_angle(250.0, 6.0, 0.5, 256.0);
        assert!((facing - 0.0).abs() < 1e-4, "{facing}");
        assert!((shortest_delta(10.0, 250.0, 256.0) + 16.0).abs() < 1e-4);
        assert!((lerp_angle(350.0, 10.0, 0.25, 360.0) - 355.0).abs() < 1e-3);
    }

    #[test]
    fn actors_move_between_ticks_and_cut_on_a_teleport() {
        let (a, b) = (
            frame([0.0, 0.0, 0.0], 0, 1.0),
            frame([30.0, 0.0, 0.0], 8, 1.0 + LOGIC_STEP as f32),
        );
        let half = blend_frame(&a, &b, 0.5);
        assert_eq!(half.actors[0].1, [15.0, 0.0, 0.0]);
        let pose = half.poses[&7];
        assert_eq!(pose.facing, 4);
        assert!((pose.time - (1.0 + LOGIC_STEP as f32 / 2.0)).abs() < 1e-5);
        assert_eq!(blend_frame(&a, &b, 1.0), b);
        // 1000 units in a tick is a teleport: shown where it landed.
        let far = frame([1000.0, 0.0, 0.0], 8, 1.0);
        assert_eq!(blend_frame(&a, &far, 0.5), far);
    }

    #[test]
    fn a_restarted_animation_and_a_big_turn_are_cuts() {
        let a = frame([0.0; 3], 0, 2.0);
        let restarted = frame([0.0; 3], 0, 0.0);
        assert_eq!(blend_frame(&a, &restarted, 0.5).poses[&7].time, 0.0);
        let turned = frame([0.0; 3], 128, 2.0);
        assert_eq!(blend_frame(&a, &turned, 0.5).poses[&7].facing, 128);
        let mut other = frame([0.0; 3], 0, 2.0);
        other.poses.get_mut(&7).unwrap().animation = Some(9);
        other.poses.get_mut(&7).unwrap().time = 2.01;
        assert_eq!(blend_frame(&a, &other, 0.5).poses[&7].time, 2.01);
    }

    #[test]
    fn the_camera_blends_and_cuts() {
        assert_eq!(blend_view([0.0, 0.0], [10.0, 4.0], 0.5), [5.0, 2.0]);
        assert_eq!(blend_view([0.0, 0.0], [900.0, 0.0], 0.5), [900.0, 0.0]);
    }

    #[test]
    fn a_snap_drops_the_older_state() {
        let mut pair = Pair::default();
        pair.push(frame([0.0; 3], 0, 0.0), false);
        pair.push(frame([10.0, 0.0, 0.0], 0, 0.03), false);
        assert_eq!(pair.frame(0.5).unwrap().actors[0].1[0], 5.0);
        pair.push(frame([20.0, 0.0, 0.0], 0, 0.06), true);
        assert!(pair.previous().is_none());
        assert_eq!(pair.frame(0.5).unwrap().actors[0].1[0], 20.0);
    }

    #[test]
    fn the_world_map_blends_but_not_across_a_warp_or_a_new_mount() {
        let at = |x: f32, ride| WorldPose {
            position: [x, 0.0, 0.0],
            yaw: 350.0,
            eye: [x, 5.0, 9.0],
            aim: [x, 1.0, 0.0],
            fov: 60.0,
            ride,
        };
        let mut next = at(8.0, 0);
        next.yaw = 10.0;
        let mid = blend_world(&at(0.0, 0), &next, 0.5);
        assert_eq!(mid.position[0], 4.0);
        assert!((mid.yaw - 0.0).abs() < 1e-3 || (mid.yaw - 360.0).abs() < 1e-3);
        assert_eq!(blend_world(&at(0.0, 0), &at(8.0, 1), 0.5), at(8.0, 1));
        assert_eq!(blend_world(&at(0.0, 0), &at(5000.0, 0), 0.5), at(5000.0, 0));
    }

    #[test]
    fn settings_parse_the_cap_and_the_vanilla_mode() {
        let default = FpsSettings::parse(None, None).unwrap();
        assert_eq!(default.cap, FpsCap::Display);
        assert!(default.interpolate);
        assert_eq!(default.present_mode(), PresentMode::AutoVsync);
        let uncapped = FpsSettings::parse(Some("0"), None).unwrap();
        assert_eq!(uncapped.cap, FpsCap::Uncapped);
        assert!(uncapped.interpolate);
        let capped = FpsSettings::parse(Some("144"), None).unwrap();
        assert_eq!(capped.cap, FpsCap::Limit(144));
        assert!(capped.frame_budget().is_some());
        let vanilla = FpsSettings::parse(Some("30"), None).unwrap();
        assert!(!vanilla.interpolate);
        assert_eq!(alpha(0.01, vanilla.interpolate), 1.0);
        assert!(FpsSettings::parse(Some("fast"), None).is_err());
        let forced = FpsSettings::parse(Some("30"), Some("on")).unwrap();
        assert!(forced.interpolate);
        let file = FpsSettings::from_config("# comment\nfps_cap = 60\ninterpolation = on\n");
        assert_eq!(file.unwrap().cap, FpsCap::Limit(60));
    }

    #[test]
    fn a_capped_vanilla_display_ticks_once_a_frame() {
        // Frames on a 1/30 s grid with wake-up jitter, the bank starting
        // empty: every frame runs exactly one tick without interpolation.
        let jitter = [0.0, -4e-5, 3e-5, -1e-5, 5e-5, -6e-5, 2e-5, 0.0, -3e-5];
        let (mut elapsed, mut last, mut ticks) = (0.0, 0.0, 0);
        for (frame, offset) in jitter.iter().cycle().take(300).enumerate() {
            let at = (frame + 1) as f64 * LOGIC_STEP + offset;
            let delta = at - last;
            last = at;
            elapsed += delta;
            let mut steps = 0;
            while tick_due(elapsed, delta, false) && steps < 4 {
                elapsed -= LOGIC_STEP;
                steps += 1;
            }
            assert_eq!(steps, 1, "frame {frame}");
            ticks += steps;
        }
        assert_eq!(ticks, 300);
        // With interpolation only clock rounding is slack.
        assert!(!tick_due(LOGIC_STEP - 4e-5, LOGIC_STEP / 2.0 - 4e-5, true));
        assert!(tick_due(LOGIC_STEP - 1e-9, LOGIC_STEP / 2.0, true));
        // At 60 fps without interpolation the tick lands on every second frame.
        let (mut elapsed, mut ticks) = (0.0, 0);
        for frame in 0..120 {
            let delta = LOGIC_STEP / 2.0 + if frame % 2 == 0 { 3e-5 } else { -3e-5 };
            elapsed += delta;
            if tick_due(elapsed, delta, false) {
                elapsed -= LOGIC_STEP;
                ticks += 1;
                assert_eq!(frame % 2, 1, "frame {frame}");
            }
        }
        assert_eq!(ticks, 60);
    }

    #[test]
    fn alpha_is_the_share_of_the_tick() {
        assert_eq!(alpha(0.0, true), 0.0);
        assert!((alpha(LOGIC_STEP / 4.0, true) - 0.25).abs() < 1e-6);
        assert_eq!(alpha(1.0, true), 1.0);
    }

    #[test]
    fn drawing_never_changes_the_state_it_draws_from() {
        // The blend reads two captured frames and returns a third: the
        // sources are the same after any number of draws at any alpha.
        let (a, b) = (frame([0.0; 3], 3, 1.0), frame([9.0, 0.0, 0.0], 9, 1.04));
        let mut pair = Pair::default();
        pair.push(a.clone(), false);
        pair.push(b.clone(), false);
        for step in 0..=24 {
            let _ = pair.frame(step as f32 / 24.0);
        }
        assert_eq!(pair.previous(), Some(&a));
        assert_eq!(pair.current(), Some(&b));
    }
}
