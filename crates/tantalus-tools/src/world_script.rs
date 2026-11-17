//! The world map's event script, run by the field VM.
//!
//! `EVT_WORLD_WORLD00` (map 9000, and the other world scripts) is an ordinary
//! event binary: its first object holds the place handlers (functions
//! `0x8000 | z << 8 & 0x3F00 | x << 2 & 0xFC | id & 3`, requested by
//! `ff9.WorldEvent`), its other objects run the weather, music and story
//! logic. The world-only opcode is `RunWorldCode` (`WPRM`, 0xC4), which calls
//! `ff9.w_frameSetParameter(code, value)`; the VM raises it as an event and
//! [`WorldEffects`] applies the codes the walking world reads.
//!
//! Stubs, kept in [`WorldEffects::unhandled`] so a run lists them: the
//! effect registry (code 10), music and sound-effect codes (20 to 24, 32,
//! 33), the camera smoothing (25), the get-off search (28), the movement
//! changes (1, 30, 31) and the autopilot (34 and up). They change no state
//! here; the walking world has no ship, airship or chocobo script yet.

use std::path::Path;

use tantalus_game::FieldWorld;
use tantalus_legacy::VariableType;

use crate::field_session::{FieldSession, decode_pool, load_event, prepare_field_world, rerun};

/// What the world script has set through `RunWorldCode`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorldEffects {
    /// `w_frameScriptParam`: the four argument registers (codes 6 to 9, 14
    /// and 16 to 19 write them).
    pub params: [i32; 4],
    /// `w_frameEventBattleProb` (code 26).
    pub battle_probability: Option<i32>,
    /// The weather the script asked for (code 0, value other than 4), and
    /// its fade time (the first parameter register at that moment).
    pub weather: Option<(i32, i32)>,
    /// `w_weatherAutoChange` (codes 0 with value 4, and 15).
    pub weather_auto: bool,
    /// `w_naviActive` (code 2): the minimap shows.
    pub navi_active: bool,
    /// `w_naviMode` (code 4).
    pub navi_mode: i32,
    /// `w_cameraFixModeY` (code 5).
    pub camera_fix_y: bool,
    /// `w_naviCursolMove` (code 27).
    pub navi_cursor: bool,
    /// Times the script called the chocobo (code 29).
    pub chocobo_calls: u32,
    /// `w_naviFadeInTime` (code 11).
    pub navi_fade_in: Option<i32>,
    /// Calls this module does not act on, in order.
    pub unhandled: Vec<(i32, i32)>,
}

impl WorldEffects {
    /// `ff9.w_frameSetParameter`.
    pub fn apply(&mut self, code: i32, value: i32) {
        let flag = value == 1;
        match code {
            0 if value == 4 => self.weather_auto = true,
            0 => {
                self.weather_auto = false;
                self.weather = Some((value, self.params[0]));
            }
            2 => self.navi_active = flag,
            4 => self.navi_mode = value,
            5 => self.camera_fix_y = flag,
            6 | 14 | 16 => self.params[0] = value,
            7 | 17 => self.params[1] = value,
            8 | 18 => self.params[2] = value,
            9 | 19 => self.params[3] = value,
            11 => self.navi_fade_in = Some(value + 5),
            15 => self.weather_auto = flag,
            26 => self.battle_probability = Some(value),
            27 => self.navi_cursor = flag,
            29 => self.chocobo_calls += 1,
            3 => {}
            _ => self.unhandled.push((code, value)),
        }
    }
}

/// Script entry of the controlled character (Zidane, model 310).
pub const ZIDANE_ENTRY: u32 = 11;
/// Script entry of the chocobo object (model 308).
pub const CHOCOBO_ENTRY: u32 = 5;
/// `WMActor.Initialize` (`WMActor.cs`): where the Blue Narciss stands when
/// the world loads, in fixed point (the script's entry 8 gives it no
/// position of its own; its place function 36 moves it to the player when
/// he casts off), as world units `[x, height, z]`.
pub const NARCISS_ANCHORAGE: [f32; 3] = [280_576.0 / 256.0, 327.0 / 256.0, -261_120.0 / 256.0];
/// `WMActor.Initialize`: where the Invincible and Hilda Garde III stand
/// when the world loads.
pub const INVINCIBLE_ANCHORAGE: [f32; 3] = [272_866.0 / 256.0, 8550.0 / 256.0, -244_552.0 / 256.0];
pub const HILDA_GARDE_ANCHORAGE: [f32; 3] = [775.1, 39.9, -317.3];

