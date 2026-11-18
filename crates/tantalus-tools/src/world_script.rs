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

/// The pad bits the script reads for the held physical and logical buttons
/// (`EventInput.ProcessInput` and `GetKeyMaskFromControl`): each logical
/// control sets its own bit and its physical button's, in the default
/// layout confirm = cross, cancel = circle, menu = triangle, special =
/// square, the bumpers and triggers L1, R1, L2, R2.
pub fn script_pad(held: i32) -> i32 {
    const LOGICAL: [(i32, i32); 9] = [
        // (bit the walking world holds, bits the script sees besides it)
        (0x20000, 0x4000),   // Confirm and cross
        (0x10000, 0x2000),   // Cancel and circle
        (0x1000, 0x1000000), // Triangle and Menu
        (0x8000, 0x80000),   // Square and Special
        (0x400, 0x100000),   // L1 and LeftBumper
        (0x800, 0x200000),   // R1 and RightBumper
        (0x100, 0x400000),   // L2 and LeftTrigger
        (0x200, 0x800000),   // R2 and RightTrigger
        (0x1, 0),            // Select
    ];
    let mut mask = held;
    for (bit, extra) in LOGICAL {
        if held & bit != 0 {
            mask |= extra;
        }
    }
    mask & 0x3FF_FFFF
}

/// Script entry of the controlled character (Zidane, model 310).
pub const ZIDANE_ENTRY: u32 = 11;
/// Script entry 8 of `EVT_WORLD_WORLD00`: object index 13, the tent (model
/// 311); the walking world moves it with the Blue Narciss ride. The ships
/// themselves are objects of the later worlds, see [`Ship`].
pub const NARCISS_ENTRY: u32 = 8;
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

/// A ship the world script keeps parked: the objects with the airship
/// models of `EVT_WORLD_WORLD03` to `09` (`SetObjectIndex` 8, 9 and 10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ship {
    BlueNarciss,
    HildaGarde3,
    Invincible,
}

impl Ship {
    /// The script's model id (`GEO_SUB_W0_008`, `_009`, `_010`).
    pub fn from_model(model: u16) -> Option<Self> {
        match model {
            321 => Some(Self::BlueNarciss),
            320 => Some(Self::HildaGarde3),
            317 => Some(Self::Invincible),
            _ => None,
        }
    }

    /// `WMActor.Initialize`: where the ship stands when the world loads.
    pub fn anchorage(self) -> [f32; 3] {
        match self {
            Self::BlueNarciss => NARCISS_ANCHORAGE,
            Self::HildaGarde3 => HILDA_GARDE_ANCHORAGE,
            Self::Invincible => INVINCIBLE_ANCHORAGE,
        }
    }
}

/// A ship standing at its anchorage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParkedShip {
    pub ship: Ship,
    pub pos: [f32; 3],
    /// The script's facing byte.
    pub facing: u8,
}

/// Frames the script runs before its objects have their start positions.
pub const START_FRAMES: u32 = 10;

/// The continent title the world map shows as it opens on a story step
/// (`ff9.w_naviTitle`, set by `w_worldSystemConstructor` for scenario 2400,
/// 5990, 9605 and 9890). When the title starts to fade in
/// (`w_naviTitleElement`, `w_frameCounterReady > w_naviFadeInTime`) the
/// engine adds 10 to its copy of the scenario counter taken when the world
/// loaded (`w_frameScenePtr`) and writes it back to global 0: 9605 becomes
/// 9615, which Oeilvert's exterior (2250) needs to give the player control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NaviTitle {
    /// `w_frameScenePtr`: the counter when the world loaded.
    pub scene: u16,
    /// `w_frameCounterReady`: frames since the world loaded.
    pub frames: u32,
    /// The title has faded in and the counter was written.
    pub shown: bool,
}

impl NaviTitle {
    /// The title for a world loaded at `scenario`, when it has one.
    pub fn for_scenario(scenario: i32) -> Option<Self> {
        matches!(scenario, 2400 | 5990 | 9605 | 9890).then_some(Self {
            scene: scenario as u16,
            frames: 0,
            shown: false,
        })
    }
}

/// `w_naviFadeInTime` when the script sets none (`w_naviSystemConstructor`).
pub const NAVI_FADE_IN: i32 = 20;

