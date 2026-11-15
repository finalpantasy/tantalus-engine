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
use tantalus_legacy::{VmStatus, decode_program};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectorStep {
    pub tick: Tick,
    pub completed_entries: Vec<EntityId>,
    pub encounters: Vec<EncounterRequest>,
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
        let mut host = FieldHost::new(&mut self.world);
        let steps = self.scene.step(tick, &mut host).map_err(|error| {
            EngineError::new(
                ErrorCategory::Internal,
                format!("field step failed: {error}"),
                "tantalus-game/director",
                true,
            )
        })?;
        self.world.advance();
        self.tick += 1;
        Ok(DirectorStep {
            tick,
            completed_entries: steps
                .iter()
                .filter(|step| step.report.status == VmStatus::Completed)
                .map(|step| step.entity)
                .collect(),
            encounters: self.world.take_encounters(),
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
