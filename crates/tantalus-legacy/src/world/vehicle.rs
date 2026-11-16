//! World vehicles: the control table as typed data, terrain access, and the ship
//! and aircraft speed and turn integrators (`ff9.w_movementShipOperation`,
//! `w_movementPlaneOperation`, `w_movementChange`, commit c716e1fa).
//!
//! Not ported, and not guessed:
//! * landing and disembarking (`w_movementGetGetoff`): every branch depends on ray
//!   casts against the block meshes through `w_movementRoundCheck`, plus
//!   `checkGettingOffPosSpecially` and event global 194;
//! * the autopilot modes of `w_movementPlaneOperation` (they read the navigation
//!   location table);
//! * camera tilt (`w_cameraRotAngle`) and the particle and sound effects;
//! * collision for ships and aircraft: [`advance_xz`] and [`advance_y`] are the
//!   kinematic part only.
//!
//! The control number that selects a vehicle comes from event global 190
//! (`w_movementChange`); this module takes it as an argument.

use super::coords::{fixed_to_unity, psx_rot};
use super::movement::{CONTROL_TABLE, check_topograph};
use super::vec3::Vec3;

/// `ff9.p1`, `p2`, `p3` (`ff9/ff9.cs:395`): input scale, acceleration divisor and
/// speed divisor.
pub const P1: i32 = 4;
pub const P2: i32 = 8;
pub const P3: i32 = 32;

/// One row of `ff9.w_moveCHRControl` with the fields the vehicle code reads
/// (`ff9/ff9.cs:1464`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VehicleSpec {
    pub control_no: usize,
    pub name: &'static str,
    /// `type`: 0 walking, 1 flying, 2 boat, 3 other (no operation in
    /// `w_movementBasicOperation`).
    pub kind: u8,
    pub flg_gake: u8,
    pub speed_move: i16,
    pub speed_rotation: u8,
    pub speed_updown: u8,
    pub speed_roll: i8,
    pub flg_fly: bool,
    pub flg_upcam: bool,
    pub fetchdist: u8,
    pub music: u8,
    pub se: u8,
    pub encount: bool,
    pub radius: i16,
    pub camrot: bool,
    /// Terrain bitmaps: `limit[0]` topographs 32 to 63, `limit[1]` 0 to 31.
    pub limit: [u32; 2],
}

#[allow(clippy::too_many_arguments)]
const fn spec(
    control_no: usize,
    name: &'static str,
    kind: u8,
    flg_gake: u8,
    speed_move: i16,
    speed_rotation: u8,
    speed_updown: u8,
    speed_roll: i8,
    fly: bool,
    upcam: bool,
    fetchdist: u8,
    music: u8,
    se: u8,
    radius: i16,
    camrot: bool,
    limit: [u32; 2],
) -> VehicleSpec {
    VehicleSpec {
        control_no,
        name,
        kind,
        flg_gake,
        speed_move,
        speed_rotation,
        speed_updown,
        speed_roll,
        flg_fly: fly,
        flg_upcam: upcam,
        fetchdist,
        music,
        se,
        encount: control_no == 0,
        radius,
        camrot,
        limit,
    }
}

const LAND: u32 = 0xD8FF_3CFF;