/// The world script, live.
pub struct WorldScript {
    session: FieldSession,
    pub effects: WorldEffects,
    /// The continent title, while the world has one to show.
    pub title: Option<NaviTitle>,
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
        let scenario = world
            .global_memory
            .get(0, VariableType::UInt16)
            .unwrap_or(0);
        Ok(Self {
            session: FieldSession::start(map, world, pool)?,
            effects: WorldEffects::default(),
            title: NaviTitle::for_scenario(scenario),
        })
    }

    /// One frame of the script. Returns the field the script changed to.
    pub fn tick(&mut self, input: i32) -> Result<Option<i32>, String> {
        Ok(self.tick_events(input)?.next_field)
    }

    /// One frame of the script with everything it started: battles
    /// (`StartBattle`) and the field it changed to.
    pub fn tick_events(&mut self, input: i32) -> Result<crate::field_session::SessionTick, String> {
        let tick = self.session.tick(input)?;
        for (code, value) in self.session.director.world_mut().take_world_codes() {
            self.effects.apply(code, value);
        }
        // `w_frameService` (`w_naviService`), then `w_frameCounterReady++`.
        let fade_in = self.effects.navi_fade_in.unwrap_or(NAVI_FADE_IN);
        if let Some(title) = self.title.as_mut() {
            if !title.shown && i64::from(title.frames) > i64::from(fade_in) {
                self.show_title();
            }
            if let Some(title) = self.title.as_mut() {
                title.frames += 1;
            }
        }
        Ok(tick)
    }

    /// The continent title fades in now (`w_naviTitleElement`, first
    /// fade-in frame): global 0 becomes the counter the world loaded with
    /// plus 10. The world-map stand-in, which does not walk the world, calls
    /// this before the party enters a place: the walking player can reach
    /// no place before the title shows, and the value written does not
    /// depend on when it shows. Does nothing when there is no title or it
    /// has shown.
    pub fn show_title(&mut self) {
        let Some(title) = self.title.as_mut() else {
            return;
        };
        if title.shown {
            return;
        }
        title.shown = true;
        let scene = i32::from(title.scene.wrapping_add(10));
        self.world_mut()
            .global_memory
            .set(0, VariableType::UInt16, scene);
    }

    /// The pad the script reads this frame (`ETb.ProcessKeyEvents`): the
    /// buttons pressed (`KEYON`), released (`KEYOFF`) and held (`KEY`).
    pub fn set_keys(&mut self, pressed: i32, held: i32, released: i32) {
        let world = self.world_mut();
        world.set_input(held);
        world.world_keys = Some([pressed, held, released]);
    }

    /// Sets what `ff9.w_frameGetParameter` returns (system variables 192
    /// and up) for the script's next frame.
    pub fn feed(&mut self, variables: &[(u8, i32)]) {
        let world = self.world_mut();
        for &(variable, value) in variables {
            world.set_system_variable(variable, value);
        }
    }

    /// Moves script object `entry` to where the walking world has it (world
    /// units `[x, height, z]` and yaw in degrees), as the engine's
    /// `WMActor` keeps the object and the script reads it back
    /// (`getvobj` members 0 to 3): the saved position in globals 64 to 72
    /// follows the player.
    pub fn place_object(&mut self, entry: u32, pos: [f32; 3], yaw: f32) {
        use tantalus_contracts::EntityId;
        let id = EntityId::new(u64::from(entry));
        let world = self.world_mut();
        if !world.actors.contains_key(&id) {
            world.add_actor(id, [0.0; 3]);
        }
        if let Some(actor) = world.actors.get_mut(&id) {
            actor.position = pos.map(|v| (v * 256.0).round());
            actor.facing = ((yaw / 360.0 * 256.0).round() as i32 & 0xFF) as i16;
        }
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
        world.set_input(0x20000);
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

    /// The ships this world's script keeps parked: its objects with a
    /// ship's model whose flags show them (`SetObjectFlags` bit 0; the
    /// script hides an object it does not want on the map) and that are not
    /// the one the player flies (`w_moveCHRControl_No`, 7 to 9). They stand
    /// where `WMActor.Initialize` puts them (`SetPosition`), turned as the
    /// script faces them.
    pub fn parked_ships(&self, flown: u8) -> Vec<ParkedShip> {
        let mut ships: Vec<_> = self
            .world()
            .actors
            .values()
            .filter_map(|actor| {
                let ship = Ship::from_model(actor.look.model?)?;
                let flown = match flown {
                    7 => Some(Ship::BlueNarciss),
                    8 => Some(Ship::HildaGarde3),
                    9 => Some(Ship::Invincible),
                    _ => None,
                };
                // Its own position when the script placed it
                // (`SetActorPositionXzy`), else `WMActor.Initialize`'s.
                let pos = if actor.position == [0.0; 3] {
                    ship.anchorage()
                } else {
                    actor.position.map(|value| value / 256.0)
                };
                (actor.flags & 1 != 0 && Some(ship) != flown).then_some(ParkedShip {
                    ship,
                    pos,
                    facing: (actor.facing & 0xFF) as u8,
                })
            })
            .collect();
        ships.sort_by_key(|parked| parked.ship as u8);
        ships
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
