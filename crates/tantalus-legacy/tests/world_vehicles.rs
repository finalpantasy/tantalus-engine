//! World vehicle tests, pinned to Memoria commit c716e1fa: the control table
//! (`Global/ff9/ff9.cs:1464`), `w_movementShipOperation` (`ff9.cs:5918`),
//! `w_movementPlaneOperation` (`ff9.cs:5980`) and `w_movementChange` (`ff9.cs:5658`).

use tantalus_legacy::world::{
    CONTROL_TABLE, PlaneInput, ShipInput, VEHICLES, Vec3, VehicleMotion, VehicleOperation,
    VehicleSpec, advance_xz, advance_y, matches_control_table, plane_step, psx_rot, ship_step,
};

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 1e-4,
        "expected {expected}, got {actual}"
    );
}

#[test]
fn vehicle_table_agrees_with_the_movement_control_table() {
    assert_eq!(VEHICLES.len(), CONTROL_TABLE.len());
    assert!(matches_control_table());
    for (index, spec) in VEHICLES.iter().enumerate() {
        assert_eq!(spec.control_no, index);
    }
}

#[test]
fn operations_follow_the_type_switch() {
    let op = |n| VehicleSpec::get(n).unwrap().operation();
    assert_eq!(op(0), VehicleOperation::Ground);
    assert_eq!(op(5), VehicleOperation::Ground);
    assert_eq!(op(6), VehicleOperation::Plane);
    assert_eq!(op(7), VehicleOperation::Ship);
    assert_eq!(op(8), VehicleOperation::Plane);
    assert_eq!(op(9), VehicleOperation::Plane);
    assert_eq!(op(10), VehicleOperation::None);
}

#[test]
fn terrain_access_per_vehicle() {
    let spec = |n| VehicleSpec::get(n).unwrap();
    // Blue Narciss: limit[0] = 0x02600000 = topographs 53, 54, 57; limit[1] = 0.
    assert_eq!(spec(7).allowed_topographs(), vec![53, 54, 57]);
    // Foot and yellow chocobo share a table; the other chocobos only add terrain.
    let foot = spec(0).allowed_topographs();
    assert_eq!(foot, spec(1).allowed_topographs());
    for chocobo in 2..=5 {
        let more = spec(chocobo).allowed_topographs();
        assert!(foot.iter().all(|t| more.contains(t)), "chocobo {chocobo}");
    }
    // Aircraft may enter every terrain the gold chocobo on the ground can.
    let gold = spec(5).allowed_topographs();
    for aircraft in [6, 8, 9] {
        let air = spec(aircraft).allowed_topographs();
        assert!(gold.iter().all(|t| air.contains(t)), "aircraft {aircraft}");
    }
    // The ship cannot enter plain land (topograph 0 bit is in limit[1]).
    assert!(!spec(7).can_enter(0));
    assert!(spec(7).can_enter(53 << 2));
    // Type 3 slot allows everything; the last slot nothing.
    assert!(spec(10).can_enter(0x1234));
    assert!(!spec(11).can_enter(0x1234));
}

#[test]
fn ship_first_step_matches_the_source_arithmetic() {
    let narciss = VehicleSpec::get(7).unwrap();
    let mut motion = VehicleMotion::default();
    let input = ShipInput {
        rotate: 127,
        left_y: -127,
        ..ShipInput::default()
    };
    ship_step(narciss, &mut motion, &input);
    // S(240 * 127 / 4) = 7620 / 256 = 29.765625; one eighth of it enters alpha.
    assert_close(motion.xz_alpha, 29.765_625 / 8.0);
    assert_close(motion.xz_speed, 29.765_625 / 8.0 / 32.0);
    // (36 * 127) >> 7 = 35 steps of 360/4096 degrees.
    assert_close(motion.rot_speed, psx_rot(35));
    assert_close(motion.rot_y, psx_rot(35));
    assert_close(motion.rot_true, motion.rot_y);
}

