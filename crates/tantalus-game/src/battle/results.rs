use serde::{Deserialize, Serialize};
use tantalus_contracts::{ContentId, EntityId, SemanticMarker, Tick};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CombatEvent {
    ActionStarted {
        actor: EntityId,
        action: ContentId,
    },
    DamageResolved {
        source: EntityId,
        target: EntityId,
        amount: i32,
        hp_after: u32,
        missed: bool,
    },
    UnitDefeated {
        unit: EntityId,
    },
    ActionInterrupted {
        actor: EntityId,
        action: ContentId,
    },
    ActionCompleted {
        actor: EntityId,
        action: ContentId,
    },
    BattleCompleted {
        outcome: BattleOutcome,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BattleOutcome {
    Victory,
    Defeat,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RewardBundle {
    pub resolution_id: u64,
    pub experience: u64,
    pub gil: u64,
    pub items: Vec<ContentId>,
    /// Per-character experience awards computed by the battle engine using
    /// the reference split rule (`BattleResultUI`: total / eligible count,
    /// remainder lost). When `Some`, only the listed characters receive
    /// experience; when `None`, the committer's fallback applies (v1: full
    /// grant to every party member).
    pub shares: Option<std::collections::BTreeMap<i32, u64>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CombatUpdate {
    pub tick: Tick,
    pub events: Vec<CombatEvent>,
    pub markers: Vec<SemanticMarker>,
    pub outcome: Option<BattleOutcome>,
    pub rewards: Option<RewardBundle>,
    pub active_action: bool,
}
