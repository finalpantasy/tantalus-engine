use crate::{
    ContentId, ContentRevision, EncounterId, EntityId, InstanceProperties, ItemInstanceId, Point3,
    Tick,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};

pub const CURRENT_SAVE_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SaveId(u64);

impl SaveId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl Display for SaveId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineIdentity {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleLock {
    pub package: ContentId,
    pub version: String,
    pub required: bool,
    pub state_schema: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileLock {
    pub content_revision: ContentRevision,
    pub content_fingerprint: String,
    pub modules: Vec<ModuleLock>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointKind {
    Progress,
    SuspendedSession,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointMetadata {
    pub kind: CheckpointKind,
    pub tick: Tick,
    pub resume_providers: Vec<ContentId>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RngSaveState {
    pub algorithm: String,
    pub state: Vec<u64>,
    pub position: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntitySaveState {
    pub id: EntityId,
    pub definition: ContentId,
    pub active: bool,
    pub position: Point3,
    pub properties: BTreeMap<ContentId, crate::PropertyValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemInstanceSaveState {
    pub id: ItemInstanceId,
    pub definition: ContentId,
    pub owner: Option<EntityId>,
    pub properties: InstanceProperties,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncounterSaveState {
    pub id: EncounterId,
    pub provider: ContentId,
    pub state: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthoritativeState {
    pub entities: Vec<EntitySaveState>,
    pub items: Vec<ItemInstanceSaveState>,
    pub encounters: Vec<EncounterSaveState>,
    pub rng: RngSaveState,
    pub progression: BTreeMap<ContentId, crate::PropertyValue>,
    pub world_flags: BTreeMap<ContentId, crate::PropertyValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleSaveSection {
    pub schema_version: u32,
    pub state: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownSaveSection {
    pub name: String,
    pub schema_version: u32,
    pub state: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationRecord {
    pub owner: String,
    pub from_version: u32,
    pub to_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommittedOutcome {
    pub resolution_id: u64,
    pub encounter: EncounterId,
    pub provider: ContentId,
    pub result: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveEnvelope {
    pub format_version: u32,
    pub engine: EngineIdentity,
    pub ruleset: ContentId,
    pub profile: ProfileLock,
    pub checkpoint: CheckpointMetadata,
    pub state: AuthoritativeState,
    pub modules: BTreeMap<ContentId, ModuleSaveSection>,
    pub migration_history: Vec<MigrationRecord>,
    pub unknown_sections: Vec<UnknownSaveSection>,
    pub outcomes: Vec<CommittedOutcome>,
}