#[test]
fn ship_converges_and_stick_choice_picks_the_larger_magnitude() {
    let narciss = VehicleSpec::get(7).unwrap();
    let mut motion = VehicleMotion::default();
    // Left stick 50 forward, right stick 100: the right stick wins.
    let input = ShipInput {
        left_y: -50,
        right: 100,
        ..ShipInput::default()
    };
    for _ in 0..400 {
        ship_step(narciss, &mut motion, &input);
    }
    let target = 240.0 * 100.0 / 4.0 / 256.0;
    assert_close(motion.xz_alpha, target);
    assert_close(motion.xz_speed, target / 32.0);
    // Reversing: right stick back beats a weaker forward left stick.
    let mut motion = VehicleMotion::default();
    let back = ShipInput {
        left_y: -10,
        right: -100,
        ..ShipInput::default()
    };
    for _ in 0..400 {
        ship_step(narciss, &mut motion, &back);
    }
    assert!(motion.xz_speed < 0.0);
    // Engine volume caps at 127 and never drops below 20.
    let out = ship_step(
        narciss,
        &mut VehicleMotion::default(),
        &ShipInput::default(),
    );
    assert_eq!(out.engine_volume, 20);
}

#[test]
fn ship_fast_forward_uses_the_float_turn_rate() {
    let narciss = VehicleSpec::get(7).unwrap();
    let mut motion = VehicleMotion::default();
    let input = ShipInput {
        rotate: 127,
        fast_forward: true,
        ..ShipInput::default()
    };
    ship_step(narciss, &mut motion, &input);
    // (Int32)(36 * 127 / 256f) = 17.
    assert_close(motion.rot_speed, psx_rot(17));
}

#[test]
fn ship_camera_buttons_turn_the_camera_by_32_steps() {
    let narciss = VehicleSpec::get(7).unwrap();
    let mut motion = VehicleMotion::default();
    let out = ship_step(
        narciss,
        &mut motion,
        &ShipInput {
            r1: true,
            ..ShipInput::default()
        },
    );
    assert!(out.camera_turning);
    assert_close(out.camera_rotation_delta, psx_rot(32));
}

#[test]
fn changing_vehicle_clears_the_smoothing_accumulators_only() {
    let mut motion = VehicleMotion {
        xz_alpha: 5.0,
        y_alpha: 2.0,
        xz_speed: 1.0,
        rot_y: 90.0,
        ..VehicleMotion::default()
    };
    motion.change_vehicle();
    assert_eq!((motion.xz_alpha, motion.y_alpha), (0.0, 0.0));
    assert_eq!((motion.xz_speed, motion.rot_y), (1.0, 90.0));
}

#[test]
fn plane_first_step_for_hilda_garde() {
    let hilda = VehicleSpec::get(8).unwrap();
    let mut motion = VehicleMotion::default();
    let input = PlaneInput {
        left_x: 127,
        right: 127,
        left_y: -127,
        ..PlaneInput::default()
    };
    plane_step(hilda, &mut motion, &input);
    let forward = 500.0 * 127.0 / 4.0 / 256.0;
    assert_close(motion.xz_alpha, forward / 8.0);
    assert_close(motion.xz_speed, forward / 8.0 / 32.0);
    // Vertical: S(-255 * -127 / 4) = 32385 / 4 = 8096 (truncated) / 256.
    let up = 8096.0 / 256.0;
    assert_close(motion.y_alpha, up / 8.0);
    assert_close(motion.y_speed, up / 8.0 / 32.0);
    // (13 * 127) >> 7 = 12, then scaled by (580 - speed * 128) / 256.
    let scale = (580.0 - motion.xz_speed * 256.0 / 2.0) / 256.0;
    assert_close(motion.rot_speed, psx_rot(12) * scale);
}

#[test]
fn plane_invincible_uses_the_600_turn_factor() {
    let invincible = VehicleSpec::get(9).unwrap();
    let mut motion = VehicleMotion::default();
    plane_step(
        invincible,
        &mut motion,
        &PlaneInput {
            left_x: 127,
            ..PlaneInput::default()
        },
    );
    assert_close(motion.rot_speed, psx_rot(12) * (600.0 / 256.0));
}

