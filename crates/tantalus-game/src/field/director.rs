//! The packaged field loop: one owner for scene, world, encounters, program
//! sources, and the tick clock, so a scenario runner drives a single struct.
//!
//! `step` runs the field (event VMs through `FieldHost`, then world
//! advancement) and surfaces encounter requests for the caller to resolve
//! into battles through the embedded coordinator. Battle ticks and commands
//! stay caller-controlled through [`FieldDirector::coordinator`]; save and
//! restore round trip through [`FieldSnapshot`].

use super::snapshot::{FieldSnapshot, restore_scene, snapshot_scene};
use super::{FieldHost, FieldScene, FieldWorld};
use crate::battle::{CombatInput, CombatRuleset, EncounterCoordinator};
use crate::field::world::EncounterRequest;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tantalus_contracts::{EngineError, EntityId, ErrorCategory, Tick};
use tantalus_legacy::{HostEvent, VmStatus, decode_program};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestedScript {
    pub level: i32,
    pub target: EntityId,
    pub tag: u8,
    pub synchronous: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectorStep {
    pub tick: Tick,
    /// Total event instructions executed across all entries this tick.
    pub operations: u64,
    pub completed_entries: Vec<EntityId>,
    pub encounters: Vec<EncounterRequest>,
    /// Script requests emitted this tick (REQ/REQEW/REQSW family).
    pub requests: Vec<RequestedScript>,
    /// Objects whose talk radius contains the player while the configured
    /// confirm mask is held. The caller decides which function to spawn.
    pub talk_triggers: Vec<EntityId>,
    /// Regions containing the player actor this tick (enter/exit detection
    /// is a caller diff), from the field's defined region polygons.
    pub region_occupancy: Vec<EntityId>,
    /// `STARTSEQ` (RunSharedScript) this tick as `(object, entry)`: the
    /// caller runs the entry's function against the calling object
    /// (`Seq(entry, uid + UID_OFFSET_SEQ)`) and finishes the object's shared
    /// script when it returns.
    #[serde(default)]
    pub shared_scripts: Vec<(EntityId, i32)>,
    /// Entries created this tick by `NEW`/`NEW2`/`NEW3` (InitializeCode,
    /// InitializeRegion, InitializeObject), in order. The reference starts
    /// only entry 0 at field load; every other object starts its init
    /// function (tag 0) when one of these creates it (`NewThread`,
    /// `new Quad`, `new Actor`, EventEngine.DoEventCode.cs:114-140).
    #[serde(default)]
    pub initialized: Vec<u8>,
    /// `DRET` (ReturnEntryFunctions) targets this tick: every function the
    /// object is running returns and it restarts its loop at the idle
    /// level (`EventEngine.Return` with `sx = 0`, `state = stateInit`).
    /// The caller drops the object's requested-function entries.
    #[serde(default)]
    pub returned: Vec<EntityId>,
}

#[derive(Debug)]
pub struct FieldDirector {
    scene: FieldScene,
    world: FieldWorld,
    coordinator: EncounterCoordinator,
    program_sources: BTreeMap<EntityId, Vec<u8>>,
    tick: u64,
}

impl FieldDirector {
    pub fn new(world: FieldWorld) -> Self {
        Self {
            scene: FieldScene::new(),
            world,
            coordinator: EncounterCoordinator::new(),
            program_sources: BTreeMap::new(),
            tick: 0,
        }
    }

    /// Register an already-decoded program without raw source bytes. Such
    /// entries cannot be snapshotted (the save is explicit about the missing
    /// source); use for probes and tests.
    pub fn add_entry_program(
        &mut self,
        entity: EntityId,
        program: tantalus_legacy::EventProgram,
    ) -> Result<(), EngineError> {
        self.scene.add_entry(entity, program)
    }

    /// See [`crate::FieldScene::add_object_function`].
    pub fn add_object_function(
        &mut self,
        key: EntityId,
        object: EntityId,
        program: tantalus_legacy::EventProgram,
    ) -> Result<(), EngineError> {
        self.scene.add_object_function(key, object, program)
    }

    pub fn world(&self) -> &FieldWorld {
        &self.world
    }

    pub fn world_mut(&mut self) -> &mut FieldWorld {
        &mut self.world
    }

    pub fn coordinator(&mut self) -> &mut EncounterCoordinator {
        &mut self.coordinator
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Per-tick player input injection: scripts read this through the
    /// host key queries (`B_KEYON` family). Held until changed.
    pub fn set_input_mask(&mut self, mask: i32) {
        self.world.set_input(mask);
    }

    pub fn input_mask(&self) -> i32 {
        self.world.input_mask()
    }

    /// Designate the player-controlled actor for host-side interaction
    /// checks (talk proximity and confirm).
    pub fn set_player(&mut self, actor: EntityId) {
        self.world.set_player(actor);
    }

    /// Remove an entry and its program source; returns whether it existed.
    /// Callers re-add it to run the function again from its start, as the
    /// reference does for per-frame region requests.
    /// See [`FieldScene::add_shared_script`].
    pub fn add_shared_script(
        &mut self,
        key: EntityId,
        object: EntityId,
        program: tantalus_legacy::EventProgram,
    ) -> Result<(), EngineError> {
        self.scene.add_shared_script(key, object, program)
    }

    pub fn remove_entry(&mut self, entity: EntityId) -> bool {
        self.program_sources.remove(&entity);
        self.scene.remove_entry(entity).is_some()
    }

    /// Configure the confirm-button bit that arms talk triggers; `None`
    /// disables detection until the caller knows the pad layout.
    pub fn set_confirm_mask(&mut self, mask: Option<i32>) {
        self.world.set_confirm_mask(mask);
    }

    /// Configure the direction-bit mapping that moves the player actor;
    /// `None` means input never moves the player.
    pub fn set_walk_bits(&mut self, bits: Option<(i32, i32, i32, i32)>) {
        self.world.set_walk_bits(bits);
    }

    /// Observed per-entry VM state (entity, pc, status, wait).
    pub fn entry_states(
        &self,
    ) -> Vec<(
        EntityId,
        u32,
        tantalus_legacy::VmStatus,
        Option<tantalus_legacy::WaitState>,
    )> {
        self.scene.entry_states()
    }

    /// Decode and register an entry program; the raw bytes become its save
    /// source. Duplicate entities are a conflict.
    pub fn add_entry(&mut self, entity: EntityId, program_bytes: &[u8]) -> Result<(), EngineError> {
        if self.program_sources.contains_key(&entity) {
            return Err(EngineError::new(
                ErrorCategory::Conflict,
                format!("field entry {entity} already exists"),
                "tantalus-game/director",
                true,
            ));
        }
        let program = decode_program(program_bytes).map_err(|error| {
            EngineError::new(
                ErrorCategory::UnknownContent,
                format!("entry {entity} program does not decode: {error}"),
                "tantalus-game/director",
                true,
            )
        })?;
        self.scene.add_entry(entity, program)?;
        self.program_sources.insert(entity, program_bytes.to_vec());
        Ok(())
    }

    /// Run one field tick: step every entry through the host, advance the
    /// world, then drain queued encounter requests for the caller.
    pub fn step(&mut self) -> Result<DirectorStep, EngineError> {
        let tick = Tick::new(self.tick);
        // HonoluluFieldMain.FF9FieldMapMain services the movie before events.
        let map = self.world.field_map_no;
        self.world.fmv.service(map);
        let mut host = FieldHost::new(&mut self.world);
        let steps = self.scene.step(tick, &mut host).map_err(|error| {
            EngineError::new(
                ErrorCategory::Internal,
                format!("field step failed: {error}"),
                "tantalus-game/director",
                true,
            )
        })?;
        // `NEW` appends the object at the tail of the active list.
        let mut initialized = Vec::new();
        for event in steps.iter().flat_map(|step| step.report.events.iter()) {
            if let HostEvent::InitializeObject { source, .. }
            | HostEvent::InitializeRegion { source, .. }
            | HostEvent::InitializeCode { source, .. } = event
            {
                self.scene.requeue_object(EntityId::new(u64::from(*source)));
                initialized.push(*source);
            }
        }
        self.world.advance();
        self.tick += 1;
        let operations: u64 = steps
            .iter()
            .map(|step| u64::from(step.report.operations))
            .sum();
        let requests = steps
            .iter()
            .flat_map(|step| step.report.events.iter())
            .filter_map(|event| match event {
                HostEvent::RequestScriptAsync { level, target, tag }
                | HostEvent::RequestScriptSync { level, target, tag } => Some(RequestedScript {
                    level: *level,
                    target: *target,
                    tag: *tag,
                    synchronous: matches!(event, HostEvent::RequestScriptSync { .. }),
                }),
                _ => None,
            })
            .collect();
        let shared_scripts = steps
            .iter()
            .flat_map(|step| step.report.events.iter())
            .filter_map(|event| match event {
                HostEvent::RunSharedScript { actor, entry } => Some((*actor, *entry)),
                _ => None,
            })
            .collect();
        let returned: Vec<EntityId> = steps
            .iter()
            .flat_map(|step| step.report.events.iter())
            .filter_map(|event| match event {
                HostEvent::ReturnEntryFunctions { object } => Some(*object),
                _ => None,
            })
            .collect();
        for object in &returned {
            self.world.clear_requests(*object);
        }
        let player = self.world.player();
        let talk_triggers = match (player, self.world.confirm_mask()) {
            (Some(player), Some(mask)) if self.world.input_mask() & mask != 0 => {
                self.world.talkable_from(player)
            }
            _ => Vec::new(),
        };
        let region_occupancy = match player.and_then(|player| {
            self.world
                .actors
                .get(&player)
                .map(|state| (player, state.position))
        }) {
            Some((player, position)) => {
                super::region::regions_at(&self.world.regions, position[0], position[2])
                    .into_iter()
                    .filter(|region| *region != player)
                    .collect()
            }
            None => Vec::new(),
        };
        Ok(DirectorStep {
            tick,
            operations,
            completed_entries: steps
                .iter()
                .filter(|step| step.report.status == VmStatus::Completed)
                .map(|step| step.entity)
                .collect(),
            encounters: self.world.take_encounters(),
            requests,
            talk_triggers,
            region_occupancy,
            shared_scripts,
            initialized,
            returned,
        })
    }

    /// Convenience passthrough so callers can begin a battle for a drained
    /// encounter without reaching through two accessors.
    pub fn begin_battle(
        &mut self,
        request: EncounterRequest,
        battle: CombatRuleset,
    ) -> Result<(), EngineError> {
        self.coordinator.begin(request, battle)
    }

    /// Advance the active battle one tick; see `EncounterCoordinator`.
    pub fn advance_battle(
        &mut self,
        input: &CombatInput,
    ) -> Result<Option<crate::battle::BattleOutcome>, EngineError> {
        let mut services = crate::battle::WorldGameServices::new(&mut self.world);
        self.coordinator.advance(input, &mut services)
    }

    pub fn finish_battle(&mut self) -> Option<EncounterRequest> {
        self.coordinator.finish()
    }

    pub fn save(&self) -> Result<FieldSnapshot, EngineError> {
        snapshot_scene(&self.scene, &self.world, &self.program_sources)
    }

    /// Restore scene, world, and tick. The coordinator is not saved: an
    /// interrupted battle restarts from the encounter per the restart
    /// semantics documented on `FieldSnapshot`.
    pub fn restore(snapshot: &FieldSnapshot) -> Result<Self, EngineError> {
        let (scene, world) = restore_scene(snapshot)?;
        Ok(Self {
            scene,
            world,
            coordinator: EncounterCoordinator::new(),
            program_sources: snapshot
                .entries
                .iter()
                .map(|entry| (entry.entity, entry.program.clone()))
                .collect(),
            tick: 0,
        })
    }
}
