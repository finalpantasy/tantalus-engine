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
    EncounterRecord, FootWalker, FrameInputs, FriendlyBattleSetting, PadInput, Vec3, WorldWalk,
    select_scene, wrap_position,
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
    /// Riding the yellow chocobo (`w_moveCHRControl` entry 1).
    pub chocobo: bool,
    /// Places visited, bit `n` for place `n + 1` (`gEventGlobal` 92 to 99).
    pub visited: u64,
    pub walker: FootWalker,
    events: BTreeMap<u16, i32>,
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

impl WorldPlay {
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
            chocobo: false,
            visited: 0,
            walker,
            events,
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

    /// Gets on or off the yellow chocobo: the same foot movement with the
    /// chocobo's control entry (faster, other ground limits).
    pub fn set_chocobo(&mut self, on: bool) {
        if on == self.chocobo {
            return;
        }
        let old = &self.walker;
        let Ok(mut walker) = FootWalker::new(usize::from(on), old.pos, old.rot_y) else {
            return;
        };
        walker.rot_true = old.rot_true;
        walker.camera = old.camera;
        walker.place(&self.walk, old.pos);
        self.walker = walker;
        self.chocobo = on;
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

        self.place = report
            .world_event
            .and_then(|hit| {
                self.events
                    .get(&event_code(hit.cell_x, hit.cell_z, hit.event))
            })
            .copied();
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
