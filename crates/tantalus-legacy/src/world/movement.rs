//! On-foot world movement, collision rules, camera yaw follow and the encounter and
//! event hooks, per `ff9.w_movementUpdate` (`Global/ff9/ff9.cs:4942`).
//!
//! One call of [`FootWalker::step`] is one `ff9.w_frameUpdate` phase-2 pass for the
//! controlled actor (Zidane, `w_moveCHRStatus[1]`, control 0). Ship, plane and
//! chocobo-flight control (types 1 to 3) are not ported; see
//! [`WorldMoveError::UnsupportedControl`].

use super::coords::{
    CELL_COLUMNS, cell_index, fixed_to_unity, id_event, id_topograph, psx_rot, unity_to_fixed,
};
use super::vec3::Vec3;
use super::walk_mesh::{MoveCache, NwpHit, RaycastMode, WorldWalk};

/// One row of `ff9.w_moveCHRControl` (`ff9/ff9.cs:1464`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlEntry {
    /// `type`: 0 walking, 1 flying, 2 boat, 3 other.
    pub kind: u8,
    pub flg_gake: u8,
    pub speed_move: i16,
    pub speed_rotation: u8,
    pub flg_fly: bool,
    pub flg_upcam: bool,
    pub encount: bool,
    pub camrot: bool,
    /// `limit[0]` covers topographs 32 to 63, `limit[1]` topographs 0 to 31.
    pub limit: [u32; 2],
}

#[allow(clippy::too_many_arguments)]
const fn entry(
    kind: u8,
    flg_gake: u8,
    speed_move: i16,
    speed_rotation: u8,
    flg_fly: bool,
    flg_upcam: bool,
    encount: bool,
    camrot: bool,
    limit: [u32; 2],
) -> ControlEntry {
    ControlEntry {
        kind,
        flg_gake,
        speed_move,
        speed_rotation,
        flg_fly,
        flg_upcam,
        encount,
        camrot,
        limit,
    }
}

/// `ff9.w_moveCHRControl` (`ff9/ff9.cs:1464-1750`). Index 0 is on foot, 1 to 5
/// chocobos, 6 gold chocobo in the air, 7 Blue Narciss, 8 Hilda Garde 3, 9
/// Invincible, 10 and 11 unused.
pub const CONTROL_TABLE: [ControlEntry; 12] = [
    entry(
        0,
        0,
        112,
        64,
        false,
        true,
        true,
        true,
        [0x0010_667F, 0xD8FF_3CFF],
    ),
    entry(
        0,
        0,
        200,
        64,
        false,
        true,
        false,
        true,
        [0x0010_667F, 0xD8FF_3CFF],
    ),
    entry(
        0,
        1,
        200,
        64,
        false,
        true,
        false,
        true,
        [0x20F8_667F, 0xD8FF_3CFF],
    ),
    entry(
        0,
        1,
        200,
        64,
        false,
        true,
        false,
        true,
        [0x20FE_667F, 0xD8FF_3CFF],
    ),
    entry(
        0,
        1,
        200,
        64,
        false,
        true,
        false,
        true,
        [0x23FF_667F, 0xD8FF_3CFF],
    ),
    entry(
        0,
        1,
        200,
        64,
        false,
        true,
        false,
        true,
        [0x23FF_667F, 0xD8FF_3CFF],
    ),
    entry(
        1,
        0,
        300,
        30,
        true,
        false,
        false,
        false,
        [0x7FFF_667F, 0xD8FF_3CFF],
    ),
    entry(2, 0, 240, 36, false, true, false, false, [0x0260_0000, 0]),
    entry(
        1,
        0,
        500,
        13,
        true,
        false,
        false,
        false,
        [0x7FFF_667F, 0xD8FF_3CFF],
    ),
    entry(
        1,
        0,
        500,
        13,
        true,
        false,
        false,
        false,
        [0x7FFF_667F, 0xD8FF_3CFF],
    ),
    entry(
        3,
        0,
        140,
        32,
        false,
        true,
        false,
        false,
        [u32::MAX, u32::MAX],
    ),
    entry(0, 0, 0, 0, false, false, false, false, [0, 0]),
];

