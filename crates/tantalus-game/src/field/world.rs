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
use tantalus_legacy::event::party::{EntryLink, PARTY_SLOT_FIRST, PARTY_SLOT_LAST, party_uid};

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
    /// Moved by pad input this tick (`FieldMapActorController.isMoving`),
    /// so the renderer plays the walk loop. Runtime-only.
    #[serde(skip)]
    pub input_walking: bool,
    /// The input move this tick was a run (`isRunning`). Runtime-only.
    #[serde(skip)]
    pub input_running: bool,
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
    /// What the renderer draws; the scripts never read it back.
    #[serde(default)]
    pub look: ActorLook,
}

/// One field camera script, as `FieldMap` receives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraRequest {
    /// `SCROLL` (`EBG_scene2DScroll`): scroll the view center to `x`, `y`
    /// (background pixels) over `frames`, then hold there.
    Move {
        x: i16,
        y: i16,
        frames: u16,
        mode: u32,
    },
    /// `UNSCROLL` (`EBG_scene2DScrollRelease`): scroll back to the player
    /// over `frames` (-1 is 30), then follow again.
    Release { frames: i32, mode: u32 },
    /// `SCROLLON`/off (`EBG_char3DScrollSetActive`).
    Services {
        active: bool,
        frames: i32,
        mode: u32,
    },
    /// `charAimHeight`.
    FollowHeight(i16),
    /// `BGVPORT` (`EBG_cameraSetViewport`): `[min x, max x, min y, max y]`.
    Bounds { camera: i32, bounds: [i16; 4] },
}

/// An actor's model and animation ids from `SetActorModel` (`SETMODEL`),
/// the stand, walk and run animations, and the one `ANIM` last started.
/// Ids are `FF9DBAll` model and animation ids.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ActorLook {
    pub model: Option<u16>,
    pub idle: Option<u16>,
    pub walk: Option<u16>,
    pub run: Option<u16>,
    /// The animation `PlayAnimation` started, until `StopAnimation`.
    pub playing: Option<u16>,
    /// `meshflags`: bit i set hides the model's `mesh<i>` child
    /// (`MESHHIDE`/`MESHSHOW`).
    pub hidden_meshes: u32,
    /// `pflags`: the flags `PUSHHIDE` saved for `POPSHOW`.
    pub saved_flags: i32,
}

impl ActorLook {
    /// Like [`Self::current`], but a run picks the run loop first.
    pub fn current_moving(&self, playing: bool, moving: bool, running: bool) -> Option<u16> {
        if running && !(playing && self.playing.is_some()) {
            return self.run.or(self.walk).or(self.idle);
        }
        self.current(playing, moving)
    }

    /// The animation to draw: the one `ANIM` started while it still runs,
    /// else walk while moving, else the stand animation.
    pub fn current(&self, playing: bool, moving: bool) -> Option<u16> {
        self.playing
            .filter(|_| playing)
            .or(if moving { self.walk.or(self.run) } else { None })
            .or(self.idle)
    }
}

fn full_out_frame() -> u8 {
    u8::MAX
}

impl ActorState {
    pub fn new(position: [f32; 3]) -> Self {
        Self {
            position,
            movement: None,
            input_walking: false,
            input_running: false,
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
            look: ActorLook::default(),
        }
    }
}

/// `Configuration.Battle.EncounterInterval` and `EncounterInitial` defaults.
const ENCOUNTER_INTERVAL: f32 = 960.0;
const ENCOUNTER_INITIAL: f32 = -1440.0;

/// `EventEngine._enCountData`, `_context.encratio`, `_encountTimer`,
/// `_encountBase` and `_lastScene`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RandomBattles {
    pub pattern: u8,
    pub scenes: [u16; 4],
    /// Rate 0-255; 0 turns random battles off.
    pub ratio: u8,
    pub timer: f32,
    /// The danger value.
    pub base: i32,
    pub last_scene: u16,
}

impl Default for RandomBattles {
    fn default() -> Self {
        Self {
            pattern: 0,
            scenes: [0; 4],
            ratio: 0,
            timer: ENCOUNTER_INITIAL,
            base: 0,
            last_scene: 0,
        }
    }
}

