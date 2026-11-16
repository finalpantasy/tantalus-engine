//! World camera aim follow (`ff9.w_cameraSetEyeAim`, `Global/ff9/ff9.cs:2773`,
//! commit c716e1fa), the part that is pure arithmetic.
//!
//! Ported: the aim point, which copies the controlled actor's x and z and eases its
//! height. Not ported: the eye, which depends on ray casts down onto the terrain
//! (`w_cellHit` around the eye, `w_cameraHit`), `w_cameraPosstatNow.cameraCorrect`,
//! `w_cameraEyeOffset` and `w_cameraDirVector` (set by the camera mode code,
//! `w_cameraChange`), the event-driven aim branch and the smooth-transition
//! branch (the source forces `w_cameraInter` to false right before testing it).

use super::vec3::Vec3;

/// Dead band of the height easing (`ff9.w_cameraSetEyeAim`: `0.5f`).
pub const AIM_DEAD_BAND: f32 = 0.5;
/// Smallest height step that is applied.
pub const AIM_MIN_STEP: f32 = 0.015_625;

/// Controlled actor `type` (`s_moveCHRControl.type`) as the aim code switches on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AimMode {
    /// Types 0 and 3: slow, speed scaled by the height gap.
    Ground,
    /// Types 1 and 2: fixed ratios, quicker up than down.
    Vehicle,
}

impl AimMode {
    pub fn from_control_kind(kind: u8) -> Option<Self> {
        match kind {
            0 | 3 => Some(Self::Ground),
            1 | 2 => Some(Self::Vehicle),
            _ => None,
        }
    }
}

/// One frame of the aim follow without an event aim. `aim_offset` is
/// `w_cameraAimOffset`; `fix_mode_y` is `w_cameraFixModeY`.
pub fn follow_aim(
    aim: Vec3,
    actor_pos: Vec3,
    aim_offset: f32,
    mode: AimMode,
    fix_mode_y: bool,
) -> Vec3 {
    let target_y = actor_pos.y + aim_offset;
    let diff_y = target_y - aim.y;
    let direction = if diff_y > AIM_DEAD_BAND {
        2
    } else if diff_y < -AIM_DEAD_BAND {
        1
    } else {
        0
    };
    let nsp = (diff_y.abs() / 32.0).clamp(0.125, 1.0);
    let mut step = 0.0_f32;
    match (mode, direction) {
        (AimMode::Ground, 1) => step = (diff_y + 0.5) * 1.0 / (32.0 * nsp),
        (AimMode::Ground, 2) => step = (diff_y - 0.5) * 1.0 / (32.0 * nsp),
        (AimMode::Vehicle, 1) => step = (diff_y + 0.5) / 2.0,
        (AimMode::Vehicle, 2) => step = (diff_y - 0.5) / 8.0,
        _ => {}
    }
    let mut out = Vec3::new(actor_pos.x, aim.y, actor_pos.z);
    if step.abs() > AIM_MIN_STEP && !fix_mode_y {
        out.y = aim.y + step * 2.0;
    }
    out
}
