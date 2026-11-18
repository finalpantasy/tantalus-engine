//! The field view's scroll, as Memoria's `FieldMap` runs it: follow the
//! player (`SceneService3DScroll`), scripted pans (`EBG_scene2DScroll`,
//! `EBG_scene2DScrollRelease`), the follow switch
//! (`EBG_char3DScrollSetActive`) and script bounds (`EBG_cameraSetViewport`).
//!
//! Positions are the frame center in the adapter's space: background pixel
//! `(vx, vy)` from the top left is `(vx - 160, 112 - vy)`.

use std::collections::HashMap;

use tantalus_contracts::LegacyFieldCamera;
use tantalus_game::field::world::CameraRequest;

use crate::projection::{project_field_point, view_range};
use crate::scene_adapter::CHAR_AIM_HEIGHT;

const HALF: [f32; 2] = [160.0, 112.0];
/// `UNSCROLL` with -1 frames.
const DEFAULT_RELEASE_FRAMES: u64 = 30;
/// The `SCROLL` mode that eases in and out (`FieldMapFlags.RotationScroll`).
const ROTATION_MODE: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Follow,
    /// Toward `end`, or back to the player when `end` is `None` (a release).
    Scroll {
        start: [f32; 2],
        end: Option<[f32; 2]>,
        from: u64,
        frames: u64,
        rotation: bool,
    },
    Hold([f32; 2]),
}

/// Steps the view for one field; feed it every game tick with
/// [`ViewScroll::observe`] and read it with [`ViewScroll::view`].
#[derive(Debug, Clone)]
pub struct ViewScroll {
    key: Option<String>,
    /// The tick each `camera_log` entry was first seen.
    stamps: Vec<u64>,
    applied: usize,
    frozen: bool,
    /// `BGCUNLOCK`: the view range is no longer clamped to the camera bounds.
    unlocked: bool,
    rotation: bool,
    mode: Mode,
    aim: f32,
    bounds: HashMap<i32, [f32; 4]>,
    at: Option<[f32; 2]>,
}

/// `BGCAM_DEF.RefreshCache`: a view wider than 320 pulls the scroll range in
/// by half the extra width on each side (all the way to its middle when the
/// range is narrower than the extra).
fn widen_range(range: [f32; 4]) -> [f32; 4] {
    let width = crate::widescreen::field_width();
    if width <= crate::widescreen::CLASSIC_WIDTH || range[0] <= -32768.0 {
        return range;
    }
    let delta = crate::widescreen::range_delta(width, range[1] - range[0]);
    [range[0] + delta, range[1] - delta, range[2], range[3]]
}

impl Default for ViewScroll {
    fn default() -> Self {
        Self {
            key: None,
            stamps: Vec::new(),
            applied: 0,
            frozen: false,
            unlocked: false,
            rotation: false,
            mode: Mode::Follow,
            aim: CHAR_AIM_HEIGHT,
            bounds: HashMap::new(),
            at: None,
        }
    }
}

impl ViewScroll {
    /// Notes the camera requests the field has logged by `tick`. A new
    /// field (another `key`, or a log that shrank) starts over.
    pub fn observe(&mut self, key: &str, log_len: usize, tick: u64) {
        if self.key.as_deref() != Some(key) || log_len < self.stamps.len() {
            *self = Self {
                key: Some(key.to_owned()),
                ..Self::default()
            };
        }
        while self.stamps.len() < log_len {
            self.stamps.push(tick);
        }
    }