/// `ff9.w_movementWaterStatus` (`ff9/ff9.cs:18`).
pub const WATER_STATUS: [u32; 2] = [0x23ED_0000, 0];
/// `ff9.w_movementGroundStatus` (`ff9/ff9.cs:9640`).
pub const GROUND_STATUS: [u32; 2] = [0x5C12_6670, 0x18FF_3CFF];
/// `ff9.w_movementSinkArray` (`ff9/ff9.cs:19`): rows are `slice_type`, columns the
/// sink context of [`slice_height`].
pub const SINK_ARRAY: [[i16; 9]; 4] = [
    [0; 9],
    [400, 150, 250, 200, 450, 450, 300, 300, 0],
    [0, 300, 300, 300, 300, 300, 300, 300, 0],
    [0; 9],
];

/// `ff9.w_movementCheckTopographID` (`ff9/ff9.cs:5641`): is the map id's topograph bit
/// set in a two-word table (`check[0]` = topographs 32 to 63).
pub fn check_topograph(check: &[u32; 2], id: i32) -> bool {
    let topograph = ((id as u32) & 0xFC) >> 2;
    if topograph > 31 {
        (check[0] >> (topograph - 32)) & 1 != 0
    } else {
        (check[1] >> topograph) & 1 != 0
    }
}

/// `ff9.w_movementGetSliceHeight` (`ff9/ff9.cs:5364`): how far a character sinks on
/// this terrain. The flag is true in forests (`imd`).
pub fn slice_height(slice_type: u8, id: i32) -> (f32, bool) {
    let context = match id_topograph(id) {
        36..=38 => 0,
        53 => 1,
        54 => 2,
        55 => 3,
        56 => 4,
        57 => 5,
        51 => 6,
        48 => 7,
        _ => 8,
    };
    let mut slice = i32::from(SINK_ARRAY[usize::from(slice_type)][context]);
    if slice != 0 {
        slice -= 100;
    }
    (fixed_to_unity(-slice), context == 0)
}

/// `ff9.rsin(Single)` / `ff9.rcos(Single)` (`ff9/ff9.cs:2104`): degrees.
fn rsin(degrees: f32) -> f32 {
    (degrees * 0.017_453_292).sin()
}

fn rcos(degrees: f32) -> f32 {
    (degrees * 0.017_453_292).cos()
}

/// `ff9.ratan2` (`ff9/ff9.cs:2097`): degrees.
fn ratan2(y: f32, x: f32) -> f32 {
    y.atan2(x) * 57.295_78
}

/// Normalizes a yaw to `[0, 360)` as `Transform.eulerAngles.y` reports it.
fn euler_yaw(degrees: f32) -> f32 {
    let wrapped = degrees % 360.0;
    if wrapped < 0.0 {
        wrapped + 360.0
    } else {
        wrapped
    }
}

/// `ff9.w_moveGetPadStateLX/LY` (`ff9/ff9.cs:6257`, `:6279`, desktop branch): axis
/// values in `-1..=1` to stick integers. Left or up are the negative axis side; a
/// stick past `0.1` is `+-128` digitally or `x * 128` in analog mode.
#[allow(clippy::manual_range_contains)]
pub fn stick_from_axis(x: f32, y: f32, analog: bool) -> (i32, i32) {
    let vx = if analog {
        if x < -0.1 || x > 0.1 {
            (x * 128.0) as i32
        } else {
            0
        }
    } else if x < -0.1 {
        -128
    } else if x > 0.1 {
        128
    } else {
        0
    };
    let vy = if analog {
        if y > 0.1 || y < -0.1 {
            (-y * 128.0) as i32
        } else {
            0
        }
    } else if y > 0.1 {
        -128
    } else if y < -0.1 {
        128
    } else {
        0
    };
    (vx, vy)
}

