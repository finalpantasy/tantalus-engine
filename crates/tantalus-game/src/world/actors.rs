use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use tantalus_contracts::{ContentId, EntityId, Point3, Tick};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActorDefinition {
    pub id: ContentId,
    pub rig: ContentId,
    pub behavior: ContentId,
    pub attack: ContentId,
    pub weapon_drop: ContentId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Aggressive,
    Fleeing,
    Passive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BehaviorDefinition {
    pub id: ContentId,
    pub disposition: Disposition,
    pub patrol_radius: u32,
    pub sight_range: u32,
    pub hearing_range: u32,
    pub pursuit_speed: u32,
    pub leash_range: u32,
    pub encounter_range: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnRegionDefinition {
    pub id: ContentId,
    pub cell: ContentId,
    pub enemy: ContentId,
    pub center: Point3,
    pub radius: u32,
    pub capacity: u16,
    pub respawn_ticks: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorLifecycle {
    Alive,
    Defeated,
    Dormant,
    Escaped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorBehaviorState {
    Patrol,
    Pursue,
    Flee,
    ReturnHome,
    Idle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldActor {
    pub id: EntityId,
    pub definition: ContentId,
    pub region: ContentId,
    pub position: Point3,
    pub home: Point3,
    pub lifecycle: ActorLifecycle,
    pub behavior: ActorBehaviorState,
    pub active: bool,
    pub encounter_pending: bool,
    pub patrol_direction: i8,
    pub respawn_at: Option<Tick>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncounterOutcome {
    Defeated,
    PlayerFled,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerObservation {
    pub position: Point3,
    pub visible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldUpdate {
    pub tick: Tick,
    pub active_cells: BTreeSet<ContentId>,
    pub player: PlayerObservation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldCommand {
    SpawnActor {
        actor: EntityId,
        position: Point3,
    },
    DespawnActor {
        actor: EntityId,
    },
    MoveActor {
        actor: EntityId,
        position: Point3,
    },
    SetBehavior {
        actor: EntityId,
        behavior: ActorBehaviorState,
    },
    EncounterRequested {
        actor: EntityId,
    },
}