/// Frames the script runs before its objects have their start positions.
pub const START_FRAMES: u32 = 10;

/// The world script, live.
pub struct WorldScript {
    session: FieldSession,
    pub effects: WorldEffects,
}

impl WorldScript {
    /// Starts world script `name` (`EVT_WORLD_WORLD00`) for world map `map`
    /// over `world`, whose global state (story counter, found Chocographs)
    /// the script reads.
    pub fn start(
        bundle: &Path,
        name: &str,
        map: i32,
        mut world: FieldWorld,
    ) -> Result<Self, String> {
        let binary = load_event(bundle, &format!("{name}.eb"))?;
        let (pool, _) = decode_pool(&binary)?;
        prepare_field_world(&mut world, map);
        // The player walks the world: `IsMovementEnabled` (system variable
        // 2) is on, which the place prompt requires before it takes confirm.
        world.user_control = true;
        Ok(Self {
            session: FieldSession::start(map, world, pool)?,
            effects: WorldEffects::default(),
        })
    }

    /// One frame of the script. Returns the field the script changed to.
    pub fn tick(&mut self, input: i32) -> Result<Option<i32>, String> {
        let tick = self.session.tick(input)?;
        for (code, value) in self.session.director.world_mut().take_world_codes() {
            self.effects.apply(code, value);
        }
        Ok(tick.next_field)
    }

    /// `ff9.WorldEvent`: asks the first object to run place handler `code`
    /// (see `world_places::event_code`). False when the script has none.
    pub fn request_event(&mut self, code: u16) -> Result<bool, String> {
        rerun(&mut self.session.director, &self.session.pool, 0, code)
    }

