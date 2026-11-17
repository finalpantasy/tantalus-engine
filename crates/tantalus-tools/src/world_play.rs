//! Walking the overworld: the foot actor on the world walk meshes, the
//! world camera, random encounters by region and place entry.
//!
//! Sources (Memoria `ff9.cs`): `w_movementControl` (the foot step, in
//! `tantalus_legacy::world`), `w_cameraSetEyeAim` (`:2773`, the camera sits
//! behind the actor: eye offset `(sin r, height, cos r) * distance` with the
//! default `w_cameraPosstat[1]`), `WorldEvent` (`:2228`, the place under the
//! actor) and `EventEngine.ProcessEncount` (the danger value) with
//! `ff9.w_worldGetBattleScenePtr` (the scene for the region and terrain).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use tantalus_legacy::world::{
    EncounterRecord, FootWalker, FrameInputs, FriendlyBattleSetting, PadInput, PlaneInput,
    RaycastMode, ShipInput, Vec3, VehicleMotion, VehicleSpec, WorldWalk, advance_xz, advance_y,
    plane_step, select_scene, ship_step, wrap_position,
};

use crate::world_data::WorldAssets;
use crate::world_places::event_code;

/// Pad bits (`play::pad`).
const UP: i32 = 0x10;
const RIGHT: i32 = 0x20;
const DOWN: i32 = 0x40;
const LEFT: i32 = 0x80;
const L1: i32 = 0x400;
const R1: i32 = 0x800;
const L2: i32 = 0x100;
const R2: i32 = 0x200;

/// `w_cameraPosstat[1]` after `w_cameraSystemConstructor`: distance 6000,
/// height 3200 and aim height 1200 over 256; perspective 350.
const CAMERA_DISTANCE: f32 = 6000.0 / 256.0;
const CAMERA_HEIGHT: f32 = 3200.0 / 256.0;
const CAMERA_AIM_HEIGHT: f32 = 1200.0 / 256.0;
/// Vertical field of view in degrees (`cameraPers / 8`).
pub const CAMERA_FOV: f32 = 350.0 / 8.0;

/// World distance the actor walks per danger step.
const ENCOUNTER_INTERVAL: f32 = 12.0;
/// Danger added per step (`encratio`).
const ENCOUNTER_RATIO: i32 = 32;
/// Ground ids on which no random battle starts (`w_movementControl`).
const NO_ENCOUNTER_TOPOGRAPH: i32 = 52;

/// One world frame's outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorldTick {
    /// A random battle starts: the scene.
    pub battle: Option<i32>,
    /// The player confirmed on a place: its number.
    pub enter: Option<i32>,
}

/// Where the camera is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldCamera {
    pub eye: [f32; 3],
    pub aim: [f32; 3],
    pub fov: f32,
}

pub struct WorldPlay {
    pub assets: Arc<WorldAssets>,
    /// The walk meshes with this play's block forms applied.
    walk: WorldWalk,
    /// Blocks in their second form (`WMBlock.SetForm(2)`).
    pub forms: BTreeSet<(usize, usize)>,
    /// Which minimap shows (`w_naviMapno`): 0 the first continent, 1 the world.
    pub navi_map: u8,
    /// The `w_moveCHRControl` entry in use: 0 on foot, 1 to 5 the chocobos,
    /// 6 the gold chocobo in the air, 7 the Blue Narciss under sail.
    pub ride: u8,
    /// Speed and heading while `ride` is 6 or 7.
    air: VehicleMotion,
    /// Where the Blue Narciss lies at anchor after the player left it.
    pub moored: Option<Vec3>,
    /// Places visited, bit `n` for place `n + 1` (`gEventGlobal` 92 to 99).
    pub visited: u64,
    pub walker: FootWalker,
    events: BTreeMap<u16, i32>,
    /// The world script, run every frame; when present its place handlers
    /// decide the place under the actor (`events` is the fallback).
    script: Option<crate::world_script::WorldScript>,
    friendly: BTreeSet<u16>,
    /// The place under the actor, if any.
    pub place: Option<i32>,
    pub frames: u64,
    pub disc: u8,
    walked: f32,
    danger: i32,
    last_scene: u16,
    rng: u32,
    /// Danger steps taken, for reports.
    pub steps: u32,
}

