pub mod director;
pub mod fmv;
pub mod host;
pub mod item_use;
pub mod region;
pub mod shop;
pub mod snapshot;
pub mod tile_animation;
pub mod world;

pub use director::{DirectorStep, FieldDirector, RequestedScript};
pub use fmv::FieldFmv;
pub use host::FieldHost;
pub use snapshot::{
    EntrySnapshot, FIELD_SNAPSHOT_FORMAT, FieldSnapshot, restore_scene, snapshot_scene,
};
pub mod settings;
pub use settings::GameSettings;
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
    /// Place of each object in the reference's active object list
    /// (`_context.activeObj`). `Obj`'s constructor (Obj.cs:32-45) appends a
    /// new object at the tail and `ProcessCode` walks the list from the
    /// head, so objects run in creation order, not entry order.
    ranks: BTreeMap<EntityId, u64>,
    next_rank: u64,
}

impl FieldScene {
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            ranks: BTreeMap::new(),
            next_rank: 0,
        }
    }

    /// `NEW` (or `STARTSEQ`) of `object`: it moves to the tail of the
    /// active list, so it runs after every object created before it.
    pub fn requeue_object(&mut self, object: EntityId) {
        self.ranks.insert(object, self.next_rank);
        self.next_rank += 1;
    }

    /// Objects never created through `NEW` (entry 0, set up by the engine
    /// before any script runs) come first, then the rest by creation.
    fn run_order(&self) -> Vec<EntityId> {
        let mut order: Vec<(Option<u64>, EntityId)> = self
            .entries
            .iter()
            .map(|(key, vm)| {
                let object = vm.own_object().or(vm.entity()).unwrap_or(*key);
                (self.ranks.get(&object).copied(), *key)
            })
            .collect();
        order.sort();
        order.into_iter().map(|(_, key)| key).collect()
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

    /// Adds one function of an object as its own VM: `key` names the VM,
    /// `object` is the entity its actor operations, self reads and requests
    /// act on. The reference runs every function of an object against one
    /// `Obj` (position, facing, flags), so the functions of one entry share
    /// `object`.
    pub fn add_object_function(
        &mut self,
        key: EntityId,
        object: EntityId,
        program: EventProgram,
    ) -> Result<(), EngineError> {
        if self.entries.contains_key(&key) {
            return Err(EngineError::new(
                ErrorCategory::Conflict,
                format!("field entry {key} already exists"),
                "tantalus-game/field",
                true,
            ));
        }
        self.entries
            .insert(key, EventVm::new(program).with_entity(object));
        Ok(())
    }

    /// Adds a shared script (`STARTSEQ`): `object`'s actor is the one it
    /// moves, `key` is its own object, which `DELETE 255` disposes.
    pub fn add_shared_script(
        &mut self,
        key: EntityId,
        object: EntityId,
        program: EventProgram,
    ) -> Result<(), EngineError> {
        self.add_object_function(key, object, program)?;
        if let Some(vm) = self.entries.remove(&key) {
            self.entries.insert(key, vm.with_own_object(key));
        }
        self.requeue_object(key);
        Ok(())
    }

    /// Drop an entry's VM (for re-running a finished function from its start).
    pub fn remove_entry(&mut self, entity: EntityId) -> Option<EventVm> {
        self.entries.remove(&entity)
    }

    pub fn step(
        &mut self,
        tick: Tick,
        host: &mut dyn LegacyEventHost,
    ) -> Result<Vec<FieldStep>, FieldError> {
        let mut reports = Vec::new();
        for entity in self.run_order() {
            let Some(vm) = self.entries.get_mut(&entity) else {
                continue;
            };
            // `DisposeObj` unlinks the object: none of its functions run
            // until a `NEW` initializes it again.
            if vm
                .entity()
                .is_some_and(|object| host.object_deleted(object))
            {
                continue;
            }
            let report = vm
                .step(tick, host)
                .map_err(|error| FieldError { entity, error })?;
            reports.push(FieldStep { entity, report });
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
