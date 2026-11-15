pub mod director;
pub mod fmv;
pub mod host;
pub mod snapshot;
pub mod world;

pub use director::{DirectorStep, FieldDirector, RequestedScript};
pub use fmv::FieldFmv;
pub use host::FieldHost;
pub use snapshot::{
    EntrySnapshot, FIELD_SNAPSHOT_FORMAT, FieldSnapshot, restore_scene, snapshot_scene,
};
pub use world::{
    ActorState, CharacterStats, EncounterRequest, FieldWorld, Jump, Movement, TimerState, Turn,
};

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{Display, Formatter};
use tantalus_contracts::{EngineError, EntityId, ErrorCategory, Tick};
use tantalus_legacy::{EventProgram, EventVm, LegacyEventHost, VmError, VmStepReport};

#[derive(Debug)]
pub struct FieldScene {
    entries: BTreeMap<EntityId, EventVm>,
}

impl FieldScene {
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    pub fn add_resumed(&mut self, entity: EntityId, vm: EventVm) -> Result<(), EngineError> {
        if self.entries.contains_key(&entity) {
            return Err(EngineError::new(
                ErrorCategory::Conflict,
                format!("field entry {entity} already exists"),
                "tantalus-game/field",
                true,
            ));
        }
        self.entries.insert(entity, vm);
        Ok(())
    }

    pub fn add_entry(
        &mut self,
        entity: EntityId,
        program: EventProgram,
    ) -> Result<(), EngineError> {
        if self.entries.contains_key(&entity) {
            return Err(EngineError::new(
                ErrorCategory::Conflict,
                format!("field entry {entity} already exists"),
                "tantalus-game/field",
                true,
            ));
        }
        self.entries
            .insert(entity, EventVm::new(program).with_entity(entity));
        Ok(())
    }

    pub fn step(
        &mut self,
        tick: Tick,
        host: &mut dyn LegacyEventHost,
    ) -> Result<Vec<FieldStep>, FieldError> {
        let mut reports = Vec::new();
        for (entity, vm) in &mut self.entries {
            let report = vm.step(tick, host).map_err(|error| FieldError {
                entity: *entity,
                error,
            })?;
            reports.push(FieldStep {
                entity: *entity,
                report,
            });
        }
        Ok(reports)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Observed per-entry VM state for snapshots: entity, program counter,
    /// status, and wait state.
    pub fn entry_states(
        &self,
    ) -> Vec<(
        EntityId,
        u32,
        tantalus_legacy::VmStatus,
        Option<tantalus_legacy::WaitState>,
    )> {
        self.entries
            .iter()
            .map(|(entity, vm)| (*entity, vm.pc(), vm.status(), vm.wait_state()))
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for FieldScene {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldStep {
    pub entity: EntityId,
    pub report: VmStepReport,
}

#[derive(Debug)]
pub struct FieldError {
    pub entity: EntityId,
    pub error: VmError,
}

impl Display for FieldError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "field entry {}: {}", self.entity, self.error)
    }
}

impl Error for FieldError {}