/// Height above the ground a flight starts at, the lowest it may go and
/// the ceiling, in world units.
const AIR_START_HEIGHT: f32 = 12.0;
/// Shortest ray the get-off search probes, and how close the player has
/// to be to board the anchored ship, in world units.
const GET_OFF_REACH: f32 = 3.0;
const BOARD_REACH: f32 = 8.0;
/// The topograph a ship may be left at (`w_movementGetGetoff`, case 7).
const PORT_TOPOGRAPH: i32 = 53;
const AIR_MIN_HEIGHT: f32 = 6.0;
const AIR_MAX_HEIGHT: f32 = 140.0;
/// How much of the turn toward the heading the camera takes each frame.
const AIR_CAMERA_FOLLOW: f32 = 0.04;

/// `wrap_position` that leaves the height alone.
fn wrap_position_keep_y(pos: Vec3) -> Vec3 {
    let wrapped = wrap_position(pos);
    Vec3::new(wrapped.x, pos.y, wrapped.z)
}

impl WorldPlay {
    /// Runs the world script alongside the walk: each frame it ticks, and a
    /// place cell requests the cell's handler (`ff9.WorldEvent`).
    pub fn set_script(&mut self, script: crate::world_script::WorldScript) {
        self.script = Some(script);
    }

    /// The running world script, if any.
    pub fn script(&self) -> Option<&crate::world_script::WorldScript> {
        self.script.as_ref()
    }

    /// Starts at world `(x, z)` facing north.
    pub fn new(
        assets: Arc<WorldAssets>,
        events: BTreeMap<u16, i32>,
        x: f32,
        z: f32,
        seed: u32,
    ) -> Result<Self, String> {
        let start = Vec3::new(x, 0.0, z);
        let mut walker = FootWalker::new(0, start, 0.0).map_err(|error| error.to_string())?;
        let walk = assets.walk.clone();
        walker.place(&walk, start);
        let friendly = assets.encounters.friendly_battles(&assets.special);
        let disc = u8::try_from(assets.disc()).unwrap_or(1);
        Ok(Self {
            assets,
            walk,
            forms: BTreeSet::new(),
            navi_map: 0,
            ride: 0,
            air: VehicleMotion::default(),
            moored: None,
            visited: 0,
            walker,
            events,
            script: None,
            friendly,
            place: None,
            frames: 0,
            disc,
            walked: 0.0,
            danger: 0,
            last_scene: 0,
            rng: seed | 1,
            steps: 0,
        })
    }

    /// Puts the blocks in `forms` into their second form
    /// (`ff9.w_worldChangeBlockSet`). Blocks without a second form stay as
    /// they are.
    pub fn set_forms(&mut self, blocks: &[(usize, usize)]) {
        for &(x, z) in blocks {
            if let Some(block) = self.walk.block_mut(x, z)
                && !block.form2.is_empty()
            {
                block.form = 2;
                self.forms.insert((x, z));
            }
        }
    }

    /// Takes the story point and the visited-places bits
    /// (`ff9.w_worldSystemConstructor`: the whole-world map from story 5990).
    pub fn set_progress(&mut self, scenario: i32, visited: u64) {
        self.navi_map = u8::from(scenario >= 5990);
        self.visited = visited;
    }

    /// Switches to the vehicle `control` (`w_moveCHRControl` entry): 0 on
    /// foot, 1 to 5 the chocobos (the foot movement with their own speed and
    /// ground limits), 6 the gold chocobo in the air and 7 the Blue Narciss
    /// (a ship on the water topographs, `VehicleSpec` kind 2). The ship only
    /// casts off from water; anything else is ignored.
    pub fn set_ride(&mut self, control: u8) {
        if control == self.ride || control > 7 {
            return;
        }
        let old = &self.walker;
        let (pos, rot_y, rot_true, camera) = (old.pos, old.rot_y, old.rot_true, old.camera);
        let flying = control == 6;
        let motor = control >= 6;
        if control == 7 && !self.water_at(pos) {
            return;
        }
        let Ok(mut walker) =
            FootWalker::new(if motor { 0 } else { usize::from(control) }, pos, rot_y)
        else {
            return;
        };
        walker.rot_true = rot_true;
        walker.camera = camera;
        walker.place(&self.walk, pos);
        if flying {
            // Take off a little above the ground, still.
            walker.pos.y += AIR_START_HEIGHT;
        }
        if motor {
            self.air = VehicleMotion {
                rot_y,
                rot_true: rot_y,
                ..VehicleMotion::default()
            };
        }
        self.walker = walker;
        self.ride = control;
    }

