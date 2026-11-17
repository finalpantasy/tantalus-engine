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
    /// The world script started this battle itself (`StartBattle`).
    pub script_battle: Option<i32>,
    /// The world script changed to this field itself.
    pub field: Option<i32>,
}

/// Where the camera is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorldCamera {
    pub eye: [f32; 3],
    pub aim: [f32; 3],
    pub fov: f32,
}

/// A chocobo standing in the world, waiting to be ridden.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParkedChocobo {
    pub pos: Vec3,
    /// Yaw in degrees (`WMActor.rot1`).
    pub rot_y: f32,
    /// The kind (gEventGlobal 191): 1 yellow, 2 light blue, 3 red, 4 deep
    /// blue, 5 gold.
    pub kind: u8,
}

/// What the world script's object setup puts where, read once the script
/// has run its first frames.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ScriptStart {
    /// The controlled character (entry 11) at `(x, z)` with its yaw.
    pub zidane: Option<(f32, f32, f32)>,
    pub chocobo: Option<ParkedChocobo>,
    /// `w_moveCHRControl_No` at the start.
    pub ride: u8,
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
    /// The chocobo the script placed (by story point), if the player owns
    /// one and is not riding it.
    pub chocobo: Option<ParkedChocobo>,
    /// `FF9StateGlobal.hintmap_id`: the selected Chocograph picture (0
    /// none, else index + 1). The script reads it as system variable 198
    /// and digs at that picture's spot (entry 5 function 14, entry 3).
    pub hintmap: i32,
    /// The Chocograph list while the script's menu is open
    /// (`ChocographUI`).
    pub chocograph: Option<tantalus_legacy::chocobo::ChocographBook>,
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
        self.chocobo = self.script_start().and_then(|start| start.chocobo);
        // `ff9.w_movementChrInitSlice`: the chocobo stands on the ground.
        if let Some(chocobo) = self.chocobo.as_mut() {
            let mut walker = FootWalker::new(0, chocobo.pos, chocobo.rot_y).ok();
            if let Some(walker) = walker.as_mut() {
                walker.place(&self.walk, chocobo.pos);
                chocobo.pos = walker.pos;
            }
        }
    }

    /// What the world script's objects were given at the start: the
    /// controlled character's position (the saved one, or the script's
    /// new-game default), the chocobo's story-point position, and the ride
    /// the player starts on. `None` without a script.
    pub fn script_start(&self) -> Option<ScriptStart> {
        let script = self.script.as_ref()?;
        // `SetActorFacing` to degrees (`EventEngine.DoEventCode`, `ROT`).
        let degrees = |facing: u8| f32::from(facing) / 256.0 * 360.0;
        Some(ScriptStart {
            zidane: script
                .zidane_start()
                .map(|(pos, facing)| (pos[0], pos[2], degrees(facing))),
            chocobo: script.chocobo_start().and_then(|(pos, facing)| {
                let kind = script.chocobo_kind();
                (kind != 0).then_some(ParkedChocobo {
                    pos: Vec3::new(pos[0], pos[1], pos[2]),
                    rot_y: degrees(facing),
                    kind,
                })
            }),
            ride: script.start_ride(),
        })
    }

    /// The running world script, mutably (tests drive its requests).
    #[doc(hidden)]
    pub fn script_mut_for_tests(&mut self) -> &mut crate::world_script::WorldScript {
        self.script.as_mut().expect("a world script")
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
            chocobo: None,
            hintmap: 0,
            chocograph: None,
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
        let (scripted, events) = self.script_place(hit_code);
        self.poll_chocograph_menu();
        tick.script_battle = events.0;
        tick.field = events.1;
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
    /// declines (a story or Chocograph condition is not met). The second
    /// value is what the script itself started: a battle and a field change.
    fn script_place(&mut self, code: Option<u16>) -> (Option<i32>, (Option<i32>, Option<i32>)) {
        self.feed_script();
        let Some(script) = self.script.as_mut() else {
            return (None, (None, None));
        };
        let mut started = (None, None);
        let collect = |script: &mut crate::world_script::WorldScript,
                       started: &mut (Option<i32>, Option<i32>)| {
            match script.tick_events(0) {
                Ok(tick) => {
                    if started.0.is_none() {
                        started.0 = tick.encounters.first().map(|request| request.battle);
                    }
                    if started.1.is_none() {
                        started.1 = tick.next_field;
                    }
                    true
                }
                Err(_) => false,
            }
        };
        if let Some(code) = code {
            let _ = script
                .world_mut()
                .map_memory
                .set(39, tantalus_legacy::VariableType::Byte, 0);
            if script.request_event(code).unwrap_or(false) {
                // The handler is short and has no waits before it picks.
                for _ in 0..2 {
                    if !collect(script, &mut started) {
                        break;
                    }
                }
                let place = script.place();
                return ((place != 0).then_some(place), started);
            }
        }
        collect(script, &mut started);
        (None, started)
    }

    /// Opens the Chocograph list when the script asked for it
    /// (`OpenMenu` 5): the book is read from the script's global state
    /// (found and opened bits 184 to 189, the chocobo's reach, byte 191).
    fn poll_chocograph_menu(&mut self) {
        let Some(script) = self.script.as_mut() else {
            return;
        };
        let world = script.world_mut();
        if !std::mem::take(&mut world.chocograph_requested) {
            return;
        }
        let bytes = world.global_memory.bytes();
        let len = tantalus_legacy::saves::ff9_tree::EventGlobal::LEN.min(bytes.len());
        if let Ok(global) =
            tantalus_legacy::saves::ff9_tree::EventGlobal::from_bytes(bytes[..len].to_vec())
        {
            self.chocograph = Some(tantalus_legacy::chocobo::ChocographBook::from_event(
                &global,
                self.hintmap,
            ));
        }
    }

    /// Picks a picture in the open Chocograph list
    /// (`ChocographUI.OnKeyConfirm`): sets the hint map the script digs by.
    pub fn chocograph_select(
        &mut self,
        index: usize,
    ) -> Result<(), tantalus_legacy::chocobo::SelectError> {
        let Some(book) = self.chocograph.as_mut() else {
            return Err(tantalus_legacy::chocobo::SelectError::OutOfRange);
        };
        book.select(index)?;
        self.hintmap = book.hintmap_id;
        Ok(())
    }

    /// Clears the selection (the list's Cancel sub menu).
    pub fn chocograph_cancel(&mut self) -> bool {
        let Some(book) = self.chocograph.as_mut() else {
            return false;
        };
        let cleared = book.cancel_selection();
        self.hintmap = book.hintmap_id;
        cleared
    }

    /// Closes the list.
    pub fn chocograph_close(&mut self) {
        self.chocograph = None;
    }

    /// What `ff9.w_frameGetParameter` answers the script this frame
    /// (system variables 192 to 211) and where the script's objects stand:
    /// the area and topograph under the actor, the camera yaw, the actor's
    /// world position, the script's own parameter registers and the phase
    /// (209: the world is running). The player's saved position
    /// (globals 64 to 72) follows the walker through its object.
    fn feed_script(&mut self) {
        let Some(script) = self.script.as_mut() else {
            return;
        };
        let id = self.walker.status.id;
        let pos = self.walker.pos;
        let area = tantalus_legacy::world::id_area(id);
        let zone = tantalus_legacy::world::area_to_zone(area).unwrap_or(0) as i32;
        let (params, camera) = (script.effects.params, self.walker.camera.rotation);
        let fixed = |value: f32| (value * 256.0) as i32;
        script.feed(&[
            (192, area),
            (193, tantalus_legacy::world::id_topograph(id)),
            // 194: the height of the ground under the actor (negated).
            (
                194,
                -fixed(self.walker.status.ground_height + self.walker.status.slice_height),
            ),
            (195, params[0]),
            (196, -params[1]),
            (197, params[2]),
            // 199: the camera yaw as a 12-bit angle shifted by 4.
            (199, (camera / 360.0 * 256.0) as i32 & 0xFF),
            (198, self.hintmap),
            (200, 0),
            (201, fixed(pos.x)),
            (202, fixed(pos.y)),
            (203, fixed(pos.z)),
            (204, 0),
            // 205 (the script's own encounter roll) stays off: the random
            // battles are rolled by `tick` itself.
            (205, 0),
            (207, zone),
            (208, fixed(self.walker.status.slice_height)),
            (209, 1),
            (211, -1),
        ]);
        let yaw = self.walker.rot_y;
        let at = [pos.x, pos.y, pos.z];
        // The controlled character; mounted, the carrier moves with him.
        script.place_object(crate::world_script::ZIDANE_ENTRY, at, yaw);
        match self.ride {
            1..=6 => script.place_object(crate::world_script::CHOCOBO_ENTRY, at, yaw),
            7 => script.place_object(crate::world_script::NARCISS_ENTRY, at, yaw),
            _ => {}
        }
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