/// `ff9.w_moveCHRControl[0..=11]` (`ff9/ff9.cs:1464-1750`). `speed_rollback` and
/// `pad2` are 0 in every row and not kept.
pub const VEHICLES: [VehicleSpec; 12] = [
    spec(
        0,
        "On foot",
        0,
        0,
        112,
        64,
        0,
        0,
        false,
        true,
        2,
        0,
        0,
        0,
        true,
        [0x0010_667F, LAND],
    ),
    spec(
        1,
        "Yellow Chocobo",
        0,
        0,
        200,
        64,
        0,
        0,
        false,
        true,
        2,
        1,
        0,
        300,
        true,
        [0x0010_667F, LAND],
    ),
    spec(
        2,
        "Light Blue Chocobo",
        0,
        1,
        200,
        64,
        0,
        0,
        false,
        true,
        2,
        1,
        0,
        300,
        true,
        [0x20F8_667F, LAND],
    ),
    spec(
        3,
        "Red Chocobo",
        0,
        1,
        200,
        64,
        0,
        0,
        false,
        true,
        2,
        1,
        0,
        300,
        true,
        [0x20FE_667F, LAND],
    ),
    spec(
        4,
        "Deep Blue Chocobo",
        0,
        1,
        200,
        64,
        0,
        0,
        false,
        true,
        2,
        1,
        0,
        300,
        true,
        [0x23FF_667F, LAND],
    ),
    spec(
        5,
        "Gold Chocobo (ground)",
        0,
        1,
        200,
        64,
        0,
        0,
        false,
        true,
        2,
        1,
        0,
        300,
        true,
        [0x23FF_667F, LAND],
    ),
    spec(
        6,
        "Gold Chocobo (air)",
        1,
        0,
        300,
        30,
        128,
        64,
        true,
        false,
        3,
        1,
        0,
        400,
        false,
        [0x7FFF_667F, LAND],
    ),
    spec(
        7,
        "Blue Narciss",
        2,
        0,
        240,
        36,
        0,
        0,
        false,
        true,
        3,
        0,
        24,
        560,
        false,
        [0x0260_0000, 0],
    ),
    spec(
        8,
        "Hilda Garde III",
        1,
        0,
        500,
        13,
        255,
        127,
        true,
        false,
        4,
        2,
        23,
        640,
        false,
        [0x7FFF_667F, LAND],
    ),
    spec(
        9,
        "Invincible",
        1,
        0,
        500,
        13,
        255,
        127,
        true,
        false,
        4,
        2,
        22,
        720,
        false,
        [0x7FFF_667F, LAND],
    ),
    spec(
        10,
        "Unused (type 3)",
        3,
        0,
        140,
        32,
        255,
        0,
        false,
        true,
        3,
        0,
        0,
        480,
        false,
        [u32::MAX, u32::MAX],
    ),
    spec(
        11,
        "Unused",
        0,
        0,
        0,
        0,
        0,
        0,
        false,
        false,
        0,
        0,
        0,
        0,
        false,
        [0, 0],
    ),
];

impl VehicleSpec {
    pub fn get(control_no: usize) -> Option<&'static VehicleSpec> {
        VEHICLES.get(control_no)
    }

    /// `w_movementCheckTopographID(control.limit, id)`: whether this vehicle may
    /// enter the terrain under a map id (`w_cellHit`'s allow test; the `flg_gake`
    /// water/ground edge rule is in `FootWalker` for ground vehicles).
    pub fn can_enter(&self, map_id: i32) -> bool {
        check_topograph(&self.limit, map_id)
    }

    /// Topographs 0 to 63 this vehicle may enter.
    pub fn allowed_topographs(&self) -> Vec<u8> {
        (0..64_u8)
            .filter(|&topograph| self.can_enter(i32::from(topograph) << 2))
            .collect()
    }

    /// Ground vehicles (chocobos) share the on-foot loop; the others have their own
    /// operation.
    pub fn operation(&self) -> VehicleOperation {
        match self.kind {
            0 => VehicleOperation::Ground,
            1 => VehicleOperation::Plane,
            2 => VehicleOperation::Ship,
            _ => VehicleOperation::None,
        }
    }
}

/// `w_movementBasicOperation`'s switch on `type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VehicleOperation {
    Ground,
    Plane,
    Ship,
    /// Type 3 has no case in the switch.
    None,
}

/// Does the table in this module agree with [`CONTROL_TABLE`] on the shared fields?
pub fn matches_control_table() -> bool {
    CONTROL_TABLE.iter().zip(VEHICLES.iter()).all(|(c, v)| {
        c.kind == v.kind
            && c.flg_gake == v.flg_gake
            && c.speed_move == v.speed_move
            && c.speed_rotation == v.speed_rotation
            && c.flg_fly == v.flg_fly
            && c.flg_upcam == v.flg_upcam
            && c.encount == v.encount
            && c.camrot == v.camrot
            && c.limit == v.limit
    })
}