    /// Gets off the vehicle (`w_movementGetGetoff`, `ff9.cs:5464`): the
    /// player lands on the first spot around the vehicle where every probe
    /// of a short ray, from the vehicle outward, is ground an on-foot player
    /// may stand on. The air chocobo has to be low, the ship has to be at
    /// the shore. The ship stays anchored where it was. False when there is
    /// no spot, and nothing changes.
    pub fn get_off(&mut self) -> bool {
        if self.ride == 0 {
            return false;
        }
        let (Some(foot), Some(spec)) = (
            VehicleSpec::get(0),
            VehicleSpec::get(usize::from(self.ride)),
        ) else {
            return false;
        };
        let origin = self.walker.pos;
        let mode = RaycastMode {
            cast_from_sky: true,
            ignore_exceptions: true,
            control_type: 0,
        };
        if self.flying() {
            let ground = self.walk.nwp_hit(origin, &mode, None);
            if ground.pno != 1000 || origin.y - ground.height > AIR_START_HEIGHT * 2.0 {
                return false;
            }
        }
        let sea = self.sailing();
        if sea {
            // Only at a port: topograph 53 under the ship (`case 7`).
            let here = self.walk.nwp_hit(origin, &mode, None);
            if here.pno != 1000 || (here.id & 0xFC) >> 2 != PORT_TOPOGRAPH {
                return false;
            }
        }
        let reach = (f32::from(spec.radius) / 256.0).clamp(GET_OFF_REACH, 12.0);
        let heading = self.walker.rot_true;
        let mut landing = None;
        'sweep: for step in 0..8 {
            for sign in [1.0_f32, -1.0] {
                // The sweep starts behind the vehicle (`PsxRot(3072)`).
                let angle = heading + 180.0 + sign * step as f32 * 22.5;
                let mut spot = origin;
                let mut clear = true;
                for probe in 1..=8 {
                    spot = advance_xz(origin, angle, reach * probe as f32 / 8.0);
                    let hit = self.walk.nwp_hit(spot, &mode, None);
                    // The pier the ship lies at counts as ground on the way.
                    let pier = sea && (hit.id & 0xFC) >> 2 == PORT_TOPOGRAPH;
                    if hit.pno != 1000 || !(pier || foot.can_enter(hit.id)) {
                        clear = false;
                        break;
                    }
                }
                if clear {
                    landing = Some(wrap_position_keep_y(spot));
                    break 'sweep;
                }
            }
        }
        let Some(spot) = landing else {
            return false;
        };
        if self.sailing() {
            self.moored = Some(origin);
        }
        self.walker.pos = spot;
        self.set_ride(0);
        true
    }

    /// Boards the anchored Blue Narciss when the player stands next to it.
    pub fn board(&mut self) -> bool {
        let Some(ship) = self.moored else {
            return false;
        };
        let pos = self.walker.pos;
        if self.ride != 0 || (pos.x - ship.x).hypot(pos.z - ship.z) > BOARD_REACH {
            return false;
        }
        self.walker.pos = ship;
        self.set_ride(7);
        if self.ride == 7 {
            self.moored = None;
            true
        } else {
            self.walker.pos = pos;
            false
        }
    }

    /// Whether the player is in the air.
    pub fn flying(&self) -> bool {
        self.ride == 6
    }

    /// Whether the player is on the Blue Narciss.
    pub fn sailing(&self) -> bool {
        self.ride == 7
    }

    /// Whether the Narciss may float at `pos` (the ship's topograph limit).
    fn water_at(&self, pos: Vec3) -> bool {
        let Some(spec) = VehicleSpec::get(7) else {
            return false;
        };
        let mode = RaycastMode {
            cast_from_sky: true,
            ignore_exceptions: true,
            control_type: 2,
        };
        let hit = self.walk.nwp_hit(pos, &mode, None);
        hit.pno == 1000 && spec.can_enter(hit.id)
    }

    /// One frame under sail (`w_movementShipOperation`): up and down are
    /// ahead and astern, left and right turn. The ship stops at the shore.
    fn sail(&mut self, held: i32) {
        let Some(spec) = VehicleSpec::get(7) else {
            return;
        };
        let axis = |negative: i32, positive: i32| match (held & negative != 0, held & positive != 0)
        {
            (true, false) => -127,
            (false, true) => 127,
            _ => 0,
        };
        let input = ShipInput {
            rotate: axis(LEFT, RIGHT),
            left_y: axis(UP, DOWN),
            l1: held & L1 != 0,
            r1: held & R1 != 0,
            ..ShipInput::default()
        };
        let output = ship_step(spec, &mut self.air, &input);
        let last = self.walker.pos;
        let mut pos = wrap_position_keep_y(advance_xz(last, self.air.rot_true, self.air.xz_speed));
        if !self.water_at(pos) {
            // Shore ahead: stay put and let the speed die.
            pos = last;
            self.air.xz_speed = 0.0;
            self.air.xz_alpha = 0.0;
        }
        let mode = RaycastMode {
            cast_from_sky: true,
            ignore_exceptions: true,
            control_type: 2,
        };
        let hit = self.walk.nwp_hit(pos, &mode, None);
        if hit.pno == 1000 {
            pos.y = hit.height;
        }
        let walker = &mut self.walker;
        walker.last = last;
        walker.pos = pos;
        walker.rot_y = self.air.rot_y;
        walker.rot_true = self.air.rot_true;
        walker.moved = pos != last;
        let mut turn = (self.air.rot_true - walker.camera.rotation) % 360.0;
        if turn > 180.0 {
            turn -= 360.0;
        }
        if turn < -180.0 {
            turn += 360.0;
        }
        walker.camera.rotation = (walker.camera.rotation + turn * AIR_CAMERA_FOLLOW) % 360.0;
        walker.camera.rotation += output.camera_rotation_delta;
    }

    /// One frame of flight (`w_movementPlaneOperation` then `w_movementControl`'s
    /// flying branch). Up and down are throttle and brake, left and right
    /// turn, L2 climbs and R2 descends.
    fn fly(&mut self, held: i32) {
        let Some(spec) = VehicleSpec::get(usize::from(self.ride)) else {
            return;
        };
        let axis = |negative: i32, positive: i32| match (held & negative != 0, held & positive != 0)
        {
            (true, false) => -127,
            (false, true) => 127,
            _ => 0,
        };
        let input = PlaneInput {
            left_x: axis(LEFT, RIGHT),
            left_y: axis(L2, R2),
            right: -axis(UP, DOWN),
            l1: held & L1 != 0,
            r1: held & R1 != 0,
            ..PlaneInput::default()
        };
        let output = plane_step(spec, &mut self.air, &input);
        let walker = &mut self.walker;
        walker.last = walker.pos;
        let mut pos = advance_xz(walker.pos, self.air.rot_true, self.air.xz_speed);
        pos = advance_y(pos, self.air.y_speed);
        pos = wrap_position_keep_y(pos);
        let mode = RaycastMode {
            cast_from_sky: true,
            ignore_exceptions: true,
            control_type: 1,
        };
        let ground = self.walk.nwp_hit(pos, &mode, None);
        let floor = if ground.pno == 1000 {
            ground.height
        } else {
            0.0
        } + AIR_MIN_HEIGHT;
        pos.y = pos.y.clamp(floor, AIR_MAX_HEIGHT.max(floor));
        walker.pos = pos;
        walker.rot_y = self.air.rot_y;
        walker.rot_true = self.air.rot_true;
        walker.moved = self.air.xz_speed.abs() > 1e-4;
        // The camera swings in behind the heading.
        let mut turn = (self.air.rot_true - walker.camera.rotation) % 360.0;
        if turn > 180.0 {
            turn -= 360.0;
        }
        if turn < -180.0 {
            turn += 360.0;
        }
        walker.camera.rotation = (walker.camera.rotation + turn * AIR_CAMERA_FOLLOW) % 360.0;
        walker.camera.rotation += output.camera_rotation_delta;
    }

    fn random8(&mut self) -> u8 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as u8
    }

    fn scene(&mut self, id: i32) -> u16 {
        let random = self.random8();
        let record: Option<EncounterRecord> = self
            .assets
            .encounters
            .battle_record(id, 0, self.disc)
            .ok()
            .copied();
        record.map_or(0, |record| {
            select_scene(&record, FriendlyBattleSetting::Off, random, &self.friendly)
        })
    }

    /// One 30 Hz frame with the pad `held` and the buttons `pressed` this frame.
    pub fn tick(&mut self, held: i32, pressed: i32, confirm: i32, menu_open: bool) -> WorldTick {
        self.frames += 1;
        let axis = |negative: i32, positive: i32| match (held & negative != 0, held & positive != 0)
        {
            (true, false) => -128,
            (false, true) => 128,
            _ => 0,
        };
        let inputs = FrameInputs {
            pad: PadInput {
                stick_x: axis(LEFT, RIGHT),
                stick_y: axis(UP, DOWN),
                l1: held & L1 != 0,
                r1: held & R1 != 0,
            },
            menu_open,
            ..FrameInputs::default()
        };
        if self.sailing() {
            self.sail(held);
            return WorldTick::default();
        }
        if self.flying() {
            self.fly(held);
            let pos = wrap_position_keep_y(self.walker.pos);
            self.walker.pos = pos;
            return WorldTick::default();
        }
        let report = self.walker.step(&self.walk, &inputs);
        // The world wraps at every edge (`WMWorld.Wrap`).
        let wrapped = wrap_position(self.walker.pos);
        if wrapped != self.walker.pos {
            let shift = Vec3::new(
                wrapped.x - self.walker.pos.x,
                0.0,
                wrapped.z - self.walker.pos.z,
            );
            self.walker.pos = wrapped;
            self.walker.last = Vec3::new(
                self.walker.last.x + shift.x,
                self.walker.last.y,
                self.walker.last.z + shift.z,
            );
        }
        let mut tick = WorldTick::default();

        let hit_code = report
            .world_event
            .map(|hit| event_code(hit.cell_x, hit.cell_z, hit.event));
        let scripted = self.script_place(hit_code);
        self.place = scripted.or_else(|| hit_code.and_then(|code| self.events.get(&code)).copied());
        if pressed & confirm != 0
            && let Some(place) = self.place
        {
            tick.enter = Some(place);
            return tick;
        }

        if self.walker.moved
            && report.encounter_enabled
            && report.topograph != NO_ENCOUNTER_TOPOGRAPH
        {
            let dx = self.walker.pos.x - self.walker.last.x;
            let dz = self.walker.pos.z - self.walker.last.z;
            self.walked += dx.hypot(dz);
            if self.walked > ENCOUNTER_INTERVAL {
                self.walked = 0.0;
                self.steps += 1;
                self.danger += ENCOUNTER_RATIO;
                if i32::from(self.random8()) < self.danger >> 3 {
                    self.danger = 0;
                    let mut scene = self.scene(report.id);
                    if scene != 0 && scene == self.last_scene {
                        scene = self.scene(report.id);
                    }
                    if scene != 0 {
                        self.last_scene = scene;
                        tick.battle = Some(i32::from(scene));
                    }
                }
            }
        }
        tick
    }

    /// One frame of the world script: when the actor stands on a place cell
    /// its handler runs and the place it chose (map variable 39) is
    /// returned; `None` without a script, off a place, or when the handler
    /// declines (a story or Chocograph condition is not met).
    fn script_place(&mut self, code: Option<u16>) -> Option<i32> {
        let script = self.script.as_mut()?;
        if let Some(code) = code {
            let _ = script
                .world_mut()
                .map_memory
                .set(39, tantalus_legacy::VariableType::Byte, 0);
            if script.request_event(code).unwrap_or(false) {
                // The handler is short and has no waits before it picks.
                for _ in 0..2 {
                    if script.tick(0).is_err() {
                        break;
                    }
                }
                let place = script.place();
                return (place != 0).then_some(place);
            }
        }
        let _ = script.tick(0);
        None
    }

    /// `w_cameraSetEyeAim` for the default camera.
    pub fn camera(&self) -> WorldCamera {
        let pos = self.walker.pos;
        let yaw = self.walker.camera.rotation.to_radians();
        WorldCamera {
            eye: [
                pos.x + yaw.sin() * CAMERA_DISTANCE,
                pos.y + CAMERA_HEIGHT,
                pos.z + yaw.cos() * CAMERA_DISTANCE,
            ],
            aim: [pos.x, pos.y + CAMERA_AIM_HEIGHT, pos.z],
            fov: CAMERA_FOV,
        }
    }
}
