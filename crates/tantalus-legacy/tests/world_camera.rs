//! World camera aim follow, pinned to `ff9.w_cameraSetEyeAim`
//! (`Global/ff9/ff9.cs:2773`, Memoria commit c716e1fa).

use tantalus_legacy::world::{AimMode, Vec3, follow_aim};

fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-5, "{a} vs {b}");
}

#[test]
fn aim_copies_actor_x_and_z() {
    let aim = follow_aim(
        Vec3::new(0.0, 5.0, 0.0),
        Vec3::new(12.0, 5.0, -30.0),
        0.0,
        AimMode::Ground,
        false,
    );
    assert_eq!((aim.x, aim.z), (12.0, -30.0));
    // Inside the dead band the height stays put.
    close(aim.y, 5.0);
}

#[test]
fn ground_height_easing_scales_with_the_gap() {
    // Gap 32 above: nsp = 1, step = (32 - 0.5) / 32, applied twice.
    let up = follow_aim(
        Vec3::ZERO,
        Vec3::new(0.0, 32.0, 0.0),
        0.0,
        AimMode::Ground,
        false,
    );
    close(up.y, 2.0 * (31.5 / 32.0));
    // Gap 1 above: nsp clamps to 0.125, step = 0.5 / 4 = 0.125, doubled.
    let near = follow_aim(
        Vec3::ZERO,
        Vec3::new(0.0, 1.0, 0.0),
        0.0,
        AimMode::Ground,
        false,
    );
    close(near.y, 0.25);
    // Below: the dead band is added.
    let down = follow_aim(
        Vec3::ZERO,
        Vec3::new(0.0, -1.0, 0.0),
        0.0,
        AimMode::Ground,
        false,
    );
    close(down.y, -0.25);
}

#[test]
fn vehicles_climb_faster_than_they_drop() {
    let target = Vec3::new(0.0, 8.0, 0.0);
    let rise = follow_aim(Vec3::ZERO, target, 0.0, AimMode::Vehicle, false);
    close(rise.y, 2.0 * (7.5 / 8.0));
    let fall = follow_aim(
        Vec3::new(0.0, 8.0, 0.0),
        Vec3::ZERO,
        0.0,
        AimMode::Vehicle,
        false,
    );
    close(fall.y, 8.0 + 2.0 * ((-8.0 + 0.5) / 2.0));
}

#[test]
fn fixed_y_mode_and_the_offset() {
    let held = follow_aim(
        Vec3::ZERO,
        Vec3::new(0.0, 32.0, 0.0),
        0.0,
        AimMode::Ground,
        true,
    );
    assert_eq!(held.y, 0.0);
    // The offset shifts the target height.
    let lifted = follow_aim(Vec3::ZERO, Vec3::ZERO, 32.0, AimMode::Ground, false);
    close(lifted.y, 2.0 * (31.5 / 32.0));
}

#[test]
fn control_kinds_map_to_modes() {
    assert_eq!(AimMode::from_control_kind(0), Some(AimMode::Ground));
    assert_eq!(AimMode::from_control_kind(3), Some(AimMode::Ground));
    assert_eq!(AimMode::from_control_kind(1), Some(AimMode::Vehicle));
    assert_eq!(AimMode::from_control_kind(2), Some(AimMode::Vehicle));
    assert_eq!(AimMode::from_control_kind(4), None);
}
