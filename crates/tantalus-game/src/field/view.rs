//! The field view position the scripts read back (`VRP`, `CalculateScreenOrigin`).
//!
//! Memoria `FieldMap` keeps `curVRP`, the top-left of the view, and moves it every frame in
//! `HonoLateUpdate`: `SceneService2DScroll` runs a scripted pan (`EBG_scene2DScroll`,
//! `EBG_scene2DScrollRelease`), else `SceneService3DScroll` follows the player clamped to the
//! camera's `vrpMin`/`vrpMax`. `EBG_sceneGetVRP` reports `curVRP + centerOffset + half field`,
//! the background pixel at the view center; this module keeps that reported value.
//!
//! The renderer has its own copy of this motion (`tantalus-client` `view_scroll`); this one only
//! feeds `GetScreenCalculatedX`/`Y` (system variables 12 and 13), which scripts like the Lindblum
//! telescope (field 606) poll.

use super::world::CameraRequest;
use tantalus_contracts::LegacyFieldCamera;

/// `FieldMap.charAimHeight` at `BG_init`.
pub const CHAR_AIM_HEIGHT: f32 = 324.0;
/// `FieldMapFlags.RotationScroll`.
const ROTATION_SCROLL: u32 = 8;
/// `UNSCROLL` with -1 frames.
const DEFAULT_RELEASE_FRAMES: i32 = 30;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    /// `flags & Generic7 == 0`: `SceneService3DScroll` follows the player.
    Follow,
    /// A pan from `start` to `end` (`flags` 1 or 2 for a `SCROLL`, 3 for a release).
    Scroll {
        start: [f32; 2],
        end: [f32; 2],
        frame: i32,
        frames: i32,
        release: bool,
    },
    /// `Unknown4`: a finished `SCROLL` holds until a release.
    Hold,
}

/// One field's view state; [`FieldView::reset`] starts it for a field's cameras.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldView {
    cameras: Vec<LegacyFieldCamera>,
    /// Per camera `[vrpMinX, vrpMaxX, vrpMinY, vrpMaxY]`, changed by `BGVPORT` and `BGCUNLOCK`.
    bounds: Vec<[f32; 4]>,
    /// `origVRPMin/Max` saved by `EBG_charLookAtUnlock`.
    saved_bounds: [f32; 4],
    camera: usize,
    /// `FieldMapFlags.Locked`.
    locked: bool,
    /// `FieldMapFlags.Active` (`SCROLLON`/`SCROLLOFF`).
    active: bool,
    rotation: bool,
    phase: Phase,
    /// The reported view position (`EBG_sceneGetVRP`).
    vrp: [f32; 2],
    aim: f32,
    /// `EventEngine.sSysX`/`sSysY`, the last `CalculateScreenOrigin` result.
    pub screen: [i32; 2],
}

impl Default for FieldView {
    fn default() -> Self {
        Self {
            cameras: Vec::new(),
            bounds: Vec::new(),
            saved_bounds: [0.0; 4],
            camera: 0,
            locked: true,
            active: true,
            rotation: false,
            phase: Phase::Follow,
            vrp: [0.0; 2],
            aim: CHAR_AIM_HEIGHT,
            screen: [0, 0],
        }
    }
}

impl FieldView {
    /// `BG_init`: flags `GenericInitial` (locked, active), the view centered in camera 0's range.
    pub fn reset(&mut self, cameras: Vec<LegacyFieldCamera>) {
        let bounds = cameras
            .iter()
            .map(|camera| {
                [
                    f32::from(camera.viewport_x[0]),
                    f32::from(camera.viewport_x[1]),
                    f32::from(camera.viewport_y[0]),
                    f32::from(camera.viewport_y[1]),
                ]
            })
            .collect();
        *self = Self {
            cameras,
            bounds,
            ..Self::default()
        };
        // `curVRP = (vrpMin + vrpMax) / 2 - centerOffset - half` in integer math, so the
        // reported value is the integer midpoint.
        if let Some(camera) = self.cameras.first() {
            self.vrp = [
                f32::from(
                    ((i32::from(camera.viewport_x[0]) + i32::from(camera.viewport_x[1])) / 2)
                        as i16,
                ),
                f32::from(
                    ((i32::from(camera.viewport_y[0]) + i32::from(camera.viewport_y[1])) / 2)
                        as i16,
                ),
            ];
        }
    }

    pub fn has_cameras(&self) -> bool {
        !self.cameras.is_empty()
    }

    /// `EBG_sceneGetVRP`: `(Int16)` of the reported position; zero without a camera
    /// (`cameraList.Count <= 0` leaves the outputs at 0).
    pub fn origin(&self) -> [i32; 2] {
        if self.cameras.is_empty() {
            return [0, 0];
        }
        self.vrp.map(|value| i32::from(value as i16))
    }

    /// `fieldmap.camIdx`, the camera `ActivateCamera` last picked.
    pub fn camera_index(&self) -> usize {
        self.camera
    }