    /// The frame center at `tick`, or `None` before there is anything to
    /// look at. `player` is the player's world position.
    pub fn view(
        &mut self,
        log: &[CameraRequest],
        camera: &LegacyFieldCamera,
        camera_index: u32,
        player: Option<[f32; 3]>,
        tick: u64,
    ) -> Option<[f32; 2]> {
        let mut range = self.range(camera, camera_index);
        while self.applied < self.stamps.len().min(log.len()) {
            let stamp = self.stamps[self.applied];
            let target = self.target(camera, range, player);
            self.advance(stamp, target);
            self.apply(log[self.applied], stamp, camera, target);
            self.applied += 1;
            range = self.range(camera, camera_index);
        }
        let target = self.target(camera, range, player);
        self.advance(tick, target);
        // Widescreen keeps scripted pans inside the narrowed range too
        // (`FieldMap.EBG_scene2DScroll` clamps `destX` to `vrpMinX..vrpMaxX`).
        if crate::widescreen::field_width() > crate::widescreen::CLASSIC_WIDTH
            && !self.unlocked
            && !(matches!(self.mode, Mode::Follow) && crate::widescreen::follow_is_fixed())
            && let Some(at) = self.at.as_mut()
        {
            at[0] = at[0].clamp(range[0], range[1].max(range[0]));
        }
        // Memoria's widescreen cases on the camera's y (`CenterCameraOnPlayer`), applied to the
        // view drawn only: the scroll keeps its own position. Memoria does not bound the
        // result; the 609/2159 parabola runs past the top of the art once the player is a
        // few pixels higher on screen than it reckons with, so it is kept inside the
        // camera's range here (unless a script unlocked the bounds).
        self.at.map(|[x, y]| {
            let moved = crate::widescreen::follow_y(y);
            if moved == y || self.unlocked {
                [x, moved]
            } else {
                [x, moved.clamp(range[2], range[3].max(range[2]).max(y))]
            }
        })
    }

    fn range(&self, camera: &LegacyFieldCamera, index: u32) -> [f32; 4] {
        if self.unlocked {
            // `EBG_charLookAtUnlock` widens the camera bounds to the `short` limits.
            return [-32768.0, 32767.0, -32768.0, 32767.0];
        }
        let range = i32::try_from(index)
            .ok()
            .and_then(|index| self.bounds.get(&index))
            .copied()
            .unwrap_or_else(|| view_range(camera));
        widen_range(range)
    }

    fn target(
        &self,
        camera: &LegacyFieldCamera,
        range: [f32; 4],
        player: Option<[f32; 3]>,
    ) -> Option<[f32; 2]> {
        let [x, y, z] = player?;
        let [tx, ty] = project_field_point(camera, [x, y + self.aim, z])?;
        let x = tx.clamp(range[0], range[1]);
        // Memoria's hand-tuned widescreen follow cases (`CenterCameraOnPlayer`).
        let x = if self.unlocked {
            x
        } else {
            crate::widescreen::follow_center(x, f32::from(camera.size[0]))
        };
        Some([x, ty.clamp(range[2], range[3])])
    }

    fn advance(&mut self, now: u64, target: Option<[f32; 2]>) {
        if self.frozen {
            return;
        }
        match self.mode {
            Mode::Follow => self.at = target.or(self.at),
            Mode::Hold(at) => self.at = Some(at),
            Mode::Scroll {
                start,
                end,
                from,
                frames,
                rotation,
            } => {
                let done = now.saturating_sub(from);
                let goal = end.or(target).unwrap_or(start);
                if done >= frames {
                    self.at = Some(goal);
                    self.mode = match end {
                        Some(end) => Mode::Hold(end),
                        None => Mode::Follow,
                    };
                } else {
                    let t = done as f32 / frames as f32;
                    let f = if rotation {
                        (1.0 - (std::f32::consts::PI * t).cos()) / 2.0
                    } else {
                        t
                    };
                    self.at = Some([
                        start[0] + (goal[0] - start[0]) * f,
                        start[1] + (goal[1] - start[1]) * f,
                    ]);
                }
            }
        }
    }

    fn apply(
        &mut self,
        request: CameraRequest,
        stamp: u64,
        camera: &LegacyFieldCamera,
        target: Option<[f32; 2]>,
    ) {
        let start = self.at.or(target).unwrap_or([0.0, 0.0]);
        match request {
            CameraRequest::Move { x, y, frames, mode } => {
                if self.frozen {
                    return;
                }
                self.rotation = mode == ROTATION_MODE;
                self.mode = Mode::Scroll {
                    start,
                    end: Some([f32::from(x) - HALF[0], HALF[1] - f32::from(y)]),
                    from: stamp,
                    frames: u64::from(frames),
                    rotation: self.rotation,
                };
            }
            CameraRequest::Release { frames, mode } => self.release(frames, mode, stamp, start),
            CameraRequest::Services {
                active,
                frames,
                mode,
            } => {
                self.frozen = !active;
                if active && frames > 0 {
                    self.release(frames, mode, stamp, start);
                }
            }
            CameraRequest::FollowHeight(height) => self.aim = f32::from(height),
            CameraRequest::FollowLock(locked) => self.unlocked = !locked,
            CameraRequest::Bounds {
                camera: index,
                bounds,
            } => {
                let [w, h] = camera.size.map(f32::from);
                let [_, max_x, min_y, max_y] = bounds.map(f32::from);
                let min_x = f32::from(crate::widescreen::camera_min_x(bounds[0]));
                let vx = [
                    (min_x + HALF[0]).min(w - HALF[0]),
                    (max_x - HALF[0]).max(HALF[0]),
                ];
                let vy = [
                    (min_y + HALF[1]).min(h - HALF[1]),
                    (max_y - HALF[1]).max(HALF[1]),
                ];
                self.bounds.insert(
                    index,
                    [
                        vx[0] - HALF[0],
                        vx[1] - HALF[0],
                        HALF[1] - vy[1],
                        HALF[1] - vy[0],
                    ],
                );
            }
        }
    }

