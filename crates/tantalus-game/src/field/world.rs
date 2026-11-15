//! Game-side world state that backs the legacy event host.
//!
//! `FieldWorld` owns the actor, window, inventory, party, and request state
//! that decoded field events observe and mutate through `FieldHost`. It is a
//! simulation-state container: presentation-only event effects (sounds,
//! camera shakes, fades) are accepted and ignored by the host, while anything
//! a wait, query, or later event can observe lives here.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use tantalus_contracts::{EntityId, FieldNavigationMesh, Point3};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Movement {
    pub destination: [f32; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    pub target: i16,
    pub speed: i32,
    pub remaining_ticks: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Jump {
    pub from: [f32; 3],
    pub to: [f32; 3],
    pub steps: u8,
    pub frame: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActorState {
    pub position: [f32; 3],
    pub movement: Option<Movement>,
    pub exit_walk: bool,
    pub turn: Option<Turn>,
    pub animation_frames: u16,
    pub animation_frame: u16,
    pub talk_radius: u8,
    pub facing: i16,
    pub flags: i32,
    pub level: i32,
    pub jump: Option<Jump>,
}

impl ActorState {
    pub fn new(position: [f32; 3]) -> Self {
        Self {
            position,
            movement: None,
            exit_walk: false,
            turn: None,
            animation_frames: 0,
            animation_frame: 0,
            // Reference PosObj defaults both radii to 16.
            talk_radius: 16,
            facing: 0,
            flags: 0,
            level: 0,
            jump: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncounterRequest {
    pub rush: i32,
    pub group: Option<i32>,
    pub battle: i32,
    pub steiner_state: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TimerState {
    pub seconds: i32,
    pub running: bool,
    pub displayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CharacterStats {
    pub current_hp: i32,
    pub max_hp: i32,
    pub current_mp: i32,
    pub max_mp: i32,
}

pub(crate) const MAX_GIL: u32 = 9_999_999;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldWorld {
    pub actors: BTreeMap<EntityId, ActorState>,
    pub windows: BTreeMap<u8, bool>,
    pub items: BTreeMap<i32, i32>,
    pub gil: u32,
    pub party: Vec<i32>,
    pub character_stats: BTreeMap<i32, CharacterStats>,
    pub field_name: Option<i32>,
    pub timer: TimerState,
    pub encounters: Vec<EncounterRequest>,
    pub field_transitions: Vec<i16>,
    world_transitions: Vec<i16>,
    /// Field movie service (`fldfmv`), driven by `FMV` and system variables 14/15.
    pub fmv: super::FieldFmv,
    /// Reference `FF9.fldMapNo` of the running field (0 when unknown).
    pub field_map_no: i32,
    /// Reference `gEventGlobal`: Global-scope event variables (2048 bytes),
    /// shared by every entry and kept across fields.
    pub global_memory: tantalus_legacy::VariableMemory,
    /// Reference `EventContext.mapvar`: Map-scope event variables (80 bytes),
    /// shared by every entry and zeroed when a field loads.
    pub map_memory: tantalus_legacy::VariableMemory,
    pending_requests: BTreeSet<EntityId>,
    pending_callers: BTreeSet<i32>,
    shared_scripts: BTreeSet<EntityId>,
    party_menu_open: bool,
    system_variables: BTreeMap<u8, i32>,
    system_lists: BTreeMap<u8, i32>,
    object_members: BTreeMap<(EntityId, u8), i32>,
    member_target: Option<EntityId>,
    uid_map: BTreeMap<i32, EntityId>,
    rng: u64,
    input_mask: i32,
    /// Walk speed in world units per tick for synthetic movement progress.
    pub walk_speed: f32,
    /// Default animation duration in ticks when a script plays an animation.
    pub animation_default_ticks: u16,
    /// Exit walks report movement active until the director marks the arrival.
    pub exit_arrivals: BTreeSet<EntityId>,
    /// Reward resolutions already applied to this world (exactly-once ledger).
    applied_resolutions: BTreeSet<u64>,
    /// Fixed-point atan lookup table from the owned install
    /// (`ratan_tbl.bin`); absent until the caller provides it.
    atan_table: Option<Vec<u8>>,
    /// Field walkmesh backing BGI character queries; absent until the caller
    /// imports the field's navigation mesh.
    navigation: Option<FieldNavigationMesh>,
    /// Last actor observed in applied host events; v1 stand-in for the
    /// reference's current-executing-object member target.
    current_actor: Option<EntityId>,
    /// The player-controlled actor, once the caller designates one.
    player: Option<EntityId>,
    /// Confirm-button bit for talk triggers; none disables detection until
    /// the caller configures the pad layout.
    confirm_mask: Option<i32>,
}

impl Default for FieldWorld {
    fn default() -> Self {
        Self::new()
    }
}

impl FieldWorld {
    pub fn new() -> Self {
        Self {
            actors: BTreeMap::new(),
            windows: BTreeMap::new(),
            items: BTreeMap::new(),
            gil: 0,
            party: Vec::new(),
            character_stats: BTreeMap::new(),
            field_name: None,
            timer: TimerState::default(),
            encounters: Vec::new(),
            field_transitions: Vec::new(),
            world_transitions: Vec::new(),
            fmv: super::FieldFmv::new(),
            field_map_no: 0,
            global_memory: tantalus_legacy::VariableMemory::global(),
            map_memory: tantalus_legacy::VariableMemory::map(),
            pending_requests: BTreeSet::new(),
            pending_callers: BTreeSet::new(),
            shared_scripts: BTreeSet::new(),
            party_menu_open: false,
            system_variables: BTreeMap::new(),
            system_lists: BTreeMap::new(),
            object_members: BTreeMap::new(),
            member_target: None,
            uid_map: BTreeMap::new(),
            rng: 0x9E37_79B9_7F4A_7C15,
            input_mask: 0,
            walk_speed: 16.0,
            animation_default_ticks: 32,
            exit_arrivals: BTreeSet::new(),
            applied_resolutions: BTreeSet::new(),
            atan_table: None,
            navigation: None,
            current_actor: None,
            player: None,
            confirm_mask: None,
        }
    }

    pub fn add_actor(&mut self, actor: EntityId, position: [f32; 3]) {
        self.actors.insert(actor, ActorState::new(position));
    }

    pub fn map_uid(&mut self, uid: i32, entity: EntityId) {
        self.uid_map.insert(uid, entity);
    }

    pub fn set_input(&mut self, mask: i32) {
        self.input_mask = mask;
    }

    pub fn set_player(&mut self, actor: EntityId) {
        self.player = Some(actor);
    }

    pub fn player(&self) -> Option<EntityId> {
        self.player
    }

    pub fn set_confirm_mask(&mut self, mask: Option<i32>) {
        self.confirm_mask = mask;
    }

    pub fn confirm_mask(&self) -> Option<i32> {
        self.confirm_mask
    }

    pub fn set_talk_radius(&mut self, actor: EntityId, radius: u8) {
        if let Some(state) = self.actors.get_mut(&actor) {
            state.talk_radius = radius;
        }
    }

    /// Objects whose talk radius contains the player while confirm is held.
    /// Objects whose talk range contains the player while confirm is held.
    /// The reference sums both radii scaled by four
    /// (`EventCollision.Collision`): `4 * (self.talkRad + other.talkRad)`.
    pub(crate) fn talkable_from(&self, player: EntityId) -> Vec<EntityId> {
        let Some(player_state) = self.actors.get(&player) else {
            return Vec::new();
        };
        let position = player_state.position;
        self.actors
            .iter()
            .filter(|(entity, state)| {
                **entity != player && {
                    let reach =
                        4 * (i32::from(player_state.talk_radius) + i32::from(state.talk_radius));
                    let dx = f64::from(state.position[0] - position[0]);
                    let dz = f64::from(state.position[2] - position[2]);
                    (dx * dx + dz * dz).sqrt() <= f64::from(reach)
                }
            })
            .map(|(entity, _)| *entity)
            .collect()
    }

    pub fn input_mask(&self) -> i32 {
        self.input_mask
    }

    pub fn close_window(&mut self, window: u8) {
        self.windows.insert(window, false);
    }

    pub fn open_party_menu(&mut self) {
        self.party_menu_open = true;
    }

    pub fn close_party_menu(&mut self) {
        self.party_menu_open = false;
    }

    pub fn party_menu_open(&self) -> bool {
        self.party_menu_open
    }

    pub fn caller_pending(&self, level: i32) -> bool {
        self.pending_callers.contains(&level)
    }

    pub fn pend_request(&mut self, entity: EntityId) {
        self.pending_requests.insert(entity);
    }

    pub fn complete_request(&mut self, entity: EntityId) {
        self.pending_requests.remove(&entity);
    }

    pub fn pend_caller(&mut self, level: i32) {
        self.pending_callers.insert(level);
    }

    pub fn complete_caller(&mut self, level: i32) {
        self.pending_callers.remove(&level);
    }

    pub fn start_shared_script(&mut self, actor: EntityId) {
        self.shared_scripts.insert(actor);
    }

    pub fn finish_shared_script(&mut self, actor: EntityId) {
        self.shared_scripts.remove(&actor);
    }

    pub fn shared_script_active(&self, actor: EntityId) -> bool {
        self.shared_scripts.contains(&actor)
    }

    pub fn mark_exit_arrival(&mut self, actor: EntityId) {
        self.exit_arrivals.insert(actor);
    }

    pub fn take_encounters(&mut self) -> Vec<EncounterRequest> {
        std::mem::take(&mut self.encounters)
    }

    pub fn take_field_transitions(&mut self) -> Vec<i16> {
        std::mem::take(&mut self.field_transitions)
    }

    pub fn take_world_transitions(&mut self) -> Vec<i16> {
        std::mem::take(&mut self.world_transitions)
    }

    pub fn request_pending(&self, entity: EntityId) -> bool {
        self.pending_requests.contains(&entity)
    }

    /// Advance movement, turns, and animations by one tick.
    pub fn advance(&mut self) {
        for state in self.actors.values_mut() {
            if let Some(movement) = &state.movement {
                let [dx, dz] = [
                    movement.destination[0] - state.position[0],
                    movement.destination[1] - state.position[2],
                ];
                let distance = f32::sqrt(dx * dx + dz * dz);
                if distance <= self.walk_speed || distance == 0.0 {
                    state.position[0] = movement.destination[0];
                    state.position[2] = movement.destination[1];
                    state.movement = None;
                } else {
                    state.position[0] += dx / distance * self.walk_speed;
                    state.position[2] += dz / distance * self.walk_speed;
                }
            }
            if let Some(turn) = &mut state.turn {
                turn.remaining_ticks = turn.remaining_ticks.saturating_sub(1);
                if turn.remaining_ticks == 0 {
                    state.turn = None;
                }
            }
            if state.animation_frames > 0 {
                state.animation_frames = state.animation_frames.saturating_sub(1);
                state.animation_frame = state.animation_frame.saturating_add(1);
            }
        }
    }

    pub(crate) fn next_random_u8(&mut self) -> u8 {
        // xorshift64* for deterministic synthetic randomness.
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        (self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 56) as u8
    }

    pub(crate) fn system_variable(&self, variable: u8) -> Option<i32> {
        self.system_variables.get(&variable).copied()
    }

    pub(crate) fn system_list(&self, index: u8) -> Option<i32> {
        self.system_lists.get(&index).copied()
    }

    pub(crate) fn set_system_list(&mut self, index: u8, value: i32) {
        self.system_lists.insert(index, value);
    }

    pub(crate) fn object_member(&self, key: &(EntityId, u8)) -> Option<i32> {
        self.object_members.get(key).copied()
    }

    pub(crate) fn set_object_member(&mut self, key: (EntityId, u8), value: i32) {
        self.object_members.insert(key, value);
    }

    pub(crate) fn member_target(&self) -> Option<EntityId> {
        self.member_target.or(self.current_actor)
    }

    pub(crate) fn set_current_actor(&mut self, actor: Option<EntityId>) {
        self.current_actor = actor;
    }

    /// Reverse UID lookup for member reads.
    pub(crate) fn uid_of(&self, entity: EntityId) -> Option<i32> {
        self.uid_map
            .iter()
            .find(|(_, mapped)| **mapped == entity)
            .map(|(uid, _)| *uid)
    }

    pub(crate) fn set_member_target(&mut self, target: Option<EntityId>) {
        self.member_target = target;
    }

    pub(crate) fn resolve_uid(&self, uid: i32) -> Option<EntityId> {
        self.uid_map.get(&uid).copied()
    }

    pub(crate) fn push_world_transition(&mut self, destination: i16) {
        self.world_transitions.push(destination);
    }

    /// Record a reward resolution as applied; returns false on a duplicate.
    pub(crate) fn apply_resolution(&mut self, resolution_id: u64) -> bool {
        self.applied_resolutions.insert(resolution_id)
    }

    pub fn add_gil(&mut self, amount: u32) {
        self.gil = self.gil.saturating_add(amount).min(MAX_GIL);
    }

    pub fn provide_atan_table(&mut self, bytes: Vec<u8>) {
        self.atan_table = Some(bytes);
    }

    pub(crate) fn atan_entry(&self, offset: usize) -> Option<u16> {
        let table = self.atan_table.as_ref()?;
        let entry = table.get(offset..offset.checked_add(2)?)?;
        Some(u16::from_le_bytes(entry.try_into().ok()?))
    }

    pub fn provide_navigation(&mut self, mesh: FieldNavigationMesh) {
        self.navigation = Some(mesh);
    }

    pub fn navigation(&self) -> Option<&FieldNavigationMesh> {
        self.navigation.as_ref()
    }

    /// Project a world position onto the walkmesh, returning the containing
    /// triangle index.
    pub(crate) fn navigation_triangle_at(&self, position: [f32; 3]) -> Option<usize> {
        let mesh = self.navigation.as_ref()?;
        let navigation = crate::navigation::FieldNavigation::new(mesh).ok()?;
        let projection = navigation
            .project(Point3 {
                x: position[0].round() as i32,
                y: position[1].round() as i32,
                z: position[2].round() as i32,
            })
            .ok()?;
        Some(projection.triangle_index)
    }
}
