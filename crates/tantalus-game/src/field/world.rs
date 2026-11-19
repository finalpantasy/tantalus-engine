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

/// Facing (0 south, 64 west, 128 north, 192 east, unrounded) of a move by
/// `(dx, dz)`: `EventEngine.angleAsm` / `FieldMapActorController` use
/// `atan2(-dx, -dz)` degrees, so moving toward -z is facing 0.
pub(crate) fn facing_toward_exact(dx: f32, dz: f32) -> f32 {
    ((-dx).atan2(-dz).to_degrees() * 256.0 / 360.0).rem_euclid(256.0)
}

/// `facing_toward_exact` rounded onto the integer facing scale.
pub(crate) fn facing_toward(dx: f32, dz: f32) -> i16 {
    facing_toward_exact(dx, dz).round().rem_euclid(256.0) as i16
}

fn all_choices() -> i32 {
    -1
}

/// `EventEngine.cEventLevelN - 1`: the level of an object's idle loop.
pub const IDLE_SCRIPT_LEVEL: i32 = 7;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Movement {
    pub destination: [f32; 2],
    /// `MoveToward_mixed` flag 1 (`MOVH`, `MOVE3H`): the actor slides without turning to
    /// face the way it moves.
    #[serde(default)]
    pub keep_facing: bool,
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
    /// `PosObj.collRad` (`SetActorRadii` argument 2, 16 by default): the push range
    /// `WalkMesh.Collision` mode 2 measures.
    #[serde(default = "default_collision_radius")]
    pub collision_radius: u8,
    /// Facing on the `getvobj` member-3 scale, 0..=255 (0 south, 64 west,
    /// 128 north, 192 east).
    pub facing: i16,
    pub flags: i32,
    /// `FieldMapActorController.charFlags & 1`: the actor is held on the
    /// walkmesh. `POS`/`DPOS` set it, `SetPathing(0)` and `POS3`/`DPOS3`
    /// clear it. Runtime-only.
    #[serde(skip, default = "path_active_default")]
    pub path_active: bool,
    /// The position was last set while pathing was off (`POS3`, `DPOS3`, a
    /// jump), so it was never checked against the mesh; the reference keeps
    /// it even if `SetPathing(1)` follows (`ResetPos` returns to the same
    /// `lastPos`). Runtime-only.
    #[serde(skip)]
    pub placed_unchecked: bool,
    pub level: i32,
    pub jump: Option<Jump>,
    /// `AFRAME` (`inFrame`/`outFrame`): the frames the next `ANIM` plays
    /// from and to; 0 and 255 play the whole animation.
    #[serde(default)]
    pub in_frame: u8,
    #[serde(default = "full_out_frame")]
    pub out_frame: u8,
    /// `AMODE` (`afHold`): the next animation freezes on its last frame when it ends.
    #[serde(default)]
    pub anim_hold: bool,
    /// `AMODE` (`afLoop`, `afPalindrome`): the animation restarts at `inFrame` when it ends.
    #[serde(default)]
    pub anim_loop: bool,
    /// `AMODE` repeat count (`loopCount`); 0 loops forever.
    #[serde(default)]
    pub anim_loops: u8,
    /// `Actor.loopCount` as the walk reads it (`MoveToward_mixed_ex`): a walk ends once
    /// a frame leaves it at 0, and 255 walks until arrival. `CLRDIST` sets 255, `MOVT`
    /// sets its count, and each walk frame takes one off any other value. The reference
    /// shares the byte with `AMODE` and the animation loop, and starts an actor at 0;
    /// here only `MOVT` and `CLRDIST` write it and it starts at 255, which keeps every
    /// other walk synchronous as before.
    #[serde(default = "full_out_frame")]
    pub walk_loops: u8,
    /// A held animation has ended: nothing advances until the next `ANIM` (`afFreeze`).
    #[serde(default)]
    pub anim_frozen: bool,
    /// `actJump`: set by `SetupJump`, cleared when the landing animation stops
    /// (`FinishJump` also resets the animation range to the whole clip).
    #[serde(default)]
    pub act_jump: bool,
    /// Length of the animation being played, for a loop to restart.
    #[serde(default)]
    pub anim_span: u16,
    /// What the renderer draws; the scripts never read it back.
    #[serde(default)]
    pub look: ActorLook,
    /// `SetMovementSpeed` (`MSPEED`, `Actor.speed`): field units per frame, 30 by default.
    #[serde(default = "default_move_speed")]
    pub move_speed: u8,
    /// `SetTurnSpeed` (`TSPEED`, `Actor.tspeed`): the speed of scripted turns, 16 by default.
    #[serde(default = "default_turn_speed")]
    pub turn_speed: u8,
}

/// One background layer script (`EBG_overlay*` setter) the renderer replays
/// on its own `FieldAnimation` to scroll layers each tick. Values are the
/// opcode operands, unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayMotion {
    /// `BGLLOOP` (`EBG_overlaySetLoop`).
    Loop {
        block: i32,
        enabled: bool,
        speed: [i16; 2],
    },
    /// `BGLPARALLAX` (`EBG_overlaySetParallax`).
    Parallax {
        block: i32,
        enabled: bool,
        speed: [i16; 2],
    },
    /// `BGLORIGIN` (`EBG_overlaySetOrigin`).
    Origin { block: i32, x: i16, y: i16 },
    /// `BGLMOVE` (`EBG_overlayMove`).
    Move { block: i32, offset: [i16; 3] },
    /// `BGLSCROLLOFFSET` (`EBG_overlaySetScrollWithOffset`).
    ScrollWithOffset {
        block: i32,
        enabled: bool,
        speed: i16,
        offset: i16,
        is_x_offset: bool,
    },
    /// `BGLLOOPTYPE` (`EBG_overlaySetLoopType`).
    LoopType { block: i32, screen_anchored: bool },
    /// `BGVDEFINE` (`EBG_overlayDefineViewport`).
    Viewport {
        viewport: i32,
        position: [i16; 2],
        size: [i16; 2],
    },
}

/// What the scripts did to one actor's color and shadow
/// (`fldmcf.SetCharColor`, `ff9shadow.FF9Shadow*Field`, `CHRSET`).
/// `None` keeps what the field's map config gives.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ActorShadowScript {
    /// `CHRCOLOR`: the model color; it also stops the map config light from
    /// coloring the actor (`FF9Char` attribute 0x400).
    pub color: Option<[u8; 3]>,
    /// `FF9Char.attr`: 4 uses a ladder, 16 hides the shadow
    /// (`SHADOWOFF`/`CHRSET`), 32 locks its rotation.
    pub attr: u32,
    /// `SHADOWSCALE`: the x and z scale, in 1/16 of the plate.
    pub scale: Option<[i32; 2]>,
    /// `SHADOWOFFSET`: x and z offset from the actor.
    pub offset: Option<[i32; 2]>,
    /// `SHADOWAMP`: the shadow strength factor (a byte).
    pub amp: Option<i32>,
    /// `SHADOWLOCK`: the locked Y rotation, before the `<< 4`.
    pub rotation: Option<i32>,
}

/// A texture animation command for one model (`GeoTexAnim.geoTexAnimPlay`, `geoTexAnimPlayOnce`,
/// `geoTexAnimStop`), with the field tick of `FieldWorld::tex_anim_clock` it ran on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TexAnimCommand {
    pub tick: u64,
    pub op: TexAnimOp,
    pub index: i32,
}

/// What a [`TexAnimCommand`] does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TexAnimOp {
    Play,
    PlayOnce,
    Stop,
}

/// `FF9Char.attr` bits the field scripts set.
pub const CHAR_ATTR_LADDER: u32 = 4;
pub const CHAR_ATTR_SHADOW_OFF: u32 = 16;
pub const CHAR_ATTR_SHADOW_LOCK: u32 = 32;

/// What field scripts set on a character beyond stats (`SetCharacterData`, `SetName`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CharacterInfo {
    /// `PLAYERNAME`: the field text id of the new name.
    pub name_text: Option<i32>,
    /// `JOIN`: the level/equipment/category/menu preset the script initialized the member with.
    pub initialized: Option<CharacterInit>,
    /// `CLEARSTATUS`: statuses cured so far (bit mask, battle status ids 0..31).
    pub cured_statuses: u32,
}

/// `JOIN` (`SetCharacterData`) arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterInit {
    pub enable_leveling: bool,
    pub equipment_set: i32,
    /// `None` when the script passed 255 (keep the current category).
    pub category: Option<u8>,
    /// `None` when the script passed 255 (keep the menu preset).
    pub menu_preset: Option<u8>,
}

