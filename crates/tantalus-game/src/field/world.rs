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
    /// Facing on the `getvobj` member-3 scale, 0..=255 (0 south, 64 west,
    /// 128 north, 192 east).
    pub facing: i16,
    pub flags: i32,
    pub level: i32,
    pub jump: Option<Jump>,
    /// `AFRAME` (`inFrame`/`outFrame`): the frames the next `ANIM` plays
    /// from and to; 0 and 255 play the whole animation.
    #[serde(default)]
    pub in_frame: u8,
    #[serde(default = "full_out_frame")]
    pub out_frame: u8,
}

fn full_out_frame() -> u8 {
    u8::MAX
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
            in_frame: 0,
            out_frame: u8::MAX,
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

/// Runtime movie frame-count lookup for the FMV service, keyed by
/// `(disc, number)`. Behavior, not state: snapshots do not carry it.
pub type FmvFrameCounts = std::sync::Arc<dyn Fn(i32, i32) -> Option<u32> + Send + Sync>;

/// Maps a content item identity to the legacy inventory item id.
pub type ItemIdMap =
    std::sync::Arc<dyn Fn(&tantalus_contracts::ContentId) -> Option<i32> + Send + Sync>;

/// Builds the default reward-item identity mapping for a vanilla item table:
/// every supplied legacy id resolves the content identity `ffix:item/<id>`.
/// Unknown identities resolve to `None` so callers can record them as
/// unmapped instead of dropping them silently.
pub fn default_item_id_map(legacy_ids: impl IntoIterator<Item = i32>) -> ItemIdMap {
    let table: BTreeMap<String, i32> = legacy_ids
        .into_iter()
        .map(|id| (format!("ffix:item/{id}"), id))
        .collect();
    std::sync::Arc::new(move |content: &tantalus_contracts::ContentId| {
        table.get(content.as_str()).copied()
    })
}

/// Computes a character's stats for a total experience amount; the
/// provider owns the level tables. Runtime-only.
pub type LevelGrowth = std::sync::Arc<dyn Fn(i32, u64) -> Option<CharacterStats> + Send + Sync>;

#[derive(Clone, Serialize, Deserialize)]
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
    /// Region polygons by region object (`SetRegion`).
    pub regions: BTreeMap<EntityId, super::region::Region>,
    /// Reference `controlUID` owner: the actor that ran `DefinePlayerControl`.
    pub control_actor: Option<EntityId>,
    /// Reference user control flag (`SetUserControl`); region and talk checks
    /// only run while it is on.
    pub user_control: bool,
    /// `ETb.gMesSignal`, read as system variable 8 (`GetDialogProgression`).
    /// Set by `SETSIGNAL` (0xE3) and by `[SIGL=n]` tags in displayed text.
    pub mes_signal: i32,
    /// `ETb.sChoose`, read as system variable 9 (`GetDialogChoice`): the
    /// option picked in the last choice window. Set by the player (or a
    /// scripted stand-in) when it closes a window.
    pub dialog_choice: i32,
    /// Text id each window last showed.
    pub window_texts: BTreeMap<u8, u16>,
    /// Field text id -> the `[SIGL=n]` values its text carries, in order.
    /// Loaded by the caller from the field's `.mes` file (this crate embeds
    /// no game text). Empty means windows raise no signals.
    pub dialog_signals: BTreeMap<u16, Vec<i32>>,
    /// Signals raised by shown windows and not yet applied; the reference
    /// applies at most one per event tick (`DialogBoxSymbols.OnSignal`).
    pending_signals: std::collections::VecDeque<i32>,
    pending_requests: BTreeSet<EntityId>,
    pending_callers: BTreeSet<i32>,
    shared_scripts: BTreeSet<EntityId>,
    party_menu_open: bool,
    system_variables: BTreeMap<u8, i32>,
    system_lists: BTreeMap<u8, i32>,
    object_members: BTreeMap<(EntityId, u8), i32>,
    member_target: Option<EntityId>,
    uid_map: BTreeMap<i32, EntityId>,
    /// Objects removed by `DELETE` (`DisposeObj`): every function of a
    /// deleted object stops until the object is initialized again.
    #[serde(default)]
    deleted_objects: BTreeSet<EntityId>,
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
    /// Direction-bit mapping `(up, down, left, right)` for player movement;
    /// none means input never moves the player (no layout guessed).
    walk_bits: Option<(i32, i32, i32, i32)>,
    /// How often scenes have read the player actor's facing (member 3) -
    /// a rising count means a facing gate is polling.
    pub player_facing_polls: u64,
    /// Movie frame-count provider for the FMV service, keyed by
    /// `(disc, number)`. Runtime-only; snapshots do not carry it.
    #[serde(skip)]
    pub fmv_frame_counts: Option<FmvFrameCounts>,
    /// Reward-item identity to legacy inventory id mapping (runtime-only).
    #[serde(skip)]
    pub item_id_map: Option<ItemIdMap>,
    /// Total experience per character id.
    pub character_exp: BTreeMap<i32, u64>,
    /// Ability learning progress per party character: character -> ability
    /// id -> AP accumulated toward the ability's max (reference `player.pa`).
    #[serde(default)]
    pub ability_pa: BTreeMap<i32, BTreeMap<i32, i32>>,
    /// Abilities a character has learned through [`FieldWorld::apply_ap`]
    /// (reached max AP). The reference derives this from `pa` on equip;
    /// denormalized here for save/inspection.
    #[serde(default)]
    pub learned_abilities: BTreeMap<i32, BTreeSet<i32>>,
    /// Equipped item ids per character and slot (weapon, head, body, wrist,
    /// accessory; `None` = empty). Legacy item ids, matching the inventory
    /// map. Stat effects are computed by the caller's equipment/level rules
    /// (this world stores ownership only).
    #[serde(default)]
    pub equipment: BTreeMap<i32, [Option<u16>; 5]>,
    /// Experience-to-stats provider (runtime-only).
    #[serde(skip)]
    pub level_growth: Option<LevelGrowth>,
}