#[test]
fn plane_roll_saturates_at_twice_the_roll_rate() {
    let hilda = VehicleSpec::get(8).unwrap();
    let mut motion = VehicleMotion::default();
    let input = PlaneInput {
        left_x: 127,
        ..PlaneInput::default()
    };
    for _ in 0..100 {
        plane_step(hilda, &mut motion, &input);
    }
    // speed_roll 127 * 2 = 254 steps.
    assert_close(motion.rot_z, psx_rot(-254).abs());
    // The other direction saturates at the negative limit.
    let mut motion = VehicleMotion::default();
    let input = PlaneInput {
        left_x: -127,
        ..PlaneInput::default()
    };
    for _ in 0..100 {
        plane_step(hilda, &mut motion, &input);
    }
    assert_close(motion.rot_z, psx_rot(-254));
}

#[test]
fn plane_shoulder_buttons_steer_but_do_not_roll() {
    let gold = VehicleSpec::get(6).unwrap();
    let mut motion = VehicleMotion::default();
    let input = PlaneInput {
        l1: true,
        ..PlaneInput::default()
    };
    plane_step(gold, &mut motion, &input);
    // L1 forces the stick to -127: (30 * -127) >> 7 = -30.
    assert_close(motion.rot_speed, psx_rot(-30));
    // The roll threshold keeps the physical stick value (0), so no roll builds.
    assert_close(motion.rot_z, 0.0);
}

#[test]
fn plane_camera_notrot_mode_turns_the_camera_instead() {
    let gold = VehicleSpec::get(6).unwrap();
    let mut motion = VehicleMotion::default();
    let out = plane_step(
        gold,
        &mut motion,
        &PlaneInput {
            r1: true,
            camera_notrot: true,
            ..PlaneInput::default()
        },
    );
    assert!(out.camera_turning);
    assert_close(out.camera_rotation_delta, psx_rot(16));
    assert_close(motion.rot_speed, 0.0);
}

#[test]
fn plane_doping_doubles_turn_and_speed() {
    let gold = VehicleSpec::get(6).unwrap();
    let base = |doping| {
        let mut motion = VehicleMotion::default();
        plane_step(
            gold,
            &mut motion,
            &PlaneInput {
                left_x: 64,
                right: 100,
                doping,
                ..PlaneInput::default()
            },
        );
        motion
    };
    let (normal, doped) = (base(false), base(true));
    assert_close(doped.xz_speed, normal.xz_speed * 2.0);
    assert_close(doped.rot_speed, normal.rot_speed * 2.0);
}

#[test]
fn inverted_flight_flips_the_climb() {
    let gold = VehicleSpec::get(6).unwrap();
    let climb = |inverted| {
        let mut motion = VehicleMotion::default();
        plane_step(
            gold,
            &mut motion,
            &PlaneInput {
                left_y: 100,
                inverted_flight_y: inverted,
                ..PlaneInput::default()
            },
        );
        motion.y_speed
    };
    assert!(climb(1) < 0.0);
    assert_close(climb(-1), -climb(1));
}

#[test]
fn kinematic_advance_follows_rot_true_plus_180() {
    let start = Vec3::new(10.0, 5.0, -20.0);
    // rot_true 0 heads along +z by cos(180 deg) = -1 per unit speed.
    let moved = advance_xz(start, 0.0, 2.0);
    assert_close(moved.x, 10.0);
    assert_close(moved.z, -22.0);
    assert_eq!(moved.y, 5.0);
    // Reverse speed is halved.
    let back = advance_xz(start, 0.0, -2.0);
    assert_close(back.z, -19.0);
    // rot_true 90: sin(270) = -1 moves -x.
    let side = advance_xz(start, 90.0, 3.0);
    assert_close(side.x, 7.0);
    assert_close(advance_y(start, 0.5).y, 5.5);
}