/// Pad input for one frame, after `w_moveGetPadStateLX/LY`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PadInput {
    pub stick_x: i32,
    pub stick_y: i32,
    /// `kPadL1` turns the camera by `-PsxRot(32)` degrees.
    pub l1: bool,
    /// `kPadR1` turns the camera by `+PsxRot(32)` degrees.
    pub r1: bool,
}

/// Frame-level flags the source reads from other systems.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameInputs {
    pub pad: PadInput,
    /// `ff9.w_frameMenuon`.
    pub menu_open: bool,
    /// `ff9.w_naviMode` (2 = map screen, input ignored).
    pub navi_mode: i32,
    /// `ff9.w_blockReady`.
    pub block_ready: bool,
    /// `ff9.w_cameraInter`.
    pub camera_inter: bool,
    /// `ff9.w_frameEncountMask`.
    pub encounter_mask: bool,
    /// `ff9.w_naviTitle`.
    pub navi_title: i32,
    /// `ff9.w_framePhase` (2 = running).
    pub frame_phase: i32,
    /// `ff9.w_frameEventEnable`.
    pub event_enable: bool,
}

impl Default for FrameInputs {
    fn default() -> Self {
        Self {
            pad: PadInput::default(),
            menu_open: false,
            navi_mode: 0,
            block_ready: true,
            camera_inter: false,
            encounter_mask: true,
            navi_title: -1,
            frame_phase: 2,
            event_enable: true,
        }
    }
}

/// `ff9.s_moveCHRStatus` fields the foot path uses (`ff9/ff9.cs:10356`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActorStatus {
    pub slice_type: u8,
    pub control: u8,
    pub flg_fly: bool,
    /// Full map id under the actor.
    pub id: i32,
    pub slice_height: f32,
    pub ground_height: f32,
}

impl ActorStatus {
    /// `w_moveCHRStatus[1]` (`ff9/ff9.cs:1766`): Zidane, `slice_type` 1, control 0.
    pub const fn zidane() -> Self {
        Self {
            slice_type: 1,
            control: 0,
            flg_fly: false,
            id: 0,
            slice_height: 0.0,
            ground_height: 0.0,
        }
    }
}

/// `w_cameraSysDataCamera` and `w_movementCamRemain` fields the follow code uses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraYaw {
    /// Camera yaw, degrees (`w_cameraSysDataCamera.rotation`).
    pub rotation: f32,
    pub rotation_max: f32,
    pub rotation_rev: f32,
    /// `w_cameraSysData.cameraNotrot`.
    pub no_rot: bool,
    /// `w_movementCamRemain`.
    pub remain: f32,
    /// `w_moveCHRControl_LR`: the player is turning the camera by hand.
    pub manual_turn: bool,
}

impl Default for CameraYaw {
    fn default() -> Self {
        Self {
            rotation: 0.0,
            rotation_max: 0.0,
            rotation_rev: 0.0,
            no_rot: false,
            remain: 0.0,
            manual_turn: false,
        }
    }
}

/// `ff9.cn1`, `cn3`, `cn4` (`ff9/ff9.cs:404-407`).
const CN1: f32 = 32.0;
const CN3: f32 = 54.6875;
const CN4: f32 = 256.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorldMoveError {
    /// Control entries of type 1, 2 or 3 (aircraft, boat) need operations that are
    /// not ported yet.
    UnsupportedControl(usize),
}

impl std::fmt::Display for WorldMoveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for WorldMoveError {}

/// An event the cell under the actor raises (`WorldEvent(x, z, id)`,
/// `ff9/ff9.cs:5090`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldEventHit {
    pub cell_x: i32,
    pub cell_z: i32,
    pub event: i32,
}

