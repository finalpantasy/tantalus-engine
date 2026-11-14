use serde::{Deserialize, Serialize};
use tantalus_contracts::{ActionMarkerKind, ContentId, EntityId, Tick};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionDefinition {
    pub id: ContentId,
    pub power: i32,
    pub hit_rate: i32,
    pub mp_cost: u32,
    pub timeline: ActionTimeline,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionTimeline {
    pub duration_ticks: u32,
    pub markers: Vec<ActionMarker>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionMarker {
    pub tick_offset: u32,
    pub kind: ActionMarkerKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BattleCommand {
    pub actor: EntityId,
    pub action: ContentId,
    pub targets: Vec<EntityId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombatInput {
    pub tick: Tick,
    pub stride: u32,
    pub command: Option<BattleCommand>,
    pub interrupt: Option<EntityId>,
}

impl CombatInput {
    pub fn new(tick: Tick) -> Self {
        Self {
            tick,
            stride: 1,
            command: None,
            interrupt: None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ActiveAction {
    pub actor: EntityId,
    pub definition: ActionDefinition,
    pub targets: Vec<EntityId>,
    pub elapsed_ticks: u32,
    pub next_marker: usize,
}