    /// The real enter path: the place prompt, entry 1's function 1 from its
    /// first instruction. A place handler (entry 0) writes the place to map
    /// variable 39 and entry 1's function 11 turns it into map variable 24,
    /// the place plus 100; the prompt function then (with movement enabled
    /// and confirm held) copies the place to variable 29, fades out and
    /// switches to the field. Runs that function on a copy of this script's
    /// world (kept when it changes field) with variable 24 set as function 11 leaves it, and returns the
    /// field it changes to (`None` when it never does within `ticks`).
    pub fn enter_place(&mut self, place: i32, ticks: u64) -> Result<Option<i32>, String> {
        let program = self
            .session
            .pool
            .get(1)
            .and_then(|functions| functions.iter().find(|(tag, _)| *tag == 1))
            .map(|(_, program)| program.clone())
            .ok_or("the world script has no place prompt (entry 1, function 1)")?;
        let mut world = self.world().clone();
        world.user_control = true;
        world.set_input(0x10000);
        world
            .map_memory
            .set(24, VariableType::Byte, place + 100)
            .ok_or("map variable 24 out of range")?;
        world
            .map_memory
            .set(39, VariableType::Byte, place)
            .ok_or("map variable 39 out of range")?;
        let mut vm = tantalus_legacy::EventVm::resume(
            program,
            tantalus_contracts::EntityId::new(1),
            0,
            tantalus_legacy::VmStatus::Running,
            None,
        )
        .map_err(|error| error.to_string())?;
        let mut changed = None;
        {
            let mut host = tantalus_game::field::FieldHost::new(&mut world);
            'run: for tick in 0..ticks {
                let report = vm
                    .step(tantalus_contracts::Tick::new(tick), &mut host)
                    .map_err(|error| error.to_string())?;
                for event in &report.events {
                    if let tantalus_legacy::HostEvent::ChangeField { field } = event {
                        changed = Some(i32::from(*field));
                        break 'run;
                    }
                }
            }
        }
        // The prompt's side effects (the visited-place bits, the story
        // variables it sets) stay with the script's world.
        if changed.is_some() {
            *self.world_mut() = world;
        }
        Ok(changed)
    }

    /// The place the last handler chose (map variable 39).
    pub fn place(&self) -> i32 {
        self.session
            .director
            .world()
            .map_memory
            .get(39, VariableType::Byte)
            .unwrap_or(0)
    }

    /// Requests function `tag` of object `entry` (a probe and the enter
    /// path's building block).
    pub fn request_function(&mut self, entry: usize, tag: u16) -> Result<bool, String> {
        rerun(&mut self.session.director, &self.session.pool, entry, tag)
    }

    /// Where the script put the world object `entry`: `SetActorPositionXzy`
    /// leaves the position in fixed point (256 to the world unit; X, height,
    /// Z) and `SetActorFacing` the 0..=255 facing
    /// (`EventEngine.DoEventCode`, `SETROT`/`SETPOS`). `None` while the
    /// object has no position yet.
    pub fn object_start(&self, entry: u32) -> Option<([f32; 3], u8)> {
        use tantalus_contracts::EntityId;
        let actor = self.world().actors.get(&EntityId::new(u64::from(entry)))?;
        if actor.position == [0.0; 3] {
            return None;
        }
        Some((
            [
                actor.position[0] / 256.0,
                actor.position[1] / 256.0,
                actor.position[2] / 256.0,
            ],
            (actor.facing & 0xFF) as u8,
        ))
    }

    /// Where the script put the chocobo object (entry 5, created when the
    /// player owns a chocobo kind, global 191): the saved world position
    /// (globals 83, 86, 88 and facing 91), which the script itself defaults
    /// for a new game and moves for story point 11100 (the Blue Narciss's
    /// first anchorage, bit 851). X, height and Z in world units and the
    /// facing byte. `None` without a chocobo object.
    pub fn chocobo_start(&self) -> Option<([f32; 3], u8)> {
        let (pos, _) = self.object_start(CHOCOBO_ENTRY)?;
        let world = self.world();
        let facing = world.global_memory.get(91, VariableType::Byte).unwrap_or(0);
        Some((pos, facing as u8))
    }

    /// Where the script put the controlled character (entry 11, Zidane's
    /// world model): the saved position in globals 64, 67, 69 and facing
    /// 72, which function 0 defaults for a new game (global 2 clear) to
    /// (945.1, -992.8) facing 77. While the player rides a vehicle (global
    /// 190) this is the vehicle's position too.
    pub fn zidane_start(&self) -> Option<([f32; 3], u8)> {
        self.object_start(ZIDANE_ENTRY)
    }

    /// `w_moveCHRControl_No` at the start (`ff9.w_movementChange` reads
    /// gEventGlobal 190): 0 on foot, 1 to 5 the chocobos, 6 the gold
    /// chocobo in the air, 7 the Blue Narciss, 8 and 9 the airships.
    pub fn start_ride(&self) -> u8 {
        let global = self.world().global_memory.get(190, VariableType::Byte);
        global.unwrap_or(0) as u8
    }

    /// The chocobo kind the player owns (gEventGlobal 191; the script
    /// creates the chocobo object for kinds 1 and up).
    pub fn chocobo_kind(&self) -> u8 {
        let global = self.world().global_memory.get(191, VariableType::Byte);
        global.unwrap_or(0) as u8
    }

    /// The scheduled functions and their state (for probes).
    pub fn running(
        &self,
    ) -> Vec<(
        tantalus_contracts::EntityId,
        u32,
        tantalus_legacy::VmStatus,
        Option<tantalus_legacy::WaitState>,
    )> {
        self.session.director.entry_states()
    }

    pub fn world(&self) -> &FieldWorld {
        self.session.director.world()
    }

    pub fn world_mut(&mut self) -> &mut FieldWorld {
        self.session.director.world_mut()
    }
}