    /// `ActivateCamera` for a valid camera index.
    pub fn set_camera(&mut self, camera: i32) {
        if let Ok(camera) = usize::try_from(camera)
            && camera < self.cameras.len()
        {
            self.camera = camera;
        }
    }

    pub fn apply(&mut self, request: CameraRequest, player: Option<[f32; 3]>) {
        match request {
            // `EBG_scene2DScroll`: from the current position to the script's point.
            CameraRequest::Move { x, y, frames, mode } => {
                if !self.active {
                    return;
                }
                if mode == ROTATION_SCROLL {
                    self.rotation = true;
                }
                self.phase = Phase::Scroll {
                    start: self.vrp,
                    end: [f32::from(x), f32::from(y)],
                    frame: 1,
                    frames: i32::from(frames as i16),
                    release: false,
                };
            }
            CameraRequest::Release { frames, mode } => self.release(frames, mode, player),
            // `EBG_char3DScrollSetActive`.
            CameraRequest::Services {
                active,
                frames,
                mode,
            } => {
                self.active = active;
                if active && frames > 0 {
                    self.release(frames, mode, player);
                }
            }
            CameraRequest::FollowHeight(height) => self.aim = f32::from(height),
            CameraRequest::FollowLock(lock) => {
                let Some(bounds) = self.bounds.get_mut(self.camera) else {
                    return;
                };
                if lock && !self.locked {
                    // `EBG_charLookAtLock` restores the saved range.
                    *bounds = self.saved_bounds;
                    self.locked = true;
                } else if !lock && self.locked {
                    // `EBG_charLookAtUnlock` widens the range to the `short` limits.
                    self.saved_bounds = *bounds;
                    *bounds = [-32768.0, 32767.0, -32768.0, 32767.0];
                    self.locked = false;
                }
            }
            // `EBG_cameraSetViewport`.
            CameraRequest::Bounds { camera, bounds } => {
                let Some(index) = usize::try_from(camera).ok() else {
                    return;
                };
                let (Some(data), Some(range)) =
                    (self.cameras.get(index), self.bounds.get_mut(index))
                else {
                    return;
                };
                let [w, h] = data.size.map(i32::from);
                let [min_x, max_x, min_y, max_y] = bounds.map(i32::from);
                *range = [
                    min_x.saturating_add(160).min(w - 160) as i16,
                    max_x.saturating_sub(160).max(160) as i16,
                    min_y.saturating_add(112).min(h - 112) as i16,
                    max_y.saturating_sub(112).max(112) as i16,
                ]
                .map(f32::from);
            }
        }
    }

    /// `EBG_scene2DScrollRelease`: back to where following the player would put the view.
    fn release(&mut self, frames: i32, mode: u32, player: Option<[f32; 3]>) {
        if !self.active {
            return;
        }
        let Some(end) = self.follow_point(player) else {
            return;
        };
        self.phase = Phase::Scroll {
            start: self.vrp,
            end,
            frame: 1,
            frames: if frames == -1 {
                DEFAULT_RELEASE_FRAMES
            } else {
                i32::from(frames as i16)
            },
            release: true,
        };
        if mode != u32::MAX {
            self.rotation = mode == ROTATION_SCROLL;
        }
    }

    /// Where `SceneService3DScroll` puts the view: the player's projected aim point (or the
    /// camera center without a player), clamped to the camera range.
    fn follow_point(&self, player: Option<[f32; 3]>) -> Option<[f32; 2]> {
        let camera = self.cameras.get(self.camera)?;
        let bounds = self.bounds.get(self.camera)?;
        let [w, h] = camera.size.map(|value| f32::from(value as i16));
        let [cx, cy] = camera.center_offset.map(f32::from);
        let raw = match player {
            Some([x, y, z]) => project(camera, [x, y + self.aim, z])?,
            None => [0.0, 0.0],
        };
        // `aimX = (w >> 1) + centerOffset.x + vertex.x - offset.x` and the y twin; with
        // `offset` folded in this is the raw projection moved to the background's center.
        let aim = [
            (w / 2.0).trunc() + cx + raw[0],
            (h / 2.0).trunc() + cy + raw[1],
        ];
        Some([
            aim[0].clamp(bounds[0].min(bounds[1]), bounds[1].max(bounds[0])),
            aim[1].clamp(bounds[2].min(bounds[3]), bounds[3].max(bounds[2])),
        ])
    }

    /// `HonoLateUpdate`: `SceneService2DScroll` then `SceneService3DScroll`, once per frame.
    pub fn service(&mut self, player: Option<[f32; 3]>) {
        if self.cameras.is_empty() {
            return;
        }
        if let Phase::Scroll {
            start,
            end,
            frame,
            frames,
            release,
        } = self.phase
        {
            if !self.active {
                return;
            }
            let total = frames.max(1) as f32;
            let progress = if self.rotation {
                // `(rcos(2048 * frame / frames + 2048) + 4096) / 8192`.
                let angle = 2048 * frame / frames.max(1) + 2048;
                (cos4096(angle) + 4096) as f32 / 8192.0
            } else {
                frame as f32 / total
            };
            self.vrp = [
                start[0] + (end[0] - start[0]) * progress,
                start[1] + (end[1] - start[1]) * progress,
            ];
            let frame = frame + 1;
            self.phase = if frame > frames {
                if release { Phase::Follow } else { Phase::Hold }
            } else {
                Phase::Scroll {
                    start,
                    end,
                    frame,
                    frames,
                    release,
                }
            };
            return;
        }
        if self.phase == Phase::Follow
            && self.active
            && let Some(point) = self.follow_point(player)
        {
            self.vrp = point;
        }
    }
}