/// The mutable speed state `w_movementChange` resets (`ff9/ff9.cs:5660`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct VehicleMotion {
    /// `w_moveCHRControl_XZAlpha`.
    pub xz_alpha: f32,
    /// `w_moveCHRControl_XZSpeed`.
    pub xz_speed: f32,
    pub y_alpha: f32,
    /// `w_moveCHRControl_YSpeed`.
    pub y_speed: f32,
    /// Actor yaw in degrees (`rot[1]`).
    pub rot_y: f32,
    /// Actor roll in degrees (`rot[2]`, aircraft only).
    pub rot_z: f32,
    /// `w_moveCHRControl_RotTrue`.
    pub rot_true: f32,
    /// `w_moveCHRControl_RotSpeed`.
    pub rot_speed: f32,
}

impl VehicleMotion {
    /// `w_movementChange` zeroes both smoothing accumulators when the vehicle
    /// changes.
    pub fn change_vehicle(&mut self) {
        self.xz_alpha = 0.0;
        self.y_alpha = 0.0;
    }
}

/// What a step reports besides the motion state.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct VehicleStepOutput {
    /// `w_moveCHRControl_LR`: the player is turning the camera by hand.
    pub camera_turning: bool,
    /// Degrees added to `w_cameraSysDataCamera.rotation` by the shoulder buttons.
    pub camera_rotation_delta: f32,
    /// Ship engine volume (`w_musicSEVolumeIntpl` argument); 0 for aircraft.
    pub engine_volume: u8,
}

/// Pad values of `w_movementShipOperation`. Sticks are signed bytes as
/// `w_moveGetPadState*` return them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShipInput {
    /// Left stick X.
    pub rotate: i32,
    /// Left stick Y as read, before the source negates it.
    pub left_y: i32,
    /// `w_moveGetPadStateR`.
    pub right: i32,
    pub l1: bool,
    pub r1: bool,
    /// `FF9StateSystem.Settings.IsFastForward`.
    pub fast_forward: bool,
}

/// `ff9.w_movementShipOperation` (`ff9/ff9.cs:5918`) without effects and with the
/// right-stick camera option off.
pub fn ship_step(
    spec: &VehicleSpec,
    motion: &mut VehicleMotion,
    input: &ShipInput,
) -> VehicleStepOutput {
    let mut front_back = -input.left_y;
    let alt = input.right;
    if alt > 0 && alt > front_back {
        front_back = alt;
    }
    if alt < 0 && alt < front_back {
        front_back = alt;
    }
    let speed_rotation = i32::from(spec.speed_rotation);
    motion.rot_speed = if input.fast_forward {
        // `(Int32)(speed * rotate / 256f)`: float division, then truncation.
        psx_rot((speed_rotation as f32 * input.rotate as f32 / 256.0) as i32)
    } else {
        psx_rot((speed_rotation * input.rotate) >> 7)
    };
    let mut out = VehicleStepOutput::default();
    if input.l1 {
        out.camera_rotation_delta -= psx_rot(32);
        out.camera_turning = true;
    }
    if input.r1 {
        out.camera_rotation_delta += psx_rot(32);
        out.camera_turning = true;
    }
    let acceleration = fixed_to_unity(i32::from(spec.speed_move) * front_back / P1);
    motion.xz_alpha += (acceleration - motion.xz_alpha) / P2 as f32;
    motion.xz_speed = motion.xz_alpha / P3 as f32;
    let volume = ((motion.xz_speed * 256.0) as i32).abs() / 8 + 20;
    out.engine_volume = volume.min(127) as u8;
    motion.rot_y = (motion.rot_y + motion.rot_speed) % 360.0;
    motion.rot_true = motion.rot_y;
    out
}

/// Pad and option values of `w_movementPlaneOperation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaneInput {
    pub left_x: i32,
    pub left_y: i32,
    /// `w_moveGetPadStateR`: forward throttle (negative brakes).
    pub right: i32,
    pub l1: bool,
    pub r1: bool,
    /// `w_cameraSysData.cameraNotrot`.
    pub camera_notrot: bool,
    /// `w_moveDoping`.
    pub doping: bool,
    /// `Configuration.AnalogControl.InvertedFlightY`, 1 or -1.
    pub inverted_flight_y: i32,
}