/// What one frame did.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameReport {
    pub pos: Vec3,
    pub id: i32,
    pub topograph: i32,
    pub block: Option<(usize, usize)>,
    pub cell: i32,
    /// Walk-mesh index and triangle of the last accepted ground hit.
    pub walk_mesh: i32,
    pub triangle: i32,
    /// Stick pushed but no rotation attempt was accepted.
    pub blocked: bool,
    /// Rotation offset (degrees) of the accepted attempt, if any.
    pub deflection: Option<f32>,
    pub encounter_enabled: bool,
    pub world_event: Option<WorldEventHit>,
}

/// The controlled on-foot actor and its camera yaw.
#[derive(Debug, Clone, PartialEq)]
pub struct FootWalker {
    /// `w_moveCHRControl_No`.
    pub control_no: usize,
    pub pos: Vec3,
    pub last: Vec3,
    /// Actor yaw, degrees (`WMActor.rot1`).
    pub rot_y: f32,
    /// `w_moveCHRControl_RotTrue`.
    pub rot_true: f32,
    pub xz_speed: f32,
    /// `w_moveCHRControl_Move`, computed at the end of the previous frame.
    pub moved: bool,
    pub status: ActorStatus,
    pub cache: MoveCache,
    pub camera: CameraYaw,
    /// `w_frameEncountEnable`: set by the last step when an encounter test may run.
    pub encounter_enable: bool,
    last_hit: NwpHit,
}

struct RoundOk {
    pos: Vec3,
    id: i32,
    hit: NwpHit,
}

impl FootWalker {
    /// Zidane on foot (`w_moveCHRControl[0]`). Control entries of another type are
    /// rejected.
    pub fn new(control_no: usize, pos: Vec3, rot_y: f32) -> Result<Self, WorldMoveError> {
        if CONTROL_TABLE.get(control_no).is_none_or(|e| e.kind != 0) {
            return Err(WorldMoveError::UnsupportedControl(control_no));
        }
        Ok(Self {
            control_no,
            pos,
            last: pos,
            rot_y: euler_yaw(rot_y),
            rot_true: 0.0,
            xz_speed: 0.0,
            moved: false,
            status: ActorStatus::zidane(),
            cache: MoveCache::default(),
            camera: CameraYaw::default(),
            encounter_enable: false,
            last_hit: NwpHit {
                height: 0.0,
                id: 0,
                pno: -1,
                block: None,
                walk_mesh: -1,
                triangle: -1,
            },
        })
    }