/// `PSX.CalculateGTE_RTPT` without `FieldMap.offset`: `y` flips going in, the camera
/// transform (rotation in 1/4096 units plus translation), the divide by depth, `y` flips
/// back. The result is y-down, in background pixels from the projection center.
fn project(camera: &LegacyFieldCamera, point: [f32; 3]) -> Option<[f32; 2]> {
    let r = |row: usize, column: usize| f32::from(camera.rotation[row * 3 + column]) / 4096.0;
    let t = [
        camera.translation.x as f32,
        camera.translation.y as f32,
        camera.translation.z as f32,
    ];
    let v = [point[0], -point[1], point[2]];
    let mut out = [0.0f32; 3];
    for (row, value) in out.iter_mut().enumerate() {
        *value = r(row, 0) * v[0] + r(row, 1) * v[1] + r(row, 2) * v[2] + t[row];
    }
    let depth = out[2].abs();
    if depth < f32::EPSILON {
        return None;
    }
    let view = f32::from(camera.projection);
    Some([out[0] * view / depth, out[1] * view / depth])
}

/// `ff9.rcos`: cosine of a 4096-per-turn angle, scaled by 4096.
fn cos4096(angle: i32) -> i32 {
    let radians = f64::from(angle.rem_euclid(4096)) * std::f64::consts::TAU / 4096.0;
    (radians.cos() * 4096.0).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use tantalus_contracts::Point3;

    fn camera() -> LegacyFieldCamera {
        LegacyFieldCamera {
            projection: 256,
            rotation: [4096, 0, 0, 0, 4096, 0, 0, 0, 4096],
            translation: Point3 {
                x: 0,
                y: 0,
                z: 1000,
            },
            center_offset: [0, 0],
            size: [512, 336],
            viewport_x: [100, 412],
            viewport_y: [99, 238],
            depth_offset: 0,
        }
    }

    #[test]
    fn the_view_starts_at_the_range_midpoint() {
        let mut view = FieldView::default();
        assert_eq!(view.origin(), [0, 0]);
        view.reset(vec![camera()]);
        assert_eq!(view.origin(), [256, 168]);
    }

    #[test]
    fn a_one_frame_scroll_lands_and_holds() {
        let mut view = FieldView::default();
        view.reset(vec![camera()]);
        let request = CameraRequest::Move {
            x: 259,
            y: 165,
            frames: 1,
            mode: ROTATION_SCROLL,
        };
        view.apply(request, None);
        view.service(None);
        assert_eq!(view.origin(), [259, 165]);
        assert_eq!(view.phase, Phase::Hold);
        // Held: following the player does not move it.
        view.service(Some([5000.0, 0.0, 0.0]));
        assert_eq!(view.origin(), [259, 165]);
    }

    #[test]
    fn a_linear_scroll_steps_each_frame() {
        let mut view = FieldView::default();
        view.reset(vec![camera()]);
        view.apply(
            CameraRequest::Move {
                x: 356,
                y: 168,
                frames: 4,
                mode: 0,
            },
            None,
        );
        view.service(None);
        assert_eq!(view.origin(), [281, 168]);
        for _ in 0..3 {
            view.service(None);
        }
        assert_eq!(view.origin(), [356, 168]);
    }

    #[test]
    fn following_clamps_to_the_range_until_unlocked() {
        let mut view = FieldView::default();
        view.reset(vec![camera()]);
        // A player far right projects past vrpMaxX.
        view.service(Some([4000.0, -CHAR_AIM_HEIGHT, 0.0]));
        assert_eq!(view.origin()[0], 412);
        view.apply(CameraRequest::FollowLock(false), None);
        view.service(Some([4000.0, -CHAR_AIM_HEIGHT, 0.0]));
        assert_eq!(view.origin()[0], 256 + 1024);
        view.apply(CameraRequest::FollowLock(true), None);
        view.service(Some([4000.0, -CHAR_AIM_HEIGHT, 0.0]));
        assert_eq!(view.origin()[0], 412);
    }

    #[test]
    fn scrolls_are_ignored_while_services_are_off() {
        let mut view = FieldView::default();
        view.reset(vec![camera()]);
        view.apply(
            CameraRequest::Services {
                active: false,
                frames: 0,
                mode: 0,
            },
            None,
        );
        view.apply(
            CameraRequest::Move {
                x: 300,
                y: 120,
                frames: 1,
                mode: 0,
            },
            None,
        );
        view.service(None);
        assert_eq!(view.origin(), [256, 168]);
    }
}