/// What field scripts set on an actor that only the renderer shows (runtime-only).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActorPresentation {
    /// `SetObjectScale` (`SCALE`): x, z, y scale (4096 = 1.0 in the reference's fixed point).
    pub scale: [i32; 3],
    /// `SetObjectScale` fourth byte: the vertical scale factor.
    pub vertical_scale: u8,
    /// `SetHeadFocusMasks` (`SETHFOCUS`): which objects this one's head follows, and which follow it.
    pub head_focus_self: u8,
    pub head_focus_target: u8,
    /// `SetHeadFocusFlags`: head tracking on/off bits (0 = off).
    pub head_focus_flags: u8,
    /// `SetWalkTurnSpeed` (`MROT`, `Actor.omega`): how fast the actor turns while it walks.
    pub walk_turn_speed: u8,
    /// `SetLeftTurnAnimation` / `SetRightTurnAnimation` (`ATURNL`, `ATURNR`).
    pub turn_left_animation: Option<u16>,
    pub turn_right_animation: Option<u16>,
    /// `SetJumpAnimation` (`AJUMP`): the animation and its two parameter bytes.
    pub jump_animation: Option<(u16, [u8; 2])>,
    /// `SetInactiveAnimation` (`ASLEEP`).
    pub inactive_animation: Option<u16>,
    /// `SetAnimationSpeed` (`ASPEED`, `Actor.aspeed0`); 16 by default.
    pub animation_speed: u8,
    /// `SetAnimationLoopCount` (`ALOOP`).
    pub animation_loop_count: u8,
    /// `SetActorModel` eye height (`PosObj.eye`, already scaled by -4).
    pub eye: i16,
}

impl Default for ActorPresentation {
    fn default() -> Self {
        Self {
            scale: [4096; 3],
            vertical_scale: 0,
            head_focus_self: 0,
            head_focus_target: 0,
            head_focus_flags: 0,
            walk_turn_speed: 16,
            turn_left_animation: None,
            turn_right_animation: None,
            jump_animation: None,
            inactive_animation: None,
            animation_speed: 16,
            animation_loop_count: 0,
            eye: 0,
        }
    }
}

/// `ApplyFadeFilter` (`FADEFILTER`): the screen filter a script asked for (runtime-only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FadeFilter {
    pub mode: i32,
    /// Ticks the fade takes; 0 applies at once.
    pub frames: i32,
    /// Target color offset per channel.
    pub color: [i32; 3],
    /// Ticks since the fade started.
    pub elapsed: i32,
    /// The offset this mode had when the fade started (`SceneDirector._prevColor`).
    pub from: [i32; 3],
}

impl FadeFilter {
    /// The offset per channel now: `ServiceFade` lerps `_prevColor` to the target color over
    /// the fade's frames (`Color32.Lerp`).
    pub fn color_now(&self) -> [i32; 3] {
        if self.frames <= 0 || self.elapsed >= self.frames {
            return self.color;
        }
        let t = self.elapsed.max(0) as f32 / self.frames as f32;
        std::array::from_fn(|i| {
            (self.from[i] as f32 + (self.color[i] - self.from[i]) as f32 * t).round() as i32
        })
    }

    /// Index of this filter's blend: 0 adds the color to the screen, 1 subtracts it
    /// (`FadeMode.Add` / `FadeMode.Sub`).
    pub fn slot(&self) -> usize {
        usize::from(self.mode & 2 != 0)
    }

    /// How far the fade has gone, 0 to 256.
    pub fn progress(&self) -> i32 {
        if self.frames <= 0 {
            256
        } else {
            (self.elapsed * 256 / self.frames).clamp(0, 256)
        }
    }
}

/// One `fldmcf.SetMCF*Light` call on the field's map config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightChange {
    /// `MCF_DEFAULT`: kind 1 default, 2 ladder; -1 is every model.
    Default {
        kind: i8,
        color: [u8; 3],
        shadow_intensity: i8,
        shadow_scale: i8,
    },
    /// `MCF_FLOOR`.
    Floor {
        floor: i8,
        color: [u8; 3],
        shadow_intensity: i8,
        shadow_scale: i8,
    },
    /// `MCF_MODEL`: -1 is every model.
    Model {
        model: i32,
        color: [u8; 3],
        shadow_intensity: i8,
        shadow_scale: i8,
    },
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
    /// `BGCLOCK`/`BGCUNLOCK` (`EBG_charLookAtLock`/`Unlock`): unlocked, the camera's view range
    /// is no longer clamped to the camera bounds.
    FollowLock(bool),
}

/// `AttachObject` (`geoAttach`): `object` rides `carrier`'s bone `bone`, offset by `AttachObjectOffset`
/// (`geoAttachOffset`). The scripts never read it back, so it is runtime-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Attachment {
    pub carrier: Option<EntityId>,
    pub bone: i32,
    pub offset: [i32; 3],
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
    /// The left and right turn animations (`SetLeftTurnAnimation`, `SetRightTurnAnimation`).
    pub turn_left: Option<u16>,
    pub turn_right: Option<u16>,
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

fn path_active_default() -> bool {
    true
}

fn full_out_frame() -> u8 {
    u8::MAX
}

/// `Actor` constructor: `speed = 30`.
fn default_move_speed() -> u8 {
    30
}

fn default_collision_radius() -> u8 {
    16
}