impl Default for PlaneInput {
    fn default() -> Self {
        Self {
            left_x: 0,
            left_y: 0,
            right: 0,
            l1: false,
            r1: false,
            camera_notrot: false,
            doping: false,
            inverted_flight_y: 1,
        }
    }
}

/// `ff9.w_movementPlaneOperation` (`ff9/ff9.cs:5980`) with autopilot off, the
/// alternate controls off, no camera tilt and no effects.
pub fn plane_step(
    spec: &VehicleSpec,
    motion: &mut VehicleMotion,
    input: &PlaneInput,
) -> VehicleStepOutput {
    let mut out = VehicleStepOutput::default();
    let left_y = input.left_y * input.inverted_flight_y;
    let mut left_x = input.left_x;
    // `cameraChangeThreshold` keeps the stick value before the shoulder override.
    let threshold = left_x;
    if !input.camera_notrot {
        if input.l1 {
            left_x = -127;
        }
        if input.r1 {
            left_x = 127;
        }
    } else {
        if input.r1 {
            out.camera_rotation_delta += psx_rot(16);
            out.camera_turning = true;
        }
        if input.l1 {
            out.camera_rotation_delta -= psx_rot(16);
            out.camera_turning = true;
        }
    }
    motion.rot_speed = psx_rot((i32::from(spec.speed_rotation) * left_x) >> 7);
    let vertical = fixed_to_unity(-i32::from(spec.speed_updown) * left_y / P1);
    motion.y_alpha += (vertical - motion.y_alpha) / P2 as f32;
    motion.y_speed = motion.y_alpha / P3 as f32;
    let forward = fixed_to_unity(i32::from(spec.speed_move) * input.right / P1);
    motion.xz_alpha += (forward - motion.xz_alpha) / P2 as f32;
    motion.xz_speed = motion.xz_alpha / P3 as f32;
    if input.doping {
        motion.rot_speed *= 2.0;
        motion.xz_speed *= 2.0;
    }
    // Control 9 (Invincible) and 8 (Hilda Garde III) turn slower at speed.
    if spec.control_no == 9 {
        motion.rot_speed *= 600.0 - motion.xz_speed * 256.0 / 2.0;
        motion.rot_speed /= 256.0;
    }
    if spec.control_no == 8 {
        motion.rot_speed *= 580.0 - motion.xz_speed * 256.0 / 2.0;
        motion.rot_speed /= 256.0;
    }
    if motion.rot_z > 180.0 {
        motion.rot_z -= 360.0;
    }
    motion.rot_y = (motion.rot_y + motion.rot_speed) % 360.0;
    motion.rot_true = motion.rot_y % 360.0;
    if motion.rot_speed > 0.0 && motion.rot_z > psx_rot(-(threshold * 2)) {
        motion.rot_z += psx_rot(threshold / 4);
    }
    if motion.rot_speed < 0.0 && motion.rot_z < psx_rot(-(threshold * 2)) {
        motion.rot_z += psx_rot(threshold / 4);
    }
    let roll_limit = psx_rot(-i32::from(spec.speed_roll) * 2);
    motion.rot_z = motion.rot_z.max(roll_limit).min(-roll_limit);
    out
}

fn rsin(degrees: f32) -> f32 {
    (degrees * 0.017_453_292).sin()
}

fn rcos(degrees: f32) -> f32 {
    (degrees * 0.017_453_292).cos()
}

/// The horizontal step of `w_movementRoundCheck` (`ff9/ff9.cs:5400`) without the
/// ground test: move `speed` along `rot_true + 180`. Reverse speed is halved first,
/// as `w_movementControl` does.
pub fn advance_xz(pos: Vec3, rot_true: f32, xz_speed: f32) -> Vec3 {
    let speed = if xz_speed < 0.0 {
        xz_speed / 2.0
    } else {
        xz_speed
    };
    let heading = rot_true + psx_rot(2048);
    Vec3::new(
        pos.x + rsin(heading) * speed,
        pos.y,
        pos.z + rcos(heading) * speed,
    )
}

/// The flying branch of `w_movementControl` adds `w_moveCHRControl_YSpeed` to the
/// actor height.
pub fn advance_y(pos: Vec3, y_speed: f32) -> Vec3 {
    Vec3::new(pos.x, pos.y + y_speed, pos.z)
}
