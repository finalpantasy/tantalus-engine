//! Field save snapshots: a versioned, serde-native envelope capturing world
//! state plus per-entry program sources and observed VM state.
//!
//! Restoring the world is exact. Entry VMs restart their programs from tick
//! zero for now: mid-function resume needs `EventVm::resume` (requested from
//! Claude), so [`EntrySnapshot::resumed_from_start`] is set whenever the saved
//! pc was past the entry point with work remaining. The world-side reward
//! ledger makes restarts safe against double commits.

use super::{FieldScene, FieldWorld};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tantalus_contracts::{EngineError, EntityId, ErrorCategory};
use tantalus_legacy::{VmStatus, WaitState, decode_program};

pub const FIELD_SNAPSHOT_FORMAT: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntrySnapshot {
    pub entity: EntityId,
    /// Raw event program bytes; re-decoded on restore.
    pub program: Vec<u8>,
    pub pc: u32,
    pub status: VmStatus,
    pub wait_state: Option<WaitState>,
    /// True when the entry restarts from tick zero on restore (mid-function
    /// resume is not available yet); false when the snapshot is at a clean
    /// boundary (pc 0 or a completed entry).
    pub resumed_from_start: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldSnapshot {
    pub format: u32,
    pub world: FieldWorld,
    pub entries: Vec<EntrySnapshot>,
}

impl FieldSnapshot {
    /// JSON round trip used by the native save envelope; format-checked.
    pub fn to_json(&self) -> Result<String, EngineError> {
        serde_json::to_string(self).map_err(|error| {
            EngineError::new(
                ErrorCategory::Internal,
                format!("field snapshot serialization failed: {error}"),
                "tantalus-game/snapshot",
                true,
            )
        })
    }

    pub fn from_json(text: &str) -> Result<Self, EngineError> {
        let snapshot: Self = serde_json::from_str(text).map_err(|error| {
            EngineError::new(
                ErrorCategory::UnknownContent,
                format!("field snapshot is not decodable: {error}"),
                "tantalus-game/snapshot",
                true,
            )
        })?;
        if snapshot.format != FIELD_SNAPSHOT_FORMAT {
            return Err(EngineError::new(
                ErrorCategory::UnknownContent,
                format!(
                    "unsupported field snapshot format {}; expected {FIELD_SNAPSHOT_FORMAT}",
                    snapshot.format
                ),
                "tantalus-game/snapshot",
                true,
            ));
        }
        Ok(snapshot)
    }
}

/// Capture the scene and world. `program_bytes` must hold the raw event bytes
/// for every entry entity (the same bytes the caller decoded when building
/// the scene); missing sources are an explicit error.
pub fn snapshot_scene(
    scene: &FieldScene,
    world: &FieldWorld,
    program_bytes: &BTreeMap<EntityId, Vec<u8>>,
) -> Result<FieldSnapshot, EngineError> {
    let mut entries = Vec::new();
    for (entity, pc, status, wait_state) in scene.entry_states() {
        let program = program_bytes.get(&entity).ok_or_else(|| {
            EngineError::new(
                ErrorCategory::UnknownContent,
                format!("entry {entity} has no program source bytes for a snapshot"),
                "tantalus-game/snapshot",
                false,
            )
        })?;
        let clean_boundary = pc == 0 || status == VmStatus::Completed;
        entries.push(EntrySnapshot {
            entity,
            program: program.clone(),
            pc,
            status,
            wait_state,
            resumed_from_start: !clean_boundary,
        });
    }
    Ok(FieldSnapshot {
        format: FIELD_SNAPSHOT_FORMAT,
        world: world.clone(),
        entries,
    })
}

/// Rebuild a scene and world from a snapshot. World state restores exactly;
/// entries restart their programs unless the snapshot was at a clean
/// boundary, in which case restart is indistinguishable from resume.
pub fn restore_scene(snapshot: &FieldSnapshot) -> Result<(FieldScene, FieldWorld), EngineError> {
    let mut scene = FieldScene::new();
    for entry in &snapshot.entries {
        let program = decode_program(&entry.program).map_err(|error| {
            EngineError::new(
                ErrorCategory::UnknownContent,
                format!("entry {} program no longer decodes: {error}", entry.entity),
                "tantalus-game/snapshot",
                true,
            )
        })?;
        scene.add_entry(entry.entity, program)?;
    }
    Ok((scene, snapshot.world.clone()))
}