    fn control(&self) -> &'static ControlEntry {
        &CONTROL_TABLE[self.control_no]
    }

    /// Places the actor the way `w_movementUpdate` grounds an actor it does not
    /// control (`ff9/ff9.cs:4975-5017`): unlimited ray, exceptions ignored, a second
    /// ray from the sky when the first misses, and the position's own height kept
    /// when nothing is hit.
    pub fn place(&mut self, world: &WorldWalk, pos: Vec3) {
        let mut mode = RaycastMode {
            cast_from_sky: false,
            ignore_exceptions: true,
            control_type: self.control().kind,
        };
        let mut hit = world.nwp_hit(pos, &mode, None);
        if hit.pno == -1 && !mode.cast_from_sky {
            mode.cast_from_sky = true;
            hit = world.nwp_hit(pos, &mode, None);
        }
        let mut height = hit.height;
        let mut id = hit.id;
        if hit.pno == -1 {
            height = pos.y;
        }
        // Zidane is in the index range 1..=6 for these remaps (`ff9.cs:4999-5017`).
        match id {
            0xFEE => {
                height = pos.y;
                id = 0xFD2;
            }
            0x1BEE => {
                height = pos.y;
                id = 0x1B44;
            }
            0x2CEE => {
                height = pos.y;
                id = 0x6C4C;
            }
            _ => {}
        }
        self.status.id = id;
        self.status.ground_height = height;
        self.pos = pos;
        self.last = self.pos;
        self.last_hit = hit;
        self.set_height();
        self.moved = false;
    }

    /// `ff9.w_movementRoundCheck` (`ff9/ff9.cs:5400`): move `speed` units along
    /// `rot_true + 180 + rotation` and accept the spot if its ground is allowed.
    fn round_check(
        &mut self,
        world: &WorldWalk,
        from: Vec3,
        rotation: f32,
        speed: f32,
    ) -> Option<RoundOk> {
        let control = *self.control();
        let mut vector = from;
        let heading = self.rot_true + psx_rot(2048) + rotation;
        vector.x += rsin(heading) * speed;
        vector.z += rcos(heading) * speed;
        let actor_topograph = id_topograph(self.status.id);
        let mode = RaycastMode {
            cast_from_sky: false,
            ignore_exceptions: false,
            control_type: control.kind,
        };
        let use_cache = !((actor_topograph == 52 || actor_topograph == 49) && control.kind != 1);
        let hit = world.nwp_hit(
            vector,
            &mode,
            if use_cache {
                Some(&mut self.cache)
            } else {
                None
            },
        );
        // `ff9.w_cellHit` (`ff9/ff9.cs:3293`).
        let mut id = hit.id;
        if (self.control_no == 3 || self.control_no == 2) && (id == 56 || id == 57) {
            id = 54;
        }
        vector.y = hit.height;
        if hit.pno < 0 {
            return None;
        }
        let mut allowed = check_topograph(&control.limit, id);
        if allowed && control.flg_gake != 0 {
            let target_water = check_topograph(&WATER_STATUS, id);
            let target_ground = check_topograph(&GROUND_STATUS, id);
            let actor_water = check_topograph(&WATER_STATUS, self.status.id);
            let actor_ground = check_topograph(&GROUND_STATUS, self.status.id);
            if target_ground && actor_water {
                allowed = false;
            }
            if actor_ground && target_water {
                allowed = false;
            }
        }
        allowed.then_some(RoundOk {
            pos: vector,
            id,
            hit,
        })
    }

    /// `ff9.w_movementHumanOperation` (`ff9/ff9.cs:5845`), without the particle
    /// effects it registers.
    fn human_operation(&mut self, pad: &PadInput) {
        let left_x = pad.stick_x;
        let left_y = -pad.stick_y;
        let move_direction = (-ratan2(left_y as f32, left_x as f32) + psx_rot(1024)) % 360.0;
        let move_speed: i32 = if left_x != 0 || left_y != 0 { 4096 } else { 0 };
        let mut pad_lr = false;
        if pad.l1 {
            pad_lr = true;
            self.camera.manual_turn = true;
            self.camera.rotation -= psx_rot(32);
        }
        if pad.r1 {
            pad_lr = true;
            self.camera.manual_turn = true;
            self.camera.rotation += psx_rot(32);
        }
        if move_speed != 0 || pad_lr {
            self.camera.rotation_max = rsin(move_direction);
            if move_speed != 0 {
                self.camera.manual_turn = false;
                self.camera.rotation_rev =
                    if move_direction >= psx_rot(1536) && move_direction <= psx_rot(2560) {
                        psx_rot(2048)
                    } else {
                        0.0
                    };
                let speed_move = i32::from(self.control().speed_move);
                self.xz_speed = fixed_to_unity((speed_move * move_speed) >> 12);
                self.rot_true = (self.camera.rotation + move_direction) % 360.0;
            }
        }
        let mut aim = self.rot_true;
        if aim - self.rot_y > psx_rot(2048) {
            aim -= psx_rot(4096);
        }
        if aim - self.rot_y < psx_rot(-2048) {
            aim += psx_rot(4096);
        }
        let diff = aim - self.rot_y;
        if left_x != 0 || left_y != 0 {
            let mut new_rot = self.rot_y;
            new_rot += diff / 3.0;
            new_rot %= 360.0;
            self.rot_y = euler_yaw(new_rot);
        }
    }

    /// `ff9.w_movementControl` (`ff9/ff9.cs:5262`), walking branch.
    fn movement_control(&mut self, world: &WorldWalk, inputs: &FrameInputs) -> MoveOutcome {
        let control = *self.control();
        if control.encount
            && self.moved
            && id_topograph(self.status.id) != 52
            && inputs.navi_title == -1
        {
            self.encounter_enable = inputs.encounter_mask;
        }
        if !inputs.menu_open && inputs.navi_mode != 2 && inputs.block_ready && !inputs.camera_inter
        {
            self.human_operation(&inputs.pad);
        }
        let num = if self.xz_speed < 0.0 {
            self.xz_speed / 2.0
        } else {
            self.xz_speed
        };
        let mut outcome = MoveOutcome::default();
        let mut num3 = 0.0_f32;
        while num3 < psx_rot(1024) {
            let mut rotation = num3;
            let mut probe = self.round_check(world, self.pos, rotation, num);
            if probe.is_none() {
                rotation = -num3;
                probe = self.round_check(world, self.pos, rotation, num);
            }
            if let Some(first) = probe {
                let dx = first.pos.x - self.last.x;
                let dy = first.pos.y - self.last.y;
                let dz = first.pos.z - self.last.z;
                let (slice, _) = slice_height(self.status.slice_type, first.id);
                let length = if slice != 0.0 {
                    (dx * dx + dz * dz).sqrt()
                } else {
                    (dx * dx + dy * dy + dz * dz).sqrt()
                };
                let scaled = if unity_to_fixed(length) != 0 {
                    let mut value = num * num / length;
                    if num < 0.0 {
                        value = -value;
                    }
                    value
                } else {
                    0.0
                };
                if let Some(second) = self.round_check(world, self.pos, rotation, scaled) {
                    self.pos = second.pos;
                    self.status.id = second.id;
                    self.status.ground_height = second.pos.y;
                    self.last_hit = second.hit;
                    outcome.accepted = true;
                    outcome.deflection = Some(rotation);
                    break;
                }
            }
            num3 += psx_rot(128);
        }
        outcome
    }

    /// `ff9.w_movementSetheight` (`ff9/ff9.cs:5215`), walking branch.
    fn set_height(&mut self) {
        let (target, in_forest) = slice_height(self.status.slice_type, self.status.id);
        if !self.status.flg_fly {
            if self.status.slice_type != 0 {
                if self.status.control == 7 {
                    self.status.slice_height = fixed_to_unity(-180);
                } else if in_forest || unity_to_fixed(target) == 0 {
                    self.status.slice_height = target;
                } else {
                    if self.status.slice_height < target - 0.082_031_25 {
                        self.status.slice_height += 0.078_125;
                    }
                    if self.status.slice_height > target + 0.082_031_25 {
                        self.status.slice_height -= 0.078_125;
                    }
                }
            }
            self.pos.y = self.status.ground_height + self.status.slice_height;
        }
    }

    /// `ff9.w_movementHumanCamOperation` (`ff9/ff9.cs:6158`): the camera yaw drifts
    /// behind the walking direction. Eye and aim positions (`w_cameraSetEyeAim`) are
    /// not ported.
    fn human_cam_operation(&mut self, frame_phase: i32) {
        let cam = &mut self.camera;
        let mut camera_rotation = (cam.rotation + cam.rotation_rev) % 360.0;
        let character_rotation = self.rot_true % 360.0;
        if camera_rotation - character_rotation >= psx_rot(2048) {
            camera_rotation -= psx_rot(4096);
        }
        if camera_rotation - character_rotation < psx_rot(-2048) {
            camera_rotation += psx_rot(4096);
        }
        let mut difference = (self.rot_true - camera_rotation) * psx_rot(3072) / psx_rot(4096);
        let cn5 = cam.rotation_max.abs() / 8.0;
        if difference.abs() < cn5 {
            difference = 0.0;
        }
        if unity_to_fixed(self.xz_speed) != 0 {
            cam.remain = 4096.0;
        } else {
            cam.remain = (cam.remain - 512.0).max(0.0);
        }
        if !cam.manual_turn && !cam.no_rot && frame_phase == 2 {
            let cn2 = difference.abs().clamp(CN3, CN4);
            let factor = difference * 1.0 / cn2;
            let mut speed_factor = ((psx_rot(2048) - difference.abs()) * CN1 + 2048.0) * 2.0;
            speed_factor = speed_factor * cam.remain / 4096.0;
            cam.rotation += factor * speed_factor / 4096.0;
        }
        cam.rotation %= 360.0;
    }

    /// One `w_frameUpdate` phase-2 pass for the controlled actor.
    pub fn step(&mut self, world: &WorldWalk, inputs: &FrameInputs) -> FrameReport {
        self.encounter_enable = false;
        self.xz_speed = 0.0;
        self.last = self.pos;
        let outcome = self.movement_control(world, inputs);
        if inputs.block_ready {
            self.set_height();
            self.human_cam_operation(inputs.frame_phase);
        }
        let world_event = if id_event(self.status.id) != 0 && inputs.event_enable {
            let cell = cell_index(self.pos.x, self.pos.z);
            Some(WorldEventHit {
                cell_x: cell % CELL_COLUMNS,
                cell_z: cell / CELL_COLUMNS,
                event: id_event(self.status.id),
            })
        } else {
            None
        };
        self.moved = unity_to_fixed(self.pos.x) != unity_to_fixed(self.last.x)
            || unity_to_fixed(self.pos.y) != unity_to_fixed(self.last.y)
            || unity_to_fixed(self.pos.z) != unity_to_fixed(self.last.z);
        FrameReport {
            pos: self.pos,
            id: self.status.id,
            topograph: id_topograph(self.status.id),
            block: world.layout.absolute_block(self.pos),
            cell: cell_index(self.pos.x, self.pos.z),
            walk_mesh: self.last_hit.walk_mesh,
            triangle: self.last_hit.triangle,
            blocked: self.xz_speed != 0.0 && !outcome.accepted,
            deflection: outcome.deflection,
            encounter_enabled: self.encounter_enable,
            world_event,
        }
    }
}