impl std::fmt::Debug for FieldWorld {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FieldWorld")
            .field("actors", &self.actors)
            .field("windows", &self.windows)
            .field("gil", &self.gil)
            .field("party", &self.party)
            .field("player", &self.player)
            .field("confirm_mask", &self.confirm_mask)
            .field("walk_bits", &self.walk_bits)
            .field(
                "fmv_frame_counts",
                &self.fmv_frame_counts.as_ref().map(|_| "provided"),
            )
            .finish()
    }
}

impl PartialEq for FieldWorld {
    fn eq(&self, other: &Self) -> bool {
        // Compare every field; the runtime frame-count provider is
        // behavior, not state.
        self.actors == other.actors
            && self.windows == other.windows
            && self.items == other.items
            && self.gil == other.gil
            && self.party == other.party
            && self.character_stats == other.character_stats
            && self.field_name == other.field_name
            && self.timer == other.timer
            && self.encounters == other.encounters
            && self.field_transitions == other.field_transitions
            && self.world_transitions == other.world_transitions
            && self.pending_requests == other.pending_requests
            && self.pending_callers == other.pending_callers
            && self.shared_scripts == other.shared_scripts
            && self.party_menu_open == other.party_menu_open
            && self.system_variables == other.system_variables
            && self.system_lists == other.system_lists
            && self.object_members == other.object_members
            && self.member_target == other.member_target
            && self.uid_map == other.uid_map
            && self.deleted_objects == other.deleted_objects
            && self.rng == other.rng
            && self.input_mask == other.input_mask
            && self.walk_speed == other.walk_speed
            && self.animation_default_ticks == other.animation_default_ticks
            && self.exit_arrivals == other.exit_arrivals
            && self.applied_resolutions == other.applied_resolutions
            && self.atan_table == other.atan_table
            && self.navigation == other.navigation
            && self.current_actor == other.current_actor
            && self.player == other.player
            && self.confirm_mask == other.confirm_mask
            && self.walk_bits == other.walk_bits
            && self.character_exp == other.character_exp
            && self.ability_pa == other.ability_pa
            && self.learned_abilities == other.learned_abilities
            && self.equipment == other.equipment
    }
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
            regions: BTreeMap::new(),
            control_actor: None,
            user_control: false,
            mes_signal: 0,
            dialog_choice: 0,
            window_texts: BTreeMap::new(),
            dialog_signals: BTreeMap::new(),
            pending_signals: std::collections::VecDeque::new(),
            pending_requests: BTreeSet::new(),
            pending_callers: BTreeSet::new(),
            shared_scripts: BTreeSet::new(),
            party_menu_open: false,
            system_variables: BTreeMap::new(),
            system_lists: BTreeMap::new(),
            object_members: BTreeMap::new(),
            member_target: None,
            uid_map: BTreeMap::new(),
            deleted_objects: BTreeSet::new(),
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
            walk_bits: None,
            player_facing_polls: 0,
            fmv_frame_counts: None,
            item_id_map: None,
            character_exp: BTreeMap::new(),
            ability_pa: BTreeMap::new(),
            learned_abilities: BTreeMap::new(),
            equipment: BTreeMap::new(),
            level_growth: None,
        }
    }

    pub fn add_actor(&mut self, actor: EntityId, position: [f32; 3]) {
        self.actors.insert(actor, ActorState::new(position));
    }

    pub fn map_uid(&mut self, uid: i32, entity: EntityId) {
        self.uid_map.insert(uid, entity);
    }

    /// `DisposeObj`: stops the object's functions. The actor state is kept
    /// so a later `NEW` of the same object finds it again (the chain host
    /// creates every entry's actor up front instead of at `NEW`).
    pub fn delete_object(&mut self, entity: EntityId) {
        self.deleted_objects.insert(entity);
    }

    /// `InitEvents` starts every field with a fresh object list, so no
    /// object stays deleted across a field change.
    pub fn clear_deleted_objects(&mut self) {
        self.deleted_objects.clear();
    }

    /// `InitEvents` (EventEngine.cs:556) frees every object of the old
    /// field: its regions, windows, pending requests, shared scripts and
    /// member overrides go with it. Party, items, globals and actors stay.
    pub fn begin_field(&mut self) {
        self.regions.clear();
        self.windows.clear();
        self.window_texts.clear();
        self.pending_requests.clear();
        self.pending_callers.clear();
        self.shared_scripts.clear();
        self.object_members.clear();
        self.deleted_objects.clear();
        self.actors.clear();
        self.control_actor = None;
        self.exit_arrivals.clear();
    }

    /// `NEW` of a deleted object makes it run again.
    pub fn restore_object(&mut self, entity: EntityId) {
        self.deleted_objects.remove(&entity);
    }

    pub fn is_object_deleted(&self, entity: EntityId) -> bool {
        self.deleted_objects.contains(&entity)
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

    pub fn set_walk_bits(&mut self, bits: Option<(i32, i32, i32, i32)>) {
        self.walk_bits = bits;
    }

    /// Move the player actor by the configured direction bits (straight XZ
    /// at walk speed; walkmesh clamping is future work).
    pub(crate) fn apply_walk_input(&mut self) {
        // Input only moves the character while the engine has granted user
        // control (scenes disable it while they drive the actor).
        if !self.user_control {
            return;
        }
        let (Some(player), Some((up, down, left, right))) = (self.player, self.walk_bits) else {
            return;
        };
        let mask = self.input_mask;
        let mut dx = 0.0f32;
        let mut dz = 0.0f32;
        if mask & up != 0 {
            dz -= self.walk_speed;
        }
        if mask & down != 0 {
            dz += self.walk_speed;
        }
        if mask & left != 0 {
            dx -= self.walk_speed;
        }
        if mask & right != 0 {
            dx += self.walk_speed;
        }
        if dx == 0.0 && dz == 0.0 {
            return;
        }
        if let Some(state) = self.actors.get_mut(&player) {
            if dx != 0.0 && dz != 0.0 {
                dx *= std::f32::consts::FRAC_1_SQRT_2;
                dz *= std::f32::consts::FRAC_1_SQRT_2;
            }
            state.position[0] += dx;
            state.position[2] += dz;
            state.movement = None;
            // Input walking faces the movement direction like scripted
            // walks (reference facing scale, 0 south .. 192 east).
            if dx != 0.0 || dz != 0.0 {
                let degrees = dx.atan2(-dz).to_degrees();
                state.facing = (128.0 + degrees * 128.0 / 180.0).round().rem_euclid(256.0) as i16;
            }
        }
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
        self.apply_walk_input();
        self.apply_dialog_signal();
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
                    // Walking faces the movement direction on the reference
                    // facing scale (0 south, 64 west, 128 north, 192 east):
                    // `128 + atan2_deg(dx, -dz) * 128 / 180`, mod 256.
                    let degrees = dx.atan2(-dz).to_degrees();
                    state.facing =
                        (128.0 + degrees * 128.0 / 180.0).round().rem_euclid(256.0) as i16;
                }
            }
            if let Some(turn) = &mut state.turn {
                turn.remaining_ticks = turn.remaining_ticks.saturating_sub(1);
                if turn.remaining_ticks == 0 {
                    state.turn = None;
                }
            }
            if state.animation_frames == 0 && state.out_frame == u8::MAX {
                // The idle animation loops (`ExecAnim` restarts it at
                // frame 0), so member 7 keeps counting; map 66 entry 2
                // waits for frame 13 of it. Real clip lengths are not
                // loaded yet, so the loop uses the default length.
                state.animation_frame =
                    (state.animation_frame + 1) % self.animation_default_ticks.max(1);
            } else if state.animation_frames > 0 {
                state.animation_frames = state.animation_frames.saturating_sub(1);
                state.animation_frame = state.animation_frame.saturating_add(1);
                // The finished animation hands over to the idle loop, which
                // starts at frame 0 (`animFrame`, `getvobj` member 7).
                // Scripts wait for that with `while member 7 != 0`.
                if state.animation_frames == 0 {
                    // A ranged animation (`AFRAME`) stops on its out
                    // frame (map 115's rope climb steps one frame per
                    // `ANIM` and reads it back).
                    state.animation_frame = if state.out_frame == u8::MAX {
                        0
                    } else {
                        u16::from(state.out_frame)
                    };
                }
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

    /// Queues the `[SIGL]` signals of a text a window just showed.
    ///
    /// Approximation: the reference raises each signal when the typewriter
    /// reaches its tag; here they are applied one per tick from the next
    /// tick on, without waiting for text speed, `[WAIT]` or page breaks.
    pub(crate) fn raise_text_signals(&mut self, text: u16) {
        if let Some(signals) = self.dialog_signals.get(&text) {
            self.pending_signals.extend(signals.iter().copied());
        }
    }

    /// `DialogBoxSymbols.OnSignal`: a negative signal increments the value,
    /// any other replaces it.
    fn apply_dialog_signal(&mut self) {
        if let Some(signal) = self.pending_signals.pop_front() {
            if signal < 0 {
                self.mes_signal = self.mes_signal.wrapping_add(1);
            } else {
                self.mes_signal = signal;
            }
        }
    }

    pub(crate) fn system_variable(&self, variable: u8) -> Option<i32> {
        self.system_variables.get(&variable).copied()
    }

    pub(crate) fn set_system_variable(&mut self, variable: u8, value: i32) {
        self.system_variables.insert(variable, value);
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

    /// Reference EXP-share eligibility for a party line (v2): the character
    /// has stats and is alive. Status-based reward disabling
    /// (Petrify/Virus/Zombie, `BattleStatus.DisableRewards`) and the
    /// `member_flag` participation bit are not yet modeled in `FieldWorld`.
    pub fn exp_eligible(&self, character: i32) -> bool {
        self.character_stats
            .get(&character)
            .is_some_and(|stats| stats.current_hp > 0)
    }

    /// Award experience to a character and recompute stats through the
    /// level-growth provider. On level-up the max HP/MP delta is added to
    /// current values (reference level-up behavior).
    pub fn apply_experience(&mut self, character: i32, experience: u64) {
        let total = self.character_exp.entry(character).or_insert(0);
        *total = total.saturating_add(experience);
        let Some(growth) = self.level_growth.clone() else {
            return;
        };
        let Some(next) = growth(character, *self.character_exp.get(&character).unwrap()) else {
            return;
        };
        let merged = match self.character_stats.get(&character) {
            Some(current) => CharacterStats {
                current_hp: (current.current_hp + (next.max_hp - current.max_hp))
                    .clamp(0, next.max_hp),
                current_mp: (current.current_mp + (next.max_mp - current.max_mp))
                    .clamp(0, next.max_mp),
                ..next
            },
            None => next,
        };
        self.character_stats.insert(character, merged);
    }

    /// `AddAp` (`BattleResultUI.cs:561-587`) for one ability of one
    /// character. The caller loops the character's equipped learnable
    /// abilities (as the result screen does); this applies the reference
    /// semantics per ability: skip when there is no AP goal or it is already
    /// learned; learn (set to max) when `cur + amount >= max`; otherwise
    /// accumulate. Returns true exactly when the ability is learned by this
    /// call.
    pub fn apply_ap(&mut self, character: i32, ability: i32, max_ap: i32, amount: i32) -> bool {
        if max_ap <= 0 || amount <= 0 {
            return false;
        }
        let pa = self
            .ability_pa
            .entry(character)
            .or_default()
            .entry(ability)
            .or_insert(0);
        if *pa >= max_ap {
            return false;
        }
        if *pa + amount >= max_ap {
            *pa = max_ap;
            self.learned_abilities
                .entry(character)
                .or_default()
                .insert(ability);
            return true;
        }
        *pa += amount;
        false
    }

    /// Equips `item` (legacy id) in `slot` (0 weapon, 1 head, 2 body, 3
    /// wrist, 4 accessory); `None` empties the slot. Ownership of the item
    /// is the inventory's concern, not this call's.
    pub fn equip(&mut self, character: i32, slot: usize, item: Option<u16>) {
        if slot < 5 {
            self.equipment.entry(character).or_default()[slot] = item;
        }
    }

    /// The character's equipped item ids, five slots, `None` when unset.
    pub fn equipped(&self, character: i32) -> [Option<u16>; 5] {
        self.equipment.get(&character).copied().unwrap_or([None; 5])
    }

    pub fn add_item_amount(&mut self, item: i32, amount: i32) {
        let entry = self.items.entry(item).or_insert(0);
        *entry = entry.saturating_add(amount).min(99);
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

    pub fn clear_navigation(&mut self) {
        self.navigation = None;
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