impl RandomBattles {
    /// `InitEncount` (`EventEngine.Initialize.cs:9`) at each field start,
    /// without `PersistentDangerValue`.
    pub fn init_field(&mut self) {
        self.timer = ENCOUNTER_INITIAL;
        self.base = 0;
        self.ratio = 0;
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

/// Tetra Master collection size (`Configuration.TetraMaster.MaxCardCount`).
pub const MAX_CARDS: usize = 100;

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

/// What the scripts did to one background tile block (`BGOVERLAY_DEF`);
/// `None` keeps the imported value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverlayChange {
    /// `BGLACTIVE` (`EBG_overlaySetActive`).
    pub active: Option<bool>,
    /// `BGLORIGIN` (`EBG_overlaySetOrigin`): the block's position.
    pub origin: Option<[i16; 2]>,
    /// `BGLATTACH` (`EBG_charAttachOverlay`): the block follows the
    /// player, offset by these pixels (`BgAttachService`).
    pub attached: Option<[i16; 2]>,
    /// The `BGLATTACH` filter color when its mode is 0 or more: the block
    /// gets an opaque frame of it (`FieldMap.CreateBorder`).
    pub border: Option<[u8; 3]>,
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
    /// Field random battles (`ENCSCENE`, `ENCRATE`, `ProcessEncount`).
    #[serde(default)]
    pub random_battles: RandomBattles,
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
    /// Key-walk turn in degrees about +y (`FieldState.twist.y`), set by
    /// `SetControlDirection` and zeroed on field load.
    pub control_twist: f32,
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
    /// Requested functions still running, per target object: the script
    /// level of each call (`Obj.level` is the lowest, the running one).
    pending_requests: BTreeMap<EntityId, Vec<i32>>,
    pending_callers: BTreeSet<i32>,
    shared_scripts: BTreeSet<EntityId>,
    party_menu_open: bool,
    system_variables: BTreeMap<u8, i32>,
    system_lists: BTreeMap<u8, i32>,
    object_members: BTreeMap<(EntityId, u8), i32>,
    member_target: Option<EntityId>,
    uid_map: BTreeMap<i32, EntityId>,
    /// The field script's entries as `SetupPartyUID` reads them. Empty when
    /// the caller did not load a script table; party uids then stay plain
    /// uids.
    #[serde(default)]
    entry_links: Vec<EntryLink>,
    /// `EventContext.partyUID`: the entry behind each party slot (local
    /// uids 251-254).
    #[serde(default)]
    party_uid: [Option<usize>; 4],
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
    /// The button that makes the player walk instead of run while held
    /// (`Control.Cancel` with the default `cfg.move == 1`); none never walks.
    #[serde(skip)]
    walk_hold_bit: Option<i32>,
    /// `DASHOFF` (`dashinh`): the player always walks. Cleared on field load.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub run_disabled: bool,
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
    /// Tetra Master cards owned, by card id, in the order won
    /// (`QuadMistDatabase.MiniGame_SetCard`).
    #[serde(default)]
    pub cards: Vec<u32>,
    /// Background tile blocks the scripts changed on this field, by
    /// overlay index; the renderer applies them over the imported scene.
    #[serde(default)]
    pub overlays: BTreeMap<u16, OverlayChange>,
    /// The field's `.bgs` tile animations, with their run-time state;
    /// empty when the caller imports none (the chain tool).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tile_animations: Vec<super::tile_animation::TileAnimation>,
    /// Camera count of the field scene; `SetFieldCamera` ignores indices
    /// past it (`FieldMap.ActivateCamera`).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tile_camera_count: u8,
    /// The current field camera, tracked only while animations exist.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub field_camera: u8,
    /// The camera scripts this field ran, in order, for the renderer's
    /// view (runtime-only: the view depends on the field projection).
    #[serde(skip)]
    pub camera_log: Vec<CameraRequest>,
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
            && self.entry_links == other.entry_links
            && self.party_uid == other.party_uid
            && self.deleted_objects == other.deleted_objects
            && self.rng == other.rng
            && self.random_battles == other.random_battles
            && self.input_mask == other.input_mask
            && self.walk_speed == other.walk_speed
            && self.walk_hold_bit == other.walk_hold_bit
            && self.run_disabled == other.run_disabled
            && self.control_twist == other.control_twist
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
            && self.overlays == other.overlays
            && self.tile_animations == other.tile_animations
            && self.field_camera == other.field_camera
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
            control_twist: 0.0,
            mes_signal: 0,
            dialog_choice: 0,
            window_texts: BTreeMap::new(),
            dialog_signals: BTreeMap::new(),
            pending_signals: std::collections::VecDeque::new(),
            pending_requests: BTreeMap::new(),
            pending_callers: BTreeSet::new(),
            shared_scripts: BTreeSet::new(),
            party_menu_open: false,
            system_variables: BTreeMap::new(),
            system_lists: BTreeMap::new(),
            object_members: BTreeMap::new(),
            member_target: None,
            uid_map: BTreeMap::new(),
            entry_links: Vec::new(),
            party_uid: [None; 4],
            deleted_objects: BTreeSet::new(),
            rng: 0x9E37_79B9_7F4A_7C15,
            random_battles: RandomBattles::default(),
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
            walk_hold_bit: None,
            run_disabled: false,
            player_facing_polls: 0,
            fmv_frame_counts: None,
            item_id_map: None,
            character_exp: BTreeMap::new(),
            ability_pa: BTreeMap::new(),
            learned_abilities: BTreeMap::new(),
            equipment: BTreeMap::new(),
            cards: Vec::new(),
            overlays: BTreeMap::new(),
            tile_animations: Vec::new(),
            tile_camera_count: 0,
            field_camera: 0,
            camera_log: Vec::new(),
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
        // `EventEngine` clears `dashinh` when a field starts.
        self.run_disabled = false;
        self.regions.clear();
        self.windows.clear();
        self.window_texts.clear();
        self.pending_requests.clear();
        self.pending_callers.clear();
        self.shared_scripts.clear();
        self.object_members.clear();
        self.deleted_objects.clear();
        self.actors.clear();
        self.overlays.clear();
        self.camera_log.clear();
        self.clear_tile_animations();
        self.control_actor = None;
        self.exit_arrivals.clear();
        self.control_twist = 0.0;
        self.random_battles.init_field();
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

    /// The button that switches running to walking while held.
    pub fn set_walk_hold_bit(&mut self, bit: Option<i32>) {
        self.walk_hold_bit = bit;
    }

    /// Move the player actor by the configured direction bits the way
    /// `FieldMapActorController.UpdateMovement` does: up +z, down -z, left
    /// -x, right +x, normalized, turned by the control twist about +y, at
    /// walk speed (walkmesh clamping is the host's job).
    pub(crate) fn apply_walk_input(&mut self) {
        // Input only moves the character while the engine has granted user
        // control (scenes disable it while they drive the actor).
        if let Some(state) = self.player.and_then(|player| self.actors.get_mut(&player)) {
            state.input_walking = false;
            state.input_running = false;
        }
        if !self.user_control {
            return;
        }
        let (Some(player), Some((up, down, left, right))) = (self.player, self.walk_bits) else {
            return;
        };
        let mask = self.input_mask;
        let mut x = 0.0f32;
        let mut z = 0.0f32;
        if mask & up != 0 {
            z += 1.0;
        }
        if mask & down != 0 {
            z -= 1.0;
        }
        if mask & left != 0 {
            x -= 1.0;
        }
        if mask & right != 0 {
            x += 1.0;
        }
        let length = x.hypot(z);
        if length == 0.0 {
            return;
        }
        // `FieldMapActorController.Update`: the player runs unless the walk
        // button is held or `DASHOFF` is on; a run is two walk steps of
        // `speed` (30) in one frame.
        let walking = self.run_disabled
            || self
                .walk_hold_bit
                .is_some_and(|bit| self.input_mask & bit != 0);
        let step = PLAYER_WALK_SPEED * if walking { 1.0 } else { 2.0 };
        // Unity's `Quaternion.Euler(0, y, 0) * v`.
        let (sin, cos) = self.control_twist.to_radians().sin_cos();
        let dx = (x * cos + z * sin) / length * step;
        let dz = (z * cos - x * sin) / length * step;
        // `ProcessEvents.cs:252`: the controlled actor's walked distance
        // feeds the encounter timer while the rate is above 0.
        self.player_walked(dx.hypot(dz));
        if let Some(state) = self.actors.get_mut(&player) {
            state.position[0] += dx;
            state.position[2] += dz;
            state.movement = None;
            state.input_walking = true;
            state.input_running = !walking;
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

    /// A function requested on `entity` at script `level` started.
    pub fn pend_request(&mut self, entity: EntityId, level: i32) {
        self.pending_requests.entry(entity).or_default().push(level);
    }

    /// The running requested function of `entity` returned: its level (the
    /// lowest, `Call` only preempts a higher one) comes off the stack.
    pub fn complete_request(&mut self, entity: EntityId) {
        if let Some(levels) = self.pending_requests.get_mut(&entity) {
            if let Some(index) = levels
                .iter()
                .enumerate()
                .min_by_key(|(_, level)| **level)
                .map(|(index, _)| index)
            {
                levels.remove(index);
            }
            if levels.is_empty() {
                self.pending_requests.remove(&entity);
            }
        }
    }

    /// `DRET`: every requested function of `entity` returned at once.
    pub fn clear_requests(&mut self, entity: EntityId) {
        self.pending_requests.remove(&entity);
    }

    /// `EventEngine.requestAcceptable`: a request at `level` runs when it is
    /// below the object's current script level. An object with no requested
    /// function running accepts any level.
    pub fn request_acceptable(&self, entity: EntityId, level: i32) -> bool {
        self.pending_requests
            .get(&entity)
            .and_then(|levels| levels.iter().min())
            .is_none_or(|current| level < *current)
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
        self.pending_requests.contains_key(&entity)
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

    /// The controlled actor moved `distance` under the player's input
    /// (`ProcessEvents.cs:252`): it feeds the encounter timer while the rate
    /// is above 0. A long move counts as walking it: one check per
    /// `EncounterInterval`, as the per-frame steps would give.
    pub fn player_walked(&mut self, mut distance: f32) {
        if self.random_battles.ratio == 0 || !distance.is_finite() {
            return;
        }
        while distance > 0.0 && self.encounters.is_empty() {
            let step = distance.min(ENCOUNTER_INTERVAL);
            self.random_battles.timer += step;
            distance -= step;
            self.process_encount();
        }
    }

    /// `EventEngine.ProcessEncount` (`ProcessEvents.cs:460`): every
    /// `EncounterInterval` of walking adds the rate to the danger value, and
    /// a `random8` under danger / 8 starts one of the field's four battles.
    fn process_encount(&mut self) {
        if !self.user_control || self.random_battles.timer <= ENCOUNTER_INTERVAL {
            return;
        }
        self.random_battles.timer = 0.0;
        self.random_battles.base += i32::from(self.random_battles.ratio);
        if i32::from(self.next_random_u8()) >= self.random_battles.base >> 3 {
            return;
        }
        self.random_battles.base = 0;
        let mut scene = self.select_scene();
        if scene == 0 {
            return;
        }
        if scene == self.random_battles.last_scene {
            scene = self.select_scene();
        }
        self.random_battles.last_scene = scene;
        self.encounters.push(EncounterRequest {
            rush: 0,
            group: None,
            battle: i32::from(scene),
            steiner_state: false,
        });
    }

    /// `EventEngine.SelectScene` (`EventEngine.cs:191`), field mode.
    fn select_scene(&mut self) -> u16 {
        const CHANCES: [[u8; 3]; 4] = [
            [96, 168, 224],
            [64, 128, 192],
            [90, 167, 244],
            [115, 217, 244],
        ];
        let roll = self.next_random_u8();
        let row = CHANCES[usize::from(self.random_battles.pattern & 3)];
        let slot = row.iter().position(|&limit| roll < limit).unwrap_or(3);
        self.random_battles.scenes[slot]
    }

    /// Seeds the world RNG (deterministic xorshift over `GameServices`
    /// random draws). Battle/sweep runners set this from their run seed so
    /// every random stream is knob-controlled; the default keeps the
    /// historical fixed stream.
    pub fn seed_rng(&mut self, seed: u64) {
        self.rng = seed;
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

    /// `GetUIDFromLocalUID` then `FindObjByUID`. A party slot names the
    /// object of its `partyUID` entry; an empty slot keeps the plain uid.
    pub(crate) fn resolve_uid(&self, uid: i32) -> Option<EntityId> {
        self.party_slot_object(uid)
            .or_else(|| self.uid_map.get(&uid).copied())
    }

    /// The object behind party-slot uid `uid` (251-254), if the slot is
    /// filled.
    pub(crate) fn party_slot_object(&self, uid: i32) -> Option<EntityId> {
        let slot = u8::try_from(uid).ok()?;
        let entry = self.party_slot(slot)??;
        self.uid_map.get(&i32::try_from(entry).ok()?).copied()
    }

    /// `Some(partyUID[slot])` when `uid` is a party slot (251-254) and a
    /// script table is loaded; `None` for any other uid.
    fn party_slot(&self, uid: u8) -> Option<Option<usize>> {
        if self.entry_links.is_empty() || !(PARTY_SLOT_FIRST..=PARTY_SLOT_LAST).contains(&uid) {
            return None;
        }
        Some(self.party_uid[usize::from(uid - PARTY_SLOT_FIRST)])
    }

    /// The entry `InitObject` (NEW3) creates for `source`: party slots
    /// resolve through `partyUID`, and an empty slot creates nothing (the
    /// reference logs a warning and skips the instruction).
    pub fn object_source(&self, source: u8) -> Option<u8> {
        match self.party_slot(source) {
            Some(entry) => entry.and_then(|entry| u8::try_from(entry).ok()),
            None => Some(source),
        }
    }

    /// Loads the field script's entry table and builds `partyUID` from it.
    pub fn set_entry_links(&mut self, links: Vec<EntryLink>) {
        self.entry_links = links;
        self.setup_party_uid();
    }

    /// `SetupPartyUID`: field start, party changes, and `SYNCPARTY`.
    pub fn setup_party_uid(&mut self) {
        let party: Vec<u8> = self
            .party
            .iter()
            .filter_map(|&character| u8::try_from(character).ok())
            .collect();
        self.party_uid = party_uid(&self.entry_links, &party);
    }

    /// `partyUID` slots (local uids 251-254).
    pub fn party_uids(&self) -> [Option<usize>; 4] {
        self.party_uid
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

    /// `BattleResultUI` card pay-out: the card is kept only while the
    /// collection holds fewer than [`MAX_CARDS`]
    /// (`Configuration.TetraMaster.MaxCardCount`, 100). Returns whether it
    /// was added.
    pub fn add_card(&mut self, card: u32) -> bool {
        if self.cards.len() >= MAX_CARDS {
            return false;
        }
        self.cards.push(card);
        true
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

    /// Imports the field's tile animations (`BGANIM_DEF` list) and its
    /// camera count; the field starts on camera 0.
    pub fn provide_tile_animations(
        &mut self,
        animations: Vec<super::tile_animation::TileAnimation>,
        camera_count: u8,
    ) {
        self.tile_animations = animations;
        self.tile_camera_count = camera_count;
        self.field_camera = 0;
    }

    pub fn clear_tile_animations(&mut self) {
        self.tile_animations.clear();
        self.tile_camera_count = 0;
        self.field_camera = 0;
    }

    /// `BgAnimationService`, once per field tick.
    pub fn service_tile_animations(&mut self) {
        if !self.tile_animations.is_empty() {
            super::tile_animation::service(
                &mut self.tile_animations,
                self.field_camera,
                &mut self.overlays,
            );
        }
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

#[cfg(test)]
mod random_battle_tests {
    use super::*;

    fn walking_world(ratio: u8) -> FieldWorld {
        let mut world = FieldWorld::new();
        world.user_control = true;
        world.set_player(EntityId::new(1));
        world.set_walk_bits(Some((1, 2, 4, 8)));
        world.set_input(1);
        world.random_battles.pattern = 1;
        world.random_battles.scenes = [301, 302, 303, 304];
        world.random_battles.ratio = ratio;
        world
    }

    #[test]
    fn walking_with_a_rate_starts_a_field_battle() {
        let mut world = walking_world(255);
        // The first 1440 units are safe (24 run ticks of 60); then a check
        // every 960.
        for _ in 0..24 {
            world.advance();
        }
        assert!(world.encounters.is_empty());
        for _ in 0..2000 {
            world.advance();
            if !world.encounters.is_empty() {
                break;
            }
        }
        let request = world.encounters.first().expect("a random battle");
        assert!((301..=304).contains(&request.battle));
        assert_eq!(request.group, None);
        assert_eq!(world.random_battles.base, 0);
    }

    #[test]
    fn rate_zero_or_no_control_never_fights() {
        let mut world = walking_world(0);
        for _ in 0..5000 {
            world.advance();
        }
        assert!(world.encounters.is_empty());
        let mut world = walking_world(255);
        world.user_control = false;
        for _ in 0..5000 {
            world.advance();
        }
        assert!(world.encounters.is_empty());
    }

    #[test]
    fn a_new_field_resets_the_rate_and_danger() {
        let mut world = walking_world(200);
        world.random_battles.base = 90;
        world.random_battles.timer = 500.0;
        world.begin_field();
        assert_eq!(world.random_battles.ratio, 0);
        assert_eq!(world.random_battles.base, 0);
        assert_eq!(world.random_battles.timer, -1440.0);
        assert_eq!(world.random_battles.scenes, [301, 302, 303, 304]);
    }
}

#[cfg(test)]
mod party_slot_tests {
    use super::*;
    use tantalus_legacy::event::party::vanilla_links;

    /// 20 entries mapped uid = entry, party Vivi and Zidane.
    fn field_world() -> FieldWorld {
        let mut world = FieldWorld::new();
        for entry in 0..20 {
            world.map_uid(entry, EntityId::new(entry as u64));
        }
        for slot in 251..256 {
            world.map_uid(slot, EntityId::new(slot as u64));
        }
        world.party = vec![1, 0];
        world.set_entry_links(vanilla_links(std::iter::repeat_n(true, 20)));
        world
    }

    #[test]
    fn party_slots_name_party_entries_in_priority_order() {
        let world = field_world();
        // Zidane's entry is 11, Vivi's 12; Zidane takes slot 0.
        assert_eq!(world.party_uids(), [Some(11), Some(12), None, None]);
        assert_eq!(world.resolve_uid(251), Some(EntityId::new(11)));
        assert_eq!(world.resolve_uid(252), Some(EntityId::new(12)));
        assert_eq!(world.object_source(251), Some(11));
        // An empty slot creates nothing and keeps its placeholder uid.
        assert_eq!(world.object_source(253), None);
        assert_eq!(world.resolve_uid(253), Some(EntityId::new(253)));
        // Other uids are untouched.
        assert_eq!(world.object_source(5), Some(5));
        assert_eq!(world.resolve_uid(5), Some(EntityId::new(5)));
    }

    #[test]
    fn party_changes_rebuild_the_slots_and_no_table_keeps_plain_uids() {
        let mut world = field_world();
        world.party = vec![3];
        world.setup_party_uid();
        assert_eq!(world.resolve_uid(251), Some(EntityId::new(14)));
        let mut plain = FieldWorld::new();
        plain.party = vec![0];
        plain.map_uid(251, EntityId::new(251));
        plain.setup_party_uid();
        assert_eq!(plain.object_source(251), Some(251));
        assert_eq!(plain.resolve_uid(251), Some(EntityId::new(251)));
    }
    #[test]
    fn seeded_world_streams_differ() {
        use crate::battle::GameServices;
        let mix = 0xC0FF_EE11_DEAD_BEEFu64;
        let mut a = FieldWorld::new();
        a.seed_rng(0x5EED ^ mix);
        let mut b = FieldWorld::new();
        b.seed_rng(48879 ^ mix);
        let draws_a: Vec<u32> = {
            let mut s = crate::battle::WorldGameServices::new(&mut a);
            (0..4).map(|_| s.random_below(65_536)).collect()
        };
        let draws_b: Vec<u32> = {
            let mut s = crate::battle::WorldGameServices::new(&mut b);
            (0..4).map(|_| s.random_below(65_536)).collect()
        };
        assert_ne!(draws_a, draws_b, "seeded world streams must differ");
    }
    #[test]
    fn wide_draws_compose_full_bytes() {
        use crate::battle::GameServices;
        // The widening fix: random_below(65_536) must return a true 16-bit
        // value. The pre-fix bug returned one u8 % 65_536, which can never
        // exceed 255 -- so a draw above 255 proves wide draws compose
        // whole bytes.
        let mut saw_wide = false;
        for seed in 0..64u64 {
            let mut world = FieldWorld::new();
            world.seed_rng(seed ^ 0xC0FF_EE11_DEAD_BEEF);
            let mut services = crate::battle::WorldGameServices::new(&mut world);
            if services.random_below(65_536) > 255 {
                saw_wide = true;
                break;
            }
        }
        assert!(saw_wide, "no 16-bit draw exceeded 255 in 64 seeds");
    }
}

/// `FieldMapActorController.speed`: player units per walk step.
pub const PLAYER_WALK_SPEED: f32 = 30.0;

fn is_zero(value: &u8) -> bool {
    *value == 0
}