#[derive(Default)]
struct MoveOutcome {
    accepted: bool,
    deflection: Option<f32>,
}

/// Inputs of the random encounter test (`ff9.w_frameGetParameter(205)`,
/// `ff9/ff9.cs:4260`). The source draws two `random8` values and tests
/// `((a << 8 | b) % (prob + 1)) == 1`; both draws happen whenever the world map
/// screen is not open and `IsNoEncounter` is off, whatever the outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncounterCheck {
    /// `FF9StateSystem.Settings.IsNoEncounter`.
    pub no_encounter: bool,
    /// `UIManager.World.CurrentState == FullMap`.
    pub full_map_open: bool,
    pub random_high: u8,
    pub random_low: u8,
    /// `ff9.w_frameEventBattleProb` (script parameter 26).
    pub battle_probability: u16,
    /// `ff9.w_frameEncountEnable` (set by the walk step).
    pub encounter_enabled: bool,
    /// `ff9.w_moveCHRControl_Move`.
    pub moved: bool,
    /// `m_GetIDTopograph` of the actor's map id.
    pub topograph: i32,
    /// `ff9.w_frameCounter`.
    pub frame_counter: i32,
}

/// `ff9.w_frameGetParameter(205)`: true when an encounter starts. Only forest
/// topographs 36 to 38 can trigger one, after frame 400.
pub fn encounter_triggers(check: &EncounterCheck) -> bool {
    if check.no_encounter || check.full_map_open {
        return false;
    }
    let roll = (u32::from(check.random_high) << 8 | u32::from(check.random_low))
        % (u32::from(check.battle_probability) + 1);
    check.encounter_enabled
        && check.moved
        && (36..=38).contains(&check.topograph)
        && check.frame_counter > 400
        && roll == 1
}