/// `Actor` constructor: `tspeed` turns at 16 (`StartTurn` treats 0 as 16).
fn default_turn_speed() -> u8 {
    16
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
            collision_radius: 16,
            facing: 0,
            flags: 0,
            path_active: true,
            placed_unchecked: false,
            level: 0,
            jump: None,
            in_frame: 0,
            out_frame: u8::MAX,
            anim_hold: false,
            anim_loop: false,
            anim_loops: 0,
            walk_loops: u8::MAX,
            anim_frozen: false,
            act_jump: false,
            anim_span: 0,
            look: ActorLook::default(),
            move_speed: default_move_speed(),
            turn_speed: default_turn_speed(),
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
    /// The field clock when it is shown (`TimerUI.Enable`), which the battle scripts read and
    /// which keeps running through the fight.
    #[serde(default)]
    pub timer: Option<TimerState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TimerState {
    /// The value `SetTime` stored (seconds).
    pub seconds: i32,
    /// `TimerUI.Play`. Stays true after the clock reaches 0: `TimerUI.Update` only clamps
    /// `time` at 0 and never clears `Play`; only `SetPlay(false)` (the script's
    /// `SetTimerRunning`) or `SetEnable` does.
    pub running: bool,
    pub displayed: bool,
    /// Field ticks counted down since `SetTime` while `running`.
    #[serde(default)]
    pub elapsed_ticks: u32,
}

impl TimerState {
    /// Field logic ticks per second (`FPS_FIELD`), the rate the countdown
    /// runs at.
    pub const TICKS_PER_SECOND: f32 = 30.0;

    /// `TimerUI.SetTime`: the time is `second - 0.01`, never below zero.
    pub fn set_time(&mut self, seconds: i32) {
        self.seconds = seconds;
        self.elapsed_ticks = 0;
    }

    /// `TimerUI.Time`: what is left, floored at zero (`TimerUI.Update`).
    pub fn time(&self) -> f32 {
        let start = (self.seconds as f32 - 0.01).max(0.0);
        (start - self.elapsed_ticks as f32 / Self::TICKS_PER_SECOND).max(0.0)
    }

    /// `GetSysvar` 17 (`GetTimerTime`): `Convert.ToInt32(TimerUI.Time)`,
    /// which rounds half to even.
    pub fn time_int(&self) -> i32 {
        self.time().round_ties_even() as i32
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CharacterStats {
    pub current_hp: i32,
    pub max_hp: i32,
    pub current_mp: i32,
    pub max_mp: i32,
}

pub(crate) const MAX_GIL: u32 = 9_999_999;

/// `ff9play.FF9PLAY_HP_MAX` and `FF9PLAY_MP_MAX`: the default `maxHpLimit`
/// and `maxMpLimit` (no vanilla support ability changes them).
const PLAYER_HP_LIMIT: i32 = 9999;
const PLAYER_MP_LIMIT: i32 = 999;

/// The HP and MP half of `ff9play.FF9Play_Update` (`ff9play.cs:263-334`):
/// from the level's `basis.max_hp` / `max_mp`, each enabled support ability
/// in `enabled` order (`player.saExtended`, `GetEnabledSA`) runs its
/// `Permanent` formula (`CharacterAbilityGems.TriggerOnEnable`,
/// `AbilityFeatures.txt`): HP+10% (SA 5) `MaxHP + MaxHP / 10`, HP+20% (SA 6)
/// `MaxHP + MaxHP / 5`, MP+10% (SA 7) and MP+20% (SA 8) the same on MP. The
/// two sizes have no boosted hierarchy in `AbilityGems.csv`, so both
/// stack. The results are capped at `maxHpLimit` 9999 and `maxMpLimit` 999.
pub fn support_max_hp_mp(basis_hp: i32, basis_mp: i32, enabled: &[i32]) -> (i32, i32) {
    let (mut hp, mut mp) = (basis_hp, basis_mp);
    for &sa in enabled {
        match sa {
            5 => hp += hp / 10,
            6 => hp += hp / 5,
            7 => mp += mp / 10,
            8 => mp += mp / 5,
            _ => {}
        }
    }
    (hp.min(PLAYER_HP_LIMIT), mp.min(PLAYER_MP_LIMIT))
}

fn lose_result() -> i32 {
    1
}

/// Tetra Master collection size (`Configuration.TetraMaster.MaxCardCount`).
pub const MAX_CARDS: usize = 100;

/// Runtime movie frame-count lookup for the FMV service, keyed by
/// `(disc, number)`. Behavior, not state: snapshots do not carry it.
pub type FmvFrameCounts = std::sync::Arc<dyn Fn(i32, i32) -> Option<u32> + Send + Sync>;

/// Runtime animation length lookup: the frame count of an animation id
/// (`EventEngineUtils.GetCharAnimFrame`). Behavior, not state.
pub type AnimationFrameCounts = std::sync::Arc<dyn Fn(u16) -> Option<u16> + Send + Sync>;

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

fn is_zero_i32(value: &i32) -> bool {
    *value == 0
}

#[derive(Clone, Serialize, Deserialize)]
pub struct FieldWorld {
    pub actors: BTreeMap<EntityId, ActorState>,
    pub windows: BTreeMap<u8, bool>,
    pub items: BTreeMap<i32, i32>,
    pub gil: u32,
    pub party: Vec<i32>,
    pub character_stats: BTreeMap<i32, CharacterStats>,
    /// Script-set character data by character id (names, init arguments, cured statuses).
    #[serde(default)]
    pub character_info: BTreeMap<i32, CharacterInfo>,
    /// `FULLMEMBER` (`SetPartyReserve`): the characters a later party menu may offer.
    #[serde(default)]
    pub party_reserve: Option<i32>,
    /// `GAMEOVER`: the script ended the game.
    #[serde(default)]
    pub game_over: bool,
    /// `DISCCHANGE`: the destination field and disc the script asked to change to.
    #[serde(default)]
    pub disc_change: Option<(u16, u8)>,
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
    /// `ETb.sChooseMask`: the options `CHOOSEPARAM` (`EnableDialogChoices`)
    /// left available, bit N for option N; -1 is all. Only a text with a
    /// `[PCHM]` tag applies it (`DialogBoxSymbols.SetupChoose`). Cleared
    /// when a field starts (`ETb.InitMesWin`). Runtime-only.
    #[serde(skip, default = "all_choices")]
    pub choice_mask: i32,
    /// Text id each window last showed.
    pub window_texts: BTreeMap<u8, u16>,
    /// Script flags and speaking actor of each window (`MES` flags, the
    /// executing object or `MESN` target). Runtime-only, for drawing; not
    /// part of snapshots or equality.
    #[serde(skip)]
    pub window_speakers: BTreeMap<u8, (u8, Option<EntityId>)>,
    /// `ETb.gMesValue`: values scripts set for `[NUMB]` and `[ITEM]` tags.
    /// Runtime-only, for drawing.
    #[serde(skip)]
    pub mes_values: BTreeMap<i32, i32>,
    /// Sound requests scripts made (`RunSoundCode*`, `SetSoundPosition`,
    /// `SetSoundObjectPosition`, `SetBattleMusicContinuation`), in order,
    /// until a client drains them with [`FieldWorld::take_sound_events`].
    /// Runtime-only: nothing in the game reads them back, so they are not
    /// part of snapshots or equality.
    #[serde(skip)]
    pub sound_events: Vec<tantalus_legacy::HostEvent>,
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
    /// Running requested functions (function key) and the object each was requested on, so
    /// that a function's return restores its object's level within the frame (runtime-only).
    #[serde(skip)]
    request_functions: BTreeMap<EntityId, EntityId>,
    /// Entries `InitObject` created an object for this field (runtime-only).
    #[serde(skip)]
    object_inits: BTreeSet<u8>,
    /// Entry objects the host registered before any `InitObject` created them (the tools'
    /// one-actor-per-entry stubs). The reference has no `Obj` for them, so `EventCollision`
    /// cannot talk to them (Madain Sari 1601 at scenario 6610: entry 9's actor stub at the
    /// origin took the player's talk before Mocha's `InitObject`). Runtime-only.
    #[serde(skip)]
    uncreated: BTreeSet<EntityId>,
    /// Extra objects of an entry (`InitObject` again with another uid): the
    /// reference makes one `Obj` per call, all running the same code.
    /// Runtime-only: object entity to its entry.
    #[serde(skip)]
    instance_sources: BTreeMap<EntityId, u8>,
    /// Instances created since the director last took them.
    #[serde(skip)]
    new_instances: Vec<(EntityId, u8)>,
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
    /// Walk speed in world units per frame at the default movement speed of 30
    /// (`MSPEED` sets `actor.speed` in units per frame).
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
    /// The field scripts' walkmesh switches since the field began, in order
    /// (`BGI_floorSetActive`, `BGI_triSetActive`); runtime-only, a new field's
    /// walkmesh starts with every triangle active.
    #[serde(skip)]
    walk_switches: Vec<WalkSwitch>,
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
    /// Real animation lengths (runtime-only). Without it an animation lasts
    /// `animation_default_ticks`.
    #[serde(skip)]
    pub animation_frame_counts: Option<AnimationFrameCounts>,
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
    /// Stats of each card in `cards` (same order), made when the collection first
    /// meets a card game (`QuadMistCard`: attack, defense, arrows, type).
    #[serde(default)]
    pub card_records: Vec<crate::cards::CardRecord>,
    /// Tetra Master record (`QuadMistDatabase` win, lose and draw counts).
    #[serde(default)]
    pub card_stats: crate::cards::CardCounts,
    /// The last card the player lost (`StackCardInfo`), handed back to the next opponent.
    #[serde(default)]
    pub card_stack: Option<crate::cards::CardRecord>,
    /// `GetTetraMasterResult`: 0 win, 1 lose, 2 draw.
    #[serde(default = "lose_result")]
    pub last_card_result: i32,
    /// An opponent a script asked to play (`MINIGAME`); the play loop starts the game.
    #[serde(default)]
    pub pending_card_game: Option<u16>,
    /// Random stream of the card game.
    #[serde(default)]
    pub card_seed: u64,
    /// Config screen values (`FF9CFG` and the ini options), saved with the game.
    #[serde(default)]
    pub settings: super::settings::GameSettings,
    /// `PLAYER.trance` by character id: the Trance gauge carried between
    /// battles (`btl_init.cs:379`, `btl_sys.SavePlayerData`).
    #[serde(default)]
    pub trance: BTreeMap<i32, u8>,
    /// Summons (`BattleAbilityId`) cast at least once, in order: the
    /// `BattleAchievement` summon flags `btl_cmd.DecideSummonType` reads.
    #[serde(default)]
    pub summons_seen: Vec<i32>,
    /// Party members moved to the back row by the Order screen (character
    /// ids; everyone else stands in the front row).
    #[serde(default)]
    pub back_row: std::collections::BTreeSet<i32>,
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
    /// The view position scripts read back with `CalculateScreenOrigin` (runtime-only: set up
    /// per field from its cameras, see [`super::view`]).
    #[serde(skip)]
    pub view: super::view::FieldView,
    /// `ff9.btlMapNo` and `ff9.btl_result` of the last battle (`GetSysvar` 24
    /// `GetLastBattleId` and 27 `GetBattleResult`); the Festival of the Hunt fields read the
    /// result when the party comes back (runtime-only).
    /// The party screen a `PARTYMENU` opened (runtime-only, see [`super::party_menu`]).
    #[serde(skip)]
    pub party_menu: Option<tantalus_legacy::party_menu::party::PartyInfo>,
    #[serde(skip)]
    pub last_battle: i32,
    #[serde(skip)]
    pub battle_result: u8,
    /// A moogle or save point script ran `MENU(4, 0)` in this field
    /// (runtime-only): the main menu may save.
    #[serde(skip)]
    pub save_point: bool,
    /// The save screen was asked for and is not open yet (runtime-only).
    #[serde(skip)]
    pub save_requested: bool,
    /// A script ran `MENU(5)`: the Chocograph list was asked for and is not
    /// open yet (runtime-only).
    #[serde(skip)]
    pub chocograph_requested: bool,
    /// The world script's pad edges (`ETb.sKeyOn`, `ETb.sKeyOff`) and held
    /// bits (`ETb.sKey`), set by the walking world each frame; `None` in the
    /// field, where `input_mask` answers all three (runtime-only).
    #[serde(skip)]
    pub world_keys: Option<[i32; 3]>,
    /// `FF9StateGlobal.hintmap_id`: the Chocograph picture the player picked
    /// (0 none, else index + 1). Saved with the world
    /// (`ff9.SaveWorld`/`LoadWorld`, the `70000_World` block).
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub hintmap_id: i32,
    /// A script ran `MENU(2, shop)`: the shop to open, not open yet
    /// (runtime-only).
    #[serde(skip)]
    pub shop_requested: Option<i32>,
    /// `RunWorldCode` calls (`ff9.w_frameSetParameter(code, value)`) the
    /// world script made since the last [`FieldWorld::take_world_codes`]
    /// (runtime-only).
    #[serde(skip)]
    pub world_codes: Vec<(i32, i32)>,
    /// Enabled support abilities per character, in the order they were
    /// switched on (reference `player.saExtended`).
    #[serde(default)]
    pub support_enabled: BTreeMap<i32, Vec<i32>>,
    /// `MENUOFF` was run: the main menu cannot open (runtime-only; the
    /// script turns it back on with `MENUON`).
    #[serde(skip)]
    pub menu_disabled: bool,
    /// The layer scroll scripts this field ran, in order (runtime-only).
    #[serde(skip)]
    pub overlay_motion: Vec<OverlayMotion>,
    /// Per-actor color and shadow scripts (runtime-only).
    #[serde(skip)]
    pub actor_shadows: BTreeMap<EntityId, ActorShadowScript>,
    /// The map config light changes this field made, in order (runtime-only).
    #[serde(skip)]
    pub light_changes: Vec<LightChange>,
    /// Texture animation commands per actor, in order (runtime-only).
    #[serde(skip)]
    pub tex_anims: BTreeMap<EntityId, Vec<TexAnimCommand>>,
    /// Field ticks run since the field started, the clock of [`TexAnimCommand::tick`]
    /// (runtime-only).
    #[serde(skip)]
    pub tex_anim_clock: u64,
    /// Scale and head-focus a script set per actor (runtime-only).
    #[serde(skip)]
    pub actor_presentation: BTreeMap<EntityId, ActorPresentation>,
    /// The speech-bubble icon `ShowBubbleIcon` raised over the player, if any (runtime-only).
    #[serde(skip)]
    pub bubble_icon: Option<i32>,
    /// Objects carried by others, keyed by the carried object (runtime-only).
    #[serde(skip)]
    pub attachments: BTreeMap<EntityId, Attachment>,
    /// `DisableIdleAnimation` (`SLEEPINH`, `idletimer = -1`): the player no longer plays the
    /// inactive animation (runtime-only).
    #[serde(skip)]
    pub idle_disabled: bool,
    /// `SetHereIcon` mode: whether the "you are here" icon shows (runtime-only).
    #[serde(skip)]
    pub here_icon: i32,
    /// `EIcon.sHereIconTimer`: ticks the here icon still has; it shows from 57 down to 1
    /// (runtime-only).
    #[serde(skip)]
    pub here_icon_timer: i32,
    /// `Obj.buffer` per object (`EBin._instance`): the Instance variables
    /// every function of an object shares (runtime-only).
    #[serde(skip)]
    instance_memory: BTreeMap<EntityId, tantalus_legacy::VariableMemory>,
    /// `SetActiveTimeEventIcon` (`EIcon.sAIconMode`): the ATE prompt's mode (runtime-only).
    #[serde(skip)]
    pub ate_icon_mode: i32,
    /// The fade offset each mode has now, `[add, subtract]` (`SceneDirector.abrColor`)
    /// (runtime-only).
    #[serde(skip)]
    pub fade_current: [[i32; 3]; 2],
    /// `SetTileShadeColor`: the tint of each background overlay, by overlay (runtime-only).
    #[serde(skip)]
    pub tile_shade: BTreeMap<i32, [u8; 3]>,
    /// The latest `ApplyFadeFilter` (runtime-only).
    #[serde(skip)]
    pub fade_filter: Option<FadeFilter>,
    /// The field's SPS effect slots (`FieldSPSSystem`; runtime-only).
    #[serde(skip)]
    pub sps: super::sps::SpsState,
    /// The frame count of each field's SPS files (runtime-only provider).
    #[serde(skip)]
    pub sps_frame_counts: Option<super::sps::SpsFrameCounts>,
    /// Experience-to-stats provider (runtime-only).
    #[serde(skip)]
    pub level_growth: Option<LevelGrowth>,
    /// The experience that reaches each level, level 1 first
    /// (`ff9level.CharacterLevelUps[lv - 1].ExperienceToLevel`; runtime-only provider).
    #[serde(skip)]
    pub level_experience: Option<std::sync::Arc<Vec<u64>>>,
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
            && self.cards == other.cards
            && self.card_records == other.card_records
            && self.card_stats == other.card_stats
            && self.card_stack == other.card_stack
            && self.last_card_result == other.last_card_result
            && self.pending_card_game == other.pending_card_game
            && self.card_seed == other.card_seed
            && self.settings == other.settings
            && self.trance == other.trance
            && self.summons_seen == other.summons_seen
            && self.back_row == other.back_row
            && self.character_stats == other.character_stats
            && self.character_info == other.character_info
            && self.party_reserve == other.party_reserve
            && self.game_over == other.game_over
            && self.disc_change == other.disc_change
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
            && self.hintmap_id == other.hintmap_id
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
            && self.support_enabled == other.support_enabled
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
            character_info: BTreeMap::new(),
            party_reserve: None,
            game_over: false,
            disc_change: None,
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
            choice_mask: -1,
            window_texts: BTreeMap::new(),
            window_speakers: BTreeMap::new(),
            mes_values: BTreeMap::new(),
            sound_events: Vec::new(),
            dialog_signals: BTreeMap::new(),
            pending_signals: std::collections::VecDeque::new(),
            pending_requests: BTreeMap::new(),
            request_functions: BTreeMap::new(),
            object_inits: BTreeSet::new(),
            uncreated: BTreeSet::new(),
            instance_sources: BTreeMap::new(),
            new_instances: Vec::new(),
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
            walk_speed: 30.0,
            animation_default_ticks: 32,
            exit_arrivals: BTreeSet::new(),
            applied_resolutions: BTreeSet::new(),
            atan_table: None,
            navigation: None,
            walk_switches: Vec::new(),
            current_actor: None,
            player: None,
            confirm_mask: None,
            walk_bits: None,
            walk_hold_bit: None,
            run_disabled: false,
            player_facing_polls: 0,
            fmv_frame_counts: None,
            animation_frame_counts: None,
            item_id_map: None,
            character_exp: BTreeMap::new(),
            ability_pa: BTreeMap::new(),
            learned_abilities: BTreeMap::new(),
            equipment: BTreeMap::new(),
            cards: Vec::new(),
            card_records: Vec::new(),
            card_stats: crate::cards::CardCounts::default(),
            card_stack: None,
            last_card_result: 1,
            pending_card_game: None,
            card_seed: 0x9E37_79B9_7F4A_7C15,
            settings: super::settings::GameSettings::default(),
            trance: BTreeMap::new(),
            summons_seen: Vec::new(),
            back_row: std::collections::BTreeSet::new(),
            overlays: BTreeMap::new(),
            tile_animations: Vec::new(),
            tile_camera_count: 0,
            field_camera: 0,
            camera_log: Vec::new(),
            view: super::view::FieldView::default(),
            party_menu: None,
            last_battle: 0,
            battle_result: 0,
            overlay_motion: Vec::new(),
            menu_disabled: false,
            save_point: false,
            save_requested: false,
            chocograph_requested: false,
            world_keys: None,
            hintmap_id: 0,
            shop_requested: None,
            world_codes: Vec::new(),
            support_enabled: BTreeMap::new(),
            actor_shadows: BTreeMap::new(),
            light_changes: Vec::new(),
            tex_anims: BTreeMap::new(),
            tex_anim_clock: 0,
            actor_presentation: BTreeMap::new(),
            bubble_icon: None,
            attachments: BTreeMap::new(),
            idle_disabled: false,
            here_icon: 0,
            here_icon_timer: 0,
            instance_memory: BTreeMap::new(),
            ate_icon_mode: 0,
            fade_current: [[0; 3]; 2],
            tile_shade: BTreeMap::new(),
            sps: super::sps::SpsState::default(),
            sps_frame_counts: None,
            fade_filter: None,
            level_growth: None,
            level_experience: None,
        }
    }

    /// Mark `entity`'s actor as a stub no `InitObject` has created yet: it cannot be talked
    /// to until the object is initialized (`EventCollision.Collision` walks live objects).
    pub fn mark_uncreated(&mut self, entity: EntityId) {
        self.uncreated.insert(entity);
    }

    pub fn add_actor(&mut self, actor: EntityId, position: [f32; 3]) {
        self.actors.insert(actor, ActorState::new(position));
    }

    /// `NEW3` for entry `source` with `uid`: the entry's own object when it
    /// has none yet (or `uid` is already its), a second object running the
    /// same code otherwise. Returns the object.
    pub fn init_object(&mut self, source: u8, uid: u8) -> EntityId {
        let base = EntityId::new(u64::from(source));
        let first = self.object_inits.insert(source);
        self.uncreated.remove(&base);
        let already = self.uid_map.get(&i32::from(uid)).copied();
        if first || already == Some(base) || self.is_object_deleted(base) {
            self.map_uid(i32::from(uid), base);
            self.restore_object(base);
            self.instance_memory.remove(&base);
            return base;
        }
        if let Some(existing) = already {
            return existing;
        }
        let entity =
            EntityId::new(((self.instance_sources.len() as u64 + 1) << 16) | u64::from(source));
        self.add_actor(entity, [0.0; 3]);
        self.map_uid(i32::from(uid), entity);
        self.instance_sources.insert(entity, source);
        self.instance_memory.remove(&entity);
        self.new_instances.push((entity, source));
        entity
    }

    /// The entry an instance object runs, when `entity` is one.
    pub fn instance_source(&self, entity: EntityId) -> Option<u8> {
        self.instance_sources.get(&entity).copied()
    }

    pub fn take_new_instances(&mut self) -> Vec<(EntityId, u8)> {
        std::mem::take(&mut self.new_instances)
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

    /// Drains the sound requests scripts made since the last call.
    pub fn take_sound_events(&mut self) -> Vec<tantalus_legacy::HostEvent> {
        std::mem::take(&mut self.sound_events)
    }

    /// `InitEvents` (EventEngine.cs:556) frees every object of the old
    /// field: its regions, windows, pending requests, shared scripts and
    /// member overrides go with it. Party, items, globals and actors stay.
    pub fn begin_field(&mut self) {
        // `EventEngine` clears `dashinh` when a field starts.
        self.run_disabled = false;
        // `InitEvents` (EventEngine.cs:558): `usercontrol = 0`; the new field's scripts give
        // control back. Kept on, a region that tests `IsMovementEnabled` (system variable 2)
        // under the arrival spot fires before the field's own scene (Gargan Roo 953 from the
        // trolley ride at 4530 left again by its exit region 4).
        self.user_control = false;
        self.actor_presentation.clear();
        self.attachments.clear();
        self.bubble_icon = None;
        self.here_icon = 0;
        self.here_icon_timer = 0;
        self.instance_memory.clear();
        self.ate_icon_mode = 0;
        self.fade_current = [[0; 3]; 2];
        self.tile_shade.clear();
        self.sps = super::sps::SpsState::default();
        self.fade_filter = None;
        self.regions.clear();
        self.windows.clear();
        self.window_texts.clear();
        self.window_speakers.clear();
        self.pending_requests.clear();
        self.request_functions.clear();
        self.choice_mask = -1;
        self.object_inits.clear();
        self.uncreated.clear();
        self.uid_map.clear();
        self.instance_sources.clear();
        self.new_instances.clear();
        self.pending_callers.clear();
        self.shared_scripts.clear();
        self.object_members.clear();
        self.deleted_objects.clear();
        self.actors.clear();
        self.overlays.clear();
        self.walk_switches.clear();
        self.camera_log.clear();
        self.view = super::view::FieldView::default();
        self.overlay_motion.clear();
        self.actor_shadows.clear();
        self.light_changes.clear();
        self.tex_anims.clear();
        self.tex_anim_clock = 0;
        self.save_point = false;
        self.save_requested = false;
        self.chocograph_requested = false;
        self.shop_requested = None;
        self.clear_tile_animations();
        self.control_actor = None;
        self.exit_arrivals.clear();
        self.control_twist = 0.0;
        self.random_battles.init_field();
    }

    /// `NEW` of a deleted object makes it run again.
    /// The object's variable buffer, zeroed on first use (`Obj.Clear`). The
    /// reference sizes it `varn` bytes rounded up plus the script stack; a
    /// full 256 bytes covers every `varn` (a byte) without the stack.
    pub fn instance_memory(&mut self, entity: EntityId) -> &mut tantalus_legacy::VariableMemory {
        self.instance_memory
            .entry(entity)
            .or_insert_with(|| tantalus_legacy::VariableMemory::zeroed(256))
    }

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
                // `MovePC` eases the turn: `Lerp(rotAngle, moveAngle, 0.4)`
                // the short way round, each frame.
                let target = facing_toward_exact(dx, dz);
                let current = f32::from(state.facing).rem_euclid(256.0);
                let turn = (target - current + 128.0).rem_euclid(256.0) - 128.0;
                // The reference turns in floats; on the integer scale the
                // last unit would never close, so it snaps.
                let next = if turn.abs() <= 1.0 {
                    target
                } else {
                    current + turn * 0.4
                };
                state.facing = next.round().rem_euclid(256.0) as i16;
            }
        }
    }

    pub fn set_talk_radius(&mut self, actor: EntityId, radius: u8) {
        if let Some(state) = self.actors.get_mut(&actor) {
            state.talk_radius = radius;
        }
    }

    /// `SetActorRadii` (`DoEventCode.cs:1457-1479`): `collRad` and `talkRad`.
    pub fn set_actor_radii(&mut self, actor: EntityId, collision_radius: u8, talk_radius: u8) {
        if let Some(state) = self.actors.get_mut(&actor) {
            state.collision_radius = collision_radius;
            state.talk_radius = talk_radius;
        }
    }

    /// The actor the controlled `player` pushes this frame: `WalkMesh.Collision` in mode 2
    /// (`WalkMesh.cs:896-957`), which `EventCollision.CollisionRequest` runs every frame the
    /// player has control and then requests the actor's tag 2 (`Request(obj, 1, 2)`,
    /// `EventCollision.cs:245`). Only objects `has_push` says have a tag 2 count. An actor is
    /// skipped when the player's flags have bit 4 and its own flags bit 2; it must be within
    /// 400 in height, 2048 on each axis and `4 * (collRad + player collRad) + speed + 60` in
    /// the plane; the deepest inside wins.
    pub fn push_target(
        &self,
        player: EntityId,
        has_push: impl Fn(EntityId) -> bool,
    ) -> Option<EntityId> {
        let player_state = self.actors.get(&player)?;
        let [px, py, pz] = player_state.position;
        let player_radius = 4.0 * f32::from(player_state.collision_radius);
        let mut best: Option<(f32, EntityId)> = None;
        let mut deepest = 0.0f32;
        for (entity, state) in &self.actors {
            if *entity == player
                || self.uncreated.contains(entity)
                || self.is_object_deleted(*entity)
                || self.regions.contains_key(entity)
                || (player_state.flags & 4 != 0 && state.flags & 2 != 0)
                || !has_push(*entity)
            {
                continue;
            }
            let dy = state.position[1] - py;
            let dx = state.position[0] - px;
            let dz = state.position[2] - pz;
            if dy <= -400.0 || dy >= 400.0 || dx.abs() >= 2048.0 || dz.abs() >= 2048.0 {
                continue;
            }
            let radius = 4.0 * f32::from(state.collision_radius)
                + player_radius
                + f32::from(state.move_speed)
                + 60.0;
            let dif = dx * dx + dz * dz - radius * radius;
            if deepest > dif && (deepest.abs() - dif.abs()).abs() > 1.0 {
                deepest = dif;
                best = Some((dif, *entity));
            }
        }
        best.map(|(_, entity)| entity)
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
                **entity != player && !self.uncreated.contains(entity) && {
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
        self.party_menu = None;
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

    /// `function` runs a request made on `object`.
    pub fn track_request_function(&mut self, function: EntityId, object: EntityId) {
        self.request_functions.insert(function, object);
    }

    /// The requested `function` returned. `EventEngine.Return` (EventEngine.cs:1127-1160) pops
    /// the call at once and restores the object's level, so an object later in the same frame's
    /// `ObjList` pass can request it again (Crystal World 2926: after battle 936 the player's
    /// level-0 function 14 returns, and Kuja's `REQ 2 8 22` in the same frame then gives the
    /// player control). `true` when the request was still pending.
    pub fn return_request_function(&mut self, function: EntityId) -> bool {
        match self.request_functions.remove(&function) {
            Some(object) => {
                self.complete_request(object);
                true
            }
            None => false,
        }
    }

    /// The requested `function` stopped without a `Return` (`DRET`, a dropped entry).
    pub fn untrack_request_function(&mut self, function: EntityId) -> bool {
        self.request_functions.remove(&function).is_some()
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

    /// `Obj.level` (`getvobj` member 6, `EBin.cs`): the level of the
    /// function the object runs. `EventEngine.Call` sets it to the request's
    /// level and `EventEngine.Return` restores the caller's; after the init
    /// function the idle loop runs at `cEventLevelN - 1` = 7
    /// (`EventEngine.Return`, `stateInit`). Field 812 waits for the player's
    /// requested walk to end by polling this above 2.
    pub fn object_level(&self, entity: EntityId) -> i32 {
        self.pending_requests
            .get(&entity)
            .and_then(|levels| levels.iter().min().copied())
            .unwrap_or(IDLE_SCRIPT_LEVEL)
    }

    /// The option a confirm press picks when the script masked the choice
    /// list: an unavailable option cannot be selected, and the cursor
    /// starts on the lowest available one (`DialogBoxSymbols.SetupChoose`:
    /// `DefaultChoice = ActiveIndexes.Min()`).
    pub fn available_choice(&self, answer: i32) -> i32 {
        let mask = self.choice_mask;
        if mask == -1 || !(0..31).contains(&answer) || mask & (1 << answer) != 0 {
            return answer;
        }
        (0..31)
            .find(|option| mask & (1 << option) != 0)
            .unwrap_or(answer)
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

    /// The `RunWorldCode` calls since the last take, as `(code, value)`.
    pub fn take_world_codes(&mut self) -> Vec<(i32, i32)> {
        std::mem::take(&mut self.world_codes)
    }

    pub fn take_world_transitions(&mut self) -> Vec<i16> {
        std::mem::take(&mut self.world_transitions)
    }

    /// A request at `level` or a more urgent one is still running on `entity`.
    pub fn request_pending_at(&self, entity: EntityId, level: i32) -> bool {
        self.pending_requests
            .get(&entity)
            .is_some_and(|levels| levels.iter().any(|pending| *pending <= level))
    }

    pub fn request_pending(&self, entity: EntityId) -> bool {
        self.pending_requests.contains_key(&entity)
    }

    /// Advance movement, turns, and animations by one tick.
    pub fn advance(&mut self) {
        self.apply_walk_input();
        self.apply_dialog_signal();
        let walkable = walkable_mesh(self.navigation.as_ref(), &self.walk_switches);
        let walk_mesh = walkable
            .as_deref()
            .and_then(|mesh| crate::navigation::FieldNavigation::new(mesh).ok());
        let player = self.player;
        // `MoveToward_mixed_ex` snaps an arriving walk onto its target except in field 2204
        // (the frog Cid chase, which waits for Cid's x to read 485 while he hops to 484)
        // and in 2209 at scenario 9850. There the walk below follows the reference step
        // for step; elsewhere it keeps the older straight-line walk.
        let keeps_step = self.field_map_no == 2204
            || (self.field_map_no == 2209
                && self
                    .global_memory
                    .get(0, tantalus_legacy::VariableType::UInt16)
                    == Some(9850));
        let atan_table = self.atan_table.as_deref();
        for (id, state) in self.actors.iter_mut() {
            let walked = state.movement.is_some();
            if let Some(movement) = &state.movement {
                let [dx, dz] = [
                    movement.destination[0] - state.position[0],
                    movement.destination[1] - state.position[2],
                ];
                let distance = f32::sqrt(dx * dx + dz * dz);
                // `MoveToward_mixed_ex` moves `Actor.speed` units a frame (30 by default).
                let walk_speed = self.walk_speed * f32::from(state.move_speed) / 30.0;
                if keeps_step {
                    // `MoveToward_mixed_ex` as the reference runs it: the actor turns toward
                    // the target (`angleAsm`, 0 when on it) by at most `omega` (16, 11.25
                    // degrees) and steps `speed` along its facing as `GetMoveVector`'s
                    // whole-unit vector; a target closer than one step ends the walk where
                    // the step lands.
                    let target = match atan_table {
                        Some(table) => tantalus_legacy::angle_asm(dx as i32, dz as i32, |at| {
                            let at = usize::try_from(at).map_err(|_| ())?;
                            let entry = table.get(at..at + 2).ok_or(())?;
                            Ok::<_, ()>(i32::from(u16::from_le_bytes([entry[0], entry[1]])))
                        })
                        .map_or(0.0, |angle| angle as f32),
                        None if dx == 0.0 && dz == 0.0 => 0.0,
                        None => facing_toward(dx, dz) as f32,
                    } * 360.0
                        / 256.0;
                    let facing = f32::from(state.facing) * 360.0 / 256.0;
                    let turn = (target - facing + 180.0).rem_euclid(360.0) - 180.0;
                    // A walk that keeps its facing (flag 1) moves straight at the target.
                    let angle = if movement.keep_facing {
                        target.to_radians()
                    } else {
                        let angle = (facing + turn.clamp(-11.25, 11.25)).to_radians();
                        state.facing =
                            ((angle.to_degrees() * 256.0 / 360.0).round()).rem_euclid(256.0) as i16;
                        angle
                    };
                    state.position[0] += (-angle.sin() * walk_speed).round();
                    state.position[2] += (-angle.cos() * walk_speed).round();
                    if distance < walk_speed {
                        state.movement = None;
                    }
                } else if distance <= walk_speed || distance == 0.0 {
                    state.position[0] = movement.destination[0];
                    state.position[2] = movement.destination[1];
                    state.movement = None;
                } else {
                    let from = Point3 {
                        x: state.position[0].round() as i32,
                        y: state.position[1] as i32,
                        z: state.position[2].round() as i32,
                    };
                    state.position[0] += dx / distance * walk_speed;
                    state.position[2] += dz / distance * walk_speed;
                    if state.path_active
                        && (Some(*id) != player || state.placed_unchecked)
                        && state.look.model.is_some()
                        && let Some(navigation) = &walk_mesh
                    {
                        {
                            // `UpdateActiveTri`/`ServiceChar`: a pathing actor
                            // takes the triangle height each step, crossing
                            // only linked triangle edges, and a step through
                            // an unlinked edge is clamped back; a clamped
                            // walk that stops closing on its target ends.
                            // The player follows it in a scripted walk from
                            // a script-set height (`POS3`, then
                            // `SetPathing(1)`: map 1823's Garnet walks the
                            // spiral ramp to the gallery, where three NPCs
                            // wait for her height to pass 1400). A player
                            // placed by `POS` still holds the 32768 default
                            // here, which the hosts read as any floor; its
                            // user-control steps are the host's input clamp.
                            let before = distance;
                            let point = Point3 {
                                x: state.position[0].round() as i32,
                                y: state.position[1] as i32,
                                z: state.position[2].round() as i32,
                            };
                            if let Some(near) = navigation.step_along(from, point) {
                                if near.position.x != point.x || near.position.z != point.z {
                                    state.position[0] = near.position.x as f32;
                                    state.position[2] = near.position.z as f32;
                                    let moved = f32::hypot(
                                        movement.destination[0] - state.position[0],
                                        movement.destination[1] - state.position[2],
                                    );
                                    if moved >= before {
                                        state.movement = None;
                                    }
                                }
                                state.position[1] = near.position.y as f32;
                            }
                        }
                    }
                    // Walking faces the movement direction on the reference
                    // facing scale (0 south, 64 west, 128 north, 192 east).
                    if !state
                        .movement
                        .as_ref()
                        .is_some_and(|movement| movement.keep_facing)
                    {
                        state.facing = facing_toward(dx, dz);
                    }
                }
            }
            // `loopCount`: each walk frame takes one off a count other than 0 and 255, and
            // the walk ends on a frame that leaves it at 0 (`syncMove = loopCount != 0`).
            if walked {
                if !matches!(state.walk_loops, 0 | u8::MAX) {
                    state.walk_loops -= 1;
                }
                if state.walk_loops == 0 {
                    state.movement = None;
                }
            }
            // `FieldMapActorController.UpdateMovement` ends every frame of a pathing actor
            // with `UpdateActiveTri`, which rebuilds its position from the triangle in
            // single precision. Kept to the reference-step fields above: the frog Cid
            // chase in 2204 ends only through the float error it leaves.
            if keeps_step
                && state.path_active
                && state.look.model.is_some()
                && let Some(navigation) = &walk_mesh
                && let Some([x, z]) = navigation.rebuild_on_triangle(
                    state.position[0],
                    state.position[1],
                    state.position[2],
                )
            {
                state.position[0] = x;
                state.position[2] = z;
            }
            if let Some(turn) = &mut state.turn {
                turn.remaining_ticks = turn.remaining_ticks.saturating_sub(1);
                if turn.remaining_ticks == 0 {
                    state.turn = None;
                }
            }
        }
    }

    /// `ProcessAnime` for every actor. The reference runs it before the tick's scripts
    /// (`ProcessEvents`), so a script reads the frame the animation reached this tick.
    pub fn advance_animations(&mut self) {
        self.tex_anim_clock += 1;
        self.advance_sps();
        if let Some(fade) = self.fade_filter.as_mut() {
            fade.elapsed = fade.elapsed.saturating_add(1);
            self.fade_current[fade.slot()] = fade.color_now();
        }
        self.here_icon_timer = (self.here_icon_timer - 1).max(0);
        // `TimerUI.Update` while `Play`: the countdown follows real time.
        if self.timer.running {
            self.timer.elapsed_ticks = self.timer.elapsed_ticks.saturating_add(1);
        }
        for state in self.actors.values_mut() {
            if state.anim_frozen {
                // `ProcessAnime` does nothing while `afFreeze` is set.
            } else if state.animation_frames == 0 && state.out_frame == u8::MAX {
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
                    // `AnimStop`: a held animation freezes on its out frame (map 115's
                    // rope climb steps one frame per `ANIM` and reads it back); a looping
                    // one restarts at `inFrame` (until its repeat count runs out); any other
                    // hands over to the idle loop at frame 0 (`DefaultAnim`).
                    let looping = state.anim_loop && {
                        if state.anim_loops == 0 {
                            true
                        } else {
                            state.anim_loops -= 1;
                            state.anim_loops != 0
                        }
                    };
                    if looping {
                        state.animation_frame = u16::from(state.in_frame);
                        state.animation_frames = state.anim_span.max(1);
                    } else if state.anim_hold {
                        state.anim_frozen = true;
                        state.animation_frame = if state.out_frame == u8::MAX {
                            0
                        } else {
                            u16::from(state.out_frame)
                        };
                    } else {
                        state.animation_frame = if state.out_frame == u8::MAX {
                            0
                        } else {
                            u16::from(state.out_frame)
                        };
                        if state.act_jump {
                            // `FinishJump`.
                            state.act_jump = false;
                            state.in_frame = 0;
                            state.out_frame = u8::MAX;
                        }
                    }
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
            timer: self.shown_timer(),
        });
    }

    /// `TimerUI` as a battle starts: only a shown clock reaches the battle scripts.
    pub fn shown_timer(&self) -> Option<TimerState> {
        self.timer.displayed.then_some(self.timer)
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

    /// Sets an engine system variable the scripts read (`GetSysvar`); the
    /// world map feeds its own (`ff9.w_frameGetParameter`, 192 and up).
    pub fn set_system_variable(&mut self, variable: u8, value: i32) {
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

    /// An object named by a script operand: a uid resolves through the party slots and the
    /// uid map, anything unknown is taken as the entity itself.
    pub(crate) fn resolve_object(&self, object: EntityId) -> EntityId {
        i32::try_from(object.get())
            .ok()
            .and_then(|uid| self.resolve_uid(uid))
            .unwrap_or(object)
    }

    /// `GetUIDFromLocalUID` then `FindObjByUID`. A party slot names the
    /// object of its `partyUID` entry; an empty slot keeps the plain uid.
    pub fn resolve_uid(&self, uid: i32) -> Option<EntityId> {
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
        // `EventState.IsEikoAbducted` reads the scenario counter (`EBin.SC_COUNTER_SVR`).
        let scenario = self
            .global_memory
            .get(0, tantalus_legacy::VariableType::UInt16)
            .unwrap_or(0);
        let avoid_eiko = tantalus_legacy::event::party::is_eiko_abducted(scenario);
        self.party_uid = party_uid(&self.entry_links, &party, avoid_eiko);
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

    /// `max.hp` / `max.mp` of `character` rebuilt from the level's basis
    /// (the level-growth provider at the character's experience) and the
    /// enabled HP+/MP+ support abilities ([`support_max_hp_mp`],
    /// `FF9Play_Update`); current HP and MP are capped at the new maximums.
    /// Without a growth provider or stats nothing changes. Run after the
    /// enabled support abilities change (`FF9Abil_SetEnableSA` ->
    /// `FF9Play_Update`).
    pub fn update_character_max(&mut self, character: i32) {
        let Some(growth) = self.level_growth.clone() else {
            return;
        };
        let exp = self.character_exp.get(&character).copied().unwrap_or(0);
        let Some(basis) = growth(character, exp) else {
            return;
        };
        let enabled = self
            .support_enabled
            .get(&character)
            .cloned()
            .unwrap_or_default();
        let (max_hp, max_mp) = support_max_hp_mp(basis.max_hp, basis.max_mp, &enabled);
        if let Some(stats) = self.character_stats.get_mut(&character) {
            stats.max_hp = max_hp;
            stats.max_mp = max_mp;
            stats.current_hp = stats.current_hp.min(max_hp);
            stats.current_mp = stats.current_mp.min(max_mp);
        }
    }

    /// Award experience to a character and recompute stats through the
    /// level-growth provider. On level-up the max HP/MP delta is added to
    /// current values (reference level-up behavior). The maximums include
    /// the enabled HP+/MP+ abilities ([`support_max_hp_mp`]).
    pub fn apply_experience(&mut self, character: i32, experience: u64) {
        let total = self.character_exp.entry(character).or_insert(0);
        *total = total.saturating_add(experience);
        let Some(growth) = self.level_growth.clone() else {
            return;
        };
        let Some(mut next) = growth(character, *self.character_exp.get(&character).unwrap()) else {
            return;
        };
        if let Some(enabled) = self.support_enabled.get(&character) {
            (next.max_hp, next.max_mp) = support_max_hp_mp(next.max_hp, next.max_mp, enabled);
            next.current_hp = next.current_hp.min(next.max_hp);
            next.current_mp = next.current_mp.min(next.max_mp);
        }
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

    /// A copy fit for a save file: the static tables a field or the game
    /// provides again (atan table, navigation, tile animations) are left out.
    pub fn for_save(&self) -> FieldWorld {
        let mut world = self.clone();
        world.atan_table = None;
        world.navigation = None;
        world.tile_animations.clear();
        world
    }

    /// Takes the runtime providers (atan table, level growth, FMV counts,
    /// item id map) from `template`, the new-game world, after a load.
    pub fn copy_providers_from(&mut self, template: &FieldWorld) {
        self.atan_table = template.atan_table.clone();
        self.level_growth = template.level_growth.clone();
        self.level_experience = template.level_experience.clone();
        self.fmv_frame_counts = template.fmv_frame_counts.clone();
        self.animation_frame_counts = template.animation_frame_counts.clone();
        self.sps_frame_counts = template.sps_frame_counts.clone();
        self.item_id_map = template.item_id_map.clone();
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

    /// `BG_init` for the field's cameras (`BGSCENE_DEF.cameraList`): the view the scripts read
    /// back starts centered in camera 0's range.
    pub fn provide_cameras(&mut self, cameras: Vec<tantalus_contracts::LegacyFieldCamera>) {
        self.view.reset(cameras);
    }

    /// `FieldMap.HonoLateUpdate`'s scroll services, once per field tick.
    pub fn service_view(&mut self) {
        let player = self.view_player();
        self.view.service(player);
    }

    /// `FieldMap.playerController.curPos`: the actor `DefinePlayerControl` named.
    pub(crate) fn view_player(&self) -> Option<[f32; 3]> {
        self.control_actor
            .and_then(|actor| self.actors.get(&actor))
            .map(|state| state.position)
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

    /// The walkmesh an actor can walk on: [`Self::navigation`] without the triangles the
    /// field scripts switched off. `WalkMesh.BGI_floorSetActive` and `BGI_triSetActive`
    /// clear bit 1 of `triFlags`, and the walk code skips such triangles
    /// (`BGI_findAccessibleTriangle`, `FieldMapActorController` neighbours with
    /// `triFlags == 0`, `GetTriangleIndexAtScreenPos`). Borrowed when nothing is off.
    pub fn walkable_navigation(&self) -> Option<std::borrow::Cow<'_, FieldNavigationMesh>> {
        walkable_mesh(self.navigation.as_ref(), &self.walk_switches)
    }

    /// `SetPathFloorActive` (`EventEngine.DoEventCode.cs` `BGI_floorSetActive`): every
    /// triangle of `floor`.
    pub(crate) fn set_floor_active(&mut self, floor: i32, active: bool) {
        self.walk_switches.push(WalkSwitch::Floor { floor, active });
    }

    /// `SetTriangleActive` (`BGI_triSetActive`): one triangle by index.
    pub(crate) fn set_triangle_active(&mut self, triangle: i32, active: bool) {
        self.walk_switches
            .push(WalkSwitch::Triangle { triangle, active });
    }

    /// `FieldMapActorController.SetPosition(pos, true, true)`: an actor with
    /// pathing on (`charFlags & 1`) stands on the walkmesh at its position
    /// (height from the triangle), or at the center of triangle 0 when no
    /// triangle holds it. No mesh or pathing off leaves the position alone.
    pub(crate) fn snap_actor_to_walkmesh(&mut self, actor: EntityId) {
        // The controlled actor keeps the height the scripts gave it: the
        // chain tool's walkmesh walk treats a height of 32768 as "unset" and
        // picks its start floor from it, and a snapped height changes the
        // route (map 153).
        if self.player == Some(actor) {
            return;
        }
        let Some(mesh) = self.navigation.as_ref() else {
            return;
        };
        let Some(state) = self.actors.get(&actor) else {
            return;
        };
        if !state.path_active || state.look.model.is_none() {
            return;
        }
        let Ok(navigation) = crate::navigation::FieldNavigation::new(mesh) else {
            return;
        };
        let [x, y, z] = state.position;
        let point = Point3 {
            x: x.round() as i32,
            y: y.clamp(i32::MIN as f32, i32::MAX as f32) as i32,
            z: z.round() as i32,
        };
        let Some(snapped) = navigation.snap_to_walkmesh(point) else {
            return;
        };
        let inside = snapped.position.x == point.x && snapped.position.z == point.z;
        if let Some(state) = self.actors.get_mut(&actor) {
            state.placed_unchecked = false;
            if inside {
                state.position[1] = snapped.position.y as f32;
            } else {
                state.position = [
                    snapped.position.x as f32,
                    snapped.position.y as f32,
                    snapped.position.z as f32,
                ];
            }
        }
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

    /// The walkmesh floor under `position`: the floor of the triangle that holds it, over every
    /// floor (nearest in height when several do), as `FieldMapActorController.activeFloor` is
    /// for an actor standing there (`BGI.BGI_charGetInfo`).
    pub(crate) fn navigation_floor_at(&self, position: [f32; 3]) -> Option<i16> {
        let mesh = self.navigation.as_ref()?;
        let navigation = crate::navigation::FieldNavigation::new(mesh).ok()?;
        let projection = navigation.snap_to_walkmesh(Point3 {
            x: position[0].round() as i32,
            y: position[1].round() as i32,
            z: position[2].round() as i32,
        })?;
        mesh.triangles
            .get(projection.triangle_index)
            .map(|triangle| triangle.floor)
    }
}

/// One script switch of walkmesh triangles (`FieldWorld::walk_switches`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WalkSwitch {
    Floor { floor: i32, active: bool },
    Triangle { triangle: i32, active: bool },
}

/// `mesh` without the triangles that `switches`, applied in order over an all-active
/// walkmesh, leave off; the same mesh when none is off.
fn walkable_mesh<'a>(
    mesh: Option<&'a FieldNavigationMesh>,
    switches: &[WalkSwitch],
) -> Option<std::borrow::Cow<'a, FieldNavigationMesh>> {
    let mesh = mesh?;
    let active = |index: usize, floor: i16| {
        switches.iter().fold(true, |active, switch| match *switch {
            WalkSwitch::Floor {
                floor: f,
                active: a,
            } if f == i32::from(floor) => a,
            WalkSwitch::Triangle {
                triangle,
                active: a,
            } if usize::try_from(triangle).is_ok_and(|t| t == index) => a,
            _ => active,
        })
    };
    if switches.is_empty()
        || mesh
            .triangles
            .iter()
            .enumerate()
            .all(|(index, triangle)| active(index, triangle.floor))
    {
        return Some(std::borrow::Cow::Borrowed(mesh));
    }
    let mut walkable = mesh.clone();
    walkable.triangles = mesh
        .triangles
        .iter()
        .enumerate()
        .filter(|(index, triangle)| active(*index, triangle.floor))
        .map(|(_, triangle)| triangle.clone())
        .collect();
    Some(std::borrow::Cow::Owned(walkable))
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

#[cfg(test)]
mod timer_tests {
    use super::TimerState;

    #[test]
    fn timer_counts_down_while_running_and_rounds_like_convert_to_int32() {
        let mut timer = TimerState::default();
        timer.set_time(61);
        // `SetTime` stores 60.99, which `Convert.ToInt32` reads as 61.
        assert_eq!(timer.time_int(), 61);
        timer.elapsed_ticks = 30;
        assert_eq!(timer.time_int(), 60);
        timer.elapsed_ticks = 61 * 30;
        assert_eq!(timer.time_int(), 0);
        timer.elapsed_ticks = 10_000;
        assert_eq!(timer.time(), 0.0);
    }
}

#[cfg(test)]
mod instance_memory_tests {
    use super::FieldWorld;
    use tantalus_contracts::EntityId;
    use tantalus_legacy::VariableType;

    #[test]
    fn instance_memory_is_per_object_and_zeroed_on_first_use() {
        let mut world = FieldWorld::new();
        let a = EntityId::new(1);
        let b = EntityId::new(2);
        assert_eq!(world.instance_memory(a).get(4, VariableType::Byte), Some(0));
        world.instance_memory(a).set(4, VariableType::Byte, 9);
        // Same object (another function or request) sees the value.
        assert_eq!(world.instance_memory(a).get(4, VariableType::Byte), Some(9));
        // Another object does not.
        assert_eq!(world.instance_memory(b).get(4, VariableType::Byte), Some(0));
    }
}

#[cfg(test)]
mod facing_tests {
    use super::{FieldWorld, facing_toward};
    use tantalus_contracts::EntityId;

    /// Pad input faces the movement direction (`FieldMapActorController`
    /// `MovePC`: `atan2(-moveX, -moveZ)`), up +z -> 128, down -z -> 0.
    #[test]
    fn pad_input_faces_the_movement_direction() {
        for (mask, expected) in [(1, 128), (2, 0), (4, 64), (8, 192)] {
            let mut world = FieldWorld::new();
            let actor = EntityId::new(1);
            world.add_actor(actor, [0.0, 0.0, 0.0]);
            world.set_player(actor);
            world.user_control = true;
            world.set_walk_bits(Some((1, 2, 4, 8)));
            world.set_input(mask);
            for _ in 0..40 {
                world.apply_walk_input();
            }
            let state = &world.actors[&actor];
            assert_eq!(state.facing, expected, "mask {mask}");
            let (x, z) = (state.position[0], state.position[2]);
            let moved = facing_toward(x, z);
            assert_eq!(moved, expected, "movement ({x}, {z}) vs facing");
        }
    }

    #[test]
    fn facing_matches_movement_direction() {
        // Reference scale: 0 south (-z), 64 west (-x), 128 north (+z), 192 east (+x).
        assert_eq!(facing_toward(0.0, -1.0), 0);
        assert_eq!(facing_toward(-1.0, 0.0), 64);
        assert_eq!(facing_toward(0.0, 1.0), 128);
        assert_eq!(facing_toward(1.0, 0.0), 192);
    }
}