    fn release(&mut self, frames: i32, mode: u32, stamp: u64, start: [f32; 2]) {
        if mode != u32::MAX {
            self.rotation = mode == ROTATION_MODE;
        }
        self.mode = Mode::Scroll {
            start,
            end: None,
            from: stamp,
            frames: u64::try_from(frames).unwrap_or(DEFAULT_RELEASE_FRAMES),
            rotation: self.rotation,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tantalus_contracts::Point3;

    /// A camera that maps world `(x, y, z)` to screen `(x, -y)`-ish is
    /// overkill here; the tests feed no player and check script motion.
    fn camera() -> LegacyFieldCamera {
        LegacyFieldCamera {
            projection: 256,
            rotation: [4096, 0, 0, 0, 4096, 0, 0, 0, 4096],
            translation: Point3 { x: 0, y: 0, z: 0 },
            center_offset: [0, 0],
            size: [640, 448],
            viewport_x: [160, 480],
            viewport_y: [112, 336],
            depth_offset: 0,
        }
    }

    #[test]
    fn a_scroll_eases_to_its_end_and_holds() {
        let log = [CameraRequest::Move {
            x: 260,
            y: 112,
            frames: 10,
            mode: 0,
        }];
        let mut view = ViewScroll {
            at: Some([0.0, 0.0]),
            ..ViewScroll::default()
        };
        view.observe("f", 1, 100);
        assert_eq!(view.view(&log, &camera(), 0, None, 105), Some([50.0, 0.0]));
        assert_eq!(view.view(&log, &camera(), 0, None, 110), Some([100.0, 0.0]));
        assert_eq!(view.view(&log, &camera(), 0, None, 500), Some([100.0, 0.0]));
        assert_eq!(view.mode, Mode::Hold([100.0, 0.0]));
    }

    #[test]
    fn rotation_mode_eases_in_and_out() {
        let log = [CameraRequest::Move {
            x: 260,
            y: 112,
            frames: 10,
            mode: ROTATION_MODE,
        }];
        let mut view = ViewScroll {
            at: Some([0.0, 0.0]),
            ..ViewScroll::default()
        };
        view.observe("f", 1, 0);
        let half = view.view(&log, &camera(), 0, None, 5).unwrap();
        assert!((half[0] - 50.0).abs() < 1e-3);
        let early = view.view(&log, &camera(), 0, None, 6).unwrap();
        assert!(early[0] > 60.0);
    }

    #[test]
    fn a_new_field_starts_over_and_services_off_freezes() {
        let mut view = ViewScroll::default();
        view.observe("a", 1, 0);
        view.observe("b", 0, 1);
        assert!(view.stamps.is_empty());
        let log = [CameraRequest::Services {
            active: false,
            frames: 0,
            mode: 0,
        }];
        view.at = Some([5.0, 5.0]);
        view.observe("b", 1, 2);
        view.view(&log, &camera(), 0, None, 3);
        assert!(view.frozen);
        assert_eq!(view.at, Some([5.0, 5.0]));
    }

    #[test]
    fn script_bounds_follow_memoria() {
        let log = [CameraRequest::Bounds {
            camera: 0,
            bounds: [0, 400, 0, 300],
        }];
        let mut view = ViewScroll::default();
        view.observe("f", 1, 0);
        view.view(&log, &camera(), 0, None, 0);
        // vrp x 160..240, y 112..188.
        assert_eq!(view.bounds[&0], [0.0, 80.0, -76.0, 0.0]);
    }
}
